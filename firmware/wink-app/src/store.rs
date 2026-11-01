//! Small persistent values in NVS (namespace `wink`).

use esp_idf_svc::nvs::{EspDefaultNvs, EspDefaultNvsPartition};
use esp_idf_svc::sys::EspError;
use log::warn;

const NS: &str = "wink";
/// Epoch this module last used for its own frames.
const TX_EPOCH: &str = "tx_epoch";
/// Last peer epoch accepted (replay protection across reboots).
const RX_EPOCH: &str = "rx_epoch";
/// Bonded remote: 6 address bytes (LE) + address type.
const REMOTE: &str = "remote";
/// u8: 1 = refuse lamp movement while the engine runs.
const DRIVE_LOCK: &str = "drivelock";

pub struct Store {
    nvs: EspDefaultNvs,
}

impl Store {
    pub fn new(part: EspDefaultNvsPartition) -> Result<Self, EspError> {
        Ok(Self { nvs: EspDefaultNvs::new(part, NS, true)? })
    }

    /// Next epoch for our own frames: stored + 1, persisted before use.
    pub fn next_tx_epoch(&mut self) -> u32 {
        let e = self.nvs.get_u32(TX_EPOCH).ok().flatten().unwrap_or(0).wrapping_add(1).max(1);
        self.set_tx_epoch(e);
        e
    }

    pub fn set_tx_epoch(&mut self, e: u32) {
        if let Err(err) = self.nvs.set_u32(TX_EPOCH, e) {
            warn!("NVS write tx_epoch failed: {err}");
        }
    }

    pub fn rx_epoch(&self) -> u32 {
        self.nvs.get_u32(RX_EPOCH).ok().flatten().unwrap_or(0)
    }

    pub fn set_rx_epoch(&mut self, e: u32) {
        if let Err(err) = self.nvs.set_u32(RX_EPOCH, e) {
            warn!("NVS write rx_epoch failed: {err}");
        }
    }

    /// Drive lock setting (off unless set).
    pub fn drive_lock(&self) -> bool {
        self.nvs.get_u8(DRIVE_LOCK).ok().flatten() == Some(1)
    }

    pub fn set_drive_lock(&mut self, on: bool) {
        if let Err(err) = self.nvs.set_u8(DRIVE_LOCK, on as u8) {
            warn!("NVS: drive lock not saved: {err}");
        }
    }

    pub fn remote(&self) -> Option<[u8; 7]> {
        let mut buf = [0u8; 7];
        match self.nvs.get_blob(REMOTE, &mut buf) {
            Ok(Some(b)) if b.len() == 7 => Some(buf),
            _ => None,
        }
    }

    pub fn set_remote(&mut self, v: Option<[u8; 7]>) {
        let r = match v {
            Some(b) => self.nvs.set_blob(REMOTE, &b),
            None => self.nvs.remove(REMOTE).map(|_| ()),
        };
        if let Err(err) = r {
            warn!("NVS write remote failed: {err}");
        }
    }
}
