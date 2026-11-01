//! GATT server for the phone (master only).
//!
//! Service `7a1e0001-…`:
//! - `7a1e0002-…` command: write, text commands (see `wink_core::command`).
//!   Requires an encrypted, authenticated (passkey) link from a bonded peer.
//! - `7a1e0004-…` link key: write 32 bytes (coupling; see `coupling.rs`). Same
//!   protection as the command characteristic.
//! - `7a1e0003-…` state: read + notify, text: `asleep`, `idle`, `wink`,
//!   `alternating`, `resting`, `overvoltage`, `undervoltage`, plus replies
//!   `error: <reason>` (`error: resting <s>` with the seconds left),
//!   `remote: <status>` and `version: <x.y.z>`. STATUS sends the state, then
//!   the remote link state and the version. Reading and subscribing also
//!   require an authenticated link.

use std::sync::mpsc::SyncSender;
use std::sync::Arc;

use esp32_nimble::enums::{AuthReq, SecurityIOCap};
use esp32_nimble::utilities::mutex::Mutex;
use esp32_nimble::utilities::BleUuid;
use esp32_nimble::{uuid128, BLEAdvertisementData, BLECharacteristic, BLEDevice, NimbleProperties};
use log::{info, warn};
use wink_core::proto::KEY_LEN;

use crate::hw::Side;
use crate::secrets::BLE_PASSKEY;
use crate::Event;

pub const SERVICE_UUID: BleUuid = uuid128!("7a1e0001-5c0b-4e57-9b1a-70726f626577");
pub const COMMAND_UUID: BleUuid = uuid128!("7a1e0002-5c0b-4e57-9b1a-70726f626577");
pub const STATE_UUID: BleUuid = uuid128!("7a1e0003-5c0b-4e57-9b1a-70726f626577");
pub const KEY_UUID: BleUuid = uuid128!("7a1e0004-5c0b-4e57-9b1a-70726f626577");

/// ATT error "Insufficient Authentication".
const ATT_ERR_INSUFFICIENT_AUTHEN: u8 = 0x05;

pub struct Phone {
    state: Arc<Mutex<BLECharacteristic>>,
    current: &'static str,
}

/// Configure security (shared with the remote client), the GATT service and
/// advertising. Must run before the remote thread starts.
pub fn start(events: SyncSender<Event>, side: Side) -> anyhow::Result<Phone> {
    let device = BLEDevice::take();
    // Names by side, so the phone can tell the two modules apart.
    let (gap_name, adv_name) = match side {
        Side::Lh => ("ProbeWink LH", "PWink-L"),
        Side::Rh => ("ProbeWink RH", "PWink-R"),
    };
    BLEDevice::set_device_name(gap_name)?;
    device
        .security()
        // Bonding + MITM (passkey) + LE Secure Connections.
        .set_auth(AuthReq::all())
        .set_passkey(BLE_PASSKEY)
        // We show nothing; the phone enters the fixed passkey. For the
        // remote (NoInputNoOutput) this falls back to Just Works.
        .set_io_cap(SecurityIOCap::DisplayOnly)
        .resolve_rpa();

    let server = device.get_server();
    server.on_connect(|_server, desc| {
        info!("phone connected: {:?} bonded={}", desc.address(), desc.bonded());
    });
    server.on_disconnect(|desc, reason| {
        info!("phone disconnected: {:?} ({reason:?})", desc.address());
    });
    server.on_authentication_complete(|_server, desc, result| {
        info!(
            "pairing with {:?}: {result:?} encrypted={} authenticated={} bonded={}",
            desc.address(),
            desc.encrypted(),
            desc.authenticated(),
            desc.bonded()
        );
    });

    let service = server.create_service(SERVICE_UUID);
    let command = service.lock().create_characteristic(
        COMMAND_UUID,
        NimbleProperties::WRITE | NimbleProperties::WRITE_ENC | NimbleProperties::WRITE_AUTHEN,
    );
    let state = service.lock().create_characteristic(
        STATE_UUID,
        NimbleProperties::READ
            | NimbleProperties::READ_ENC
            | NimbleProperties::READ_AUTHEN
            | NimbleProperties::NOTIFY
            | NimbleProperties::NOTIFY_INDICATE_AUTHEN,
    );
    state.lock().set_value(b"idle");

    let events_cmd = events.clone();
    command.lock().on_write(move |args| {
        let desc = args.desc();
        // The ATT permissions already demand an authenticated link; this is
        // the explicit "bonded phones only" check on top.
        if !(desc.encrypted() && desc.authenticated() && desc.bonded()) {
            warn!("command from unbonded/unauthenticated peer {:?} rejected", desc.address());
            args.reject_with_error_code(ATT_ERR_INSUFFICIENT_AUTHEN);
            return;
        }
        let data = args.recv_data().to_vec();
        if events_cmd.try_send(Event::PhoneWrite(data)).is_err() {
            warn!("event queue full, phone command dropped");
        }
    });

    let key = service.lock().create_characteristic(
        KEY_UUID,
        NimbleProperties::WRITE | NimbleProperties::WRITE_ENC | NimbleProperties::WRITE_AUTHEN,
    );
    let key_events = events.clone();
    key.lock().on_write(move |args| {
        let desc = args.desc();
        if !(desc.encrypted() && desc.authenticated() && desc.bonded()) {
            warn!("link key from unbonded/unauthenticated peer {:?} rejected", desc.address());
            args.reject_with_error_code(ATT_ERR_INSUFFICIENT_AUTHEN);
            return;
        }
        let data = args.recv_data();
        let Ok(k) = <[u8; KEY_LEN]>::try_from(data) else {
            warn!("link key write of {} bytes ignored (need {KEY_LEN})", data.len());
            return;
        };
        if key_events.try_send(Event::LinkKey(k)).is_err() {
            warn!("event queue full, link key dropped");
        }
    });

    let advertising = device.get_advertising();
    // ESP32-C3: advertising stops after a bond is created
    // (https://github.com/taks/esp32-nimble/issues/70); restart it.
    advertising.lock().on_complete(|_| {
        if let Err(e) = BLEDevice::take().get_advertising().lock().start() {
            warn!("advertising restart failed: {e:?}");
        }
    });
    advertising.lock().set_data(
        BLEAdvertisementData::new().name(adv_name).add_service_uuid(SERVICE_UUID),
    )?;
    advertising.lock().start()?;
    info!("phone service advertising; bonds: {:?}", device.bonded_addresses());

    Ok(Phone { state, current: "idle" })
}

impl Phone {
    /// Set the state (readable value) and notify subscribers.
    pub fn set_state(&mut self, state: &'static str) {
        self.current = state;
        let mut c = self.state.lock();
        c.set_value(state.as_bytes());
        c.notify();
    }

    /// Notify a reply (error, remote status) without changing the readable
    /// state value.
    pub fn reply(&self, text: &str) {
        let mut c = self.state.lock();
        c.set_value(text.as_bytes());
        c.notify();
        c.set_value(self.current.as_bytes());
    }
}
