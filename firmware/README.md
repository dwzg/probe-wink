# probe-wink firmware

Rust (std) firmware for the two headlamp modules, on ESP-IDF v5.5.5 via
`esp-idf-svc`. One binary for both sides: the SIDE jumper on GPIO6 picks
LH/master or RH/slave at boot.

**Status: v1.** The pure logic in `wink-core` is unit-tested on the host. The
firmware runs on an ESP32 DEVKITV1 test bench (see below): it boots as master,
brings up ESP-NOW, advertises over BLE and accepts a bonded phone. The custom
ESP32-C3 board, the relay path and the car are untested; see [Untested](#untested).

## Layout

| Path                         | Contents                                                       |
|------------------------------|----------------------------------------------------------------|
| `wink-core/`                 | `no_std` logic, no ESP dependencies, host unit tests           |
| `wink-core/src/config.rs`    | **all timing and voltage constants**                           |
| `wink-app/`                  | ESP-IDF binary: GPIO, ADC, BLE (NimBLE), ESP-NOW, sleep, watchdog |
| `wink-app/sdkconfig.defaults`| ESP-IDF configuration (NimBLE roles, watchdog, console, …)     |
| `wink-app/partitions.csv`    | 4 MB layout with two OTA slots                                 |
| `espflash.toml`              | makes `espflash` / `cargo run` use that partition table        |
| `.cargo/config.toml`         | target, linker, runner, `build-std`, ESP-IDF version           |

`wink-core` modules: `command` (text parser), `remote` (press patterns, HID
report → button), `controller` (wink / alt / stop / rest), `safety` (on-time
limit, heartbeat watchdog and sender), `voltage` (coil supply guard), `awake`
(15 min window), `wake` (which sense line to arm), `proto` (authenticated
ESP-NOW frames, replay protection).

## Toolchain

- Rust nightly with `rust-src` (pinned to `nightly` by `rust-toolchain.toml`;
  built here with rustc 1.101.0-nightly 2026-10-16). The C3 is RISC-V, so the
  board needs no Espressif Xtensa toolchain; only the ESP32 devkit test bench
  does (see below).
- `cargo install ldproxy` (linker wrapper, required)
- `cargo install espflash --locked` (flashing / monitor; built fine here,
  v4.7.0)
- Python 3 with `venv`, `git`, and `libclang` (for bindgen) on the host. The
  first build downloads ESP-IDF v5.5.5 and its tools into `firmware/.embuild/`
  (about 5.5 GB; ignored by git) and takes a few minutes.

## Secrets (required at build time)

The build fails unless both are set:

| Variable           | Format              | Used for                                  |
|--------------------|---------------------|-------------------------------------------|
| `WINK_ESPNOW_KEY`  | 64 hex chars (32 B) | HMAC key of the master ↔ slave link       |
| `WINK_BLE_PASSKEY` | 6 digits            | passkey the phone enters when pairing     |

```sh
export WINK_ESPNOW_KEY=$(openssl rand -hex 32)
export WINK_BLE_PASSKEY=$(python3 -c 'import secrets; print(f"{secrets.randbelow(10**6):06}")')
```

Keep both somewhere safe (e.g. a password manager or a git-ignored
`.envrc`). **Both modules must be flashed from a build with the same key.**
A key with all bytes equal is rejected at compile time. Changing either value
triggers a rebuild of `wink-app` only.

## Build, test, flash

All commands from `firmware/`.

```sh
# host unit tests of the logic (93 tests)
cargo test -p wink-core --target x86_64-unknown-linux-gnu

# firmware (default target riscv32imc-esp-espidf from .cargo/config.toml)
cargo build -p wink-app             # debug, ~1.5 MB image
cargo build -p wink-app --release   # size-optimized

# flash + serial monitor over the native USB (GPIO18/19)
cargo run -p wink-app --release
# or explicitly
espflash flash --monitor target/riscv32imc-esp-espidf/release/wink-app
espflash monitor
```

`--target x86_64-unknown-linux-gnu` is needed for the tests because the
workspace's default target is the ESP32-C3; `build-std` also applies, so the
first test run compiles `std` for the host once.

`espflash.toml` points espflash at `wink-app/partitions.csv` and writes the
app to `ota_0`. The path is relative, so run espflash from `firmware/`. On a
chip that has ever booted from `ota_1` (after OTA exists), add
`--erase-parts otadata` so the bootloader goes back to `ota_0`. Without the
partition table espflash would use its own single-app layout, which also
works for v1 but would need reflashing once OTA exists.

Flash one module with the SIDE jumper open (LH, master) and one with it
bridged (RH, slave). Same image.

## ESP32 DEVKITV1 test bench

The same firmware builds for an ESP32 DEVKITV1 (ESP-WROOM-32, Xtensa) to test
BLE (phone page, passkey pairing, bonding), the BLE remote and the control logic
before the custom board exists. The chip is selected by `MCU`; the code switches
on the `esp32` / `esp32c3` cfg flags that esp-idf-sys emits (`hw.rs`, `sleep.rs`),
and `wink-app/sdkconfig.defaults.esp32` is layered on top of the shared defaults.

```sh
espup install --targets esp32       # once: Xtensa Rust toolchain ("esp") + ~/export-esp.sh
export WINK_ESPNOW_KEY=... WINK_BLE_PASSKEY=...
scripts/devkit build
scripts/devkit flash /dev/ttyUSB0   # flash + serial monitor (CP2102/CH340 on the devkit)
```

| Devkit pin | Role |
|---|---|
| GPIO2 (onboard blue LED) | "relay": lit while the relay would be energized |
| GPIO4 | status LED (optional: LED + 1k to GND) |
| GPIO27 | SIDE: open = LH/master (the phone connects only to the master), to GND = RH/slave |
| GPIO32 / GPIO33 | SENSE_UP / SENSE_DOWN with internal pull-ups; a jumper to GND = line live |

Differences from the board: the supply reading is simulated at 12.6 V, there is
no temperature sensor (the relay limit uses its 85 °C value, which 12.6 V passes),
and deep sleep wakes through EXT0/EXT1 instead of the C3's GPIO wake. Not
testable here: the relay driver, sense buffers, power path and sleep current.
ESP-NOW master/slave needs a second board.

## Pins (ESP32-C3-WROOM-02 board, hardware rev C)

| GPIO  | Name       | Direction        | Notes                                                        |
|-------|------------|------------------|--------------------------------------------------------------|
| 0     | SENSE_UP   | digital in, **active low**, no internal pull | stock UP line via N-MOSFET buffer + 330k pull-up to 3V3: line live → LOW, dead → HIGH (switching point about 1.4–7 V on the line over part spread and temperature, so 12 V is solidly live); deep-sleep wake on LOW |
| 1     | SENSE_DOWN | digital in, **active low**, no internal pull | stock DOWN line, same buffer; deep-sleep wake on LOW |
| 3     | VSENSE     | analog, ADC1_CH3 | module +12 V (after the reverse diode, the coil rail) via 1 MΩ / 68 kΩ (ratio 0.06367), 12 dB attenuation |
| 4, 5  | —          | not connected    | reset at boot (input off, pull-up) and held in deep sleep |
| 6     | SIDE       | in, pull-up      | read once at boot: high (open) = LH/master, low (bridged) = RH/slave |
| 7     | LED        | out, active high | on while a relay is on; blinks while pairing a remote; off in sleep (GPIO5 on rev B) |
| 10    | RELAY      | out              | high = coil energized = swapped; NPN driver, 100k base pull-down on board (GPIO4 on rev B) |
| 9     | BOOT       | strapping        | not used                                                     |
| 2, 8  | —          | strapping        | not used                                                     |
| 18/19 | USB D-/D+  | USB-Serial-JTAG  | flashing and logs (console is on USB, not UART0)             |
| 20/21 | UART0 RX/TX | —               | not used by the firmware (not connected on rev C; the console is on USB) |

GPIO0/1 are digital only and GPIO3 is analog only, so ADC use and the
deep-sleep GPIO wake do not share a pin. GPIO0/1 run without internal pulls
(the board's 330k pull-ups set the level), also in deep sleep:
`CONFIG_ESP_SLEEP_GPIO_ENABLE_INTERNAL_RESISTORS=n` in `sdkconfig.defaults`
stops ESP-IDF from switching on a pull on each wake pin inside
`esp_deep_sleep_start` (default `y`, which per-pin `gpio_pullup_dis` cannot
override). The sense polarity is one constant, `config::SENSE_POLARITY`
(`ActiveLow` for rev B/C). GPIO10 has no internal pull at or after reset on
the C3 (datasheet v2.4, table 2-1), so the base pull-down alone keeps the
relay off through boot. In deep sleep GPIO3 (VSENSE, 0.7–1.1 V, near the
logic threshold) has its input buffer switched off and held, and the relay
pin is held low.

## Configuration constants

All in `wink-core/src/config.rs`. `Timing::check()` and a `const` assert run at
compile time and refuse combinations that break the safety rules (e.g. a
wink longer than the on-time limit).

| Constant                | Value      | Meaning                                                   |
|-------------------------|------------|-----------------------------------------------------------|
| `travel_ms` (T)         | **800**    | placeholder, measure on the car ([docs/testing.md](../docs/testing.md#measure-on-the-car-first))                 |
| `dwell_ms`              | 500        | wink hold = T + dwell                                     |
| `alt_period_cfg_ms`     | 1200       | alternation half-period P; effective P = max(this, ⌈1.3·T⌉) |
| `alt_max_s`             | 10         | `ALT n` is clamped to this                                |
| `alt_default_s`         | 8          | remote long press and bare `ALT`                          |
| `rest_ms`               | 20 000     | ALT refused this long after a run ends or is stopped      |
| `heartbeat_interval_ms` | 200        | master → slave relay frames while RH is on                |
| `heartbeat_timeout_ms`  | 1 000      | slave drops its relay without a frame for this long       |
| `off_repeats`           | 3          | extra "off" frames after RH goes off                      |
| `relay_max_on_ms`       | 12 000     | per-module backstop, whatever the commands say            |
| `awake_window_ms`       | 15 min     | deep sleep after this long without a command              |
| `wake_fallback_ms`      | 60 000     | timer wake when both sense lines read high                |
| `ping_interval_ms`      | 5 000      | master keep-alive so the slave stays awake with it        |
| `REMOTE_TIMING`         | 700 / 400 / 60 ms | long press / pattern gap / merge window             |
| `ESPNOW_CHANNEL`        | 1          | Wi-Fi channel of the link                                 |
| `SUPPLY.under_mv` / `over_mv` | 11 900 / 16 500 | energize band: refuse to switch on outside (`undervoltage` / `overvoltage`); `under_mv` is the ceiling and the value without a temperature |
| `SUPPLY.under_min_mv`   | 10 000     | floor of the temperature-scaled energize minimum          |
| `SUPPLY.pull_in_*`      | 9 000 mV at 23 °C, +37 mV/°C, +600 mV margin | relay pull-in model; energize minimum = clamp(model, floor, ceiling) at the die temperature |
| `SUPPLY.release_under_mv` / `release_over_mv` | 9 000 / 16 800 | hold band: an energized relay is released outside it (2 readings in a row) |
| `SUPPLY.under_recover_mv` / `over_recover_mv` | 12 200 / 16 000 | a fault clears only inside this band (lower edge moves with the energize minimum, 300 mV above it) … |
| `SUPPLY.recover_ms`     | 500        | … for this long                                           |
| `SUPPLY.ratio`          | 68 / 1068  | VSENSE divider (1 MΩ / 68 kΩ)                             |
| `SUPPLY.adc_max_valid_mv` / `adc_raw_saturated` | 2 450 mV / 4000 | at or above = saturated = too high |

## Behaviour

**Phone (master only).** Service `7a1e0001-5c0b-4e57-9b1a-70726f626577`,
advertised as `PWink` (GAP name `ProbeWink`).

- Command characteristic `7a1e0002-…` (write): `WINK L|R|B`, `ALT [n]`,
  `STOP`, `PAIR REMOTE`, `FORGET REMOTE`, `STATUS`. Case-insensitive.
- State characteristic `7a1e0003-…` (read, notify): `asleep`, `idle`, `wink`,
  `alternating`, `resting`, `overvoltage`, `undervoltage`; replies
  `error: busy|resting|overvoltage|undervoltage|<parse error>` and
  `remote: pairing|paired|none found|pairing failed|connected|disconnected|forgotten`.
- Security: LE Secure Connections with MITM and bonding; the module has
  IO capability DisplayOnly with the fixed passkey `WINK_BLE_PASSKEY`, so the
  phone asks for it. Write, read and subscribe need an authenticated
  (passkey) link, and the write handler additionally checks that the peer is
  bonded. A peer that pairs with Just Works gets a bond but no access.
- nRF Connect works for testing: connect, write `WINK L` as text to the
  command characteristic, enter the passkey when asked.

**Remote (master only).** `PAIR REMOTE` scans 30 s for a device advertising
the HID service 0x1812, connects, bonds (Just Works), subscribes to every
notifying input report (0x2A4D) and stores the address in NVS. After that the
master keeps trying to connect to it while awake. A report with any non-zero
byte = button down. Single = WINK L, double = WINK R, triple = WINK B, long
press (≥ 700 ms) = ALT 8, or STOP while alternating. Presses less than 60 ms
apart are merged (remotes that send volume-up and Enter per press).

**Modes.** While a wink or alternation runs, everything except STOP is refused
(`error: busy`). An alternation runs ⌊n·1000 / P⌋ whole phases (at least one)
so the last lamp always completes its travel; LH goes first. STOP drops all
relays at once, even mid-travel (a lamp may stop part-way; see [docs/how-it-works.md](../docs/how-it-works.md#timing)).

**Relay and supply limits.** Relay: TE IM06GR, 12 V coil, 1029 Ω. Its
pull-in voltage is 9.0 V max at 23 °C and rises with coil temperature to
about 11.3 V at 85 °C; it allows about 20 V on the coil at 85 °C, 16.8 V
worst case at 1 A contact current. The energize minimum therefore follows
the C3's die temperature: pull-in at that temperature plus 0.6 V, between
10.0 V (cold or room temperature) and 11.9 V (85 °C, also used if the
sensor fails). The rail sits one reverse-diode drop (about 0.85 V) below the
battery, so with the engine off a 12.0 V battery gives about 11.15 V: enough
at room temperature, refused in a hot engine bay where the coil may not pull
in. The energize band tops out at 16.5 V. The hold band is wider on purpose: a
pulled-in coil holds far below its pull-in voltage, and releasing during a
wink would stop the lamp part-way. So a supply dip from the retractor motors
does not cut a running wink, while 16.8 V (the datasheet worst case) does.
Note that an alternation energizes a relay at every phase change, so a rail
that sags below the energize minimum while the motors run will stop the alternation with
`undervoltage`. Measure the rail at the module during a wink.

**Relay chain on each module:** requested state → supply-voltage gate →
12 s on-time limiter → GPIO10. The slave's requested state comes from the
heartbeat watchdog. A supply fault on either module stops the running mode on
the master and refuses new winks / ALT until it clears; the state reads
`overvoltage` / `undervoltage`. The voltage gate needs a reading less than
250 ms old; one reading outside the energize band refuses energizing, two in
a row outside the hold band (≈ 200 ms) release an energized relay.

**Sleep.** 15 min after the last command (phone or remote), with no relay on,
no mode running and no remote pairing, the master sends `Sleep` to the slave
and both deep-sleep. The slave also sleeps 15 min after the last valid frame
from the master (pings every 5 s keep it awake while the master is). Before
sleeping each module reads SENSE_UP / SENSE_DOWN and arms a wake on LOW for
the pin(s) reading HIGH (dead line); both HIGH (switch unpowered, key out) →
arm both; both LOW (both lines live, should not happen) → 60 s timer wake. The relay pin is held low through
sleep (`gpio_hold_en` + `gpio_deep_sleep_hold_en`) on top of the gate
pull-down.

**Watchdog.** The task watchdog (5 s, panic → reboot) watches the control
loop on both modules; the idle task is watched too.

## ESP-NOW authentication

Frames are broadcast and not encrypted (ESP-NOW encryption needs known peer
MACs and does not cover broadcast). Each 33-byte frame carries sender role,
epoch and sequence number, and a 16-byte truncated HMAC-SHA256 over the rest,
keyed with `WINK_ESPNOW_KEY`. The master increments its epoch on every boot
and stores it in NVS; the slave accepts only frames newer than everything
before and stores the last accepted epoch in NVS, so frames recorded earlier
are refused after a reboot too. If the master's epoch falls behind (NVS
erased), the slave answers with an authenticated `StaleEpoch` and the master
jumps past it. Frames claiming the wrong role for their message type are
dropped. See `wink-core/src/proto.rs`.

Limits of this scheme: the master protects slave frames only in memory (a
replayed old slave status after a master reboot could briefly show a stale
supply fault; it cannot move a relay). Jamming is not prevented, but losing
the link makes the slave drop its relay within 1 s.

## Untested

Run so far (2026-11-01, ESP32 DEVKITV1): boot as master, ESP-NOW init, BLE
advertising, phone pairing. Nothing below has been checked yet.

- Everything on the chip: boot, GPIO, relay drive, LED, side detection.
- ADC: curve-fitting calibration on this module, the 2450 mV saturation
  threshold, and the voltage thresholds against a real supply. If eFuse
  calibration is missing the code falls back to uncalibrated readings and logs
  a warning.
- BLE: advertising, passkey pairing and bonding with Android / iOS, the
  authenticated-only GATT permissions, notifications, reconnect after deep
  sleep (bonds persist via `CONFIG_BT_NIMBLE_NVS_PERSIST`).
- Remote: whether a given shutter remote advertises 0x1812, accepts our
  central's pairing, uses report mode (not boot protocol), keeps a stable
  address (an RPA-using remote would not be found again), and how it
  advertises after bonding (directed advertising toward us is untested).
  `ble_gap_conn_cancel` to interrupt a pending connect for `PAIR REMOTE` is
  untested.
- NimBLE peripheral + central at once (advertising and connected to the
  phone while scanning / connecting to the remote).
- Wi-Fi + BLE coexistence on the master with Wi-Fi power save off (set by
  `EspNow::take()`); heartbeat timing over the air with BLE traffic.
- ESP-NOW delivery and the whole master ↔ slave protocol on real radios.
- Deep sleep: wake on LOW through the MOSFET buffers, that no internal
  pull is active in sleep with `CONFIG_ESP_SLEEP_GPIO_ENABLE_INTERNAL_RESISTORS=n`,
  the timer fallback, sleep current, the relay hold through sleep.
- The sense buffers' switching point (about 1.4–7 V) against the real harness.
- The die temperature against the air in the box (the pull-in model assumes
  the die is at least as warm as the relay), and sleep current with GPIO3's
  input off.
- Task watchdog behaviour and reset recovery.
- All timing constants against the car (T is a placeholder).
- The supply thresholds against the IM06GR on the bench, and the rail sag at
  the module while the retractor motors run.

## TODO

- Measure T ([docs/testing.md](../docs/testing.md#measure-on-the-car-first)) and set `travel_ms`.
- Wi-Fi OTA (`wink-app/src/ota.rs` is a stub; partition table is ready).
- Pairing window for phones: today anyone with the passkey can bond at any
  time while the module is awake. A static passkey is weak against an active
  attacker during pairing; consider allowing new phone bonds only for a short
  time after a cold power-up or a button/switch sequence.
- Bond storage holds 6 bonds and evicts the oldest when full
  (`ble_store_util_status_rr`): repeated Just Works bonding by strangers could
  push out the phone's bond. Reject Just Works bonds from the peripheral side.
- Restrict remote pairing to the remote's appearance / name, and support
  two-button remotes (map buttons separately).
- Report the slave's relay / supply state to the phone in more detail.
- Supply check uses an 8-sample average every 100 ms; tune after measuring
  ADC noise on the board.
