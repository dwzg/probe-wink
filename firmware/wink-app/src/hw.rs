//! Board pins. Two targets share this code:
//!
//! **ESP32-C3-WROOM-02 custom board (rev C)**, the product:
//!
//! | GPIO | Use                                                        |
//! |------|------------------------------------------------------------|
//! | 0    | SENSE_UP, digital in, ACTIVE LOW (MOSFET buffer, 330k pull-up), deep-sleep wake |
//! | 1    | SENSE_DOWN, digital in, active low (same), deep-sleep wake |
//! | 3    | VSENSE, ADC1_CH3, module +12 V via 1 MΩ / 68 kΩ            |
//! | 4, 5 | not connected (rev C); reset to input-off + pull-up, held in sleep |
//! | 6    | SIDE, in with pull-up: high = LH/master, low = RH/slave    |
//! | 7    | status LED, out, active high (rev C; GPIO5 on rev B)       |
//! | 10   | RELAY, out, high = coil on (NPN, 100k base pull-down; rev C, GPIO4 on rev B) |
//!
//! Die temperature: the C3's internal sensor, for the relay's pull-in limit.
//!
//! **ESP32 DEVKITV1 (ESP-WROOM-32) test bench**, built with `MCU=esp32`
//! (`scripts/devkit`). No relay, buffers or divider; for BLE, ESP-NOW and
//! control-logic tests:
//!
//! | GPIO | Use                                                        |
//! |------|------------------------------------------------------------|
//! | 2    | "RELAY": the onboard blue LED (lit = relay would be on)    |
//! | 4    | status LED (optional, external LED + resistor to GND)      |
//! | 27   | SIDE, internal pull-up: open = LH/master, to GND = RH/slave |
//! | 32   | SENSE_UP, internal pull-up: jumper to GND = line live      |
//! | 33   | SENSE_DOWN, same                                           |
//!
//! The supply reading is simulated at 12.6 V (no divider on the devkit), and
//! the ESP32 has no temperature sensor in ESP-IDF 5, so the relay limit uses
//! its 85 °C value.

use esp_idf_svc::hal::gpio::{AnyIOPin, Input, Output, PinDriver, Pull};
use esp_idf_svc::sys::{self, EspError};
use log::warn;
#[cfg(not(esp32))]
use wink_core::config::SUPPLY;
use wink_core::voltage::AdcReading;

#[cfg(not(esp32))]
mod pins {
    pub const SENSE_UP: u8 = 0;
    pub const SENSE_DOWN: u8 = 1;
    pub const RELAY: i32 = 10;
    pub const VSENSE: Option<i32> = Some(3);
    /// Module pins 3 and 4 (GPIO4, GPIO5) are not connected on rev C.
    pub const UNUSED: &[i32] = &[4, 5];
    /// The board has external pull-ups on the sense lines.
    pub const SENSE_PULL: super::Pull = super::Pull::Floating;
}

#[cfg(esp32)]
mod pins {
    pub const SENSE_UP: u8 = 32;
    pub const SENSE_DOWN: u8 = 33;
    pub const RELAY: i32 = 2;
    pub const VSENSE: Option<i32> = None;
    pub const UNUSED: &[i32] = &[];
    /// Nothing on the devkit's pins: internal pull-ups, a jumper to GND = live.
    pub const SENSE_PULL: super::Pull = super::Pull::Up;
}

pub const PIN_SENSE_UP: u8 = pins::SENSE_UP;
pub const PIN_SENSE_DOWN: u8 = pins::SENSE_DOWN;
pub const PIN_RELAY: i32 = pins::RELAY;
pub const PIN_VSENSE: Option<i32> = pins::VSENSE;
pub const PINS_UNUSED: &[i32] = pins::UNUSED;
/// Re-read the die temperature at most this often.
#[cfg(esp_idf_soc_temp_sensor_supported)]
const TEMP_INTERVAL_US: i64 = 2_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    /// LH module, master.
    Lh,
    /// RH module, slave.
    Rh,
}

/// Drive the relay line low before anything else runs. The board's 100k
/// pull-down already holds it off; this makes the output state explicit as
/// early as possible.
pub fn relay_off_early() {
    unsafe {
        sys::gpio_set_level(PIN_RELAY, 0);
        sys::gpio_set_direction(PIN_RELAY, sys::gpio_mode_t_GPIO_MODE_OUTPUT);
        sys::gpio_set_level(PIN_RELAY, 0);
        sys::gpio_pullup_dis(PIN_RELAY);
        sys::gpio_pulldown_en(PIN_RELAY);
        // Released only now that the output latch is low (sleep::enter
        // holds the pin low through deep sleep).
        sys::gpio_hold_dis(PIN_RELAY);
        sys::gpio_deep_sleep_hold_dis();
        // Unconnected pads: input buffer off, pull-up on (no current path).
        for &pin in PINS_UNUSED {
            sys::gpio_hold_dis(pin);
            sys::gpio_reset_pin(pin);
        }
        if let Some(pin) = PIN_VSENSE {
            sys::gpio_hold_dis(pin);
        }
    }
}

/// Read the side jumper once at boot.
pub fn read_side(pin: AnyIOPin<'static>) -> Result<Side, EspError> {
    let p = PinDriver::input(pin, Pull::Up)?;
    esp_idf_svc::hal::delay::Ets::delay_us(200);
    let side = if p.is_high() { Side::Lh } else { Side::Rh };
    // Not needed after boot. Dropping the driver resets the pin without
    // pull, so a bridged jumper draws nothing through the pull-up.
    drop(p);
    Ok(side)
}

#[cfg(not(esp32))]
mod supply {
    use esp_idf_svc::hal::adc::attenuation::DB_12;
    use esp_idf_svc::hal::adc::oneshot::config::{AdcChannelConfig, Calibration};
    use esp_idf_svc::hal::adc::oneshot::{AdcChannelDriver, AdcDriver};
    use esp_idf_svc::hal::adc::{ADC1, ADCU1};
    use esp_idf_svc::hal::gpio::{ADCPin, Gpio3};
    use esp_idf_svc::sys::EspError;
    use log::warn;

    pub type Adc = AdcChannelDriver<'static, <Gpio3<'static> as ADCPin>::AdcChannel, AdcDriver<'static, ADCU1>>;

    pub fn new(adc1: ADC1<'static>, gpio3: Gpio3<'static>) -> Result<Adc, EspError> {
        let adc = AdcDriver::new(adc1)?;
        let mut cfg = AdcChannelConfig { attenuation: DB_12, calibration: Calibration::Curve, ..Default::default() };
        // Curve fitting needs the eFuse calibration data. Without it the
        // driver fails; fall back to the uncalibrated conversion (then the
        // thresholds are off by up to a few percent).
        match AdcChannelDriver::new(adc, gpio3, &cfg) {
            Ok(d) => Ok(d),
            Err(e) => {
                warn!("ADC curve calibration unavailable ({e}); using uncalibrated readings");
                cfg.calibration = Calibration::None;
                // The first attempt consumed the driver and pin; steal them
                // back (nothing else uses ADC1 or GPIO3).
                let adc = AdcDriver::new(unsafe { ADC1::steal() })?;
                AdcChannelDriver::new(adc, unsafe { Gpio3::steal() }, &cfg)
            }
        }
    }
}

/// What `Board::new` needs from the chip's peripherals for the supply sense.
#[cfg(not(esp32))]
pub type SupplyPins = (esp_idf_svc::hal::adc::ADC1<'static>, esp_idf_svc::hal::gpio::Gpio3<'static>);
#[cfg(esp32)]
pub type SupplyPins = ();

pub struct Board {
    relay: PinDriver<'static, Output>,
    led: PinDriver<'static, Output>,
    sense_up: PinDriver<'static, Input>,
    sense_down: PinDriver<'static, Input>,
    #[cfg(not(esp32))]
    supply: supply::Adc,
    relay_on: bool,
    #[cfg(esp_idf_soc_temp_sensor_supported)]
    tsens: Option<sys::temperature_sensor_handle_t>,
    #[cfg(esp_idf_soc_temp_sensor_supported)]
    temp: Option<(i64, i32)>,
}

/// Start the internal temperature sensor (range 20..100 °C, the band that
/// matters for the relay). `None` if it fails; the supply guard then uses
/// its 85 °C limit.
#[cfg(esp_idf_soc_temp_sensor_supported)]
fn temp_sensor() -> Option<sys::temperature_sensor_handle_t> {
    let cfg = sys::temperature_sensor_config_t {
        range_min: 20,
        range_max: 100,
        clk_src: sys::soc_periph_temperature_sensor_clk_src_t_TEMPERATURE_SENSOR_CLK_SRC_DEFAULT,
        ..Default::default()
    };
    let mut h: sys::temperature_sensor_handle_t = core::ptr::null_mut();
    unsafe {
        if sys::temperature_sensor_install(&cfg, &mut h) != sys::ESP_OK {
            warn!("temperature sensor install failed; relay limit uses 85 °C");
            return None;
        }
        if sys::temperature_sensor_enable(h) != sys::ESP_OK {
            warn!("temperature sensor enable failed; relay limit uses 85 °C");
            return None;
        }
    }
    Some(h)
}

impl Board {
    pub fn new(
        sense_up: AnyIOPin<'static>,
        sense_down: AnyIOPin<'static>,
        relay: AnyIOPin<'static>,
        led: AnyIOPin<'static>,
        supply_pins: SupplyPins,
    ) -> Result<Self, EspError> {
        let mut relay = PinDriver::output(relay)?;
        relay.set_low()?;
        let mut led = PinDriver::output(led)?;
        led.set_low()?;
        let sense_up = PinDriver::input(sense_up, pins::SENSE_PULL)?;
        let sense_down = PinDriver::input(sense_down, pins::SENSE_PULL)?;
        #[cfg(not(esp32))]
        let supply = supply::new(supply_pins.0, supply_pins.1)?;
        #[cfg(esp32)]
        {
            let () = supply_pins;
            warn!("devkit build: supply reading simulated at 12.6 V, no temperature sensor");
        }

        Ok(Self {
            relay,
            led,
            sense_up,
            sense_down,
            #[cfg(not(esp32))]
            supply,
            relay_on: false,
            #[cfg(esp_idf_soc_temp_sensor_supported)]
            tsens: temp_sensor(),
            #[cfg(esp_idf_soc_temp_sensor_supported)]
            temp: None,
        })
    }

    pub fn set_relay(&mut self, on: bool) {
        if on == self.relay_on {
            return;
        }
        let r = if on { self.relay.set_high() } else { self.relay.set_low() };
        if let Err(e) = r {
            // Cannot happen for a configured output; make sure it is off.
            warn!("relay write failed: {e}");
            unsafe { sys::gpio_set_level(PIN_RELAY, 0) };
            self.relay_on = false;
            return;
        }
        self.relay_on = on;
    }

    pub fn relay_on(&self) -> bool {
        self.relay_on
    }

    pub fn set_led(&mut self, on: bool) {
        let _ = if on { self.led.set_high() } else { self.led.set_low() };
    }

    /// Raw pin levels (SENSE_UP high, SENSE_DOWN high). Active low: a live
    /// harness line reads LOW; see `config::SENSE_POLARITY`.
    pub fn sense_pins(&self) -> (bool, bool) {
        (self.sense_up.is_high(), self.sense_down.is_high())
    }

    /// Die temperature in °C (cached for `TEMP_INTERVAL_US`), `None` if the
    /// chip has no sensor or it failed.
    #[cfg(esp_idf_soc_temp_sensor_supported)]
    pub fn die_temp_c(&mut self) -> Option<i32> {
        let h = self.tsens?;
        let now = unsafe { sys::esp_timer_get_time() };
        if let Some((at, t)) = self.temp {
            if now - at < TEMP_INTERVAL_US {
                return Some(t);
            }
        }
        let mut c: f32 = 0.0;
        if unsafe { sys::temperature_sensor_get_celsius(h, &mut c) } != sys::ESP_OK {
            warn!("temperature read failed");
            self.temp = None;
            return None;
        }
        let t = c.round() as i32;
        self.temp = Some((now, t));
        Some(t)
    }

    #[cfg(not(esp_idf_soc_temp_sensor_supported))]
    pub fn die_temp_c(&mut self) -> Option<i32> {
        None
    }

    /// Average `SUPPLY.samples` ADC readings of the supply-sense pin.
    #[cfg(not(esp32))]
    pub fn read_supply(&mut self) -> Result<AdcReading, EspError> {
        let n = SUPPLY.samples.max(1) as u32;
        let mut raw_sum = 0u32;
        let mut mv_sum = 0u32;
        for _ in 0..n {
            let raw = self.supply.read_raw()?;
            raw_sum += raw as u32;
            mv_sum += self.supply.raw_to_mv(raw)? as u32;
        }
        Ok(AdcReading { raw: (raw_sum / n) as u16, pin_mv: mv_sum / n })
    }

    /// Devkit: a steady 12.6 V rail, as the custom board's divider would
    /// report it.
    #[cfg(esp32)]
    pub fn read_supply(&mut self) -> Result<AdcReading, EspError> {
        let c = wink_core::config::SUPPLY;
        let pin_mv = 12_600 * c.ratio_num / c.ratio_den;
        Ok(AdcReading { raw: (pin_mv * 4095 / 2900) as u16, pin_mv })
    }
}
