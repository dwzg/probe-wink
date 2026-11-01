//! Timing constants.
//!
//! Every figure that depends on the car lives here. The travel time T is not
//! known yet (docs/testing.md, "Measure on the car first": film one cycle and read it off the frames); 800 ms is a
//! placeholder. Change [`TIMING`] and rebuild; [`Timing::check`] runs at
//! compile time and rejects combinations that break the safety rules.

use crate::wake::Polarity;

/// All timing parameters of the system, in milliseconds unless noted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timing {
    /// T: stock travel time of one lamp, slower side and slower direction.
    pub travel_ms: u32,
    /// Pause at the far end of a wink before the relay drops.
    pub dwell_ms: u32,
    /// Configured alternation half-period P. The effective value is never
    /// below 1.3 × T, see [`Timing::alt_period_ms`].
    pub alt_period_cfg_ms: u32,
    /// Alternation cap in seconds: longer `ALT n` requests are clipped.
    pub alt_max_s: u32,
    /// Duration used by the remote's long press and by a bare `ALT`.
    pub alt_default_s: u32,
    /// Rest after an alternation run (finished or stopped); ALT is refused
    /// during it.
    pub rest_ms: u32,
    /// Master sends a heartbeat this often while the RH relay should be on.
    pub heartbeat_interval_ms: u32,
    /// Slave drops its relay if no heartbeat arrived for this long.
    pub heartbeat_timeout_ms: u32,
    /// Number of "relay off" frames the master sends after RH goes off.
    pub off_repeats: u8,
    /// Per-module backstop: a relay is never on longer than this in one go.
    pub relay_max_on_ms: u32,
    /// Go to deep sleep after this long without a command.
    pub awake_window_ms: u32,
    /// Timer wake used when both sense lines are live at sleep time (no line left to arm).
    pub wake_fallback_ms: u32,
    /// Master pings the slave this often while awake, so that the slave's
    /// own awake timer follows the master's.
    pub ping_interval_ms: u32,
}

/// The values the firmware is built with.
pub const TIMING: Timing = Timing {
    travel_ms: 800, // PLACEHOLDER: measure on the car (docs/testing.md)
    dwell_ms: 500,
    alt_period_cfg_ms: 1200,
    alt_max_s: 10,
    alt_default_s: 8,
    rest_ms: 20_000,
    heartbeat_interval_ms: 200,
    heartbeat_timeout_ms: 1_000,
    off_repeats: 3,
    relay_max_on_ms: 12_000,
    awake_window_ms: 15 * 60 * 1000,
    wake_fallback_ms: 60_000,
    ping_interval_ms: 5_000,
};

const _: () = match TIMING.check() {
    Ok(()) => (),
    Err(_) => panic!("config::TIMING violates a safety rule, see Timing::check"),
};

/// Press-pattern timing for the BLE remote.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RemoteTiming {
    /// A press held at least this long is a long press.
    pub long_press_ms: u32,
    /// After a release, wait this long for another press before deciding
    /// how many presses the pattern had.
    pub gap_ms: u32,
    /// A press that follows a release within this time is the same press
    /// (some remotes send two HID reports per button, e.g. volume-up and
    /// Enter, each with its own press and release).
    pub merge_ms: u32,
}

pub const REMOTE_TIMING: RemoteTiming = RemoteTiming {
    long_press_ms: 700,
    gap_ms: 400,
    merge_ms: 60,
};

/// Wi-Fi channel used by ESP-NOW on both modules.
pub const ESPNOW_CHANNEL: u8 = 1;

/// Polarity of the SENSE_UP / SENSE_DOWN pins. Board rev B: N-MOSFET buffer
/// with a 330k pull-up, harness line live → pin LOW.
pub const SENSE_POLARITY: Polarity = Polarity::ActiveLow;

/// Supply-voltage check for the relay coil.
///
/// The module's +12 V rail (after the reverse-polarity diode; the rail that
/// feeds the relay coil) reaches GPIO3 / ADC1_CH3 through 1 MΩ / 68 kΩ
/// (boards rev B/C). Relay: TE IM06GR, 12 V, 1029 Ω. Its pull-in voltage is
/// 9.0 V max at 23 °C and rises with coil temperature (copper resistance) to
/// about 11.3 V at 85 °C; it allows about 20 V on the coil at 85 °C (16.8 V
/// worst case at 1 A contact current).
///
/// The energize minimum follows the die temperature of the C3 (internal
/// sensor; the die runs a few °C above the air in the box, which errs on the
/// safe side): pull-in at that temperature plus a margin, clamped to
/// `under_min_mv ..= under_mv`. Without a temperature it is `under_mv`, the
/// 85 °C value. A fixed 11.5 V refused winks with the engine off: the rail
/// sits one diode drop (about 0.85 V) below the battery.
///
/// Two sets of limits:
/// - *energize* (`under_mv` ..= `over_mv`): a relay is only switched on
///   inside this band; one reading outside refuses it and latches a fault;
/// - *hold* (`release_under_mv` ..= `release_over_mv`): an energized relay
///   is released only outside this wider band, after `release_after` bad
///   readings in a row. A pulled-in coil holds well below its pull-in
///   voltage, and a release mid-travel stops the lamp part-way, so the
///   motor's own supply dip during a wink should not cut it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SupplySense {
    /// Divider ratio numerator (bottom resistor, kΩ).
    pub ratio_num: u32,
    /// Divider ratio denominator (top + bottom, kΩ).
    pub ratio_den: u32,
    /// Refuse to energize above this rail voltage ("overvoltage").
    pub over_mv: u32,
    /// Refuse to energize below this rail voltage ("undervoltage") when the
    /// temperature is unknown; also the ceiling of the temperature-scaled
    /// minimum.
    pub under_mv: u32,
    /// Floor of the temperature-scaled energize minimum.
    pub under_min_mv: u32,
    /// Coil pull-in model: `pull_in_ref_mv` at `pull_in_ref_c` and below,
    /// rising `pull_in_mv_per_c` per °C above it.
    pub pull_in_ref_mv: u32,
    pub pull_in_ref_c: i32,
    pub pull_in_mv_per_c: u32,
    /// Added to the modelled pull-in voltage.
    pub pull_in_margin_mv: u32,
    /// Release an energized relay above this.
    pub release_over_mv: u32,
    /// Release an energized relay below this.
    pub release_under_mv: u32,
    /// A fault clears only after `recover_ms` of readings inside
    /// `under_recover_mv ..= over_recover_mv`. The lower edge moves with the
    /// energize minimum (it keeps its distance to `under_mv`).
    pub under_recover_mv: u32,
    pub over_recover_mv: u32,
    /// Calibrated pin readings at or above this count as saturated (the
    /// 12 dB range is specified to 2500 mV), which means "too high".
    pub adc_max_valid_mv: u32,
    /// Raw 12-bit readings at or above this count as saturated.
    pub adc_raw_saturated: u16,
    /// A fault clears only after this long of in-band readings.
    pub recover_ms: u32,
    /// Read this often (while awake; the relay loop needs ~100 ms).
    pub interval_ms: u32,
    /// The relay may only be energized with a reading at most this old.
    pub max_age_ms: u32,
    /// Bad readings in a row (hold band) that release an energized relay.
    /// One reading outside the energize band refuses energizing.
    pub release_after: u8,
    /// ADC samples averaged per reading.
    pub samples: u8,
}

pub const SUPPLY: SupplySense = SupplySense {
    ratio_num: 68,
    ratio_den: 1068,
    over_mv: 16_500,
    under_mv: 11_900,
    under_min_mv: 10_000,
    pull_in_ref_mv: 9_000,
    pull_in_ref_c: 23,
    pull_in_mv_per_c: 37,
    pull_in_margin_mv: 600,
    release_over_mv: 16_800, // datasheet worst case (85 °C, 1 A contacts)
    release_under_mv: 9_000, // 75 % of nominal; coil holds, see above
    under_recover_mv: 12_200,
    over_recover_mv: 16_000,
    adc_max_valid_mv: 2_450,
    adc_raw_saturated: 4_000,
    recover_ms: 500,
    interval_ms: 100,
    max_age_ms: 250,
    release_after: 2,
    samples: 8,
};

const _: () = {
    let s = SUPPLY;
    assert!(s.release_under_mv <= s.under_min_mv);
    assert!(s.under_min_mv <= s.under_mv);
    // The model at 85 °C must fit under the ceiling.
    assert!(s.pull_in_ref_mv + s.pull_in_mv_per_c * (85 - s.pull_in_ref_c) as u32 + s.pull_in_margin_mv <= s.under_mv);
    assert!(s.under_mv < s.under_recover_mv);
    assert!(s.under_recover_mv < s.over_recover_mv);
    assert!(s.over_recover_mv < s.over_mv);
    assert!(s.over_mv <= s.release_over_mv);
    // The highest threshold must be measurable below ADC saturation.
    assert!(
        (s.release_over_mv as u64 * s.ratio_num as u64 / (s.ratio_den as u64)) < s.adc_max_valid_mv as u64
    );
    assert!(s.release_after >= 1 && s.samples >= 1);
    assert!(s.max_age_ms > s.interval_ms);
};

/// Parse the 6-digit BLE passkey at compile time (from `WINK_BLE_PASSKEY`).
/// Panics (= compile error in a `const`) unless it is exactly six digits.
pub const fn parse_passkey(s: &str) -> u32 {
    let b = s.as_bytes();
    if b.len() != 6 {
        panic!("WINK_BLE_PASSKEY must be exactly 6 digits");
    }
    let mut v = 0u32;
    let mut i = 0;
    while i < 6 {
        if !b[i].is_ascii_digit() {
            panic!("WINK_BLE_PASSKEY must be exactly 6 digits");
        }
        v = v * 10 + (b[i] - b'0') as u32;
        i += 1;
    }
    v
}

/// Reasons [`Timing::check`] rejects a configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigError {
    /// T must be positive.
    ZeroTravel,
    /// T + dwell does not fit in the relay on-time limit.
    WinkTooLong,
    /// The alternation half-period does not fit in the relay on-time limit.
    PeriodTooLong,
    /// The heartbeat timeout must be several heartbeat intervals long.
    HeartbeatTooTight,
    /// The default alternation is longer than the cap.
    AltDefaultOverCap,
    /// Remote timing inconsistent (merge window must be shorter than the gap).
    RemoteTiming,
}

impl Timing {
    /// Wink hold: T + dwell.
    pub const fn wink_hold_ms(&self) -> u32 {
        self.travel_ms + self.dwell_ms
    }

    /// Effective alternation half-period P = max(configured, ceil(1.3 × T)).
    pub const fn alt_period_ms(&self) -> u32 {
        let min = (self.travel_ms * 13).div_ceil(10);
        if self.alt_period_cfg_ms > min {
            self.alt_period_cfg_ms
        } else {
            min
        }
    }

    /// Validate the safety relations between the constants.
    pub const fn check(&self) -> Result<(), ConfigError> {
        if self.travel_ms == 0 {
            return Err(ConfigError::ZeroTravel);
        }
        if self.wink_hold_ms() >= self.relay_max_on_ms {
            return Err(ConfigError::WinkTooLong);
        }
        if self.alt_period_ms() >= self.relay_max_on_ms {
            return Err(ConfigError::PeriodTooLong);
        }
        if self.heartbeat_timeout_ms < 3 * self.heartbeat_interval_ms {
            return Err(ConfigError::HeartbeatTooTight);
        }
        if self.alt_default_s > self.alt_max_s {
            return Err(ConfigError::AltDefaultOverCap);
        }
        if REMOTE_TIMING.merge_ms >= REMOTE_TIMING.gap_ms {
            return Err(ConfigError::RemoteTiming);
        }
        Ok(())
    }

    /// Clamp a requested alternation length to 1..=cap seconds.
    pub const fn clamp_alt_secs(&self, secs: u32) -> u32 {
        if secs == 0 {
            1
        } else if secs > self.alt_max_s {
            self.alt_max_s
        } else {
            secs
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_is_valid() {
        assert_eq!(TIMING.check(), Ok(()));
    }

    #[test]
    fn wink_hold_is_t_plus_dwell() {
        assert_eq!(TIMING.wink_hold_ms(), 1300);
    }

    #[test]
    fn period_never_below_1_3_t() {
        let t = Timing { travel_ms: 1000, alt_period_cfg_ms: 500, ..TIMING };
        assert_eq!(t.alt_period_ms(), 1300);
        let t = Timing { travel_ms: 1001, alt_period_cfg_ms: 0, ..TIMING };
        assert_eq!(t.alt_period_ms(), 1302); // ceil(1301.3)
        let t = Timing { travel_ms: 800, alt_period_cfg_ms: 2000, ..TIMING };
        assert_eq!(t.alt_period_ms(), 2000);
    }

    #[test]
    fn check_rejects_unsafe_values() {
        assert_eq!(Timing { travel_ms: 0, ..TIMING }.check(), Err(ConfigError::ZeroTravel));
        assert_eq!(
            Timing { travel_ms: 11_600, dwell_ms: 500, ..TIMING }.check(),
            Err(ConfigError::WinkTooLong)
        );
        assert_eq!(
            Timing { travel_ms: 1000, alt_period_cfg_ms: 12_000, ..TIMING }.check(),
            Err(ConfigError::PeriodTooLong)
        );
        assert_eq!(
            Timing { heartbeat_timeout_ms: 500, ..TIMING }.check(),
            Err(ConfigError::HeartbeatTooTight)
        );
        assert_eq!(
            Timing { alt_default_s: 11, ..TIMING }.check(),
            Err(ConfigError::AltDefaultOverCap)
        );
    }

    #[test]
    fn passkey() {
        assert_eq!(parse_passkey("004711"), 4711);
        assert_eq!(parse_passkey("999999"), 999_999);
    }

    #[test]
    #[should_panic]
    fn passkey_rejects_short() {
        parse_passkey("12345");
    }

    #[test]
    #[should_panic]
    fn passkey_rejects_letters() {
        parse_passkey("12345a");
    }

    #[test]
    fn alt_secs_clamped() {
        assert_eq!(TIMING.clamp_alt_secs(0), 1);
        assert_eq!(TIMING.clamp_alt_secs(5), 5);
        assert_eq!(TIMING.clamp_alt_secs(10), 10);
        assert_eq!(TIMING.clamp_alt_secs(11), 10);
        assert_eq!(TIMING.clamp_alt_secs(u32::MAX), 10);
    }
}
