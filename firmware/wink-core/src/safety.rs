//! Safety rules each module enforces on its own.
//!
//! - [`OnTimeLimiter`]: a relay is never on longer than `relay_max_on_ms` in
//!   one go, whatever the commands say. Once tripped it stays off until the
//!   request goes off.
//! - [`HeartbeatWatchdog`] (slave): the RH relay is only on while "on"
//!   frames keep arriving at least every `heartbeat_timeout_ms`.
//! - [`HeartbeatSender`] (master): when to send relay frames to the slave.

use crate::config::Timing;
use crate::Millis;

#[derive(Debug, Clone)]
pub struct OnTimeLimiter {
    max_on: Millis,
    on_since: Option<Millis>,
    tripped: bool,
}

impl OnTimeLimiter {
    pub const fn new(max_on_ms: u32) -> Self {
        Self { max_on: max_on_ms as Millis, on_since: None, tripped: false }
    }

    /// Filter a requested relay state. Returns the state to drive.
    pub fn apply(&mut self, now: Millis, requested: bool) -> bool {
        if !requested {
            self.on_since = None;
            self.tripped = false;
            return false;
        }
        if self.tripped {
            return false;
        }
        let since = *self.on_since.get_or_insert(now);
        if now.saturating_sub(since) >= self.max_on {
            self.tripped = true;
            self.on_since = None;
            return false;
        }
        true
    }

    /// The limit cut the relay and the request is still on.
    pub fn tripped(&self) -> bool {
        self.tripped
    }
}

#[derive(Debug, Clone)]
pub struct HeartbeatWatchdog {
    timeout: Millis,
    last_on: Option<Millis>,
}

impl HeartbeatWatchdog {
    pub const fn new(timeout_ms: u32) -> Self {
        Self { timeout: timeout_ms as Millis, last_on: None }
    }

    /// A relay frame from the master arrived.
    pub fn on_frame(&mut self, now: Millis, relay_on: bool) {
        self.last_on = relay_on.then_some(now);
    }

    /// Should the relay be on now?
    pub fn relay_wanted(&mut self, now: Millis) -> bool {
        match self.last_on {
            Some(t) if now.saturating_sub(t) <= self.timeout && now >= t => true,
            Some(_) => {
                self.last_on = None;
                false
            }
            None => false,
        }
    }

    /// Drop any pending "on" (e.g. before sleep).
    pub fn clear(&mut self) {
        self.last_on = None;
    }
}

#[derive(Debug, Clone)]
pub struct HeartbeatSender {
    interval: Millis,
    off_repeats: u8,
    state: bool,
    last_sent: Option<Millis>,
    off_left: u8,
}

impl HeartbeatSender {
    pub const fn new(t: &Timing) -> Self {
        Self {
            interval: t.heartbeat_interval_ms as Millis,
            off_repeats: t.off_repeats,
            state: false,
            last_sent: None,
            off_left: 0,
        }
    }

    /// Given the desired RH relay state, return `Some(state)` when a relay
    /// frame should go out now: immediately on a change, every interval while
    /// on, and `off_repeats` more times after it goes off.
    pub fn poll(&mut self, now: Millis, rh_on: bool) -> Option<bool> {
        if rh_on != self.state {
            self.state = rh_on;
            self.off_left = if rh_on { 0 } else { self.off_repeats };
            self.last_sent = Some(now);
            return Some(rh_on);
        }
        let due = self
            .last_sent
            .is_some_and(|t| now.saturating_sub(t) >= self.interval);
        if !due {
            return None;
        }
        if rh_on {
            self.last_sent = Some(now);
            Some(true)
        } else if self.off_left > 0 {
            self.off_left -= 1;
            self.last_sent = Some(now);
            Some(false)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::TIMING;

    #[test]
    fn limiter_passes_short_on() {
        let mut l = OnTimeLimiter::new(12_000);
        assert!(l.apply(0, true));
        assert!(l.apply(11_999, true));
        assert!(!l.apply(12_000, false));
        assert!(l.apply(12_001, true));
    }

    #[test]
    fn limiter_cuts_at_limit_and_latches() {
        let mut l = OnTimeLimiter::new(12_000);
        assert!(l.apply(1000, true));
        assert!(!l.apply(13_000, true));
        assert!(l.tripped());
        assert!(!l.apply(60_000, true), "stays off while still requested");
        assert!(!l.apply(60_001, false));
        assert!(!l.tripped());
        assert!(l.apply(60_002, true));
    }

    #[test]
    fn limiter_holds_even_with_sparse_calls() {
        let mut l = OnTimeLimiter::new(12_000);
        assert!(l.apply(0, true));
        assert!(!l.apply(100_000, true));
    }

    #[test]
    fn watchdog_needs_frames() {
        let mut w = HeartbeatWatchdog::new(1000);
        assert!(!w.relay_wanted(0));
        w.on_frame(100, true);
        assert!(w.relay_wanted(100));
        assert!(w.relay_wanted(1100));
        assert!(!w.relay_wanted(1101));
        // Stays off after the timeout even if time "returns".
        assert!(!w.relay_wanted(500));
    }

    #[test]
    fn watchdog_heartbeat_keeps_on() {
        let mut w = HeartbeatWatchdog::new(1000);
        for t in (0..10_000).step_by(200) {
            w.on_frame(t, true);
            assert!(w.relay_wanted(t + 150));
        }
    }

    #[test]
    fn watchdog_off_frame_turns_off() {
        let mut w = HeartbeatWatchdog::new(1000);
        w.on_frame(0, true);
        w.on_frame(50, false);
        assert!(!w.relay_wanted(60));
        w.on_frame(70, true);
        w.clear();
        assert!(!w.relay_wanted(80));
    }

    #[test]
    fn watchdog_tolerates_lost_frames_up_to_timeout() {
        // Master sends every 200 ms; four lost frames in a row still keep it on.
        let mut w = HeartbeatWatchdog::new(TIMING.heartbeat_timeout_ms);
        w.on_frame(0, true);
        assert!(w.relay_wanted(999));
    }

    #[test]
    fn sender_schedule() {
        let mut s = HeartbeatSender::new(&TIMING);
        assert_eq!(s.poll(0, false), None);
        assert_eq!(s.poll(10, true), Some(true));
        assert_eq!(s.poll(100, true), None);
        assert_eq!(s.poll(210, true), Some(true));
        assert_eq!(s.poll(300, true), None);
        assert_eq!(s.poll(410, true), Some(true));
        // Off: immediately, then three repeats, then silence.
        assert_eq!(s.poll(420, false), Some(false));
        assert_eq!(s.poll(500, false), None);
        assert_eq!(s.poll(620, false), Some(false));
        assert_eq!(s.poll(820, false), Some(false));
        assert_eq!(s.poll(1020, false), Some(false));
        assert_eq!(s.poll(1220, false), None);
        assert_eq!(s.poll(5000, false), None);
    }

    #[test]
    fn sender_keeps_slave_alive_through_whole_on_period() {
        let mut s = HeartbeatSender::new(&TIMING);
        let mut w = HeartbeatWatchdog::new(TIMING.heartbeat_timeout_ms);
        for t in 0..5000u64 {
            if let Some(on) = s.poll(t, true) {
                w.on_frame(t, on);
            }
            assert!(w.relay_wanted(t), "slave dropped at {t}");
        }
        // Master stops sending (dies): slave drops within the timeout.
        let mut off_at = None;
        for t in 5000..7000u64 {
            if !w.relay_wanted(t) {
                off_at = Some(t);
                break;
            }
        }
        let off_at = off_at.expect("slave never dropped");
        assert!(off_at <= 5000 + TIMING.heartbeat_timeout_ms as u64, "{off_at}");
    }
}
