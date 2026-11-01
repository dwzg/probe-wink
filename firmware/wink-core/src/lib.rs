//! Pure logic of the probe-wink headlamp module.
//!
//! Nothing in this crate touches hardware. Every function takes the current
//! time as a plain millisecond count and returns what the hardware should do,
//! so the whole behaviour can be unit-tested on the host:
//!
//! ```text
//! cargo test -p wink-core --target x86_64-unknown-linux-gnu
//! ```
//!
//! Modules:
//! - [`config`]: timing constants (travel time T, dwell, limits).
//! - [`command`]: parser for the text commands written by the phone.
//! - [`remote`]: press-pattern decoder for the BLE shutter remote.
//! - [`controller`]: the wink / alternate / rest state machine (master).
//! - [`safety`]: on-time limiter, heartbeat watchdog and heartbeat sender.
//! - [`awake`]: the 15-minute awake window.
//! - [`voltage`]: supply-voltage guard for the relay coil.
//! - [`wake`]: which sense line to arm for the deep-sleep wake.
//! - [`proto`]: authenticated ESP-NOW frames and replay protection.
#![cfg_attr(not(test), no_std)]

pub mod awake;
pub mod command;
pub mod config;
pub mod controller;
pub mod proto;
pub mod remote;
pub mod safety;
pub mod voltage;
pub mod wake;

/// Monotonic time in milliseconds since boot.
pub type Millis = u64;
