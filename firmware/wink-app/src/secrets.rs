//! Build-time secrets. Both come from environment variables at compile time
//! and the build fails if they are missing or malformed (see README).

use wink_core::config::parse_passkey;
use wink_core::proto::{key_is_trivial, parse_key_hex, Key};

/// Shared ESP-NOW HMAC key (32 bytes, 64 hex characters).
pub const ESPNOW_KEY: Key = parse_key_hex(env!(
    "WINK_ESPNOW_KEY",
    "set WINK_ESPNOW_KEY to 64 hex characters, e.g. `export WINK_ESPNOW_KEY=$(openssl rand -hex 32)`; see firmware/README.md"
));

const _: () = assert!(
    !key_is_trivial(&ESPNOW_KEY),
    "WINK_ESPNOW_KEY looks like a placeholder (all bytes equal)"
);

/// 6-digit BLE passkey the phone has to enter when pairing.
pub const BLE_PASSKEY: u32 = parse_passkey(env!(
    "WINK_BLE_PASSKEY",
    "set WINK_BLE_PASSKEY to a 6-digit number; see firmware/README.md"
));
