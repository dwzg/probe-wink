//! The awake window: deep sleep after a stretch without commands.

use crate::Millis;

#[derive(Debug, Clone)]
pub struct AwakeTimer {
    window: Millis,
    last_activity: Millis,
}

impl AwakeTimer {
    pub const fn new(now: Millis, window_ms: u32) -> Self {
        Self { window: window_ms as Millis, last_activity: now }
    }

    /// A command arrived (or, on the slave, any valid frame from the master).
    pub fn activity(&mut self, now: Millis) {
        if now > self.last_activity {
            self.last_activity = now;
        }
    }

    /// Time to sleep? Never while `busy` (a relay on, a mode running, the
    /// remote being paired).
    pub fn should_sleep(&self, now: Millis, busy: bool) -> bool {
        !busy && now.saturating_sub(self.last_activity) >= self.window
    }

    pub fn remaining_ms(&self, now: Millis) -> Millis {
        self.window.saturating_sub(now.saturating_sub(self.last_activity))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: u32 = 15 * 60 * 1000;

    #[test]
    fn sleeps_after_window() {
        let t = AwakeTimer::new(1000, W);
        assert!(!t.should_sleep(1000, false));
        assert!(!t.should_sleep(1000 + W as u64 - 1, false));
        assert!(t.should_sleep(1000 + W as u64, false));
    }

    #[test]
    fn activity_extends() {
        let mut t = AwakeTimer::new(0, W);
        t.activity(600_000);
        assert!(!t.should_sleep(W as u64, false));
        assert!(t.should_sleep(600_000 + W as u64, false));
        assert_eq!(t.remaining_ms(600_000), W as u64);
    }

    #[test]
    fn busy_prevents_sleep() {
        let t = AwakeTimer::new(0, W);
        assert!(!t.should_sleep(10 * W as u64, true));
    }

    #[test]
    fn activity_in_the_past_ignored() {
        let mut t = AwakeTimer::new(5000, W);
        t.activity(10);
        assert_eq!(t.remaining_ms(5000), W as u64);
    }
}
