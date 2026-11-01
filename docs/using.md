# Using it

## Wake the modules first

The modules sleep almost all the time. A headlamp switch change wakes both of
them for 15 minutes: flick the lamps up, or turn to the 4th position (lamps up,
lights off). Each command restarts the 15 minutes. The phone and the remote can
only reach the LH module while it is awake.

## Phone

The phone talks to the **LH module only**. It advertises as `PWink` (GAP name
`ProbeWink`).

- **Android app** ([android/](../android/README.md)): setup, all commands,
  remote pairing, and home-screen widgets that wink in one tap without opening
  the app.
- **nRF Connect** or any BLE tool works for testing: write the commands below
  as text to the command characteristic.

**Pairing.** The first connection asks for a 6-digit passkey. It is the
`WINK_BLE_PASSKEY` the firmware was built with. Only bonded phones that paired
with the passkey get access; a phone that pairs with "Just Works" gets a bond
but no access. The module keeps six bonds and drops the oldest when full. If
pairing fails after a reflash, remove the old "ProbeWink" pairing in the phone's
Bluetooth settings and connect again.

### Commands

Written as text to the command characteristic `7a1e0002-…`, case-insensitive:

| Command         | Effect                                                   |
|-----------------|----------------------------------------------------------|
| `WINK L`        | LH lamp goes over, holds, returns                        |
| `WINK R`        | same for RH                                              |
| `WINK B`        | both lamps flip and return                               |
| `ALT n`         | alternate for n seconds (capped at 10; bare `ALT` = 8)   |
| `STOP`          | all relays off at once, even mid-travel                  |
| `STATUS`        | the module re-sends its state, remote link, firmware version and drive lock |
| `PAIR REMOTE`   | scan 30 s for a BLE remote and bond with it              |
| `FORGET REMOTE` | forget the stored remote                                 |
| `DRIVELOCK ON`  | refuse winks and alternate while the engine runs (stored; off by default) |
| `DRIVELOCK OFF` | allow them again                                         |
| `UNCOUPLE`      | erase the link key (the app's Couple writes a new one)   |

While a wink or alternation runs, everything except `STOP` is refused. LH and RH
are as seen from the driver's seat.

## BLE remote

Cheap BLE camera-shutter remotes act as a keyboard (HID service 0x1812). Send
`PAIR REMOTE` from the phone, then switch the remote on or press its button. The
master connects, bonds and stores the remote; after that it reconnects to it
whenever the master is awake.

| Press           | Command                                   |
|-----------------|-------------------------------------------|
| single          | `WINK L`                                  |
| double          | `WINK R`                                  |
| triple          | `WINK B`                                  |
| long (≥ 700 ms) | `ALT 8`, or `STOP` while alternating      |

Presses must follow within 400 ms of each other to count as one pattern.
Presses less than 60 ms apart are merged, for remotes that send two keys per
press. Which remotes work is untested; see [testing.md](testing.md#untested).

## What the lamps do

| Switch           | Wink on one side                                  |
|------------------|---------------------------------------------------|
| OFF (lamps down) | that lamp rises, holds, comes back down           |
| HEAD or 4th position (lamps up) | that lamp drops, holds, comes back up |

With the lights on, a lamp winked down stays lit inside the hood cavity. Use the
4th position for winks with the lamps up.

Alternation flips the lamps in turn, LH first, and both end at the switch
position.

## States

The state characteristic `7a1e0003-…` (read, notify) reports:

| State          | Meaning                                                    |
|----------------|------------------------------------------------------------|
| `asleep`       | the module is going to deep sleep; the link drops          |
| `idle`         | awake, ready                                               |
| `wink`         | a wink is running                                          |
| `alternating`  | an alternation is running                                  |
| `resting`      | 20 s after an alternation; `ALT` is refused, winks work    |
| `undervoltage` | supply too low for the relay to pull in reliably (10.0–11.9 V on the rail, higher when hot); winks and `ALT` refused until it recovers |
| `overvoltage`  | supply above 16.5 V; winks and `ALT` refused until it recovers |

Replies to commands:

- `error: busy` (a mode is running), `error: resting`, `error: overvoltage`,
  `error: undervoltage`, or `error: <parse error>`.
- `remote: pairing | paired | none found | pairing failed | connected |
  disconnected | forgotten`.

A supply fault on either module stops a running mode. `undervoltage` is likely
with the engine off and a hot engine bay: the rail sits about 0.85 V below the
battery, and the relay needs more voltage when hot. Run the engine if it
happens.

## Limits

The firmware enforces these on both modules, whatever the phone or remote asks
for:

| Limit                 | Value                                          |
|-----------------------|------------------------------------------------|
| Alternation length    | 10 s maximum                                   |
| Rest after alternation | 20 s                                          |
| Relay on-time         | 12 s per module, in one go                     |
| RH without heartbeat  | relay off after 1 s                            |
| Supply to switch on   | 10.0–11.9 V minimum (temperature), 16.5 V maximum |
| Awake window          | 15 min after the last command                  |

## Status LED

Inside the housing, on GPIO7: on while a relay is on, blinking while the master
pairs a remote, off in sleep.

## On the road

A control that changes the headlamps may be illegal while driving in many
places (in Germany it is a TÜV question). Treat wink and alternation as
parked-car features. The modules cannot see the parking brake or the vehicle
speed, so this is on the driver.

The optional **drive lock** helps against accidental presses (app switch "Block
winks while the engine runs", or `DRIVELOCK ON`). With it on, the LH module
refuses winks and alternate, from the phone and the remote alike, while its
supply shows the alternator charging: rail at or above 12.7 V, about 13.5 V at
the battery. Stop always works. It is off by default, and it also blocks
winking with the engine idling while parked. The threshold is still to be
checked on the car ([testing](testing.md#measure-on-the-car-first)).
