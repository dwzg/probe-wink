//! Authenticated ESP-NOW frames between master (LH) and slave (RH).
//!
//! ESP-NOW broadcast frames are not encrypted, and the two modules do not
//! know each other's MAC address in advance, so authentication is done here:
//!
//! - Both modules share a 32-byte secret, compiled in from the environment
//!   variable `WINK_ESPNOW_KEY` (see the README).
//! - Every frame carries the sender role, an *epoch* and a sequence number,
//!   and ends in a 16-byte tag: HMAC-SHA256(key, header ‖ body), truncated.
//! - The master bumps its epoch once per boot and stores it in NVS; the
//!   sequence number counts up within an epoch. The receiver accepts a frame
//!   only if (epoch, seq) is newer than everything it has accepted before.
//!   The slave stores the last accepted epoch in NVS, so frames recorded in
//!   an earlier session are refused after a reboot as well.
//! - If the master's epoch ever falls behind (NVS erased), the slave answers
//!   with an authenticated [`Message::StaleEpoch`] carrying its last epoch,
//!   and the master jumps past it.
//!
//! Frame layout (33 bytes):
//!
//! | offset | len | field                                     |
//! |--------|-----|-------------------------------------------|
//! | 0      | 2   | magic `PW`                                |
//! | 2      | 1   | version (1)                               |
//! | 3      | 1   | sender: 1 master, 2 slave                 |
//! | 4      | 4   | epoch, little endian                      |
//! | 8      | 4   | seq, little endian                        |
//! | 12     | 1   | message kind                              |
//! | 13     | 4   | message payload                           |
//! | 17     | 16  | tag: HMAC-SHA256 over bytes 0..17, first 16 |

use hmac::{Hmac, Mac};
use sha2::Sha256;

pub const KEY_LEN: usize = 32;
pub const TAG_LEN: usize = 16;
pub const FRAME_LEN: usize = 33;
const BODY_LEN: usize = FRAME_LEN - TAG_LEN;
const MAGIC: [u8; 2] = *b"PW";
const VERSION: u8 = 1;

pub type Key = [u8; KEY_LEN];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Master = 1,
    Slave = 2,
}

/// Supply fault as carried in a slave status frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SupplyState {
    Ok = 0,
    Over = 1,
    Under = 2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Message {
    /// Master → slave: RH relay on/off. Repeated as the heartbeat.
    Relay { on: bool },
    /// Master → slave: still awake (keeps the slave's awake timer going).
    Ping,
    /// Master → slave: going to deep sleep now; follow.
    Sleep,
    /// Slave → master: relay state, on-time limit, supply state.
    Status { relay_on: bool, limit_tripped: bool, supply: SupplyState },
    /// Slave → master: your epoch is not newer than `last_epoch`.
    StaleEpoch { last_epoch: u32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    pub sender: Role,
    pub epoch: u32,
    pub seq: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodeError {
    Length,
    Magic,
    Version,
    BadTag,
    Sender,
    Kind,
}

type HmacSha256 = Hmac<Sha256>;

fn mac(key: &Key, body: &[u8]) -> HmacSha256 {
    // HMAC accepts keys of any length; this cannot fail.
    let mut m = <HmacSha256 as Mac>::new_from_slice(key).expect("hmac key");
    m.update(body);
    m
}

fn encode_msg(msg: Message) -> (u8, [u8; 4]) {
    match msg {
        Message::Relay { on } => (1, [on as u8, 0, 0, 0]),
        Message::Ping => (2, [0; 4]),
        Message::Sleep => (3, [0; 4]),
        Message::Status { relay_on, limit_tripped, supply } => {
            (4, [relay_on as u8, limit_tripped as u8, supply as u8, 0])
        }
        Message::StaleEpoch { last_epoch } => (5, last_epoch.to_le_bytes()),
    }
}

fn decode_msg(kind: u8, p: [u8; 4]) -> Option<Message> {
    let flag = |b: u8| match b {
        0 => Some(false),
        1 => Some(true),
        _ => None,
    };
    Some(match kind {
        1 => Message::Relay { on: flag(p[0])? },
        2 => Message::Ping,
        3 => Message::Sleep,
        4 => Message::Status {
            relay_on: flag(p[0])?,
            limit_tripped: flag(p[1])?,
            supply: match p[2] {
                0 => SupplyState::Ok,
                1 => SupplyState::Over,
                2 => SupplyState::Under,
                _ => return None,
            },
        },
        5 => Message::StaleEpoch { last_epoch: u32::from_le_bytes(p) },
        _ => return None,
    })
}

/// Which roles may send which messages.
fn allowed(sender: Role, msg: &Message) -> bool {
    match msg {
        Message::Relay { .. } | Message::Ping | Message::Sleep => sender == Role::Master,
        Message::Status { .. } | Message::StaleEpoch { .. } => sender == Role::Slave,
    }
}

/// Build a frame.
pub fn encode(key: &Key, h: Header, msg: Message) -> [u8; FRAME_LEN] {
    let mut f = [0u8; FRAME_LEN];
    f[0..2].copy_from_slice(&MAGIC);
    f[2] = VERSION;
    f[3] = h.sender as u8;
    f[4..8].copy_from_slice(&h.epoch.to_le_bytes());
    f[8..12].copy_from_slice(&h.seq.to_le_bytes());
    let (kind, payload) = encode_msg(msg);
    f[12] = kind;
    f[13..17].copy_from_slice(&payload);
    let tag = mac(key, &f[..BODY_LEN]).finalize().into_bytes();
    f[BODY_LEN..].copy_from_slice(&tag[..TAG_LEN]);
    f
}

/// Check and parse a frame. The tag is verified (in constant time) before
/// any field is trusted. Replay is checked separately with [`ReplayGuard`].
pub fn decode(key: &Key, data: &[u8]) -> Result<(Header, Message), DecodeError> {
    if data.len() != FRAME_LEN {
        return Err(DecodeError::Length);
    }
    if data[0..2] != MAGIC {
        return Err(DecodeError::Magic);
    }
    if data[2] != VERSION {
        return Err(DecodeError::Version);
    }
    mac(key, &data[..BODY_LEN])
        .verify_truncated_left(&data[BODY_LEN..])
        .map_err(|_| DecodeError::BadTag)?;

    let sender = match data[3] {
        1 => Role::Master,
        2 => Role::Slave,
        _ => return Err(DecodeError::Sender),
    };
    let epoch = u32::from_le_bytes([data[4], data[5], data[6], data[7]]);
    let seq = u32::from_le_bytes([data[8], data[9], data[10], data[11]]);
    let msg = decode_msg(data[12], [data[13], data[14], data[15], data[16]])
        .ok_or(DecodeError::Kind)?;
    if !allowed(sender, &msg) {
        return Err(DecodeError::Sender);
    }
    Ok((Header { sender, epoch, seq }, msg))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Freshness {
    /// Newer than anything before, same epoch.
    Fresh,
    /// First frame of a newer epoch: persist `epoch` if you keep it in NVS.
    NewEpoch,
    /// Replayed or out of order: drop it.
    Stale,
}

/// Replay protection for one peer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReplayGuard {
    epoch: u32,
    /// Highest accepted seq in `epoch`; `None` = the epoch is closed (after a
    /// reboot we do not know which seqs were used, so only newer epochs pass).
    seq: Option<u32>,
}

impl ReplayGuard {
    /// Start from the last epoch accepted before (0 if none).
    pub const fn from_persisted(epoch: u32) -> Self {
        Self { epoch, seq: None }
    }

    pub fn check(&mut self, epoch: u32, seq: u32) -> Freshness {
        if epoch > self.epoch {
            self.epoch = epoch;
            self.seq = Some(seq);
            Freshness::NewEpoch
        } else if epoch == self.epoch && self.seq.is_some_and(|s| seq > s) {
            self.seq = Some(seq);
            Freshness::Fresh
        } else {
            Freshness::Stale
        }
    }

    pub fn epoch(&self) -> u32 {
        self.epoch
    }

    /// The sender of an authentic but stale frame with this epoch is behind
    /// us (it rebooted with a lost counter), as opposed to a duplicate within
    /// the current session.
    pub fn is_behind(&self, epoch: u32) -> bool {
        epoch < self.epoch || (epoch == self.epoch && self.seq.is_none())
    }
}

/// Sender side: epoch + sequence counter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Counter {
    role: Role,
    epoch: u32,
    seq: u32,
}

impl Counter {
    /// `epoch` must be newer than any epoch used before (stored + 1).
    pub const fn new(role: Role, epoch: u32) -> Self {
        Self { role, epoch, seq: 0 }
    }

    pub fn next(&mut self) -> Header {
        if self.seq == u32::MAX {
            // Practically unreachable (27 years at 5 frames/s); move on to a
            // new epoch rather than wrap.
            self.epoch = self.epoch.wrapping_add(1);
            self.seq = 0;
        }
        self.seq += 1;
        Header { sender: self.role, epoch: self.epoch, seq: self.seq }
    }

    /// The receiver says our epoch is too old: jump past `last_epoch`.
    /// Returns the new epoch (persist it) if it changed.
    pub fn bump_past(&mut self, last_epoch: u32) -> Option<u32> {
        if last_epoch >= self.epoch && last_epoch < u32::MAX {
            self.epoch = last_epoch + 1;
            self.seq = 0;
            Some(self.epoch)
        } else {
            None
        }
    }

    pub fn epoch(&self) -> u32 {
        self.epoch
    }
}

/// Parse a 64-character hex key at compile time. Panics (= compile error in
/// a `const`) on bad input.
pub const fn parse_key_hex(s: &str) -> Key {
    let b = s.as_bytes();
    if b.len() != 2 * KEY_LEN {
        panic!("WINK_ESPNOW_KEY must be exactly 64 hex characters");
    }
    let mut out = [0u8; KEY_LEN];
    let mut i = 0;
    while i < KEY_LEN {
        out[i] = (hex_nibble(b[2 * i]) << 4) | hex_nibble(b[2 * i + 1]);
        i += 1;
    }
    out
}

const fn hex_nibble(c: u8) -> u8 {
    match c {
        b'0'..=b'9' => c - b'0',
        b'a'..=b'f' => c - b'a' + 10,
        b'A'..=b'F' => c - b'A' + 10,
        _ => panic!("WINK_ESPNOW_KEY contains a non-hex character"),
    }
}

/// True if every byte of the key is the same (e.g. all zeros): almost
/// certainly a placeholder, not a real secret.
pub const fn key_is_trivial(k: &Key) -> bool {
    let mut i = 1;
    while i < KEY_LEN {
        if k[i] != k[0] {
            return false;
        }
        i += 1;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: Key = parse_key_hex("000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f");
    const OTHER: Key = parse_key_hex("ff0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f");

    fn h(sender: Role, epoch: u32, seq: u32) -> Header {
        Header { sender, epoch, seq }
    }

    #[test]
    fn roundtrip_all_messages() {
        let msgs = [
            (Role::Master, Message::Relay { on: true }),
            (Role::Master, Message::Relay { on: false }),
            (Role::Master, Message::Ping),
            (Role::Master, Message::Sleep),
            (
                Role::Slave,
                Message::Status { relay_on: true, limit_tripped: false, supply: SupplyState::Over },
            ),
            (Role::Slave, Message::StaleEpoch { last_epoch: 0xDEAD_BEEF }),
        ];
        for (role, m) in msgs {
            let hd = h(role, 7, 42);
            let f = encode(&KEY, hd, m);
            assert_eq!(decode(&KEY, &f), Ok((hd, m)));
        }
    }

    #[test]
    fn wrong_key_rejected() {
        let f = encode(&KEY, h(Role::Master, 1, 1), Message::Relay { on: true });
        assert_eq!(decode(&OTHER, &f), Err(DecodeError::BadTag));
    }

    #[test]
    fn any_bit_flip_rejected() {
        let f = encode(&KEY, h(Role::Master, 1, 1), Message::Relay { on: false });
        for byte in 3..FRAME_LEN {
            for bit in 0..8 {
                let mut g = f;
                g[byte] ^= 1 << bit;
                assert!(decode(&KEY, &g).is_err(), "flip {byte}:{bit} accepted");
            }
        }
    }

    #[test]
    fn framing_errors() {
        let f = encode(&KEY, h(Role::Master, 1, 1), Message::Ping);
        assert_eq!(decode(&KEY, &f[..32]), Err(DecodeError::Length));
        assert_eq!(decode(&KEY, &[0u8; 40]), Err(DecodeError::Length));
        let mut g = f;
        g[0] = b'X';
        assert_eq!(decode(&KEY, &g), Err(DecodeError::Magic));
        let mut g = f;
        g[2] = 2;
        assert_eq!(decode(&KEY, &g), Err(DecodeError::Version));
    }

    #[test]
    fn role_spoofing_rejected() {
        // A validly signed frame claiming the slave sent a relay command.
        let f = encode(&KEY, h(Role::Slave, 1, 1), Message::Relay { on: true });
        assert_eq!(decode(&KEY, &f), Err(DecodeError::Sender));
        let f = encode(&KEY, h(Role::Master, 1, 1), Message::StaleEpoch { last_epoch: 9 });
        assert_eq!(decode(&KEY, &f), Err(DecodeError::Sender));
    }

    #[test]
    fn replay_guard_sequence() {
        let mut g = ReplayGuard::from_persisted(0);
        assert_eq!(g.check(1, 1), Freshness::NewEpoch);
        assert_eq!(g.check(1, 2), Freshness::Fresh);
        assert_eq!(g.check(1, 2), Freshness::Stale, "duplicate");
        assert_eq!(g.check(1, 1), Freshness::Stale, "old");
        assert_eq!(g.check(1, 10), Freshness::Fresh, "gaps are fine");
        assert_eq!(g.check(1, 9), Freshness::Stale);
        assert_eq!(g.check(2, 1), Freshness::NewEpoch);
        assert_eq!(g.check(1, 100), Freshness::Stale, "older epoch");
        assert_eq!(g.epoch(), 2);
    }

    #[test]
    fn replay_guard_after_reboot_refuses_last_epoch() {
        // Slave persisted epoch 5; an attacker replays frames from epoch 5.
        let mut g = ReplayGuard::from_persisted(5);
        assert_eq!(g.check(5, 1000), Freshness::Stale);
        assert_eq!(g.check(4, 1), Freshness::Stale);
        assert_eq!(g.check(6, 1), Freshness::NewEpoch);
    }

    #[test]
    fn behind_vs_duplicate() {
        let mut g = ReplayGuard::from_persisted(5);
        assert!(g.is_behind(5), "closed epoch after reboot");
        assert!(g.is_behind(3));
        assert!(!g.is_behind(6));
        g.check(6, 1);
        assert!(!g.is_behind(6), "duplicate in the live session");
        assert!(g.is_behind(5));
    }

    #[test]
    fn counter_and_resync() {
        let mut c = Counter::new(Role::Master, 3);
        assert_eq!(c.next(), h(Role::Master, 3, 1));
        assert_eq!(c.next(), h(Role::Master, 3, 2));
        // Slave says it already saw epoch 10.
        assert_eq!(c.bump_past(10), Some(11));
        assert_eq!(c.next(), h(Role::Master, 11, 1));
        // A replayed, older StaleEpoch changes nothing.
        assert_eq!(c.bump_past(4), None);
        assert_eq!(c.epoch(), 11);
        assert_eq!(c.bump_past(u32::MAX), None);
    }

    #[test]
    fn end_to_end_master_to_slave() {
        let mut tx = Counter::new(Role::Master, 1);
        let mut rx = ReplayGuard::from_persisted(0);
        let mut recorded = Vec::new();
        for i in 0..5 {
            let f = encode(&KEY, tx.next(), Message::Relay { on: i % 2 == 0 });
            recorded.push(f);
            let (hd, _) = decode(&KEY, &f).unwrap();
            assert_ne!(rx.check(hd.epoch, hd.seq), Freshness::Stale);
        }
        // Replaying any recorded frame fails.
        for f in &recorded {
            let (hd, _) = decode(&KEY, f).unwrap();
            assert_eq!(rx.check(hd.epoch, hd.seq), Freshness::Stale);
        }
    }

    #[test]
    fn key_parsing() {
        assert_eq!(KEY[0], 0);
        assert_eq!(KEY[31], 0x1f);
        let k = parse_key_hex("ABCDEFabcdef0000000000000000000000000000000000000000000000000000");
        assert_eq!(&k[..6], &[0xAB, 0xCD, 0xEF, 0xAB, 0xCD, 0xEF]);
        assert!(key_is_trivial(&[0u8; 32]));
        assert!(!key_is_trivial(&KEY));
    }

    #[test]
    #[should_panic]
    fn key_parsing_rejects_short() {
        parse_key_hex("abcd");
    }

    #[test]
    #[should_panic]
    fn key_parsing_rejects_non_hex() {
        parse_key_hex("zz0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f");
    }
}
