//! Which deep-sleep wake source to arm.
//!
//! The C3 wakes from deep sleep on a GPIO *level*. Arming a line that is
//! already at its wake level would wake the chip at once, so only the
//! *dead* command line(s) are armed, to wake when they go live. The stock
//! headlamp switch powers exactly one command line; a switch change makes
//! the other one live.
//!
//! In terms of the harness lines:
//!
//! | UP line | DOWN line | Arm                                   |
//! |---------|-----------|---------------------------------------|
//! | live    | dead      | DOWN                                  |
//! | dead    | live      | UP                                    |
//! | dead    | dead      | both (switch unpowered, e.g. key out) |
//! | live    | live      | timer fallback (should not happen)    |
//!
//! Boards rev B/C buffer each line with an N-MOSFET and a pull-up (330k on rev C), so the
//! pins are **active low**: live line → pin LOW, dead line → pin HIGH, and
//! the wake level is LOW. [`Polarity`] converts between pin levels and line
//! states; the board's polarity is [`crate::config::SENSE_POLARITY`].

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Polarity {
    /// Pin high = line live (rev A: plain resistor divider).
    ActiveHigh,
    /// Pin low = line live (rev B: MOSFET buffer with pull-up).
    ActiveLow,
}

impl Polarity {
    /// Is the harness line live, given the pin level?
    pub const fn is_live(self, pin_high: bool) -> bool {
        match self {
            Polarity::ActiveHigh => pin_high,
            Polarity::ActiveLow => !pin_high,
        }
    }

    /// Pin level that means "live", i.e. the deep-sleep wake level.
    pub const fn wake_level_high(self) -> bool {
        matches!(self, Polarity::ActiveHigh)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WakePlan {
    /// Wake when any armed line goes live (pin reaches the wake level).
    Gpio { up: bool, down: bool },
    /// Neither line can be armed; wake after `ms` and decide again.
    Timer { ms: u32 },
}

/// Decide from the line states.
pub fn plan(up_live: bool, down_live: bool, fallback_ms: u32) -> WakePlan {
    match (up_live, down_live) {
        (true, false) => WakePlan::Gpio { up: false, down: true },
        (false, true) => WakePlan::Gpio { up: true, down: false },
        (false, false) => WakePlan::Gpio { up: true, down: true },
        (true, true) => WakePlan::Timer { ms: fallback_ms },
    }
}

/// Decide from the pin levels as read.
pub fn plan_from_pins(up_pin_high: bool, down_pin_high: bool, pol: Polarity, fallback_ms: u32) -> WakePlan {
    plan(pol.is_live(up_pin_high), pol.is_live(down_pin_high), fallback_ms)
}

impl WakePlan {
    /// Bit mask for `esp_deep_sleep_enable_gpio_wakeup`.
    pub fn gpio_mask(self, up_pin: u8, down_pin: u8) -> u64 {
        match self {
            WakePlan::Gpio { up, down } => {
                (if up { 1u64 << up_pin } else { 0 }) | (if down { 1u64 << down_pin } else { 0 })
            }
            WakePlan::Timer { .. } => 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::SENSE_POLARITY;

    #[test]
    fn table_by_line_state() {
        assert_eq!(plan(true, false, 60_000), WakePlan::Gpio { up: false, down: true });
        assert_eq!(plan(false, true, 60_000), WakePlan::Gpio { up: true, down: false });
        assert_eq!(plan(false, false, 60_000), WakePlan::Gpio { up: true, down: true });
        assert_eq!(plan(true, true, 60_000), WakePlan::Timer { ms: 60_000 });
    }

    #[test]
    fn board_is_active_low() {
        assert_eq!(SENSE_POLARITY, Polarity::ActiveLow);
        assert!(!SENSE_POLARITY.wake_level_high(), "rev B wakes on LOW");
    }

    #[test]
    fn active_low_pins() {
        let p = Polarity::ActiveLow;
        // UP pin low (UP live), DOWN pin high (dead): arm DOWN.
        assert_eq!(plan_from_pins(false, true, p, 1), WakePlan::Gpio { up: false, down: true });
        // UP pin high (dead), DOWN pin low (live): arm UP.
        assert_eq!(plan_from_pins(true, false, p, 1), WakePlan::Gpio { up: true, down: false });
        // Both pins high: switch unpowered, arm both.
        assert_eq!(plan_from_pins(true, true, p, 1), WakePlan::Gpio { up: true, down: true });
        // Both pins low: both lines live, timer fallback.
        assert_eq!(plan_from_pins(false, false, p, 60_000), WakePlan::Timer { ms: 60_000 });
    }

    #[test]
    fn active_high_pins() {
        let p = Polarity::ActiveHigh;
        assert_eq!(plan_from_pins(true, false, p, 1), WakePlan::Gpio { up: false, down: true });
        assert_eq!(plan_from_pins(false, false, p, 1), WakePlan::Gpio { up: true, down: true });
        assert_eq!(plan_from_pins(true, true, p, 1), WakePlan::Timer { ms: 1 });
        assert!(p.wake_level_high());
    }

    #[test]
    fn never_arms_a_pin_already_at_wake_level() {
        for pol in [Polarity::ActiveLow, Polarity::ActiveHigh] {
            for up_pin in [false, true] {
                for down_pin in [false, true] {
                    if let WakePlan::Gpio { up, down } = plan_from_pins(up_pin, down_pin, pol, 1) {
                        let wake = pol.wake_level_high();
                        assert!(!(up && up_pin == wake), "{pol:?} up armed at wake level");
                        assert!(!(down && down_pin == wake), "{pol:?} down armed at wake level");
                        assert!(up || down);
                    }
                }
            }
        }
    }

    #[test]
    fn masks() {
        assert_eq!(plan(true, false, 1).gpio_mask(0, 1), 0b10);
        assert_eq!(plan(false, true, 1).gpio_mask(0, 1), 0b01);
        assert_eq!(plan(false, false, 1).gpio_mask(0, 1), 0b11);
        assert_eq!(plan(true, true, 1).gpio_mask(0, 1), 0);
    }
}
