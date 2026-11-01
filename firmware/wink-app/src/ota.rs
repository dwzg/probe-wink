//! Over-the-air updates. NOT IMPLEMENTED in v1.
//!
//! TODO(ota): Wi-Fi OTA for both modules (they end up buried behind the
//! headlamps). Open points:
//! - Partition table with two OTA slots (`partitions.csv` is already laid out
//!   that way, so v1 boards can be updated without reflashing the table).
//! - How to enter update mode: a phone command (e.g. `OTA <ssid>`) on the
//!   master, forwarded to the slave over ESP-NOW (authenticated).
//! - Joining an AP changes the Wi-Fi channel; ESP-NOW uses a fixed channel,
//!   so the link is down during an update. Relays must be off and stay off.
//! - Image authenticity: sign images (ESP-IDF secure boot v2 or an app-level
//!   signature check) so only our firmware is accepted.
//! - Rollback: `CONFIG_BOOTLOADER_APP_ROLLBACK_ENABLE` and mark the new image
//!   valid only after the control loop has run.

/// Placeholder so the call site exists. Does nothing.
pub fn start_if_requested() {
    // TODO(ota): implement.
}
