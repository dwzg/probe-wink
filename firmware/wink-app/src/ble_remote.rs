//! BLE HID client for a camera-shutter remote (master only).
//!
//! Runs in its own thread with a small async executor. The master is the
//! GATT client / central here while staying a peripheral for the phone.
//!
//! - Pairing (`PAIR REMOTE`): scan up to 30 s for an advertiser that lists
//!   the HID service (0x1812), connect, pair + bond (Just Works: the remote
//!   has no IO), subscribe, and store its address in NVS.
//! - Normal operation: keep trying to connect to the stored address
//!   (remotes usually advertise only for a while after a button press),
//!   re-encrypt with the stored bond, subscribe to every input report
//!   (0x2A4D with notify) and turn reports into press / release edges.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::SyncSender;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use esp32_nimble::utilities::BleUuid;
use esp32_nimble::{BLEAddress, BLEAddressType, BLEClient, BLEDevice, BLEError, BLEScan};
use esp_idf_svc::hal::task::block_on;
use esp_idf_svc::nvs::EspDefaultNvsPartition;
use esp_idf_svc::sys;
use log::{info, warn};
use wink_core::remote::HidButton;

use crate::store::Store;
use crate::{millis, Event};

const HID_SERVICE: BleUuid = BleUuid::Uuid16(0x1812);
const HID_REPORT: BleUuid = BleUuid::Uuid16(0x2A4D);
const PAIR_SCAN_MS: i32 = 30_000;

/// Requests from the control loop to the remote thread.
#[derive(Default)]
pub struct RemoteCtl {
    pair: AtomicBool,
    forget: AtomicBool,
    pairing: AtomicBool,
}

impl RemoteCtl {
    pub fn request_pair(&self) {
        self.pair.store(true, Ordering::SeqCst);
        cancel_pending_connect();
    }

    pub fn request_forget(&self) {
        self.forget.store(true, Ordering::SeqCst);
        cancel_pending_connect();
    }

    /// Pairing scan in progress (keeps the module awake).
    pub fn pairing(&self) -> bool {
        self.pairing.load(Ordering::SeqCst) || self.pair.load(Ordering::SeqCst)
    }

    fn pending(&self) -> bool {
        self.pair.load(Ordering::SeqCst) || self.forget.load(Ordering::SeqCst)
    }
}

/// Abort a connection attempt to the stored remote so a pair / forget
/// request does not wait for the 30 s connect timeout.
fn cancel_pending_connect() {
    // Returns BLE_HS_EALREADY if no connect is pending; that is fine.
    unsafe {
        sys::ble_gap_conn_cancel();
    }
}

pub fn spawn(
    ctl: Arc<RemoteCtl>,
    nvs: EspDefaultNvsPartition,
    events: SyncSender<Event>,
) -> std::io::Result<()> {
    std::thread::Builder::new()
        .name("remote".into())
        .stack_size(16 * 1024)
        .spawn(move || {
            let mut store = match Store::new(nvs) {
                Ok(s) => s,
                Err(e) => {
                    warn!("remote: NVS unavailable ({e}), remote disabled");
                    return;
                }
            };
            block_on(run(ctl, &mut store, events));
        })
        .map(|_| ())
}

fn decode_addr(b: [u8; 7]) -> Option<BLEAddress> {
    let ty = BLEAddressType::try_from(b[6]).ok()?;
    let mut a = [0u8; 6];
    a.copy_from_slice(&b[..6]);
    Some(BLEAddress::from_le_bytes(a, ty))
}

fn encode_addr(a: &BLEAddress) -> [u8; 7] {
    let mut b = [0u8; 7];
    b[..6].copy_from_slice(&a.as_le_bytes());
    b[6] = a.addr_type() as u8;
    b
}

fn say(events: &SyncSender<Event>, text: &'static str) {
    let _ = events.try_send(Event::RemoteInfo(text));
}

async fn run(ctl: Arc<RemoteCtl>, store: &mut Store, events: SyncSender<Event>) {
    let device = BLEDevice::take();
    let mut stored = store.remote().and_then(decode_addr);
    info!("remote: stored = {:?}", stored);

    loop {
        if ctl.forget.swap(false, Ordering::SeqCst) {
            if let Some(a) = stored.take() {
                let _ = device.delete_bond(&a);
            }
            store.set_remote(None);
            say(&events, "remote: forgotten");
            continue;
        }

        if ctl.pair.load(Ordering::SeqCst) {
            ctl.pairing.store(true, Ordering::SeqCst);
            ctl.pair.store(false, Ordering::SeqCst);
            say(&events, "remote: pairing");
            match pair(device, &ctl, store, &mut stored, &events).await {
                Ok(()) => {}
                Err(e) => {
                    warn!("remote: pairing failed: {e:?}");
                    say(&events, "remote: pairing failed");
                }
            }
            ctl.pairing.store(false, Ordering::SeqCst);
            continue;
        }

        match stored {
            Some(addr) => {
                let mut client = device.new_client();
                match client.connect(&addr).await {
                    Ok(()) => {
                        match setup(&mut client, &events).await {
                            Ok(button) => hold(&client, &ctl, &events, button),
                            Err(e) => warn!("remote: setup failed: {e:?}"),
                        }
                        let _ = client.disconnect();
                    }
                    // Not in range / not advertising / cancelled. Retry.
                    Err(_) => std::thread::sleep(Duration::from_millis(200)),
                }
            }
            None => std::thread::sleep(Duration::from_millis(300)),
        }
    }
}

async fn pair(
    device: &BLEDevice,
    ctl: &RemoteCtl,
    store: &mut Store,
    stored: &mut Option<BLEAddress>,
    events: &SyncSender<Event>,
) -> Result<(), BLEError> {
    let mut scan = BLEScan::new();
    scan.active_scan(true).interval(100).window(90);
    let found = scan
        .start(device, PAIR_SCAN_MS, |dev, data| {
            data.is_advertising_service(&HID_SERVICE).then(|| dev.addr())
        })
        .await?;
    let Some(addr) = found else {
        say(events, "remote: none found");
        return Ok(());
    };
    info!("remote: found HID device {addr:?}");

    // Replace any earlier remote.
    if let Some(old) = stored.take() {
        let _ = device.delete_bond(&old);
        store.set_remote(None);
    }

    let mut client = device.new_client();
    client.connect(&addr).await?;
    let button = match setup(&mut client, events).await {
        Ok(b) => b,
        Err(e) => {
            let _ = client.disconnect();
            let _ = device.delete_bond(&addr);
            return Err(e);
        }
    };
    *stored = Some(addr);
    store.set_remote(Some(encode_addr(&addr)));
    ctl.pairing.store(false, Ordering::SeqCst);
    say(events, "remote: paired");
    hold(&client, ctl, events, button);
    let _ = client.disconnect();
    Ok(())
}

/// Secure the link (re-encrypt with the bond, or pair and bond if new) and
/// subscribe to every notifying input report.
async fn setup(
    client: &mut BLEClient,
    events: &SyncSender<Event>,
) -> Result<Arc<Mutex<HidButton>>, BLEError> {
    client.secure_connection().await?;

    let button = Arc::new(Mutex::new(HidButton::new()));
    let mut reports = 0usize;
    let service = client.get_service(HID_SERVICE).await?;
    for chr in service.get_characteristics().await? {
        if chr.uuid() != HID_REPORT || !chr.can_notify() {
            continue;
        }
        let idx = reports;
        let button = button.clone();
        let events = events.clone();
        chr.on_notify(move |data| {
            let edge = button.lock().ok().and_then(|mut b| b.report(idx, data));
            if let Some(edge) = edge {
                let _ = events.try_send(Event::RemoteEdge(edge, millis()));
            }
        });
        chr.subscribe_notify(false).await?;
        reports += 1;
    }
    if reports == 0 {
        warn!("remote: no notifying input report found");
        return BLEError::fail().map(|_| button);
    }
    info!("remote: connected, {reports} input report(s)");
    Ok(button)
}

/// Forward button edges (from the notify callbacks) until the link drops or
/// the control loop asks for pair / forget.
fn hold(client: &BLEClient, ctl: &RemoteCtl, events: &SyncSender<Event>, button: Arc<Mutex<HidButton>>) {
    let _ = events.try_send(Event::RemoteLink(true));
    while client.connected() && !ctl.pending() {
        std::thread::sleep(Duration::from_millis(100));
    }
    if let Some(edge) = button.lock().ok().and_then(|mut b| b.reset()) {
        let _ = events.try_send(Event::RemoteEdge(edge, millis()));
    }
    let _ = events.try_send(Event::RemoteLink(false));
}
