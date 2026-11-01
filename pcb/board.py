#!/usr/bin/env python3
"""Probe wink module: PCB source (rev C, 46 x 40 mm, antenna overhanging the right edge).

Needs build/probe-wink.net (python3 check.py writes it).
Run:  python3 board.py [--no-route]
"""
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, "..", "tools"))
import pcbnew  # noqa: E402
from kicad.board import Board, drc, foreign_copper, path_length, render  # noqa: E402

BUILD = os.path.join(HERE, "build")
PCB = os.path.join(HERE, "probe-wink.kicad_pcb")
X0, Y0, W, H = 100.0, 100.0, 46.0, 40.0
REV = "C"

b = Board(os.path.join(BUILD, "probe-wink.net"), track=0.25, clearance=0.2, via=(0.6, 0.3), min_drill=0.2,
          declared=os.path.join(HERE, "intended_netlist.json"), fp_dirs=[os.path.join(HERE, "lib")])
b.netclass("Power", ["+12V", "VBAT_IN", "GND", "BUCK_SW", "COIL_LO"], track=0.4)
b.netclass("Supply3V3", ["+3V3"], track=0.3)
# harness lines see coupled bursts and have no TVS: keep them away from the logic
b.netclass("Harness", ["HARN_UP", "HARN_DOWN", "RET_UP", "RET_DOWN"], track=0.4, clearance=0.5)
b.outline_rounded(X0, Y0, W, H, r=1.5)

# ---- fixed placements -------------------------------------------------------------
# Espressif: antenna outside the base board, feed point close to the edge. The board ends
# where the module's antenna section starts (6.86 mm from the footprint origin).
U2 = (X0 + W - 6.86, 119.0)
UX, UY = 121.0, 111.6           # buck IC; rot 270 puts FB/GND/CB on top, SHDN/VIN/SW below
FIXED = {
    "U2": (U2[0], U2[1], 270),
    "J1": (103.4, 130.0, 90),                    # pin 1 bottom ... pin 6 (GND) top
    "K1": (113.2, 124.0, 0),
    # input: reverse diode, TVS (ground pad next to the harness ground), bulk electrolytic
    "D2": (111.6, 104.0, 180), "D1": (111.2, 110.8, 180), "C12": (122.2, 104.0, 0),
    # buck (TI LMR16006 11.1/11.2): Cin straight under VIN, catch diode beside it so the
    # Cin-switch-diode loop closes on top copper; L1 on the SW side; Cout at the L1/D3 junction;
    # FB divider left of the FB pin, away from L1; boot cap across CB and SW.
    "U1": (UX, UY, 270),
    "C3": (123.8, UY, 270),
    "C2": (UX, UY + 3.9, 270),
    "D3": (123.95, UY + 4.1, 270),
    "L1": (128.3, 114.0, 0),
    "C4": (127.0, 118.4, 180), "C13": (127.0, 120.6, 180),
    "C1": (118.4, 116.2, 270), "C11": (115.8, 116.2, 270),
    "R10": (117.5, 108.7, 180), "R9": (117.5, 110.8, 0), "R8": (117.5, 112.8, 0),
    # module 3V3 pin: 100 nF then 10 uF straight above pin 1, fed from the top edge
    "C6": (143.8, 107.6, 180), "C5": (143.8, 104.9, 180),
    # EN: RC right at pin 2 (EN jogs left over the free pins 3/4)
    "C7": (140.8, 106.9, 90), "R11": (140.8, 102.55, 270),
    # flyback diode above the relay, clear of the J1 legend; 12 V test pad on the input rail
    "D4": (111.0, 117.3, 0), "TP4": (112.4, 114.6, 0),
    # strap pull-ups and LED straight above their pins, 3V3 end touching the top-edge rail
    "R14": (133.9, 104.4, 270), "R13": (135.9, 104.4, 270), "R7": (137.9, 106.8, 90), "D5": (137.9, 103.3, 270),
}
for ref, (x, y, r) in FIXED.items():
    b.place(ref, x, y, r)
# back side, behind the module's top row: EN/BOOT pogo pads and the side jumper (bare copper)
b.place("J2", 139.27, 103.93, 180, back=True)
b.place("JP1", 142.0, 108.6, 0, back=True)
# Each sense buffer is one cell: divider top/bottom, filter cap and zener on a short gate bus,
# MOSFET (gate on the bus) at its end, pull-up beside the drain.
# (x0, yg) = divider column, gate bus height.
CELLS = {"DOWN": ("Q3", "R3", "R4", "C9", "D7", "R18", 106.4, 134.5),    # next to J1 pin 1 (HARN_DOWN)
         "UP": ("Q2", "R1", "R2", "C8", "D6", "R17", 118.9, 133.0)}
for q, rt, rb, cc, dz, rp, x0, yg in CELLS.values():
    b.place(rt, x0, yg - 1.9, 270); b.place(rb, x0, yg + 1.9, 270)
    b.place(cc, x0 + 2.2, yg + 1.9, 270); b.place(dz, x0 + 4.4, yg + 1.9, 270)
    b.place(q, x0 + 7.4, yg + 0.95, 0); b.place(rp, x0 + 10.4, yg - 0.5, 270)


def near(ref, tgt, rots=(0, 90, 180, 270)):
    """Place ref at the free spot nearest to tgt (a point or a (ref, pad) tuple)."""
    if isinstance(tgt[0], str):
        tgt = b.pad_xy(*tgt)
    b.place_near(ref, tgt[0], tgt[1], rots=rots)


# ---- module top strip: EN discharge diode, side jumper, LED, strap pull-ups
# ---- relay driver at the coil pins
b.place("Q1", 119.3, 123.6, 90)            # below the buck's solid-plane area, collector up to the coil
near("R5", ("Q1", 1)); near("R6", ("Q1", 1))
# ---- sense buffers (UP, DOWN) and supply divider below the module, near their pins
# USB pogo pads on the back (bare copper), right under their pins
b.place("TP1", 137.64, 131.2, 0, back=True); b.place("TP2", 139.9, 133.0, 0, back=True)
near("R12", ("U2", 16))
near("C10", ("U2", 15)); near("R16", ("U2", 15)); near("R15", ("U2", 15))
# ---- programming header, test pads (3V3 and 12V far apart), fiducials
near("TP5", (133.0, 137.0))
near("TP3", (128.6, 123.4))
near("FID1", (101.8, 101.8)); near("FID2", (127.9, 109.3)); near("FID3", (131.5, 138.2))

# fiducials: keep tracks out of the 2 mm mask opening + clearance (Freerouting ignores pad-local clearance)
for f in ("FID1", "FID2", "FID3"):
    fx, fy = b.pad_xy(f, "") if False else (pcbnew.ToMM(b.fps[f].GetPosition().x), pcbnew.ToMM(b.fps[f].GetPosition().y))
    b.keepout(fx - 1.8, fy - 1.8, 3.6, 3.6, layers=(pcbnew.F_Cu,), name=f + "-clear")
missing = set(b.fps) - b.placed
if missing:
    sys.exit("not placed: %s" % sorted(missing))
for ref in ("TP1", "TP2", "TP3", "TP4", "TP5", "FID1", "FID2", "FID3"):
    b.ref_text(ref, visible=False)
# ---- hand routing: the paths that decide whether the buck and the module behave ---------
SW, PWR, V33, SIG = 0.6, 0.6, 0.5, 0.25


def hand_route():
    T = b.track
    # input: harness -> reverse diode -> +12V rail (TVS, bulk) -> input caps -> VIN
    T("VBAT_IN", [("J1", 5), ("D1", 2)], PWR)
    T("+12V", [("D2", 1), ("C12", 1)], PWR)
    T("+12V", [("D2", 1), (115.0, 107.6), (113.2, 109.4), ("D1", 1)], PWR)
    T("+12V", [("D1", 1), (113.9, 111.5), (113.9, 114.72), ("C11", 1), ("C1", 1), ("C2", 1)], PWR)
    T("+12V", [("TP4", 1), (113.9, 114.6)], PWR)
    T("+12V", [("R8", 1), (116.5, 114.72)], SIG)
    T("+12V", [("U1", 5), ("C2", 1)], V33)
    T("BUCK_EN", [("R8", 2), ("U1", 4)], SIG)
    # feedback: divider at the FB pin, ground through its own via into the solid bottom plane
    T("BUCK_FB", [("R10", 1), ("R9", 2), (119.5, 110.46), ("U1", 3)], SIG)
    T("GND", [("R10", 2), (116.59, 107.4), (121.0, 107.4), (121.0, 108.9)], 0.3)   # FB ground to the IC ground
    b.via("GND", 116.59, 107.4)
    # FB divider top: 3V3 from the top-edge rail, around the input stage, never through the hot loop
    # (up between the 12 V trace and R10's ground via; under the 12 V link only for the last 4 mm)
    T("+3V3", [("R9", 1), (115.7, 110.8), (115.7, 106.6), (115.85, 106.25)], SIG)
    b.via("+3V3", 115.85, 106.25)
    T("+3V3", [(115.85, 106.25), (115.85, 101.75)], SIG, layer=pcbnew.B_Cu)
    b.via("+3V3", 115.85, 101.75)
    T("+3V3", [(115.85, 101.75), (115.85, 101.3), (131.0, 101.3)], SIG)
    T("GND", [("U1", 2), (121.0, 108.9)], 0.4)
    b.via("GND", 121.0, 108.9)
    T("BUCK_CB", [("U1", 1), ("C3", 1)], 0.4)
    # switch node: IC -> boot cap -> catch diode -> inductor, about 6 mm, top only
    T("BUCK_SW", [("U1", 6), ("C3", 2), ("D3", 1), (126.45, 114.3), ("L1", 1)], SW)
    # hot loop return: Cin, diode and Cout grounds together on top; tied down at Cout (TI 11.1/5)
    T("GND", [("C11", 2), ("C1", 2), ("C2", 2), ("D3", 2), ("C4", 2), ("C13", 2)], 0.8)
    for y in (118.9, 120.6):
        b.via("GND", 124.75, y)
        T("GND", [(124.75, y), (126.05, y)], 0.6)
    # +3V3: inductor -> output caps; riser to the top edge; along it to the module's pin 1
    T("+3V3", [("L1", 2), (130.15, 117.4), (129.15, 118.4), ("C4", 1), ("C13", 1)], V33)
    T("+3V3", [("L1", 2), (130.15, 111.6), (131.0, 110.75), (131.0, 101.3), (144.75, 101.3),
               ("C5", 1), ("C6", 1), ("U2", 1)], V33)
    for r in ("R13", "R14"):
        T("+3V3", [(r, 1), (b.pad_xy(r, 1)[0], 101.3)], SIG)
    T("GND", [("D5", 1), (139.3, 102.4)], SIG)
    b.via("GND", 139.3, 102.4)
    T("GND", [("U2", 9), (133.14, 108.3)], 0.4)
    b.via("GND", 133.14, 108.3)
    T("SIDE", [("U2", 5), (139.14, 108.5)], SIG)
    b.via("SIDE", 139.14, 108.5)
    T("SIDE", [(139.14, 108.5), ("JP1", 1)], SIG, layer=pcbnew.B_Cu)
    # relay drive: module pin 10 along the open strip above the sense cells to the base resistor
    T("RELAY_DRV", [("U2", 10), (133.14, 128.7), (118.41, 128.7), ("R5", 1)], SIG)
    T("GND", [("C5", 2), ("C6", 2)], 0.5)
    b.via("GND", 142.85, 106.25)
    b.via("GND", 142.85, 103.4)
    T("GND", [("C5", 2), (142.85, 103.4)], 0.5)
    # EN: up from pin 2, jog left over the unused pins 3/4, RC and pull-up right there
    T("EN", [("U2", 2), (143.64, 109.15), (143.3, 108.85), (142.0, 108.85), (142.0, 103.46), ("R11", 2)], SIG)
    # EN, BOOT and 3V3 down to the back-side pogo pads J2, right behind the module
    T("EN", [(142.0, 103.46), (142.0, 102.7)], SIG)
    b.via("EN", 142.0, 102.7)
    T("EN", [(142.0, 102.7), ("J2", 1)], SIG, layer=pcbnew.B_Cu)
    T("BOOT", [("U2", 8), (134.64, 107.3), ("R14", 2)], SIG)
    b.via("BOOT", 134.64, 107.3)
    j3 = b.pad_xy("J2", 3)
    T("BOOT", [(134.64, 107.3), (135.04, 107.7), (j3[0] - 0.6, 107.7), ("J2", 3)], SIG, layer=pcbnew.B_Cu)
    b.via("+3V3", 135.9, 102.2)
    T("+3V3", [(135.9, 102.2), ("J2", 2)], SIG, layer=pcbnew.B_Cu)
    T("EN", [(142.0, 107.85), ("C7", 1)], SIG)
    T("GND", [("C7", 2), (140.8, 104.75)], 0.4)
    b.via("GND", 140.8, 104.75)
    # sense cells: gate bus and drain link
    for q, rt, rb, cc, dz, rp, x0, yg in CELLS.values():
        g = b.fps[q].Pads()[0].GetNetname()
        T(g, [(rt, 2), (x0, yg), (rb, 1)], SIG)
        T(g, [(x0, yg), (x0 + 2.2, yg), (cc, 1)], SIG)
        T(g, [(x0 + 2.2, yg), (x0 + 4.4, yg), (dz, 1)], SIG)
        T(g, [(x0 + 4.4, yg), (q, 1)], SIG)
        b.via("GND", x0 + 6.0, yg + 2.8)
        T("GND", [(q, 2), (x0 + 6.0, yg + 2.8)], SIG)
        T(b.fps[rp].Pads()[1].GetNetname(), [(q, 3), (rp, 2)], SIG)


hand_route()
# keep the bottom plane whole under the buck and the module; no top tracks under the module
b.keepout(116.2, 102.0, 15.4, 20.0, layers=(pcbnew.B_Cu,), name="buck-plane", tracks=True, vias=False, fills=False)
b.keepout(132.4, 111.3, 13.6, 15.4, layers=(pcbnew.F_Cu, pcbnew.B_Cu), name="module-plane",
          tracks=True, vias=False, fills=False)    # GND stitching under the module is wanted

overlaps, outside = b.courtyard_overlaps()
print("courtyard overlaps:", overlaps or "none")
print("outside board:", outside or "none")

# ---- silkscreen (JLC minimum: 1.0 mm text, 0.15 mm line)
TXT = dict(size=1.0, thickness=0.15)
# J1 legend: a fixed column right of the pads, one label per pad, same height as its pad
for n, lab in zip(range(1, 7), ["DN", "UP", "rU", "rD", "12V", "GND"]):
    x, y = b.pad_xy("J1", n)
    b.text(lab, x + 1.6, y + (0.3 if lab == "GND" else 0), justify="left", **TXT)
b.clip_silk("U2")
for ref, lab in [("TP3", "3V3"), ("TP5", "GND")]:
    if b.label_near(ref, 1, lab, **TXT) is None:
        print("no room for label", lab, "at", ref)
failed = b.place_refs(size=1.0, thickness=0.15)
# back side: pogo pads, USB pads, side jumper, then name, revision and wiring legend
BK = dict(layer=pcbnew.B_SilkS, size=1.0, thickness=0.15)


def back_text(s, x, y, justify="center"):
    t_ = b.text(s, x, y, justify=justify, **BK)
    t_.SetMirrored(True)
    return t_


for ref in ("J2", "JP1", "TP1", "TP2"):
    b.ref_text(ref, visible=False)
for n, lab, dx in ((1, "EN", 2.5), (2, "3V3", -2.6), (3, "BOOT", 2.6), (4, "GND", -2.6)):
    x, y = b.pad_xy("J2", n)
    back_text(lab, x + dx, y)
x, y = b.pad_xy("JP1", 1)
back_text("RH", x + 0.65, y + 2.2)
for ref, lab in (("TP1", "D-"), ("TP2", "D+")):
    x, y = b.pad_xy(ref, 1)
    back_text(lab, x - 1.0, y, justify="left")
for i, line in enumerate(["PROBE WINK  rev " + REV, "github.com/dwzg/probe-wink", "CERN-OHL-P-2.0",
                          "J1: DN UP  harness BK/BL BK/R in",
                          "    rU rD  to retractor BK/R BK/BL",
                          "    12V GND  BK/W BK taps (1 A fuse)",
                          "JP1 bridged = RH (slave)"]):
    if line:
        back_text(line, 135.0, 123.0 + i * 1.8, justify="left")
print("refs on fab layer only (no clear silk spot):", failed or "none")


# ---- route, pour, stitch, check
os.makedirs(BUILD, exist_ok=True)
if "--no-route" not in sys.argv:
    locked = sum(1 for t_ in b.board.GetTracks() if t_.IsLocked())
    log = b.autoroute(BUILD, passes=150)
    print("hand-routed items kept through the autorouter: %d of %d"
          % (sum(1 for t_ in b.board.GetTracks() if t_.IsLocked()), locked))
    print([l for l in log.splitlines() if "completed" in l][-1][-150:])
# extra ground vias where a top ground island would otherwise hang on one via
for ref, pad in (("R16", 2), ("C10", 2), ("Q2", 2), ("R4", 2), ("C9", 2)):
    if b.via_near("GND", ref, pad) is None:
        print("no room for a GND via at", ref, pad)
for layer in (pcbnew.F_Cu, pcbnew.B_Cu):
    b.zone("GND", layer)
print("stitching vias:", b.stitch("GND", pitch=3.0))
print("dangling vias/stubs removed:", b.prune_dangling())
print("unrouted connections:", b.unrouted())
b.save(PCB)
b.write_project(os.path.join(HERE, "probe-wink.kicad_pro"), min_track=0.12)
drc(PCB, os.path.join(BUILD, "drc.rpt"))
print([l.strip() for l in open(os.path.join(BUILD, "drc.rpt")) if l.startswith("** Found")])
# ---- layout checks DRC cannot do: routed copper length between pads that must sit together
# (TI LMR16006 11.1, Espressif ESP32-C3 hardware design guidelines). Limits in mm / vias.
CHECKS = [("BUCK_SW", "U1.6", "L1.1", 8, 0), ("BUCK_SW", "U1.6", "D3.1", 4, 0), ("BUCK_SW", "C3.2", "U1.6", 3, 0),
          ("BUCK_CB", "C3.1", "U1.1", 3, 0), ("+12V", "C2.1", "U1.5", 3, 0), ("GND", "C2.2", "D3.2", 5, 0),
          ("GND", "D3.2", "C4.2", 4, 0), ("BUCK_FB", "U1.3", "R9.2", 3, 0), ("BUCK_FB", "U1.3", "R10.1", 4, 0),
          ("+3V3", "U2.1", "C6.1", 4, 0), ("+3V3", "U2.1", "C5.1", 7, 0), ("+3V3", "L1.2", "U2.1", 45, 0),
          ("EN", "U2.2", "C7.1", 6, 0), ("EN", "U2.2", "R11.2", 10, 0),
          ("GND", "R10.2", "U1.2", 10, 0), ("EN", "U2.2", "J2.1", 15, 1), ("BOOT", "U2.8", "J2.3", 15, 1)]
pb = pcbnew.LoadBoard(PCB)
bad = 0
print("layout checks (routed copper):")
for net, a, c, lim, vias in CHECKS:
    r = path_length(pb, b.resolve(net), a, c)
    ok = r is not None and r[0] <= lim and r[1] <= vias
    bad += not ok
    print("  %-4s %-8s %-6s -> %-6s %s  (limit %g mm, %d vias)" % ("ok" if ok else "FAIL", net, a, c,
          "%.1f mm, %d vias" % r if r else "not connected", lim, vias))
# the buck hot loop (Cin, IC, catch diode, inductor pad) carries only its own nets
HOT = (119.5, 113.3, 126.0, 117.8)
stray = foreign_copper(pb, HOT, {b.resolve(n) for n in ("BUCK_SW", "+12V", "GND", "BUCK_CB")})
print("  %-4s no foreign copper in the buck hot loop %s" % ("ok" if not stray else "FAIL", stray or ""))
bad += bool(stray)
print("layout checks failed:", bad)
render(PCB, os.path.join(BUILD, "pcb.png"), layers="F.Cu,F.SilkS,Edge.Cuts", width=2000)
render(PCB, os.path.join(BUILD, "pcb_back.png"), layers="B.Cu,Edge.Cuts", width=2000)
print("wrote", PCB)
