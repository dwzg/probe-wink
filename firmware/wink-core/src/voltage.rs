//! Supply-voltage guard for the relay coil.
//!
//! The app reads GPIO3 (ADC1_CH3, 12 dB attenuation) and passes the reading
//! in; this module converts it to the rail voltage and decides whether the
//! relay may be energized. Thresholds: [`crate::config::SUPPLY`].
//!
//! Rules:
//! - Energizing needs a reading no older than `max_age_ms` inside the
//!   *energize* band (`energize_under_mv(temp) ..= over_mv`). One reading
//!   outside it refuses energizing and latches a fault. The lower edge
//!   follows the coil's pull-in voltage at the die temperature
//!   ([`energize_under_mv`]).
//! - An energized relay is released after `release_after` readings in a row
//!   outside the wider *hold* band (`release_under_mv ..= release_over_mv`).
//!   ADC noise is about ±35 mV at the pin (±0.55 V on the rail), so one
//!   sample is not enough to cut a wink.
//! - A latched fault clears after `recover_ms` of readings inside the
//!   recovery band (`under_recover_mv ..= over_recover_mv`).
//! - A saturated ADC (raw or calibrated near full scale) counts as too high.

use crate::config::SupplySense;
use crate::Millis;

/// One ADC reading of the supply-sense pin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AdcReading {
    /// Raw 12-bit value (average of the samples).
    pub raw: u16,
    /// Calibrated pin voltage in mV.
    pub pin_mv: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Ok,
    Over,
    Under,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fault {
    Overvoltage,
    Undervoltage,
}

/// Rail voltage in mV for a reading, or `None` if the ADC is saturated.
pub fn rail_mv(cfg: &SupplySense, r: AdcReading) -> Option<u32> {
    if r.raw >= cfg.adc_raw_saturated || r.pin_mv >= cfg.adc_max_valid_mv {
        return None;
    }
    Some((r.pin_mv as u64 * cfg.ratio_den as u64 / cfg.ratio_num as u64) as u32)
}

fn verdict(rail: Option<u32>, under: u32, over: u32) -> Verdict {
    match rail {
        None => Verdict::Over,
        Some(mv) if mv > over => Verdict::Over,
        Some(mv) if mv < under => Verdict::Under,
        Some(_) => Verdict::Ok,
    }
}

/// Lowest rail voltage at which a relay may be switched on, for a die
/// temperature in °C (`None`: unknown, use the 85 °C ceiling `under_mv`).
pub fn energize_under_mv(cfg: &SupplySense, temp_c: Option<i32>) -> u32 {
    let Some(t) = temp_c else {
        return cfg.under_mv;
    };
    let above = (t - cfg.pull_in_ref_c).max(0) as u32;
    let pull_in = cfg.pull_in_ref_mv + cfg.pull_in_mv_per_c * above;
    (pull_in + cfg.pull_in_margin_mv).clamp(cfg.under_min_mv, cfg.under_mv)
}

/// Is the engine running (alternator charging)? `None` (saturated or no
/// reading) counts as running, so the drive lock errs towards refusing.
pub fn engine_running(cfg: &SupplySense, rail_mv: Option<u32>) -> bool {
    rail_mv.is_none_or(|mv| mv >= cfg.engine_running_mv)
}

/// May a relay be switched on at this reading? (energize band, temperature unknown)
pub fn classify_energize(cfg: &SupplySense, r: AdcReading) -> Verdict {
    verdict(rail_mv(cfg, r), cfg.under_mv, cfg.over_mv)
}

/// May an energized relay stay on at this reading? (hold band)
pub fn classify_hold(cfg: &SupplySense, r: AdcReading) -> Verdict {
    verdict(rail_mv(cfg, r), cfg.release_under_mv, cfg.release_over_mv)
}

#[derive(Debug, Clone)]
pub struct SupplyGuard {
    cfg: SupplySense,
    last_at: Option<Millis>,
    /// Rail voltage of the last reading; `None` = saturated.
    last_rail_mv: Option<u32>,
    bad_in_row: u8,
    fault: Option<Fault>,
    /// Die temperature, °C; `None` until the app reports one.
    temp_c: Option<i32>,
    /// Start of the current run of in-recovery-band readings while faulted.
    good_since: Option<Millis>,
}

fn as_fault(v: Verdict) -> Option<Fault> {
    match v {
        Verdict::Over => Some(Fault::Overvoltage),
        Verdict::Under => Some(Fault::Undervoltage),
        Verdict::Ok => None,
    }
}

impl SupplyGuard {
    pub const fn new(cfg: SupplySense) -> Self {
        Self { cfg, last_at: None, last_rail_mv: None, bad_in_row: 0, fault: None, good_since: None, temp_c: None }
    }

    /// Report the die temperature (°C), or `None` if the sensor failed.
    pub fn set_temp_c(&mut self, temp_c: Option<i32>) {
        self.temp_c = temp_c;
    }

    /// Current energize minimum, mV.
    pub fn under_mv(&self) -> u32 {
        energize_under_mv(&self.cfg, self.temp_c)
    }

    /// Should the app take a reading now? `energizing`: a relay is about to
    /// be switched on (forces a fresh reading if the last one is older than
    /// one loop tick).
    pub fn reading_due(&self, now: Millis, energizing: bool) -> bool {
        match self.last_at {
            None => true,
            Some(t) => {
                let age = now.saturating_sub(t);
                age >= self.cfg.interval_ms as Millis || (energizing && age >= 20)
            }
        }
    }

    /// Feed a reading. `energized`: a relay is currently on.
    pub fn update(&mut self, now: Millis, r: AdcReading, energized: bool) {
        let rail = rail_mv(&self.cfg, r);
        self.last_at = Some(now);
        self.last_rail_mv = rail;

        let under = self.under_mv();
        let c = &self.cfg;
        let v = if energized {
            verdict(rail, c.release_under_mv, c.release_over_mv)
        } else {
            verdict(rail, under, c.over_mv)
        };
        if v == Verdict::Ok {
            self.bad_in_row = 0;
        } else {
            self.bad_in_row = self.bad_in_row.saturating_add(1);
        }

        match self.fault {
            None => {
                let needed = if energized { self.cfg.release_after } else { 1 };
                if self.bad_in_row >= needed {
                    self.fault = as_fault(v);
                    self.good_since = None;
                }
            }
            Some(_) => {
                // A fresh bad reading may change the fault kind.
                if let Some(f) = as_fault(v) {
                    self.fault = Some(f);
                    self.good_since = None;
                    return;
                }
                let recover_under = under + (c.under_recover_mv - c.under_mv);
                let in_band = rail.is_some_and(|mv| mv >= recover_under && mv <= c.over_recover_mv);
                if !in_band {
                    self.good_since = None;
                    return;
                }
                let since = *self.good_since.get_or_insert(now);
                if now.saturating_sub(since) >= self.cfg.recover_ms as Millis {
                    self.fault = None;
                    self.good_since = None;
                    self.bad_in_row = 0;
                }
            }
        }
    }

    /// May a relay be on right now? `energized`: it already is.
    pub fn permits(&self, now: Millis, energized: bool) -> bool {
        if self.fault.is_some() {
            return false;
        }
        let Some(t) = self.last_at else {
            return false;
        };
        if now.saturating_sub(t) > self.cfg.max_age_ms as Millis {
            return false;
        }
        // Switching on needs the last reading inside the energize band; an
        // energized relay stays on until the hold rules latch a fault.
        energized || verdict(self.last_rail_mv, self.under_mv(), self.cfg.over_mv) == Verdict::Ok
    }

    pub fn fault(&self) -> Option<Fault> {
        self.fault
    }

    pub fn last_rail_mv(&self) -> Option<u32> {
        self.last_rail_mv
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::SUPPLY;

    /// Reading for a given rail voltage, as an ideal ADC would report it.
    fn at(rail_mv: u32) -> AdcReading {
        let pin = rail_mv as u64 * SUPPLY.ratio_num as u64 / SUPPLY.ratio_den as u64;
        let raw = (pin * 4095 / 2900).min(4095) as u16;
        AdcReading { raw, pin_mv: pin as u32 }
    }

    /// A guard that has seen a good reading at t=0.
    fn ready() -> SupplyGuard {
        let mut g = SupplyGuard::new(SUPPLY);
        g.update(0, at(12_600), false);
        g
    }

    #[test]
    fn rail_conversion() {
        // 1 MΩ / 68 kΩ: 0.8 V at the pin = 12.565 V on the rail.
        assert_eq!(rail_mv(&SUPPLY, AdcReading { raw: 1130, pin_mv: 800 }), Some(12_564));
        assert_eq!(rail_mv(&SUPPLY, AdcReading { raw: 1068, pin_mv: 68 * 12 }), Some(12_816));
        assert_eq!(rail_mv(&SUPPLY, AdcReading { raw: 4095, pin_mv: 800 }), None);
        assert_eq!(rail_mv(&SUPPLY, AdcReading { raw: 3000, pin_mv: 2450 }), None);
        // 24 V jump start is well inside the ADC range (1.53 V at the pin).
        assert_eq!(classify_energize(&SUPPLY, at(24_000)), Verdict::Over);
        assert!(rail_mv(&SUPPLY, at(24_000)).is_some());
    }

    #[test]
    fn energize_band() {
        // Temperature unknown: the 85 °C ceiling applies.
        assert_eq!(classify_energize(&SUPPLY, at(11_800)), Verdict::Under);
        assert_eq!(classify_energize(&SUPPLY, at(12_000)), Verdict::Ok);
        assert_eq!(classify_energize(&SUPPLY, at(12_600)), Verdict::Ok);
        assert_eq!(classify_energize(&SUPPLY, at(14_400)), Verdict::Ok);
        assert_eq!(classify_energize(&SUPPLY, at(16_400)), Verdict::Ok);
        assert_eq!(classify_energize(&SUPPLY, at(16_600)), Verdict::Over);
        assert_eq!(classify_energize(&SUPPLY, at(0)), Verdict::Under);
        assert_eq!(classify_energize(&SUPPLY, AdcReading { raw: 4095, pin_mv: 2900 }), Verdict::Over);
    }

    #[test]
    fn hold_band_is_wider() {
        assert_eq!(classify_hold(&SUPPLY, at(11_000)), Verdict::Ok);
        assert_eq!(classify_hold(&SUPPLY, at(9_100)), Verdict::Ok);
        assert_eq!(classify_hold(&SUPPLY, at(8_900)), Verdict::Under);
        assert_eq!(classify_hold(&SUPPLY, at(16_700)), Verdict::Ok);
        assert_eq!(classify_hold(&SUPPLY, at(16_900)), Verdict::Over);
        assert_eq!(classify_hold(&SUPPLY, AdcReading { raw: 4095, pin_mv: 2900 }), Verdict::Over);
    }

    #[test]
    fn no_reading_no_relay() {
        let g = SupplyGuard::new(SUPPLY);
        assert!(g.reading_due(0, false));
        assert!(!g.permits(0, false));
        assert!(!g.permits(0, true));
    }

    #[test]
    fn good_reading_permits_until_stale() {
        let mut g = SupplyGuard::new(SUPPLY);
        g.update(1000, at(12_500), false);
        assert!(g.permits(1000, false));
        assert!(g.permits(1000 + SUPPLY.max_age_ms as u64, false));
        assert!(!g.permits(1001 + SUPPLY.max_age_ms as u64, false));
        assert!(!g.reading_due(1050, false));
        assert!(g.reading_due(1100, false));
        assert!(g.reading_due(1020, true));
        assert!(!g.reading_due(1010, true));
    }

    #[test]
    fn one_bad_reading_refuses_energizing() {
        let mut g = SupplyGuard::new(SUPPLY);
        g.update(0, at(16_600), false);
        assert_eq!(g.fault(), Some(Fault::Overvoltage));
        assert!(!g.permits(0, false));

        let mut g = SupplyGuard::new(SUPPLY);
        g.update(0, at(11_400), false);
        assert_eq!(g.fault(), Some(Fault::Undervoltage));
        assert!(!g.permits(0, false));
    }

    #[test]
    fn supply_dip_while_energized_does_not_release() {
        // Motor current pulls the rail to 10.5 V during a wink: the coil
        // holds, the relay stays on, no fault.
        let mut g = ready();
        for t in (100..1500).step_by(100) {
            g.update(t, at(10_500), true);
            assert_eq!(g.fault(), None, "t={t}");
            assert!(g.permits(t, true));
            // ...but a relay that is off would not be switched on now.
            assert!(!g.permits(t, false));
        }
        // After the relay is released, the next idle reading latches the
        // undervoltage until the rail recovers.
        g.update(1600, at(10_500), false);
        assert_eq!(g.fault(), Some(Fault::Undervoltage));
    }

    #[test]
    fn between_energize_and_hold_limit_stays_on() {
        let mut g = ready();
        g.update(100, at(16_700), true);
        g.update(200, at(16_700), true);
        assert_eq!(g.fault(), None);
        assert!(g.permits(200, true));
        assert!(!g.permits(200, false));
    }

    #[test]
    fn energized_relay_needs_two_bad_readings_to_release() {
        let mut g = ready();
        g.update(100, at(17_500), true);
        assert_eq!(g.fault(), None);
        assert!(g.permits(100, true));
        g.update(200, at(17_500), true);
        assert_eq!(g.fault(), Some(Fault::Overvoltage));
        assert!(!g.permits(200, true));

        let mut g = ready();
        g.update(100, at(8_500), true);
        g.update(200, at(8_500), true);
        assert_eq!(g.fault(), Some(Fault::Undervoltage));
    }

    #[test]
    fn isolated_glitch_while_energized_is_tolerated() {
        let mut g = ready();
        g.update(100, at(18_000), true);
        g.update(200, at(12_500), true);
        g.update(300, at(18_000), true);
        assert_eq!(g.fault(), None);
        assert!(g.permits(300, true));
    }

    #[test]
    fn saturation_releases_like_overvoltage() {
        let mut g = ready();
        let sat = AdcReading { raw: 4095, pin_mv: 3100 };
        g.update(100, sat, true);
        g.update(200, sat, true);
        assert_eq!(g.fault(), Some(Fault::Overvoltage));
    }

    #[test]
    fn fault_recovers_only_after_hold_in_recovery_band() {
        let mut g = SupplyGuard::new(SUPPLY);
        g.update(0, at(17_000), false);
        // 16.3 V: inside the energize band but above the recovery band.
        g.update(100, at(16_300), false);
        g.update(2000, at(16_300), false);
        assert_eq!(g.fault(), Some(Fault::Overvoltage));
        // In band, but not long enough.
        g.update(2100, at(14_000), false);
        g.update(2500, at(14_000), false);
        assert_eq!(g.fault(), Some(Fault::Overvoltage));
        // An out-of-recovery-band reading restarts the hold.
        g.update(2550, at(16_200), false);
        g.update(2600, at(14_000), false);
        g.update(3000, at(14_000), false);
        assert_eq!(g.fault(), Some(Fault::Overvoltage));
        g.update(3100, at(14_000), false);
        assert_eq!(g.fault(), None);
        assert!(g.permits(3100, false));
    }

    #[test]
    fn undervoltage_recovers_above_12_2_without_temperature() {
        let mut g = SupplyGuard::new(SUPPLY);
        g.update(0, at(11_000), false);
        g.update(100, at(12_000), false); // energize-OK, below recovery
        g.update(1000, at(12_000), false);
        assert_eq!(g.fault(), Some(Fault::Undervoltage));
        g.update(1100, at(12_300), false);
        g.update(1599, at(12_300), false);
        assert_eq!(g.fault(), Some(Fault::Undervoltage));
        g.update(1600, at(12_300), false);
        assert_eq!(g.fault(), None);
    }

    #[test]
    fn engine_running_threshold() {
        assert!(!engine_running(&SUPPLY, Some(12_000))); // engine off, 12.8 V battery
        assert!(!engine_running(&SUPPLY, Some(12_400))); // surface charge after driving
        assert!(engine_running(&SUPPLY, Some(13_300)));  // charging, 14.1 V battery
        assert!(engine_running(&SUPPLY, None));          // no reading: refuse
    }

    #[test]
    fn energize_minimum_follows_temperature() {
        // Cold or room temperature: the 10.0 V floor (pull-in 9.0 V + 0.6 V margin is below it).
        assert_eq!(energize_under_mv(&SUPPLY, Some(-30)), 10_000);
        assert_eq!(energize_under_mv(&SUPPLY, Some(25)), 10_000);
        // 60 °C: 9.0 + 37 x 37 mV + 0.6 = 10.969 V.
        assert_eq!(energize_under_mv(&SUPPLY, Some(60)), 10_969);
        // 85 °C: 11.294 + 0.6 = 11.894 V, just under the ceiling.
        assert_eq!(energize_under_mv(&SUPPLY, Some(85)), 11_894);
        // Hotter than the model: clamped to the ceiling.
        assert_eq!(energize_under_mv(&SUPPLY, Some(110)), SUPPLY.under_mv);
        assert_eq!(energize_under_mv(&SUPPLY, None), SUPPLY.under_mv);
    }

    #[test]
    fn engine_off_battery_can_wink_at_room_temperature() {
        // 12.0 V battery minus the reverse diode (~0.85 V) = 11.15 V on the rail.
        let mut g = SupplyGuard::new(SUPPLY);
        g.set_temp_c(Some(25));
        g.update(0, at(11_150), false);
        assert!(g.permits(10, false));
        // The same rail with the box at 85 °C is refused: the coil may not pull in.
        let mut hot = SupplyGuard::new(SUPPLY);
        hot.set_temp_c(Some(85));
        hot.update(0, at(11_150), false);
        assert!(!hot.permits(10, false));
        assert_eq!(hot.fault(), Some(Fault::Undervoltage));
    }

    #[test]
    fn recovery_band_moves_with_the_minimum() {
        let mut g = SupplyGuard::new(SUPPLY);
        g.set_temp_c(Some(25));
        g.update(0, at(9_500), false);
        assert_eq!(g.fault(), Some(Fault::Undervoltage));
        // Recovery edge at 25 °C: 10.0 + 0.3 V.
        g.update(100, at(10_400), false);
        g.update(600, at(10_400), false);
        assert_eq!(g.fault(), None);
    }

    #[test]
    fn fault_kind_follows_latest_bad_reading() {
        let mut g = SupplyGuard::new(SUPPLY);
        g.update(0, at(11_000), false);
        g.update(100, at(20_000), false);
        assert_eq!(g.fault(), Some(Fault::Overvoltage));
    }
}
