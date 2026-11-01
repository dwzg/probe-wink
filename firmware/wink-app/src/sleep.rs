//! Deep sleep with GPIO wake on the command sense lines.

use esp_idf_svc::sys;
use log::{info, warn};
use wink_core::config::{SENSE_POLARITY, TIMING};
use wink_core::wake::{self, WakePlan};

use crate::hw::{Board, PIN_SENSE_DOWN, PIN_SENSE_UP};

/// Log why we woke up.
pub fn log_wake_cause() {
    let cause = unsafe { sys::esp_sleep_get_wakeup_cause() };
    #[allow(non_upper_case_globals)]
    let what = match cause {
        #[cfg(not(esp32))]
        sys::esp_sleep_source_t_ESP_SLEEP_WAKEUP_GPIO => {
            let mask = unsafe { sys::esp_sleep_get_gpio_wakeup_status() };
            info!("wake: GPIO, mask {mask:#x}");
            return;
        }
        #[cfg(esp32)]
        sys::esp_sleep_source_t_ESP_SLEEP_WAKEUP_EXT0 => "EXT0 (sense line)",
        #[cfg(esp32)]
        sys::esp_sleep_source_t_ESP_SLEEP_WAKEUP_EXT1 => {
            let mask = unsafe { sys::esp_sleep_get_ext1_wakeup_status() };
            info!("wake: EXT1, mask {mask:#x}");
            return;
        }
        sys::esp_sleep_source_t_ESP_SLEEP_WAKEUP_TIMER => "timer (fallback)",
        sys::esp_sleep_source_t_ESP_SLEEP_WAKEUP_UNDEFINED => "power-on or reset",
        _ => "other",
    };
    info!("wake: {what} ({cause})");
}

/// Relay off, LED off, arm the wake source and enter deep sleep.
pub fn enter(board: &mut Board) -> ! {
    board.set_relay(false);
    board.set_led(false);

    // Pin levels as read; rev B pins are active low (line live = LOW).
    let (up_pin, down_pin) = board.sense_pins();
    let plan = wake::plan_from_pins(up_pin, down_pin, SENSE_POLARITY, TIMING.wake_fallback_ms);
    info!(
        "sleep: SENSE_UP pin={} ({}), SENSE_DOWN pin={} ({}) -> {plan:?}",
        if up_pin { "high" } else { "low" },
        if SENSE_POLARITY.is_live(up_pin) { "live" } else { "dead" },
        if down_pin { "high" } else { "low" },
        if SENSE_POLARITY.is_live(down_pin) { "live" } else { "dead" },
    );
    // Wake when an armed (dead) line goes live: LOW on rev B/C.
    let wake_high = SENSE_POLARITY.wake_level_high();

    unsafe {
        match plan {
            WakePlan::Gpio { .. } => {
                let mask = plan.gpio_mask(PIN_SENSE_UP, PIN_SENSE_DOWN);
                // No internal pulls: the board's 330k pull-ups define the
                // level. ESP-IDF would otherwise enable a pull towards the
                // inactive level on every wake pin at sleep entry; that is
                // switched off with CONFIG_ESP_SLEEP_GPIO_ENABLE_INTERNAL_RESISTORS=n
                // in sdkconfig.defaults (the per-pin calls below cannot
                // override it, it runs inside esp_deep_sleep_start).
                let r = arm_gpio_wake(mask, wake_high);
                if r != sys::ESP_OK {
                    warn!("GPIO wake setup failed ({r}); using timer wake");
                    sys::esp_sleep_enable_timer_wakeup(TIMING.wake_fallback_ms as u64 * 1000);
                }
            }
            WakePlan::Timer { ms } => {
                sys::esp_sleep_enable_timer_wakeup(ms as u64 * 1000);
            }
        }
        // Keep the relay pin low through sleep in addition to the board's
        // gate pull-down. On the C3 a digital pad needs both calls to hold
        // in deep sleep. Released again at boot (hw::relay_off_early).
        sys::gpio_hold_en(crate::hw::PIN_RELAY);
        // The supply-sense pin sits at 0.7-1.1 V, near the logic threshold:
        // input buffer off so it cannot draw current through the sleep.
        // Unused pads keep their boot state (input off, pull-up).
        if let Some(pin) = crate::hw::PIN_VSENSE {
            sys::gpio_set_direction(pin, sys::gpio_mode_t_GPIO_MODE_DISABLE);
            sys::gpio_hold_en(pin);
        }
        for &pin in crate::hw::PINS_UNUSED {
            sys::gpio_hold_en(pin);
        }
        sys::gpio_deep_sleep_hold_en();
        sys::esp_deep_sleep_start();
    }
}

/// ESP32-C3: the deep-sleep GPIO wake, any armed pin at the wake level.
#[cfg(not(esp32))]
unsafe fn arm_gpio_wake(mask: u64, wake_high: bool) -> sys::esp_err_t {
    for pin in [PIN_SENSE_UP, PIN_SENSE_DOWN] {
        sys::gpio_pullup_dis(pin as _);
        sys::gpio_pulldown_dis(pin as _);
    }
    let mode = if wake_high {
        sys::esp_deepsleep_gpio_wake_up_mode_t_ESP_GPIO_WAKEUP_GPIO_HIGH
    } else {
        sys::esp_deepsleep_gpio_wake_up_mode_t_ESP_GPIO_WAKEUP_GPIO_LOW
    };
    sys::esp_deep_sleep_enable_gpio_wakeup(mask, mode)
}

/// ESP32 (devkit): no GPIO wake. EXT1 can only wake when *all* its pins are
/// low, so with two armed pins the first goes to EXT0 and the second to EXT1;
/// either one going low wakes the chip, like the C3's "any pin". The pins keep
/// their internal pull-ups through sleep (RTC peripherals stay powered).
#[cfg(esp32)]
unsafe fn arm_gpio_wake(mask: u64, wake_high: bool) -> sys::esp_err_t {
    if wake_high {
        warn!("devkit: only wake-on-LOW is implemented");
        return sys::ESP_ERR_NOT_SUPPORTED;
    }
    sys::esp_sleep_pd_config(sys::esp_sleep_pd_domain_t_ESP_PD_DOMAIN_RTC_PERIPH, sys::esp_sleep_pd_option_t_ESP_PD_OPTION_ON);
    let pins: Vec<i32> = (0..40).filter(|b| mask & (1u64 << b) != 0).collect();
    for &pin in &pins {
        sys::rtc_gpio_pullup_en(pin);
        sys::rtc_gpio_pulldown_dis(pin);
    }
    let mut r = sys::ESP_OK;
    if let Some(&first) = pins.first() {
        r = sys::esp_sleep_enable_ext0_wakeup(first, 0);
    }
    if r == sys::ESP_OK {
        if let Some(&second) = pins.get(1) {
            r = sys::esp_sleep_enable_ext1_wakeup(1u64 << second, sys::esp_sleep_ext1_wakeup_mode_t_ESP_EXT1_WAKEUP_ALL_LOW);
        }
    }
    r
}
