#!/usr/bin/env python3
"""Probe wink module (rev C board): 3D-printable sealed housing, FDM in ASA/ABS.

Inputs : build/board.json and build/probe-wink-board.step (pcb/export_enclosure.py)
Outputs: build/housing-base.{step,stl}, build/housing-lid.{step,stl} (lid STL in print
         orientation), build/gasket-tpu.stl, build/assembly.step, build/assembly-cut.step;
         base and lid STEP/STL also to models/ (committed)
Checks : check.py (interference, groove, screws, fingers, lid FE, print overhangs)

Concept
  Base = open box with a flat gasket flange on top; lid = flat plate with a deep locating
  skirt (also its stiffening rim) and press fingers with crush nubs that clamp the board
  edge strip straight down onto the support ledge (the board is too full for interior pins).
  The cable gland sits in the gland end wall, pointing sideways: horizontal when the box is
  mounted floor-down, pointing down when mounted on a vertical panel gland-end-down (best).
  That end wall is flush with the flange so the gland seals on one flat printed face.
  Its hex must clear the mounting surface, which sets the box height (not the parts).
  Seal: 1.5 mm silicone cord in a 2.2 x 1.2 groove (20 % squeeze, 67 % fill as drawn,
  70-79 % as printed and hot). Lid closes plastic-on-plastic on the lands (hard stop).
  Eight M3 screws outside the groove (4 corners + 2 per long side), all >= 17 mm from the
  antenna. Lid 4.0 mm (M12 box; 3.6 mm for the M16 box): lid FE (est.) keeps >= 15 % squeeze.
  The board is held by 7 press fingers on its edge strip (crush nubs 0.6 mm, 0.2 mm preload)
  and located in x by 4 end stops; it is removed for programming (pogo pads underneath).
  Breather vent inside on the chamber floor, on a raised printed top surface, venting
  through the floor into an underside channel that runs out of the gland end face.

Print (0.4 mm nozzle, 0.2 mm layers, ASA, enclosure, no supports)
  Base upright on its floor, lid outer face down. 6 walls (2.4 mm = solid walls),
  6 top / 6 bottom layers, 25 % gyroid. All critical heights are on the 0.2 mm grid.
  The gland hole is horizontal: flat-topped teardrop (4-6 mm bridge), tapped after printing.
  Optional: ironing on the base rim top (sealing land) and the lid's last layer.

Hardware per module (M12 gland default; M16 variant via PW_GLAND=M16)
  1x cable gland M12x1.5, nylon, IP68, clamping 3.5-7 mm, hex 15 AF, thread 8 mm
     (e.g. Lapp SKINTOP ST-M 12x1.5): tap the printed hole M12x1.5, no locknut.
     Lead: sheathed 6 x 0.5 mm^2 (e.g. LiYY 6x0.5, about 6.2 mm OD; the lines carry
     signal-level current only). M16 (4.5-10 mm, hex 20 AF) for a 6 x 0.75 mm^2 lead
     (about 8-8.5 mm), 5 mm taller box.
  8x M3 heat-set insert, OD 4.6, L 5.7 (e.g. Ruthex M3x5.7), hole 4.0 x 7.0 deep
  8x M3x10 ISO 7380 / ISO 4762 stainless + 8x M3 washer ISO 7089 (7 OD, 0.5 thick):
     4.0 lid + 0.5 washer leaves 5.5 mm in the 5.7 mm insert, tip 1.5 mm above the hole bottom
     (M16 box: 3.6 lid leaves 5.9 mm, full insert, tip 1.1 mm above the bottom)
     Torque 0.4-0.5 N m. No anaerobic threadlocker on ASA/ABS (stress cracking); if any,
     a drop of non-anaerobic thread lock (e.g. Loctite 425/Vibra-Tite) or nothing.
  1x silicone cord 1.5 mm, 50-60 Shore A, length printed by this script (+2 mm),
     butt joint with silicone-rated CA + primer, joint on a long side
     (printed TPU bead gasket-tpu.stl: 78 % fill, but TPU 95A needs ~1.4 kN and lifts the lid
     off the seal (FE est.); only usable in TPU <= ~70A)
  1x adhesive PTFE breather vent, 10 mm OD (active area >= 3 mm) on the inside seat
  4x M5 mounting bolts + M5 penny washers ISO 7093 (15 OD) + 4x compression limiter
     8 OD x 5.3 ID x 4.9 L (steel or brass tube) in the ear slots
"""
import json
import math
import os
from types import SimpleNamespace

from build123d import (Align, Axis, Box, Circle, Compound, Cylinder, Plane, Polygon, Pos, Rectangle,
                       RectangleRounded, Rot, SlotCenterToCenter, Text, export_step, export_stl, extrude,
                       import_step, loft)

HERE = os.path.dirname(os.path.abspath(__file__))
B = os.path.join(HERE, "build")
MODELS = os.path.join(HERE, "models")    # committed copies of base and lid (STEP + STL)
STEP_TIME = "2000-01-01T00:00:00"        # fixed STEP header time: an unchanged rerun leaves git clean
board = json.load(open(os.path.join(B, "board.json")))
parts = {p["ref"]: p for p in board["parts"]}
BOTTOM_REFS = {"J2", "TP1", "TP2", "JP1"}   # bare pads on the bottom side (rev C)

GRID = 0.2                 # layer height: all critical z values are multiples of this


def snap(v):
    """Round up to the layer grid."""
    return round(math.ceil(v / GRID - 1e-6) * GRID, 3)


# ---------------------------------------------------------------- parameters (mm)
WALL = 2.4                 # 6 perimeters at 0.4 mm
FLOOR = 2.4
LID = 3.6                  # lid FE: 3.0 gives 14.7 % min squeeze, 3.6 gives 16 % (check.py --sweep)
STANDOFF = 3.0             # board underside above the floor (J1 wire stubs, bottom pads)
GAP_X, GAP_Y = 0.25, 0.3   # board edge to end stops (x) and to the cavity wall (y)
TOP_CLEAR = 1.5            # tallest part to the lid underside, minimum
PART_H = board.get("tallest_above_board", 6.0)
CHAMBER = 14.0             # wiring chamber between the gland wall and the board edge
ANT_OVERHANG = board.get("antenna_overhang", 0.0)   # 6.24: module tip beyond the board edge
ANT_AIR = 6.0              # air from the module tip to the end wall (plastic only, see notes)
R_IN = 2.0                 # cavity corner radius
CORD_D = 1.5               # silicone cord
GROOVE_W, GROOVE_D = 2.2, 1.2
LAND_IN, LAND_OUT = 1.6, 1.6
FLANGE_T = 3.0             # flange thickness at the rim, 45 deg underside below it
INSERT = dict(hole=4.0, od=4.6, length=5.7, depth=7.0)
INSERT_WALL = 1.6          # plastic around the insert OD (to the groove and outside)
SCREW = dict(d=3.0, length=10.0, clear=3.4, washer_t=0.5, washer_od=7.0)
N_MID = 2                  # extra lid screws per long side (plus the 4 corners)
SKIRT_T, SKIRT_D, SKIRT_GAP = 1.6, 8.0, 0.3   # deep skirt = the lid's stiffening rim (C12 clears by 2.6)
SKIRT_NOTCH_T = 4.0        # skirt thickness above the cable notch at the gland end
SKIRT_END_T = (4.0, 1.6)   # skirt thickness on the short ends (gland end thick; antenna end thin, less plastic at RF)
F_LEN, F_DEPTH, F_N = (5.0, 3.0, 2.5), 1.0, 7  # press fingers on the board edge strip, above the ledge
F_CLEAR = 1.0              # finger footprint to any top-side courtyard (>= 1.25 to part bodies)
F_UPPER = 1.6              # finger depth above the part zone (stiffer column)
TIP_D, TIP_L, PIN_PRELOAD = 0.6, 0.6, 0.2   # crush nub; nominal interference with the board top
LEDGE = 1.2                # board rests on this edge strip
STOP_T = 1.6
EAR = dict(t=5.0, w=16.0, slot_w=8.4, travel=3.0, end=3.0, tool_r=7.0)
VENT = dict(hole=3.0, seat_d=12.6, seat_h=1.0, chan_w=3.0, chan_d=1.0, y=12.0)
GLAND = os.environ.get("PW_GLAND", "M12")
GLANDS = {   # tap drill, hex across flats, thread length, clamping range, outside length
    "M12": dict(thread=12.0, tap=10.5, af=15.0, thread_len=8.0, cable=(3.5, 7.0), out_len=20.0),
    "M16": dict(thread=16.0, tap=14.5, af=20.0, thread_len=8.0, cable=(4.5, 10.0), out_len=24.0),
}
# Lid thickness per box: the shorter M12 box needs 4.0 for the same gasket squeeze (lid FE,
# check.py --sweep); the M16 box 3.6.
LID = {"M12": 4.0, "M16": 3.6}[GLAND]

# ---------------------------------------------------------------- board (true outline from the STEP)
PCB_STEP = import_step(os.path.join(B, "probe-wink-board.step"))
_kids = list(PCB_STEP.children[0].children) if len(PCB_STEP.children) == 1 else list(PCB_STEP.children)
_pcb = [k for k in _kids if str(getattr(k, "label", "")).endswith("_PCB")][0]
_pbb = _pcb.bounding_box()
bw, bh = round(_pbb.size.X, 3), round(_pbb.size.Y, 3)      # 46.0 x 40.0 (board.json adds line width)
bt = board["board"]["thickness"]
_mod = [k for k in _kids if "ESP32" in str(getattr(k, "label", ""))][0].bounding_box()
MODULE = dict(x0=_mod.min.X - _pbb.min.X, x1=_mod.max.X - _pbb.min.X,
              y0=_pbb.max.Y - _mod.max.Y, y1=_pbb.max.Y - _mod.min.Y, h=_mod.max.Z - _pbb.min.Z - bt)


def rect_dist(x, y, p):
    """Distance from a point to a part courtyard rectangle (board coords)."""
    dx = max(p["x0"] - x, 0.0, x - (p["x0"] + p["w"]))
    dy = max(p["y0"] - y, 0.0, y - (p["y0"] + p["h"]))
    return math.hypot(dx, dy)


def pick_fingers():
    """Press fingers on the board edge strip (directly above the support ledge): footprints
    F_LEN (5 or 3 mm) x F_DEPTH at a board edge, >= F_CLEAR from every top-side courtyard
    (fiducials: >= 1.5 from the dot centre), away from the J1 wire zone. Returns
    board-coordinate rects (x0, x1, y0, y1)."""
    top = [p for p in board["parts"] if p["ref"] not in BOTTOM_REFS]

    def clear(r):
        x0, x1, y0, y1 = r
        for p in top:
            if p["ref"].startswith("FID"):
                cx, cy = p["x0"] + p["w"] / 2, p["y0"] + p["h"] / 2
                d = math.hypot(max(x0 - cx, 0, cx - x1), max(y0 - cy, 0, cy - y1))
                if d < 1.5:
                    return False
                continue
            dx = max(p["x0"] - x1, 0.0, x0 - (p["x0"] + p["w"]))
            dy = max(p["y0"] - y1, 0.0, y0 - (p["y0"] + p["h"]))
            if math.hypot(dx, dy) < F_CLEAR:
                return False
        return True

    j1 = parts["J1"]
    cand = []
    for ln in F_LEN:
        c = ln / 2
        while c <= bw - ln / 2 + 1e-6:                   # long edges
            for (y0, y1) in ((0.0, F_DEPTH), (bh - F_DEPTH, bh)):
                cand.append((c - ln / 2, c + ln / 2, y0, y1))
            c += 0.1
        c = ln / 2
        while c <= bh - ln / 2 + 1e-6:                   # short edges
            if not (j1["y0"] - 2 < c + ln / 2 and c - ln / 2 < j1["y0"] + j1["h"] + 2):
                cand.append((0.0, F_DEPTH, c - ln / 2, c + ln / 2))
            cand.append((bw - F_DEPTH, bw, c - ln / 2, c + ln / 2))
            c += 0.1
    cand = [r for r in cand if clear(r)]
    ctr = lambda r: ((r[0] + r[1]) / 2, (r[2] + r[3]) / 2)
    # spread: corners first, then edge middles; >= 9 mm apart; longer fingers win ties
    targets = [(0, 0), (bw, 0), (0, bh), (bw, bh), (bw / 2, 0), (bw / 2, bh), (bw, bh / 2)]
    out = []
    for tx, ty in targets:
        cc = [r for r in cand if all(math.dist(ctr(r), ctr(q)) > 9 for q in out)]
        if cc and len(out) < F_N:
            out.append(min(cc, key=lambda r: math.dist(ctr(r), (tx, ty)) - 0.5 * max(r[1] - r[0], r[3] - r[2])))
    return [tuple(round(v, 2) for v in r) for r in out]


def rrect(w, h, r, x=0.0, y=0.0, z=0.0):
    return Plane.XY.offset(z) * Pos(x, y) * RectangleRounded(w, h, r)


def build(gland=GLAND, n_mid=N_MID, lid_t=LID, skirt_d=SKIRT_D, ant_air=ANT_AIR, skirt_end_t=SKIRT_END_T,
          n_ant=0):
    G = GLANDS[gland]
    z_board = FLOOR + STANDOFF
    z_btop = round(z_board + bt, 3)
    # gland: hex corners 0.5 above the mounting plane; hex face fully on the base end face;
    # tapped hole >= 1.6 below the groove bottom
    z_g = snap(G["af"] / math.sqrt(3) + 0.5)
    z_top = max(snap(z_btop + PART_H + TOP_CLEAR),
                snap(z_g + G["thread"] / 2 + 1.6 + GROOVE_D),
                snap(z_g + G["af"] / 2 + 0.4))

    # skirt no deeper than 1 mm above the tallest part (M12 box: 4.0, M16 box: 8.0)
    skirt_d = min(skirt_d, math.floor((z_top - z_btop - PART_H - 1.0) / GRID + 1e-6) * GRID)
    # cavity in board coordinates (x right, y down), then centred
    cx0, cx1 = -GAP_X - CHAMBER, bw + ANT_OVERHANG + ant_air
    cy0, cy1 = -GAP_Y, bh + GAP_Y
    L_in, W_in = cx1 - cx0, cy1 - cy0
    xc, yc = (cx0 + cx1) / 2, (cy0 + cy1) / 2

    def bxy(x, y):
        """Board coordinates (y down) -> housing coordinates (y up, centred on the cavity)."""
        return (x - xc, yc - y)

    FL = LAND_IN + GROOVE_W + LAND_OUT              # flange width from the cavity wall
    mid = LAND_IN + GROOVE_W / 2                    # groove centre line from the cavity wall
    R_OUT, R_FL = R_IN + WALL, R_IN + FL
    r_lobe = INSERT["od"] / 2 + INSERT_WALL
    r_s = R_IN + LAND_IN + GROOVE_W + INSERT_WALL + INSERT["od"] / 2   # corner screw from arc centre
    d_s = LAND_IN + GROOVE_W + INSERT_WALL + INSERT["od"] / 2         # side screw from cavity wall
    corner_ext = r_s / math.sqrt(2) + r_lobe - R_IN                   # corner lobe beyond the wall (x)
    END_T = snap(max(FL, G["thread_len"], corner_ext))                # gland end wall, flush face
    fx0, fx1 = -L_in / 2 - END_T, L_in / 2 + FL
    fW = W_in + 2 * FL
    fL, fxc = fx1 - fx0, (fx0 + fx1) / 2
    L_out, W_out = L_in + 2 * WALL, W_in + 2 * WALL

    # lid screws: 4 on the corner diagonals + n_mid per long side, all outside the groove
    cxc, cyc = L_in / 2 - R_IN, W_in / 2 - R_IN
    corners = [(sx * (cxc + r_s / math.sqrt(2)), sy * (cyc + r_s / math.sqrt(2))) for sx in (-1, 1)
               for sy in (-1, 1)]
    xl, xr = -(cxc + r_s / math.sqrt(2)), cxc + r_s / math.sqrt(2)
    mids = [(xl + k * (xr - xl) / (n_mid + 1), sy * (W_in / 2 + d_s)) for k in range(1, n_mid + 1)
            for sy in (-1, 1)]
    j1 = parts["J1"]
    gy = bxy(0, j1["y0"] + j1["h"] / 2)[1]          # gland on the J1 pad column centre line
    ant_screws = [(L_in / 2 + d_s, gy)] if n_ant else []      # nylon, printed pilot, no insert
    screws = corners + mids + ant_screws

    fingers = pick_fingers()

    def lobe_plan(sx, sy, diag):
        f = Pos(sx, sy) * Circle(r_lobe)
        if diag:     # bar from the screw to the cavity corner arc centre
            ax, ay = math.copysign(cxc, sx), math.copysign(cyc, sy)
            ang = math.degrees(math.atan2(sy - ay, sx - ax))
            f += Pos((sx + ax) / 2, (sy + ay) / 2) * Rot(0, 0, ang) * Rectangle(math.hypot(sx - ax, sy - ay),
                                                                                2 * r_lobe)
        elif (sx, sy) in ant_screws:
            f += Pos((sx + L_in / 2) / 2, sy) * Rectangle(abs(sx - L_in / 2), 2 * r_lobe)
        else:
            yw = math.copysign(W_in / 2, sy)
            f += Pos(sx, (sy + yw) / 2) * Rectangle(2 * r_lobe, abs(sy - yw))
        return f

    # ------------------------------------------------------------ base
    body = extrude(rrect(L_out, W_out, R_OUT), z_top)
    flange_face = rrect(fL, fW, R_FL, x=fxc)
    end_block = extrude(flange_face & Pos(fx0 + (END_T + R_IN + 1) / 2, 0) * Rectangle(END_T + R_IN + 1, fW + 1),
                        z_top)
    z_fl = z_top - FLANGE_T
    flange = loft([rrect(L_out, W_out, R_OUT, z=z_fl - (FL - WALL)), rrect(fL, fW, R_FL, x=fxc, z=z_fl)])
    flange += Pos(0, 0, z_fl) * extrude(flange_face, FLANGE_T)
    base = body + end_block + flange
    z_lobe_b = z_top - INSERT["depth"] - 1.0        # solid under the insert hole
    for (sx, sy) in screws:
        diag = (sx, sy) in corners
        lobe = extrude(lobe_plan(sx, sy, diag), z_top)
        if (sx, sy) in ant_screws:
            e = sx + r_lobe - L_out / 2
            lobe -= Pos(L_out / 2 + e, sy, z_lobe_b) * Rot(0, -45, 0) * \
                Box(60, 4 * r_lobe, 60, align=(Align.CENTER, Align.CENTER, Align.MAX))
        elif not diag:   # 45 deg underside back to the wall: prints without support
            e = abs(sy) + r_lobe - W_out / 2
            cut = Pos(sx, math.copysign(W_out / 2 + e, sy), z_lobe_b) * Rot(math.copysign(45, sy), 0, 0) * \
                Box(4 * r_lobe, 60, 60, align=(Align.CENTER, Align.CENTER, Align.MAX))
            lobe -= cut
        base += lobe
    # mounting ears at floor level, on the long sides between the corner and first mid screws
    xm = [x for x, _ in mids if _ > 0] or [0.0]
    ear_x = [(xl + min(xm)) / 2, (xr + max(xm)) / 2] if mids else [xl + 14, xr - 14]
    y_bolt = fW / 2 + EAR["tool_r"]                 # innermost bolt position: socket clears flange
    y_end = y_bolt + EAR["travel"] + EAR["slot_w"] / 2 + EAR["end"]
    ears = []
    for ex in ear_x:
        for sg in (1, -1):
            plate = Pos(ex, sg * (W_out / 2 - 1 + (y_end - W_out / 2 + 1) / 2)) * \
                RectangleRounded(EAR["w"], y_end - W_out / 2 + 1, 3.0)
            slot = Pos(ex, sg * (y_bolt + EAR["travel"] / 2)) * \
                SlotCenterToCenter(EAR["travel"], EAR["slot_w"], rotation=90)
            ears.append((ex, sg * y_bolt))
            base += extrude(plate, EAR["t"])
            base -= extrude(slot, EAR["t"])
    # cavity
    base -= Pos(0, 0, FLOOR) * extrude(rrect(L_in, W_in, R_IN), z_top)
    # ledge along the board outline (the press fingers push straight down onto it), end stops
    lx0, ly0 = bxy(-GAP_X, bh + GAP_Y)
    ledge = Pos(lx0 + (bw + 2 * GAP_X) / 2, ly0 + (bh + 2 * GAP_Y) / 2, FLOOR) * \
        Box(bw + 2 * GAP_X, bh + 2 * GAP_Y, STANDOFF, align=(Align.CENTER, Align.CENTER, Align.MIN))
    ix, iy = bxy(bw / 2, bh / 2)
    ledge -= Pos(ix, iy, FLOOR) * Box(bw - 2 * LEDGE, bh - 2 * LEDGE, STANDOFF,
                                      align=(Align.CENTER, Align.CENTER, Align.MIN))
    base += ledge
    stops = []
    h_stop = z_btop - 0.2 - FLOOR
    for (x0, x1, y0, y1) in [(-GAP_X - STOP_T, -GAP_X, 0.6, 3.6), (-GAP_X - STOP_T, -GAP_X, bh - 3.6, bh - 0.6),
                             (bw + GAP_X, bw + GAP_X + STOP_T, 2.0, 7.0),
                             (bw + GAP_X, bw + GAP_X + STOP_T, bh - 7.0, bh - 2.0)]:
        sx, sy = bxy((x0 + x1) / 2, (y0 + y1) / 2)
        s = Pos(sx, sy, FLOOR) * Box(x1 - x0, y1 - y0, h_stop, align=(Align.CENTER, Align.CENTER, Align.MIN))
        stops.append(s)
        base += s
    # breather vent: raised seat on the chamber floor, hole down into an underside channel
    vx, vy = -L_in / 2 + CHAMBER / 2, VENT["y"]
    base += Pos(vx, vy, FLOOR) * Cylinder(VENT["seat_d"] / 2, VENT["seat_h"],
                                          align=(Align.CENTER, Align.CENTER, Align.MIN))
    base -= Pos(vx, vy, 0) * Cylinder(VENT["hole"] / 2, FLOOR + VENT["seat_h"] + 1,
                                      align=(Align.CENTER, Align.CENTER, Align.MIN))
    ey = gy + math.copysign(G["af"] / math.sqrt(3) + 3.0, vy - gy)   # exit beside the gland hex
    ex_ = fx0 - 1.0
    ang = math.degrees(math.atan2(ey - vy, ex_ - vx))
    clen = math.hypot(ey - vy, ex_ - vx)
    base -= Pos((vx + ex_) / 2, (vy + ey) / 2, 0) * Rot(0, 0, ang) * \
        Box(clen, VENT["chan_w"], VENT["chan_d"], align=(Align.CENTER, Align.CENTER, Align.MIN))
    # gland hole through the flush end wall: tap drill circle + flat-topped teardrop (prints
    # without support; flat top stays inside the thread major diameter, sealing face intact)
    r = G["tap"] / 2
    h_t = G["thread"] / 2 - 0.1
    tear = Circle(r) + Polygon((0, 0), (r / math.sqrt(2), r / math.sqrt(2)), (r * math.sqrt(2) - h_t, h_t),
                               (-(r * math.sqrt(2) - h_t), h_t), (-r / math.sqrt(2), r / math.sqrt(2)), align=None)
    base -= extrude(Plane.YZ.offset(fx0 - 1) * Pos(gy, z_g) * tear, END_T + 2 + R_IN)
    # inserts
    for (sx, sy) in screws:   # nylon screw at the antenna end: 2.5 mm pilot, self-tapped, no insert
        dia = 2.5 if (sx, sy) in ant_screws else INSERT["hole"]
        base -= Pos(sx, sy, z_top) * Cylinder(dia / 2, INSERT["depth"], align=(Align.CENTER, Align.CENTER, Align.MAX))
    # gasket groove: cut last, after every additive feature
    groove = Pos(0, 0, z_top - GROOVE_D) * extrude(
        rrect(L_in + 2 * (LAND_IN + GROOVE_W), W_in + 2 * (LAND_IN + GROOVE_W), R_IN + LAND_IN + GROOVE_W)
        - rrect(L_in + 2 * LAND_IN, W_in + 2 * LAND_IN, R_IN + LAND_IN), GROOVE_D + 1)
    base -= groove

    # ------------------------------------------------------------ lid
    plan = rrect(fL, fW, R_FL, x=fxc)
    for (sx, sy) in screws:
        plan += lobe_plan(sx, sy, (sx, sy) in corners)
    lid = Pos(0, 0, z_top) * extrude(plan, lid_t)
    # locating skirt, also the perimeter stiffening rim
    so = rrect(L_in - 2 * SKIRT_GAP, W_in - 2 * SKIRT_GAP, R_IN - SKIRT_GAP)
    si = rrect(L_in - 2 * (SKIRT_GAP + SKIRT_T), W_in - 2 * (SKIRT_GAP + SKIRT_T), 0.4)
    lid += Pos(0, 0, z_top - skirt_d) * extrude(so - si, skirt_d)
    for sg, t in zip((-1, 1), skirt_end_t):      # thicker short-end skirts (screwless 50 mm spans)
        if t > SKIRT_T:
            lid += Pos(sg * (L_in / 2 - SKIRT_GAP), 0, z_top) * Box(t, W_in - 2 * SKIRT_GAP - 2 * R_IN - 2 * t, skirt_d,
                                                                   align=(Align.MAX if sg > 0 else Align.MIN,
                                                                          Align.CENTER, Align.MAX))
    # over the cable at the gland end: notch the skirt, thicken it above the notch
    z_notch = z_g + G["cable"][1] / 2 + 1.0
    nw = G["cable"][1] + 4
    lid += Pos(-L_in / 2 + SKIRT_GAP, gy, z_top) * Box(SKIRT_NOTCH_T, nw + 12, z_top - z_notch,
                                                       align=(Align.MIN, Align.CENTER, Align.MAX))
    lid -= Pos(-L_in / 2, gy, z_notch) * Box(12, nw, 20, align=(Align.CENTER, Align.CENTER, Align.MAX))
    # press fingers on the board edge strip with crush nubs (nominal PIN_PRELOAD interference)
    z_tip = z_btop - PIN_PRELOAD
    z_up = snap(z_btop + PART_H + 1.0)              # above every part: finger may widen here
    pins, nubs = [], []
    for (x0, x1, y0, y1) in fingers:
        horiz = (x1 - x0) > (y1 - y0)               # finger on a long (y) edge
        if horiz:
            ux0, ux1 = x0, x1
            uy0, uy1 = (0.0, F_UPPER) if y0 < bh / 2 else (bh - F_UPPER, bh)
        else:
            uy0, uy1 = y0, y1
            ux0, ux1 = (0.0, F_UPPER) if x0 < bw / 2 else (bw - F_UPPER, bw)

        def col(a0, a1, b0, b1, z0, z1):
            px, py = bxy((a0 + a1) / 2, (b0 + b1) / 2)
            return Pos(px, py, z0) * Box(a1 - a0, b1 - b0, z1 - z0, align=(Align.CENTER, Align.CENTER, Align.MIN))
        pin = col(x0, x1, y0, y1, z_tip + TIP_L, z_up + 0.4) + col(ux0, ux1, uy0, uy1, z_up, z_top)
        nx, ny = bxy((x0 + x1) / 2, (y0 + y1) / 2)
        nub = Pos(nx, ny, z_tip) * Cylinder(TIP_D / 2, TIP_L + 0.2, align=(Align.CENTER, Align.CENTER, Align.MIN))
        pins.append(pin + nub)
        nubs.append((nx, ny))
        lid += pin + nub
    for (sx, sy) in screws:
        lid -= Pos(sx, sy, z_top) * Cylinder(SCREW["clear"] / 2, lid_t, align=(Align.CENTER, Align.CENTER, Align.MIN))
    # debossed label and antenna marker
    txt = (Pos(*bxy(bw * 0.38, bh * 0.35)) * Text("PROBE WINK", 5.5)
           + Pos(*bxy(bw * 0.38, bh * 0.62)) * Text("L / R", 4.0)
           + Pos(*bxy(bw + ANT_OVERHANG / 2, bh / 2)) * Text("((·", 4.0))
    lid -= Pos(0, 0, z_top + lid_t - 0.6) * extrude(txt, 0.6)

    # ------------------------------------------------------------ printed TPU bead (fallback seal)
    TPU_B, TPU_H = 1.2, 1.6
    gasket = extrude(rrect(L_in + 2 * mid + TPU_B, W_in + 2 * mid + TPU_B, R_IN + mid + TPU_B / 2)
                     - rrect(L_in + 2 * mid - TPU_B, W_in + 2 * mid - TPU_B, R_IN + mid - TPU_B / 2), TPU_H)

    # ------------------------------------------------------------ board, hardware envelopes (for checks)
    o = _pbb.min
    off = Pos(bxy(0, 0)[0] - o.X, bxy(0, 0)[1] - _pbb.max.Y, z_board - o.Z)
    pcb = PCB_STEP.moved(off)
    pcb_board = _pcb.moved(off)                     # parts carry global placements: move each
    pcb_parts = [k.moved(off) for k in _kids if k is not _pcb]
    extras = []
    if "K1" in parts:   # relay has no 3D model in the library: show its envelope
        k = parts["K1"]
        kx, ky = bxy(k["x0"] + k["w"] / 2, k["y0"] + k["h"] / 2)
        extras.append(Pos(kx, ky, z_btop) * Box(6.0, 10.0, board.get("relay_height", 5.65),
                                                align=(Align.CENTER, Align.CENTER, Align.MIN)))
    hexr = G["af"] / math.sqrt(3)
    gl_out = (Plane.YZ.offset(fx0) * Pos(gy, z_g) * extrude(RectangleRounded(2 * hexr, 2 * hexr, hexr - 0.01),
                                                            -G["out_len"]))
    cable_in = Plane.YZ.offset(-L_in / 2) * Pos(gy, z_g) * extrude(Circle(G["cable"][1] / 2), 5.0)
    metal = [p for p in screws if p not in ant_screws]
    inserts = [Pos(sx, sy, z_top) * Cylinder(INSERT["od"] / 2, INSERT["length"],
                                             align=(Align.CENTER, Align.CENTER, Align.MAX)) for sx, sy in metal]
    tip = SCREW["length"] - lid_t - SCREW["washer_t"]
    screw_sol = [Pos(sx, sy, z_top - tip) * Cylinder(SCREW["d"] / 2, SCREW["length"],
                                                     align=(Align.CENTER, Align.CENTER, Align.MIN))
                 for sx, sy in metal]
    ax0, ay0 = bxy(bw, MODULE["y1"])
    antenna = Pos(ax0, ay0, z_btop) * Box(MODULE["x1"] - bw, MODULE["y1"] - MODULE["y0"], MODULE["h"],
                                          align=(Align.MIN, Align.MIN, Align.MIN))
    groove_line = rrect(L_in + 2 * mid, W_in + 2 * mid, R_IN + mid, z=z_top).wire()

    return SimpleNamespace(**{k: v for k, v in locals().items() if not k.startswith("_")})


def export(h):
    # build/ gets everything; models/ (in git) gets base and lid, M16 variant as *-m16.*
    sfx = "" if h.gland == "M12" else "-" + h.gland.lower()
    lid_print = h.lid.rotate(Axis.X, 180)
    for d, s in ((B, ""), (MODELS, sfx)):
        os.makedirs(d, exist_ok=True)
        export_step(h.base, os.path.join(d, "housing-base%s.step" % s), timestamp=STEP_TIME)
        export_stl(h.base, os.path.join(d, "housing-base%s.stl" % s))
        export_step(h.lid, os.path.join(d, "housing-lid%s.step" % s), timestamp=STEP_TIME)
        export_stl(lid_print, os.path.join(d, "housing-lid%s.stl" % s))
    export_stl(h.gasket, os.path.join(B, "gasket-tpu.stl"))
    asm = Compound(children=[h.base, h.lid, h.pcb, *h.extras])
    export_step(asm, os.path.join(B, "assembly.step"))
    cut = Pos(0, h.gy, 0) * Box(500, 500, 500, align=(Align.CENTER, Align.MIN, Align.CENTER))
    export_step(Compound(children=[h.base - cut, h.lid - cut, h.pcb, *h.extras]),
                os.path.join(B, "assembly-cut.step"))


def summary(h):
    bb, lb = h.base.bounding_box(), h.lid.bounding_box()
    print("gland %s at z %.1f; base rim z_top %.1f; board top %.1f; lid underside %.1f" %
          (h.gland, h.z_g, h.z_top, h.z_btop, h.z_top))
    print("body %.1f x %.1f (gland end wall %.1f), flange %.1f x %.1f, height %.1f, over ears/gland-free %.1f x %.1f" %
          (h.L_in + h.END_T + WALL, h.W_out, h.END_T, h.fL, h.fW, h.z_top + h.lid_t, bb.size.X, bb.size.Y))
    print("cavity %.1f x %.1f x %.1f; base bbox z %.2f..%.2f; lid bbox z %.2f..%.2f" %
          (h.L_in, h.W_in, h.z_top - FLOOR, bb.min.Z, bb.max.Z, lb.min.Z, lb.max.Z))
    print("screws (housing xy):", [(round(x, 1), round(y, 1)) for x, y in h.screws])
    print("press fingers (board rects):", h.fingers)
    print("cord length on the groove centre line: %.0f mm" % h.groove_line.length)
    print("volume: base %.1f cm3, lid %.1f cm3, TPU bead %.2f cm3" %
          (h.base.volume / 1000, h.lid.volume / 1000, h.gasket.volume / 1000))


if __name__ == "__main__":
    H = build()
    export(H)
    summary(H)
