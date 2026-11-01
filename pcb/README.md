# PCB: probe-wink module (rev C)

One module per headlamp. 46 × 40 mm, 2 layers, ESP32-C3-WROOM-02 (or -02U with an
external antenna: same pads) with the antenna overhanging the right edge, TE IM06GR
swap relay, LMR16006X buck from the retractor's own 12 V feed. The module connects
through a 6-wire lead to a Y-adapter between the factory retractor plug and socket;
motor current stays in the adapter.

The KiCad files are **generated**. Edit the Python sources, not the `.kicad_*` files:

| File | Role |
|---|---|
| `schematic.py` | Parts, values, LCSC numbers, nets → `probe-wink.kicad_sch`, `intended_netlist.json` |
| `check.py` | ERC, netlist export, compare against the declared nets |
| `board.py` | Outline, placement, hand-routed power paths, keep-outs, Freerouting, pours, stitching, DRC, layout checks |
| `fab.py` | JLCPCB Gerber/drill zip, BOM, CPL, assembly drawings → `build/fab/` |
| `export_enclosure.py` | Board STEP and geometry JSON for the housing → `../enclosure/build/` |
| `lib/probe-wink.pretty` | Project footprints (back-side pogo pads for J2) |
| `../tools/kicad/` | Generic generator library (`sexp`, `schematic`, `board`, `fab`, `geometry`) |

## Rebuild

```sh
make pcb                      # from the repository root: the first four steps below

cd pcb
python3 schematic.py          # schematic + lib tables
python3 check.py              # ERC + netlist (board.py reads build/probe-wink.net)
python3 board.py              # about 90 s; add --no-route while iterating on placement
python3 fab.py                # build/fab/
python3 export_enclosure.py   # ../enclosure/build/board.json + probe-wink-board.step
```

Needs KiCad 10 (`pcbnew` Python module, `kicad-cli`), Freerouting on `PATH`,
`rsvg-convert` for the PNG renders.

`board.py` runs the Freerouting strategies in parallel and caches each result
under `build/route-cache/` by a hash of the routing input. A rerun with unchanged
placement (silkscreen, rules) takes about 10 s.

Status at last generation: ERC 0, schematic overlap check 0, netlist matches the
declared nets. DRC 0 errors, 0 unconnected; one warning, the WROOM-02 footprint
differing from the library copy (intentional). 18 of 18 layout checks pass.

## Layout

DRC checks clearances. It does not check whether parts that must be close are close.
`board.py` therefore hand-places and hand-routes the paths that decide whether the
buck and the module behave, and the autorouter only does the rest:

- **Buck** (TI LMR16006 §11.1/11.2): U1 rotated so VIN sits above Cin (C2). The
  catch diode D3 sits beside Cin, so the Cin–switch–diode loop closes on top copper.
  L1 is on the SW side, the output caps are at the L1/D3 junction, and the ground
  ties into the plane at the output caps. The FB divider is at the FB pin, away
  from L1. The back copper under the buck is a solid plane (track keep-out).
- **Module supply:** 100 nF then 10 µF straight above pin 1, fed along the top edge.
  EN jogs left over the unused pins 3/4 to its RC and pull-up.
- **Module body:** no tracks on either layer under the module; GND stitching allowed.
- **Bottom plane:** no tracks under the input stage, FB divider and buck (keep-out
  116–131.6 × 102–122). The FB divider's ground runs on top to the IC ground; its 3V3
  comes from the top-edge rail and crosses under the 12 V link by a 4 mm hop at the edge.
- **Programming:** J2 (EN, 3V3, BOOT, GND) is a 2 × 2 pogo pad set on the back right
  behind the module's top row, so its traces stay local. USB D−/D+ pads are behind the
  bottom row.
- **Harness lines** (HARN_*, RET_*) have their own net class: 0.4 mm, 0.5 mm clearance
  (coupled bursts, no TVS on these lines).
- After routing, extra GND vias go next to the sense and ADC grounds (`via_near`), and
  autorouter leftovers that lead nowhere are removed (`prune_dangling`).
- **Sense buffers:** two compact cells (divider, filter cap, zener on a short gate
  bus, MOSFET, pull-up), placed next to the harness pads.

After routing, `board.py` measures the routed copper between pad pairs
(`kicad.board.path_length`) and fails loudly if a limit is exceeded:

| Path | Rev B | Rev C | Limit |
|---|---|---|---|
| SW: U1 → L1 | 14.7 mm, 2 vias | 6.1 mm | 8 mm, top only |
| SW: U1 → D3 | 11.8 mm, 2 vias | 3.6 mm | 4 mm |
| FB: U1 → R9 | 7.2 mm, 2 vias | 1.7 mm | 3 mm |
| 3V3: module pin 1 → 100 nF | 43.1 mm, 4 vias | 2.6 mm | 4 mm |
| EN: pin 2 → C7 | 16.7 mm, 2 vias | 5.0 mm | 6 mm |
| FB divider GND → IC GND | 27 mm through the planes | 8.8 mm, top | 10 mm |
| Foreign nets in the buck hot loop | 3V3 feed to R9 through it | none | none |

## Circuit notes

- **Swap:** K1 pole 1 (COM 3 / NC 2 / NO 4) feeds the retractor's UP input, pole 2
  (COM 6 / NC 7 / NO 5) its DOWN input. At rest NC passes the stock lines through;
  energized, they cross. Lead wires: 1 harness BK/BL (DOWN), 2 harness BK/R (UP),
  3 to retractor BK/R, 4 to retractor BK/BL, 5 BK/W tap (1 A fuse in the adapter), 6 BK.
- **Input:** M7 reverse diode, SMCJ30A TVS (a load dump ends at the 1 A tap fuse), 22 µF
  bulk electrolytic, 2 × 2.2 µF + 100 nF ceramics.
- **Buck:** LMR16006X at 0.7 MHz, 22 µH (SWPA5040S), PMEG10010ELR catch diode (Schottky
  leakage at 85 °C was the largest sleep load with an SS210), 2 × 10 µF X7R out,
  Vout = 0.765 × (1 + 330k/100k) = 3.29 V.
- **Relay drive:** MMBTA06 NPN with a 2.2k base resistor, about 1.1 mA base drive,
  saturated down to −55 °C (a 2N7002 has no Rds(on) spec at 3.3 V gate drive and may
  not pull the coil in when cold), 100k base pull-down, 1N4148W flyback.
- **Sense:** HARN_UP/HARN_DOWN → 330k/220k divider, 100 nF, 12 V zener → 2N7002 →
  330k pull-up → GPIO0/1, active low, deep-sleep wake. Switching point about
  1.4–7 V on the line.
- **Supply sense:** +12 V → 1 M / 68k → GPIO3 (ADC). The firmware's relay minimum
  follows the die temperature (see [firmware/README.md](../firmware/README.md)).
- **EN:** 10k / 1 µF RC at pin 2 (the C3's brown-out reset covers supply dips).
- **Pins:** GPIO10 relay, GPIO7 LED, GPIO6 side jumper; GPIO4/5 and UART0 (GPIO20/21)
  unused (console on native USB).
- **Programming:** native USB (D−/D+ on the back-side pads TP1/TP2, 3V3 and GND on
  TP3/TP5). ROM download mode works over USB; to force it, hold BOOT low on J2 and
  pulse EN. J2.2 is 3V3 only: never feed it 5 V.
- **Side select:** JP1 (back side) open = LH/master, bridged = RH/slave.

## Assembly

- JLCPCB assembles every part with `Assembly = SMT` (45 placements, 26 BOM lines),
  top side only. All LCSC numbers were checked in stock on 2026-11-01.
- Hand-soldered: the 6-wire lead into J1: sheathed 6 × 0.5 mm² cable (e.g. LiYY 6×0.5,
  about 6.2 mm OD, fits the housing's M12 gland), 1.15 mm holes. The lines carry signal
  current only; the motor current stays in the Y-adapter.
  JP1 is bridged with solder on the RH module only. The back side has no parts,
  only pads.
- Assembly drawings with every reference: `build/fab/probe-wink-assembly-{top,bottom}.pdf`
  (most references don't fit on the silkscreen).

## Review before ordering

- Check every part's rotation in JLC's placement preview; record corrections in
  `ROT_OFFSET` in `fab.py`.
- The ESP32-C3-WROOM-02-N4 is rated −40…85 °C ambient. The -H4 (105 °C, LCSC
  C2944070) has the same pads but showed 0 stock at JLC; consider it for a hot
  mounting spot.
- Radio: test placement with two dev boards in the car first (see
  [docs/testing.md](../docs/testing.md#radio-in-the-car)).
