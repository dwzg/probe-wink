//! Mode state machine of the master: wink, alternate, stop, rest.
//!
//! The controller owns no hardware. Feed it commands and time; it says which
//! relays should be on and what state to report. Safety limits that each
//! module enforces on its own (on-time limit, heartbeat timeout, supply
//! voltage) sit downstream in [`crate::safety`] and [`crate::voltage`].
//!
//! Policy:
//! - While a wink or an alternation runs, every command except `STOP` (and a
//!   long press, which maps to `STOP` while alternating) is refused as busy.
//! - `ALT` is refused during the rest that follows an alternation, whether
//!   the run finished or was stopped. Winks are allowed during the rest.
//! - An alternation always ends on a phase boundary so the last lamp has
//!   finished its travel: it runs `max(1, floor(n·1000 / P))` phases of P.
//! - While a supply fault is reported ([`Controller::set_inhibit`]), any
//!   running mode stops and wink / alt commands are refused.

use crate::command::{Command, Side};
use crate::config::Timing;
use crate::voltage::Fault;
use crate::Millis;

/// Desired relay states. `true` = energized = that lamp swapped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Relays {
    pub lh: bool,
    pub rh: bool,
}

impl Relays {
    pub const OFF: Relays = Relays { lh: false, rh: false };
    pub const fn any(self) -> bool {
        self.lh || self.rh
    }
}

/// State reported on the notify characteristic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Asleep,
    Idle,
    Wink,
    Alternating,
    Resting,
    Overvoltage,
    Undervoltage,
}

impl Status {
    pub const fn as_str(self) -> &'static str {
        match self {
            Status::Asleep => "asleep",
            Status::Idle => "idle",
            Status::Wink => "wink",
            Status::Alternating => "alternating",
            Status::Resting => "resting",
            Status::Overvoltage => "overvoltage",
            Status::Undervoltage => "undervoltage",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reject {
    /// A wink or alternation is running; only STOP is accepted.
    Busy,
    /// ALT during the rest period; `remaining_ms` until it is allowed.
    Resting { remaining_ms: u32 },
    /// Supply voltage out of range.
    Supply(Fault),
    /// Not a mode command (PAIR REMOTE etc.); handled by the app.
    NotMine,
    /// Drive lock on and the engine runs (decided by the app, see `voltage::engine_running`).
    EngineRunning,
}

impl Reject {
    pub const fn as_str(self) -> &'static str {
        match self {
            Reject::Busy => "busy",
            Reject::Resting { .. } => "resting",
            Reject::Supply(Fault::Overvoltage) => "overvoltage",
            Reject::Supply(Fault::Undervoltage) => "undervoltage",
            Reject::NotMine => "not a mode command",
            Reject::EngineRunning => "engine running",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Idle,
    Wink { side: Side, until: Millis },
    Alt { start: Millis, period: Millis, phases: u32 },
}

#[derive(Debug, Clone)]
pub struct Controller {
    t: Timing,
    mode: Mode,
    rest_until: Option<Millis>,
    inhibit: Option<Fault>,
}

impl Controller {
    pub const fn new(t: Timing) -> Self {
        Self { t, mode: Mode::Idle, rest_until: None, inhibit: None }
    }

    pub fn timing(&self) -> &Timing {
        &self.t
    }

    /// Apply a command. On success the new mode is in effect immediately.
    pub fn command(&mut self, now: Millis, cmd: Command) -> Result<(), Reject> {
        self.advance(now);
        match cmd {
            Command::Stop => {
                self.stop(now);
                Ok(())
            }
            Command::Wink(side) => {
                self.check_inhibit()?;
                if self.is_running() {
                    return Err(Reject::Busy);
                }
                let until = now + self.t.wink_hold_ms() as Millis;
                self.mode = Mode::Wink { side, until };
                Ok(())
            }
            Command::Alt { secs } => {
                self.check_inhibit()?;
                if self.is_running() {
                    return Err(Reject::Busy);
                }
                if let Some(until) = self.rest_until {
                    return Err(Reject::Resting {
                        remaining_ms: until.saturating_sub(now).min(u32::MAX as u64) as u32,
                    });
                }
                let secs = self.t.clamp_alt_secs(secs);
                let period = self.t.alt_period_ms() as Millis;
                let phases = ((secs as Millis * 1000) / period).max(1) as u32;
                self.mode = Mode::Alt { start: now, period, phases };
                Ok(())
            }
            Command::PairRemote | Command::ForgetRemote | Command::Status | Command::DriveLock(_) => {
                Err(Reject::NotMine)
            }
        }
    }

    /// Report a supply fault (or its end). A fault stops any running mode.
    pub fn set_inhibit(&mut self, now: Millis, fault: Option<Fault>) {
        self.advance(now);
        if fault.is_some() && self.is_running() {
            self.stop(now);
        }
        self.inhibit = fault;
    }

    /// Advance time and return the desired relay states.
    pub fn tick(&mut self, now: Millis) -> Relays {
        self.advance(now);
        self.relays(now)
    }

    /// Desired relays at `now` (call [`Controller::tick`] first).
    pub fn relays(&self, now: Millis) -> Relays {
        match self.mode {
            Mode::Idle => Relays::OFF,
            Mode::Wink { side, .. } => Relays { lh: side.lh(), rh: side.rh() },
            Mode::Alt { start, period, .. } => {
                let phase = now.saturating_sub(start) / period;
                let lh = phase % 2 == 0;
                Relays { lh, rh: !lh }
            }
        }
    }

    pub fn status(&self) -> Status {
        match (self.inhibit, self.mode) {
            (Some(Fault::Overvoltage), _) => Status::Overvoltage,
            (Some(Fault::Undervoltage), _) => Status::Undervoltage,
            (None, Mode::Wink { .. }) => Status::Wink,
            (None, Mode::Alt { .. }) => Status::Alternating,
            (None, Mode::Idle) if self.rest_until.is_some() => Status::Resting,
            (None, Mode::Idle) => Status::Idle,
        }
    }

    /// A wink or alternation is running.
    pub fn is_running(&self) -> bool {
        self.mode != Mode::Idle
    }

    pub fn is_alternating(&self) -> bool {
        matches!(self.mode, Mode::Alt { .. })
    }

    /// When the current alternation ends (for logs and tests).
    pub fn alt_end(&self) -> Option<Millis> {
        match self.mode {
            Mode::Alt { start, period, phases } => Some(start + period * phases as Millis),
            _ => None,
        }
    }

    fn check_inhibit(&self) -> Result<(), Reject> {
        match self.inhibit {
            Some(f) => Err(Reject::Supply(f)),
            None => Ok(()),
        }
    }

    fn stop(&mut self, now: Millis) {
        if self.is_alternating() {
            self.rest_until = Some(now + self.t.rest_ms as Millis);
        }
        self.mode = Mode::Idle;
    }

    fn advance(&mut self, now: Millis) {
        match self.mode {
            Mode::Wink { until, .. } if now >= until => self.mode = Mode::Idle,
            Mode::Alt { start, period, phases } => {
                let end = start + period * phases as Millis;
                if now >= end {
                    self.mode = Mode::Idle;
                    self.rest_until = Some(end + self.t.rest_ms as Millis);
                }
            }
            _ => {}
        }
        if let Some(until) = self.rest_until {
            if now >= until {
                self.rest_until = None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::TIMING;

    const HOLD: Millis = TIMING.wink_hold_ms() as Millis; // 1300
    const P: Millis = TIMING.alt_period_ms() as Millis; // 1200
    const REST: Millis = TIMING.rest_ms as Millis;

    fn c() -> Controller {
        Controller::new(TIMING)
    }

    const L: Relays = Relays { lh: true, rh: false };
    const R: Relays = Relays { lh: false, rh: true };
    const B: Relays = Relays { lh: true, rh: true };

    #[test]
    fn idle_at_start() {
        let mut c = c();
        assert_eq!(c.tick(0), Relays::OFF);
        assert_eq!(c.status(), Status::Idle);
    }

    #[test]
    fn wink_each_side() {
        for (side, want) in [(Side::Left, L), (Side::Right, R), (Side::Both, B)] {
            let mut c = c();
            c.command(1000, Command::Wink(side)).unwrap();
            assert_eq!(c.tick(1000), want);
            assert_eq!(c.status(), Status::Wink);
            assert_eq!(c.tick(1000 + HOLD - 1), want);
            assert_eq!(c.tick(1000 + HOLD), Relays::OFF);
            assert_eq!(c.status(), Status::Idle);
        }
    }

    #[test]
    fn wink_holds_at_least_travel_time() {
        assert!(HOLD >= TIMING.travel_ms as Millis);
    }

    #[test]
    fn busy_while_running() {
        let mut c = c();
        c.command(0, Command::Wink(Side::Left)).unwrap();
        assert_eq!(c.command(10, Command::Wink(Side::Right)), Err(Reject::Busy));
        assert_eq!(c.command(10, Command::Alt { secs: 5 }), Err(Reject::Busy));
        // After the wink ends, a new one is accepted.
        c.command(HOLD, Command::Wink(Side::Right)).unwrap();
        assert_eq!(c.tick(HOLD), R);
    }

    #[test]
    fn stop_ends_wink_immediately() {
        let mut c = c();
        c.command(0, Command::Wink(Side::Both)).unwrap();
        c.command(100, Command::Stop).unwrap();
        assert_eq!(c.tick(100), Relays::OFF);
        // A stopped wink does not start a rest.
        assert_eq!(c.status(), Status::Idle);
        c.command(200, Command::Alt { secs: 2 }).unwrap();
    }

    #[test]
    fn stop_when_idle_is_ok() {
        let mut c = c();
        assert_eq!(c.command(0, Command::Stop), Ok(()));
        assert_eq!(c.status(), Status::Idle);
    }

    #[test]
    fn alternation_phases() {
        let mut c = c();
        c.command(0, Command::Alt { secs: 8 }).unwrap();
        assert_eq!(c.status(), Status::Alternating);
        // floor(8000 / 1200) = 6 phases → 7200 ms.
        assert_eq!(c.alt_end(), Some(6 * P));
        assert_eq!(c.tick(0), L);
        assert_eq!(c.tick(P - 1), L);
        assert_eq!(c.tick(P), R);
        assert_eq!(c.tick(2 * P), L);
        assert_eq!(c.tick(5 * P), R);
        assert_eq!(c.tick(6 * P - 1), R);
        assert_eq!(c.tick(6 * P), Relays::OFF);
        assert_eq!(c.status(), Status::Resting);
    }

    #[test]
    fn alternation_never_has_both_on() {
        let mut c = c();
        c.command(0, Command::Alt { secs: 10 }).unwrap();
        for t in (0..12_000).step_by(7) {
            let r = c.tick(t);
            assert!(!(r.lh && r.rh), "both on at {t}");
        }
    }

    #[test]
    fn every_phase_lasts_at_least_1_3_t() {
        let mut c = c();
        c.command(0, Command::Alt { secs: 10 }).unwrap();
        let mut last = c.tick(0);
        let mut since = 0;
        for t in 1..15_000 {
            let r = c.tick(t);
            if r != last {
                if last.any() {
                    assert!((t - since) * 10 >= TIMING.travel_ms as Millis * 13, "short phase at {t}");
                }
                last = r;
                since = t;
            }
        }
    }

    #[test]
    fn alternation_clamped_to_cap() {
        let mut c = c();
        c.command(0, Command::Alt { secs: 600 }).unwrap();
        let end = c.alt_end().unwrap();
        assert!(end <= TIMING.alt_max_s as Millis * 1000, "end {end}");
        assert_eq!(end, (10_000 / P) * P);
    }

    #[test]
    fn short_alternation_runs_one_full_phase() {
        let t = Timing { travel_ms: 1000, alt_period_cfg_ms: 0, ..TIMING }; // P = 1300
        let mut c = Controller::new(t);
        c.command(0, Command::Alt { secs: 1 }).unwrap();
        assert_eq!(c.alt_end(), Some(1300));
        assert_eq!(c.tick(1299), L);
        assert_eq!(c.tick(1300), Relays::OFF);
    }

    #[test]
    fn rest_blocks_alt_but_not_wink() {
        let mut c = c();
        c.command(0, Command::Alt { secs: 2 }).unwrap(); // 1 phase of 1200
        let end = c.alt_end().unwrap();
        c.tick(end);
        assert_eq!(c.status(), Status::Resting);
        assert_eq!(
            c.command(end + 5000, Command::Alt { secs: 2 }),
            Err(Reject::Resting { remaining_ms: (REST - 5000) as u32 })
        );
        c.command(end + 5000, Command::Wink(Side::Left)).unwrap();
        assert_eq!(c.status(), Status::Wink);
        c.tick(end + 5000 + HOLD);
        assert_eq!(c.status(), Status::Resting);
        assert_eq!(c.command(end + REST - 1, Command::Alt { secs: 2 }).is_err(), true);
        c.command(end + REST, Command::Alt { secs: 2 }).unwrap();
    }

    #[test]
    fn stopping_alt_starts_rest() {
        let mut c = c();
        c.command(0, Command::Alt { secs: 8 }).unwrap();
        c.command(3000, Command::Stop).unwrap();
        assert_eq!(c.tick(3000), Relays::OFF);
        assert_eq!(c.status(), Status::Resting);
        assert!(matches!(c.command(3000 + REST - 1, Command::Alt { secs: 8 }), Err(Reject::Resting { .. })));
        c.command(3000 + REST, Command::Alt { secs: 8 }).unwrap();
    }

    #[test]
    fn rest_expires_without_tick() {
        let mut c = c();
        c.command(0, Command::Alt { secs: 1 }).unwrap();
        // No tick for a minute; the next command sees both the end and the rest expiry.
        c.command(60_000, Command::Alt { secs: 1 }).unwrap();
        assert_eq!(c.status(), Status::Alternating);
    }

    #[test]
    fn non_mode_commands() {
        let mut c = c();
        assert_eq!(c.command(0, Command::PairRemote), Err(Reject::NotMine));
        assert_eq!(c.command(0, Command::ForgetRemote), Err(Reject::NotMine));
        assert_eq!(c.command(0, Command::Status), Err(Reject::NotMine));
    }

    #[test]
    fn supply_fault_stops_and_refuses() {
        let mut c = c();
        c.command(0, Command::Alt { secs: 8 }).unwrap();
        c.set_inhibit(1000, Some(Fault::Overvoltage));
        assert_eq!(c.tick(1000), Relays::OFF);
        assert_eq!(c.status(), Status::Overvoltage);
        assert_eq!(
            c.command(1100, Command::Wink(Side::Left)),
            Err(Reject::Supply(Fault::Overvoltage))
        );
        assert_eq!(c.command(1100, Command::Stop), Ok(()));
        c.set_inhibit(2000, Some(Fault::Undervoltage));
        assert_eq!(c.status(), Status::Undervoltage);
        c.set_inhibit(3000, None);
        // The stopped alternation left a rest behind.
        assert_eq!(c.status(), Status::Resting);
        c.command(3000, Command::Wink(Side::Right)).unwrap();
        assert_eq!(c.tick(3000), R);
    }

    #[test]
    fn status_strings() {
        assert_eq!(Status::Asleep.as_str(), "asleep");
        assert_eq!(Status::Alternating.as_str(), "alternating");
        assert_eq!(Status::Overvoltage.as_str(), "overvoltage");
        assert_eq!(Reject::Busy.as_str(), "busy");
    }
}
