//! Press-pattern decoder for a one-button BLE shutter remote.
//!
//! Input: press and release events with timestamps, plus regular
//! [`PressDecoder::poll`] calls. Output: at most one [`Pattern`] per call.
//!
//! - 1 press  → [`Pattern::Single`] (WINK L)
//! - 2 presses → [`Pattern::Double`] (WINK R)
//! - 3 presses → [`Pattern::Triple`] (WINK B)
//! - press held ≥ `long_press_ms` → [`Pattern::Long`] (ALT start / stop),
//!   reported while the button is still down. Short presses just before a
//!   long press are dropped.
//! - 4 or more presses in a row → nothing (treated as noise).
//!
//! A pattern is complete when `gap_ms` passes after the last release without
//! a new press. A press that arrives within `merge_ms` of a release is merged
//! with the previous press: some remotes send two HID reports per button.

use crate::command::{Command, Side};
use crate::config::RemoteTiming;
use crate::Millis;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pattern {
    Single,
    Double,
    Triple,
    Long,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Idle,
    /// Button down since `since`; `count` complete presses before it.
    Down { since: Millis, count: u8 },
    /// Long press already reported; waiting for the release.
    LongHeld,
    /// Button up since `released`; `count` complete presses so far.
    /// `last_since` is when the last press started (for merging).
    Up { released: Millis, count: u8, last_since: Millis },
}

#[derive(Debug, Clone)]
pub struct PressDecoder {
    cfg: RemoteTiming,
    state: State,
}

const MAX_COUNT: u8 = 9;

impl PressDecoder {
    pub const fn new(cfg: RemoteTiming) -> Self {
        Self { cfg, state: State::Idle }
    }

    /// Forget any partial pattern (e.g. the remote disconnected).
    pub fn reset(&mut self) {
        self.state = State::Idle;
    }

    /// True while a pattern is being entered.
    pub fn busy(&self) -> bool {
        self.state != State::Idle
    }

    /// Button went down.
    pub fn press(&mut self, now: Millis) -> Option<Pattern> {
        let out = self.poll(now);
        self.state = match self.state {
            State::Idle => State::Down { since: now, count: 0 },
            State::Up { released, count, last_since } => {
                if now.saturating_sub(released) < self.cfg.merge_ms as Millis {
                    State::Down { since: last_since, count: count.saturating_sub(1) }
                } else {
                    State::Down { since: now, count }
                }
            }
            // Duplicate press report: ignore.
            s @ (State::Down { .. } | State::LongHeld) => s,
        };
        out
    }

    /// Button went up.
    pub fn release(&mut self, now: Millis) -> Option<Pattern> {
        let out = self.poll(now);
        self.state = match self.state {
            State::Down { since, count } => State::Up {
                released: now,
                count: (count + 1).min(MAX_COUNT),
                last_since: since,
            },
            State::LongHeld => State::Idle,
            // Release without press: ignore.
            s @ (State::Idle | State::Up { .. }) => s,
        };
        out
    }

    /// Advance time. Call every few tens of milliseconds.
    pub fn poll(&mut self, now: Millis) -> Option<Pattern> {
        match self.state {
            State::Down { since, .. }
                if now.saturating_sub(since) >= self.cfg.long_press_ms as Millis =>
            {
                self.state = State::LongHeld;
                Some(Pattern::Long)
            }
            State::Up { released, count, .. }
                if now.saturating_sub(released) >= self.cfg.gap_ms as Millis =>
            {
                self.state = State::Idle;
                match count {
                    1 => Some(Pattern::Single),
                    2 => Some(Pattern::Double),
                    3 => Some(Pattern::Triple),
                    _ => None,
                }
            }
            _ => None,
        }
    }
}

/// Map a pattern to a command. A long press toggles alternation: it starts
/// `ALT alt_secs` when not alternating and stops it when alternating.
pub fn pattern_command(p: Pattern, alternating: bool, alt_secs: u32) -> Command {
    match p {
        Pattern::Single => Command::Wink(Side::Left),
        Pattern::Double => Command::Wink(Side::Right),
        Pattern::Triple => Command::Wink(Side::Both),
        Pattern::Long if alternating => Command::Stop,
        Pattern::Long => Command::Alt { secs: alt_secs },
    }
}

/// Track the button state across several HID report characteristics.
///
/// A remote may notify on more than one input report (consumer control and
/// keyboard). The button counts as down while any report is non-zero.
#[derive(Debug, Clone, Default)]
pub struct HidButton {
    down_mask: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edge {
    Press,
    Release,
}

impl HidButton {
    pub const fn new() -> Self {
        Self { down_mask: 0 }
    }

    /// Feed one input report from report characteristic number `idx` (< 32).
    /// Returns an edge when the combined button state changed.
    pub fn report(&mut self, idx: usize, data: &[u8]) -> Option<Edge> {
        let bit = 1u32 << (idx.min(31));
        let was = self.down_mask != 0;
        if data.iter().any(|&b| b != 0) {
            self.down_mask |= bit;
        } else {
            self.down_mask &= !bit;
        }
        match (was, self.down_mask != 0) {
            (false, true) => Some(Edge::Press),
            (true, false) => Some(Edge::Release),
            _ => None,
        }
    }

    pub fn reset(&mut self) -> Option<Edge> {
        let was = self.down_mask != 0;
        self.down_mask = 0;
        was.then_some(Edge::Release)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::REMOTE_TIMING;

    const LONG: Millis = REMOTE_TIMING.long_press_ms as Millis;
    const GAP: Millis = REMOTE_TIMING.gap_ms as Millis;

    /// Run a script of (press_at, release_at) pairs and poll every 10 ms up
    /// to `end`. Returns every pattern with the time it was reported.
    fn run(presses: &[(Millis, Millis)], end: Millis) -> Vec<(Millis, Pattern)> {
        let mut d = PressDecoder::new(REMOTE_TIMING);
        let mut out = Vec::new();
        let mut events: Vec<(Millis, bool)> = Vec::new();
        for &(p, r) in presses {
            events.push((p, true));
            events.push((r, false));
        }
        events.sort_by_key(|e| e.0);
        let mut ei = 0;
        let mut t = 0;
        while t <= end {
            while ei < events.len() && events[ei].0 <= t {
                let (et, down) = events[ei];
                let r = if down { d.press(et) } else { d.release(et) };
                if let Some(p) = r {
                    out.push((et, p));
                }
                ei += 1;
            }
            if let Some(p) = d.poll(t) {
                out.push((t, p));
            }
            t += 10;
        }
        out
    }

    fn patterns(v: &[(Millis, Pattern)]) -> Vec<Pattern> {
        v.iter().map(|x| x.1).collect()
    }

    #[test]
    fn single() {
        let out = run(&[(100, 200)], 2000);
        assert_eq!(patterns(&out), vec![Pattern::Single]);
        // Reported once the gap after the release has passed.
        assert_eq!(out[0].0, 200 + GAP);
    }

    #[test]
    fn double_and_triple() {
        assert_eq!(patterns(&run(&[(0, 100), (300, 400)], 2000)), vec![Pattern::Double]);
        assert_eq!(
            patterns(&run(&[(0, 100), (300, 400), (600, 700)], 3000)),
            vec![Pattern::Triple]
        );
    }

    #[test]
    fn four_presses_are_ignored() {
        let out = run(&[(0, 100), (250, 350), (500, 600), (750, 850)], 3000);
        assert!(out.is_empty(), "{out:?}");
    }

    #[test]
    fn separate_singles() {
        let out = run(&[(0, 100), (100 + GAP + 100, 100 + GAP + 200)], 3000);
        assert_eq!(patterns(&out), vec![Pattern::Single, Pattern::Single]);
    }

    #[test]
    fn long_press_reported_while_held() {
        let out = run(&[(1000, 3000)], 5000);
        assert_eq!(patterns(&out), vec![Pattern::Long]);
        assert_eq!(out[0].0, 1000 + LONG);
    }

    #[test]
    fn short_then_long_gives_long_only() {
        let out = run(&[(0, 100), (300, 1500)], 3000);
        assert_eq!(patterns(&out), vec![Pattern::Long]);
    }

    #[test]
    fn long_then_short_gives_both() {
        let out = run(&[(0, 1000), (1200, 1300)], 3000);
        assert_eq!(patterns(&out), vec![Pattern::Long, Pattern::Single]);
    }

    #[test]
    fn just_under_long_is_single() {
        let out = run(&[(0, LONG - 10)], 3000);
        assert_eq!(patterns(&out), vec![Pattern::Single]);
    }

    #[test]
    fn double_report_remote_merges_into_one_press() {
        // Remote sends volume-up press/release then Enter press/release
        // 20 ms apart for one physical press.
        let out = run(&[(0, 80), (100, 180)], 2000);
        assert_eq!(patterns(&out), vec![Pattern::Single]);
        // Two physical presses of such a remote.
        let out = run(&[(0, 80), (100, 180), (400, 480), (500, 580)], 3000);
        assert_eq!(patterns(&out), vec![Pattern::Double]);
    }

    #[test]
    fn merged_presses_keep_original_start_for_long_detection() {
        // 0..400 + merge + 420..800: one press 800 ms long.
        let out = run(&[(0, 400), (420, 800)], 3000);
        assert_eq!(patterns(&out), vec![Pattern::Long]);
    }

    #[test]
    fn late_events_without_poll_still_resolve() {
        let mut d = PressDecoder::new(REMOTE_TIMING);
        assert_eq!(d.press(0), None);
        assert_eq!(d.release(100), None);
        // No poll for a long time; the next press reports the old single.
        assert_eq!(d.press(5000), Some(Pattern::Single));
        // Release long after: the long press is reported on the release.
        assert_eq!(d.release(9000), Some(Pattern::Long));
        assert_eq!(d.poll(20_000), None);
        assert!(!d.busy());
    }

    #[test]
    fn stray_events_are_ignored() {
        let mut d = PressDecoder::new(REMOTE_TIMING);
        assert_eq!(d.release(0), None);
        assert!(!d.busy());
        assert_eq!(d.press(10), None);
        assert_eq!(d.press(20), None); // duplicate press
        assert_eq!(d.release(50), None);
        assert_eq!(d.release(55), None); // duplicate release
        assert_eq!(d.poll(50 + GAP), Some(Pattern::Single));
    }

    #[test]
    fn reset_drops_partial_pattern() {
        let mut d = PressDecoder::new(REMOTE_TIMING);
        d.press(0);
        d.reset();
        assert_eq!(d.poll(10_000), None);
    }

    #[test]
    fn time_going_backwards_does_not_panic() {
        let mut d = PressDecoder::new(REMOTE_TIMING);
        d.press(1000);
        assert_eq!(d.release(500), None);
        assert_eq!(d.poll(0), None);
    }

    #[test]
    fn mapping() {
        assert_eq!(pattern_command(Pattern::Single, false, 8), Command::Wink(Side::Left));
        assert_eq!(pattern_command(Pattern::Double, false, 8), Command::Wink(Side::Right));
        assert_eq!(pattern_command(Pattern::Triple, false, 8), Command::Wink(Side::Both));
        assert_eq!(pattern_command(Pattern::Long, false, 8), Command::Alt { secs: 8 });
        assert_eq!(pattern_command(Pattern::Long, true, 8), Command::Stop);
    }

    #[test]
    fn hid_button_combines_reports() {
        let mut b = HidButton::new();
        assert_eq!(b.report(0, &[0xE9, 0x00]), Some(Edge::Press)); // vol-up
        assert_eq!(b.report(1, &[0, 0, 0x28, 0, 0, 0, 0, 0]), None); // Enter
        assert_eq!(b.report(0, &[0, 0]), None);
        assert_eq!(b.report(1, &[0; 8]), Some(Edge::Release));
        assert_eq!(b.report(1, &[0; 8]), None);
        assert_eq!(b.report(0, &[]), None);
        assert_eq!(b.report(40, &[1]), Some(Edge::Press)); // index clamped
        assert_eq!(b.reset(), Some(Edge::Release));
        assert_eq!(b.reset(), None);
    }
}
