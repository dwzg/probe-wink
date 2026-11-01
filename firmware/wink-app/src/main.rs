//! probe-wink firmware: one binary for both modules. GPIO6 selects the role
//! at boot: open (high) = LH master, bridged to GND (low) = RH slave.

use std::sync::mpsc::sync_channel;
use std::sync::Arc;

use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::hal::peripherals::Peripherals;
use esp_idf_svc::hal::task::watchdog::{TWDTConfig, TWDTDriver};
use esp_idf_svc::nvs::EspDefaultNvsPartition;
use log::info;
use wink_core::proto::{Counter, Role};
use wink_core::remote::Edge;
use wink_core::Millis;

mod ble_phone;
mod ble_remote;
mod coupling;
mod hw;
mod link;
mod master;
mod ota;
mod secrets;
mod sleep;
mod slave;
mod store;

/// Everything the control loop reacts to. Produced by BLE / Wi-Fi callbacks
/// and the remote thread; consumed only by the control loop.
pub enum Event {
    /// Raw write to the phone command characteristic (authenticated peer).
    PhoneWrite(Vec<u8>),
    /// Button edge from the remote, with its timestamp.
    RemoteEdge(Edge, Millis),
    /// Remote connected / disconnected.
    RemoteLink(bool),
    /// Remote pairing progress, forwarded to the phone.
    RemoteInfo(&'static str),
    /// ESP-NOW frame (not yet authenticated).
    Radio(Vec<u8>),
    /// New link key written by the phone (coupling).
    LinkKey(wink_core::proto::Key),
}

/// Milliseconds since boot.
pub fn millis() -> Millis {
    (unsafe { esp_idf_svc::sys::esp_timer_get_time() } / 1000) as Millis
}

fn main() -> anyhow::Result<()> {
    // Relay off before anything else.
    hw::relay_off_early();
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();
    info!("probe-wink {}", env!("CARGO_PKG_VERSION"));
    sleep::log_wake_cause();

    let p = Peripherals::take()?;
    // Pin map per chip; see hw.rs.
    #[cfg(not(esp32))]
    let (side_pin, up, down, relay, led, supply) =
        (p.pins.gpio6.into(), p.pins.gpio0.into(), p.pins.gpio1.into(), p.pins.gpio10.into(), p.pins.gpio7.into(),
         (p.adc1, p.pins.gpio3));
    #[cfg(esp32)]
    let (side_pin, up, down, relay, led, supply) =
        (p.pins.gpio27.into(), p.pins.gpio32.into(), p.pins.gpio33.into(), p.pins.gpio2.into(), p.pins.gpio4.into(),
         ());
    let side = hw::read_side(side_pin)?;
    info!("side: {side:?}");

    let mut board = hw::Board::new(up, down, relay, led, supply)?;
    board.set_relay(false);

    // Task watchdog on the control loop (timeout from sdkconfig, panics and
    // resets on expiry; the relay drops with the reset).
    let mut twdt = TWDTDriver::new(p.twdt, &TWDTConfig::default())?;
    let mut wdt = twdt.watch_current_task()?;

    let sysloop = EspSystemEventLoop::take()?;
    let nvs = EspDefaultNvsPartition::take()?;
    let mut store = store::Store::new(nvs.clone())?;
    let (tx, rx) = sync_channel::<Event>(32);
    let key = store.link_key();
    info!("{}", coupling::link_line(key.as_ref()));

    match side {
        hw::Side::Lh => {
            let counter = Counter::new(Role::Master, store.next_tx_epoch());
            let link = link::Link::start(p.modem, sysloop, nvs.clone(), counter, key, tx.clone())?;
            let phone = ble_phone::start(tx.clone(), side)?;
            let remote = Arc::new(ble_remote::RemoteCtl::default());
            ble_remote::spawn(remote.clone(), nvs, tx)?;
            master::Master { board, link, phone, remote, store, events: rx }.run(&mut wdt)
        }
        hw::Side::Rh => {
            let counter = Counter::new(Role::Slave, store.next_tx_epoch());
            let link = link::Link::start(p.modem, sysloop, nvs, counter, key, tx.clone())?;
            let phone = ble_phone::start(tx, side)?;
            slave::Slave { board, link, phone, store, events: rx }.run(&mut wdt)
        }
    }
}
