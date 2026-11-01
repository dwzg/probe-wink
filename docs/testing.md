# Testing

## Status

| What                      | Tested how                                              |
|---------------------------|---------------------------------------------------------|
| Firmware logic (`wink-core`) | host unit tests (`make test`)                        |
| Firmware on a chip        | ESP32 DEVKITV1 test bench (below)                       |
| Android app               | protocol unit tests and Android lint; not yet run on a phone |
| PCB                       | ERC, netlist comparison, DRC and layout checks only; not built |
| Housing                   | `enclosure/check.py` measurements on the model; not printed |
| Module on the car         | nothing yet                                             |

## ESP32 devkit test bench

The firmware also builds for an ESP32 DEVKITV1 (ESP-WROOM-32). That tests BLE
(Android app, passkey pairing, bonding), the BLE remote and the control logic
before the custom board exists.

```sh
espup install --targets esp32        # once: Xtensa Rust toolchain
export WINK_ESPNOW_KEY=... WINK_BLE_PASSKEY=...
make devkit                          # build only
firmware/scripts/devkit flash /dev/ttyUSB0   # build, flash, serial monitor
```

| Devkit pin | Role                                                       |
|------------|------------------------------------------------------------|
| GPIO2 (onboard LED) | stands in for the relay: lit while it would be energized |
| GPIO4      | status LED (optional: LED + 1k to GND)                     |
| GPIO27     | SIDE: open = LH/master, to GND = RH/slave                  |
| GPIO32 / GPIO33 | SENSE_UP / SENSE_DOWN with internal pull-ups; a jumper to GND = line live |

Differences from the module: the supply reading is fixed at 12.6 V, there is no
temperature sensor, and deep sleep wakes through EXT0/EXT1 instead of the C3's
GPIO wake. The bench cannot test the relay driver, the sense buffers, the power
path or the sleep current. ESP-NOW master/slave needs a second board.

Details: [firmware/README.md](../firmware/README.md#esp32-devkitv1-test-bench).

## Bench test of a built module

On a bench supply (12.6–13.8 V; below about 10–11.9 V the firmware refuses to
switch the relay on). A changeover switch that puts 12 V on lead 1 or lead 2
stands in for the headlamp switch:

1. At power-up the relay stays off.
2. At rest, lead 2 connects to lead 3 and lead 1 to lead 4. Energized, lead 2
   connects to lead 4 and lead 1 to lead 3. Leads 1 and 2 never connect.
3. Switching a command line live wakes the module from deep sleep.
4. Measure the sleep current.
5. Failsafe: with the RH relay on, power off the master. The RH relay must drop
   within about 1 s.
6. Supply limits: the relay refuses to switch on below the energize minimum and
   above 16.5 V (`undervoltage` / `overvoltage`).

## Measure on the car first

The wiring diagrams come from low-resolution scans of the manual, and some
values are placeholders. Before building:

1. Back-probe a retractor plug with a multimeter to ground. BK/W should read
   battery voltage, also with the key out.
2. Switch OFF, lamps down: expect about 12 V on BK/BL and about 0 V or open on
   BK/R.
3. Switch to HEAD, lamps up: expect the reverse. Note whether the inactive line
   floats or is grounded. The module's sense inputs assume +12 V commands.
4. Pull the key and repeat. If the switch still moves the lamps, the modules can
   wake with the key out. If both lines read 0 V, they only wake with the key
   in.
5. Film one up and one down cycle and read the travel time T off the frames.
   Use the slower direction and the slower side. Set `travel_ms` in
   `firmware/wink-core/src/config.rs` (it is 800 ms, a placeholder).

Once a module is built, also measure the +12 V rail at the module during a wink:
an alternation switches a relay on at every phase change, so a rail that sags
below the energize minimum while the motors run stops it with `undervoltage`.

- Battery voltage at idle with the alternator charging, and right after
  switching the engine off: the drive lock's threshold (`SUPPLY.engine_running_mv`,
  12.7 V on the rail, about 13.5 V at the battery) must sit between the two.

## Radio in the car

Before ordering boards, check the radio path with two dev boards at the two
mounting spots: ESP-NOW between the sides, and BLE from the phone outside the
car and from the cabin. Two ESP32-C3 SuperMini boards (same chip as the module)
are planned for this and for the C3-specific tests (temperature sensor, GPIO
wake, ESP-NOW master/slave).

## In the car

1. Modules asleep: both lamps follow the switch exactly as stock.
2. Flick the switch: both modules wake, and the phone sees `PWink`.
3. Switch OFF, wink LH: LH rises, holds, returns; RH stays down. Repeat for RH.
4. Switch HEAD or 4th position, wink one side: that lamp retracts and comes
   back up.
5. Alternate: every flip completes, and both lamps return to the switch position
   at the end.
6. Pull the 1 A fuse in the LH adapter during a run (this powers down the
   master only): the RH lamp returns to the switch position within about a
   second.
7. Stop mid-travel with STOP: note where the lamp stops, then cycle the switch
   and check it recovers.

## Untested

Nothing has run on the module hardware or on the car. The full list is in
[firmware/README.md](../firmware/README.md#untested); the main open points:

- The whole module: boot, relay drive, sense buffers (switching point about
  1.4–7 V on the line), ADC calibration and the supply thresholds, sleep
  current.
- The relay's real pull-in voltage against the temperature model, and the C3's
  die temperature against the air in the box.
- Which BLE shutter remotes work: advertising 0x1812, pairing with the module,
  report mode, a stable address.
- BLE peripheral and central at once, Wi-Fi/BLE coexistence on the master,
  ESP-NOW delivery and timing over the air.
- Deep-sleep wake through the sense buffers.
- All timing constants against the car (T is a placeholder).
- Mid-travel behaviour after an early relay release (expected from the circuit,
  not seen).
- Motor duty: the manual gives no rating. Alternation runs both motors almost
  continuously, hence the 10 s cap and 20 s rest.
