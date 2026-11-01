//! Build-time secret: the BLE passkey. It comes from an environment variable at
//! compile time and the build fails if it is missing or malformed (see README).
//! The ESP-NOW link key is not built in: the phone writes it to both modules
//! when coupling them (see `coupling.rs`).

use wink_core::config::parse_passkey;

/// 6-digit BLE passkey the phone has to enter when pairing.
pub const BLE_PASSKEY: u32 = parse_passkey(env!(
    "WINK_BLE_PASSKEY",
    "set WINK_BLE_PASSKEY to a 6-digit number; see firmware/README.md"
));
