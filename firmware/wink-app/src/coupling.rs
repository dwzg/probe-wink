//! Coupling the two modules: the phone writes the same random link key to both
//! (key characteristic, bonded + authenticated link only). The key lives in NVS;
//! without one the module is "not coupled" and its radio link stays silent.

use log::{info, warn};
use wink_core::proto::{fingerprint, key_is_trivial, Key};

use crate::ble_phone::Phone;
use crate::hw::Side;
use crate::store::Store;

/// `link: coupled 1a2b3c4d` or `link: not coupled`. The fingerprint lets the phone
/// compare both modules' keys without reading them.
pub fn link_line(key: Option<&Key>) -> String {
    match key {
        Some(k) => {
            let f = fingerprint(k);
            format!("link: coupled {:02x}{:02x}{:02x}{:02x}", f[0], f[1], f[2], f[3])
        }
        None => "link: not coupled".into(),
    }
}

pub fn side_line(side: Side) -> &'static str {
    match side {
        Side::Lh => "side: lh",
        Side::Rh => "side: rh",
    }
}

/// Store a new link key and restart, so the radio link comes up with it. The
/// replay counters start over: frames under the old key are meaningless now.
pub fn apply_key(store: &mut Store, phone: &Phone, key: Key) {
    if key_is_trivial(&key) {
        warn!("link key rejected: trivial");
        phone.reply("error: bad key");
        return;
    }
    store.set_link_key(Some(key));
    store.set_rx_epoch(0);
    info!("link key stored ({}); restarting", link_line(Some(&key)));
    phone.reply(&link_line(Some(&key)));
    restart();
}

/// Erase the link key and restart.
pub fn uncouple(store: &mut Store, phone: &Phone) {
    store.set_link_key(None);
    store.set_rx_epoch(0);
    info!("link key erased; restarting");
    phone.reply("link: not coupled");
    restart();
}

fn restart() {
    // Let the notification go out first.
    std::thread::sleep(std::time::Duration::from_millis(400));
    unsafe { esp_idf_svc::sys::esp_restart() }
}
