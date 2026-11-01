# Design notes

The main decisions and why. Rev A was a first design on paper (ESP32-C3
SuperMini, cut harness); rev B and rev C are the custom board. This page
describes rev C.

## Y-adapter instead of cutting the harness

Rev A cut BK/R and BK/BL once per side and crimped the ends into a 6-pin
connector, with BK/W and BK spliced in. Rev C uses a Y-adapter between the
factory plug and the retractor instead:

- Nothing on the car is cut. Unplugging the adapter restores the stock wiring.
- Motor power and ground pass straight through the adapter. The module only
  ever sees signal-level current: its relay switches the retractor's command
  inputs, which feed the retractor's own relay coil.
- So the module lead can be a thin sheathed 6 × 0.5 mm² cable in a small gland.

## Swap relay

One DPDT relay per side crosses the UP and DOWN lines. Off means stock: at
power-up, during reset, in sleep, with a dead module, or with the fuse pulled.
Each common only ever touches one harness line, so the shared lines are never
bridged, and the swap works whatever the polarity of the command lines.

The relay is a TE IM06GR (12 V coil, 1029 Ω, SMD, assembled by JLCPCB; the
Hongfa HFD4/12-SR fits the same footprint).

## Relay driver: NPN, not a MOSFET

The coil is driven by an MMBTA06 NPN with a 2.2k base resistor, about 1.1 mA of
base drive, saturated down to −55 °C. A 2N7002 has no Rds(on) specified at
3.3 V gate drive and may not pull the coil in when cold. A 100k base pull-down
keeps the relay off through boot, reset and sleep; GPIO10 has no internal pull
at reset. A 1N4148W catches the coil's flyback.

## Temperature-scaled undervoltage

The relay's pull-in voltage is 9.0 V max at 23 °C and rises with coil
temperature to about 11.3 V at 85 °C. The module's rail sits one reverse-diode
drop (about 0.85 V) below the battery, so a 12.0 V battery with the engine off
gives about 11.15 V.

A fixed 11.5 V minimum refused winks with the engine off. A lower fixed
minimum would switch when a hot coil may not pull in. So the energize
minimum follows the C3's die temperature: modelled pull-in plus 0.6 V, clamped
between 10.0 V and 11.9 V; 11.9 V if the sensor fails. The die runs a few
degrees above the air in the box, which errs on the safe side.

Energizing and holding use different bands. A relay is switched on only inside
10.0–11.9 V (minimum) to 16.5 V. Once on, it is released only outside
9.0–16.8 V, two readings in a row: a pulled-in coil holds far below its
pull-in voltage, and releasing mid-wink would stop the lamp part-way. 16.8 V is
the relay's worst case at 85 °C and 1 A contact current.

## Sense buffers

Each harness command line goes through a 330k / 220k divider, 100 nF, a 12 V
zener on the gate and a 2N7002 with a 330k pull-up to 3.3 V. A live line pulls
the pin low. The switching point is about 1.4–7 V on the line over part spread
and temperature, so 12 V is solidly live. GPIO0 and GPIO1 run without internal
pulls, also in deep sleep, and wake the chip on low. The sense lines are on the
harness side of the relay, so they always see the switch, not the swap.

## Supply and input protection

- LMR16006X buck (60 V input rating) at 0.7 MHz, 22 µH, 3.29 V out.
- PMEG10010ELR catch diode: with an SS210, its leakage at 85 °C was the largest
  load in sleep.
- M7 reverse diode, SMCJ30A TVS, 22 µF bulk plus ceramics. The 1 A fuse in the
  adapter's BK/W tap ends a load dump.
- Supply sense through 1 M / 68k to the ADC, so the firmware can gate the relay.

## Layout checks beyond DRC

DRC checks clearances, not whether parts that must be close are close. The
board script hand-places and hand-routes the buck's hot loop, the feedback
divider, the module's supply decoupling and EN, and then measures the routed
copper between pad pairs. It fails the build if a limit is exceeded, for
example SW node U1 → D3 under 4 mm, U1 → L1 under 8 mm on top copper only,
3V3 pin → 100 nF under 4 mm, and no foreign nets inside the buck's hot loop.
Rev B had these at 11.8 mm, 14.7 mm and 43.1 mm. The harness lines have their
own net class (0.4 mm tracks, 0.5 mm clearance) because they carry coupled
bursts and have no TVS. Full table: [pcb/README.md](../pcb/README.md#layout).

## Radio and module

ESP32-C3-WROOM-02 with the antenna overhanging the board edge, so there is no
board under it, and 6 mm of air to the housing's end wall (plastic only;
screws, inserts and mounting bolts at least 15 mm away). The -02U variant
(external antenna) has the same pads. The C3 runs BLE to the phone and remote
and ESP-NOW to the other side at once.

ESP-NOW frames are broadcast and not encrypted (ESP-NOW encryption needs known
peer MACs and does not cover broadcast). Each frame carries a truncated
HMAC-SHA256, an epoch that survives reboots and a sequence number, so recorded
frames are refused. Jamming cannot be prevented, but losing the link makes the
RH relay drop within 1 s.

## Gland, lead and seal

- M12 gland with a sheathed 6 × 0.5 mm² lead (about 6.2 mm OD), because the
  lead carries signal current only. An M16 gland for a 6 × 0.75 mm² lead makes
  the box 5 mm taller; the gland's hex must clear the mounting surface, and that
  sets the box height.
- The gland sits in an end wall flush with the flange, so it seals on one flat
  printed face. The hole is a flat-topped teardrop so it prints without support,
  then gets tapped.
- Seal: 1.5 mm silicone cord in a groove, lid closing plastic on plastic as a
  hard stop. A printed TPU 95A gasket was modelled and rejected: it needs about
  1.4 kN and lifts the lid off the seal.
- The lid is 4.0 mm thick with an 8 mm skirt and eight M3 screws, so the
  estimated lid deflection still leaves at least 16 % cord squeeze between the
  screws.
- A PTFE breather vent equalizes pressure without letting water in.
- No connector through the wall for programming: the board comes out, and pogo
  pads on its back give USB, EN and BOOT.

## Generated design files

The schematic, board, fab outputs and housing are all generated from Python
(`pcb/*.py`, `enclosure/housing.py`, using the generic library in
`tools/kicad/`). Every value lives in one place, each run repeats ERC, the
netlist comparison, DRC, the layout checks and the housing checks, and a
change is a reviewable diff.
