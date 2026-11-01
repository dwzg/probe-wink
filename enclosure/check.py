#!/usr/bin/env python3
"""Measured checks for housing.py (rev C). Prints a pass/fail table; exit code 1 on any FAIL.

Everything here is measured on the generated solids (section probes, ray casts, boolean
intersection volumes, BRep distances, mesh normals), except where a line says "est.":
the lid FE (grillage + gasket springs), gasket force, mass and print time are estimates.

  enclosure/.venv/bin/python enclosure/check.py            # default gland (M12)
  PW_GLAND=M16 enclosure/.venv/bin/python enclosure/check.py
  enclosure/.venv/bin/python enclosure/check.py --sweep    # lid FE for other screw counts / lid
"""
import math
import sys

import numpy as np
import scipy.sparse as sp
import scipy.sparse.linalg as spla
from scipy import ndimage

import housing as hs
from build123d import Align, Box, Compound, Cylinder, Plane, Pos, Vector

ROWS = []


def row(name, ok, measured, required, kind="V"):
    ROWS.append((name, ok, measured, required, kind))


# ---------------------------------------------------------------- geometry helpers
def inside(shape, p):
    return shape.is_inside(Vector(*p), 1e-4)


def edge_along(shape, p, d, smax, inside_start):
    """Distance from p along unit vector d to the first change of material state (bisection
    after a coarse 0.05 mm scan)."""
    p, d = np.array(p, float), np.array(d, float)
    step, t = 0.05, 0.0
    while t < smax:
        if inside(shape, p + (t + step) * d) != inside_start:
            lo, hi = t, t + step
            for _ in range(12):
                m = (lo + hi) / 2
                if inside(shape, p + m * d) == inside_start:
                    lo = m
                else:
                    hi = m
            return (lo + hi) / 2
        t += step
    return smax


def tess(shape, tol=0.03):
    v, t = shape.tessellate(tol, 0.2)
    return np.array([(q.X, q.Y, q.Z) for q in v]), np.array(t)


def columns(shape, x0, y0, nx, ny, d, tol=0.03):
    """Vertical ray casts on a grid: returns {(i, j): [(z_in, z_out), ...]}."""
    V, T = tess(shape, tol)
    hits = {}
    ox, oy = x0 + 0.00123, y0 + 0.00171            # avoid rays through mesh edges
    for tri in V[T]:
        (ax, ay, az), (bx, by, bz), (cx, cy, cz) = tri
        den = (by - cy) * (ax - cx) + (cx - bx) * (ay - cy)
        if abs(den) < 1e-12:
            continue
        i0 = max(int(math.ceil((min(ax, bx, cx) - ox) / d)), 0)
        i1 = min(int(math.floor((max(ax, bx, cx) - ox) / d)), nx - 1)
        j0 = max(int(math.ceil((min(ay, by, cy) - oy) / d)), 0)
        j1 = min(int(math.floor((max(ay, by, cy) - oy) / d)), ny - 1)
        if i1 < i0 or j1 < j0:
            continue
        I, J = np.meshgrid(np.arange(i0, i1 + 1), np.arange(j0, j1 + 1), indexing="ij")
        px, py = ox + I * d, oy + J * d
        l1 = ((by - cy) * (px - cx) + (cx - bx) * (py - cy)) / den
        l2 = ((cy - ay) * (px - cx) + (ax - cx) * (py - cy)) / den
        l3 = 1 - l1 - l2
        m = (l1 >= 0) & (l2 >= 0) & (l3 >= 0)
        z = l1 * az + l2 * bz + l3 * cz
        for i, j, zz in zip(I[m], J[m], z[m]):
            hits.setdefault((int(i), int(j)), []).append(float(zz))
    out = {}
    for k, zs in hits.items():
        zs = sorted(zs)
        if len(zs) % 2:
            zs = zs[:-1]
        out[k] = [(zs[n], zs[n + 1]) for n in range(0, len(zs), 2) if zs[n + 1] - zs[n] > 1e-4]
    return out


def leaf_parts(pcb):
    kids = list(pcb.children)
    while len(kids) == 1:
        kids = list(kids[0].children)
    board_solid = [k for k in kids if str(k.label).endswith("_PCB")][0]
    return board_solid, [k for k in kids if not str(k.label).endswith("_PCB")]


# ---------------------------------------------------------------- gasket models (est.)
def shore_to_E(sh):
    """Gent's relation, Shore A -> Young's modulus (MPa)."""
    return 0.0981 * (56 + 7.62336 * sh) / (0.137505 * (254 - 2.54 * sh))


def lindley(eps):
    """O-ring/cord line force per (E * d), Lindley 1967."""
    return 1.25 * eps ** 1.5 + 50 * eps ** 6


def lindley_d(eps):
    return 1.875 * eps ** 0.5 + 300 * eps ** 5


# ---------------------------------------------------------------- lid FE (grillage, est.)
def lid_fe(h, gasket, f_nub=14.0, E=1600.0, nu=0.35, hn=2.0, ds=0.5, cols=None, land=None):
    """Grillage of beams on a hn grid; section properties from ray-cast columns of the lid solid
    (ds sampling, T-sections with the skirt included, torsion from the plate only).
    Gasket: compression-only springs along the groove centre line. Screws: w = 0 under the
    washer. Lands: unilateral contact w >= 0. Fingers: constant upward nub crush force.
    Returns max lift on the gasket line, min squeeze, share of line below 10 %, total forces."""
    G = E / (2 * (1 + nu))
    lb = h.lid.bounding_box()
    x0, y0 = lb.min.X - 0.5, lb.min.Y - 0.5
    nxs, nys = int((lb.size.X + 1) / ds) + 1, int((lb.size.Y + 1) / ds) + 1
    if cols is None:
        cols = columns(h.lid, x0 + ds / 2, y0 + ds / 2, nxs, nys, ds)
    sx = lambda i: x0 + ds / 2 + i * ds
    nnx, nny = int(lb.size.X / hn) + 2, int(lb.size.Y / hn) + 2
    k_per = hn / ds

    def sec(ilist, jlist, along_x):
        """EI and GJ of a strip: ilist/jlist index sample columns; harmonic mean along the beam."""
        EIs, GJs = [], []
        outer = ilist if along_x else jlist
        inner = jlist if along_x else ilist
        for a in outer:
            A = S = I0 = J0 = 0.0
            web = []                                  # widths/depths of material below the plate
            for b in inner:
                iv = cols.get((a, b) if along_x else (b, a), [])
                for za, zb in iv:
                    A += ds * (zb - za)
                    S += ds * (zb * zb - za * za) / 2
                    I0 += ds * (zb ** 3 - za ** 3) / 3
                tp = sum(min(zb, h.z_top + h.lid_t) - max(za, h.z_top) for za, zb in iv if zb > h.z_top + 1e-3)
                J0 += ds * min(tp, h.lid_t) ** 3 / 6  # plate share (grillage convention)
                web.append(sum(min(zb, h.z_top) - za for za, zb in iv if za < h.z_top - 1e-3))
            # webs (skirt, fingers): St Venant torsion of each contiguous run, open section
            run = []
            for dpt in web + [0.0]:
                if dpt > 0.05:
                    run.append(dpt)
                elif run:
                    wdt, hgt = ds * len(run), sum(run) / len(run)
                    a_, b_ = max(wdt, hgt), min(wdt, hgt)
                    J0 += a_ * b_ ** 3 / 3 * (1 - 0.63 * b_ / a_)
                    run = []
            if A <= 0:
                return 0.0, 0.0
            zc = S / A
            EIs.append(E * (I0 - A * zc * zc))
            GJs.append(G * J0 if J0 > 0 else 1e-9)
        return len(EIs) / sum(1 / e for e in EIs), len(GJs) / sum(1 / g for g in GJs)

    node = {}
    nodes = []
    for i in range(nnx):
        for j in range(nny):
            x, y = x0 + i * hn, y0 + j * hn
            ci, cj = int(round((x - x0 - ds / 2) / ds)), int(round((y - y0 - ds / 2) / ds))
            if any(cols.get((ci + a, cj + b)) for a in (-1, 0) for b in (-1, 0)):
                node[(i, j)] = len(nodes)
                nodes.append((x, y))
    n = len(nodes)
    rows, colsK, vals = [], [], []

    def add(dofs, k):
        for a, da in enumerate(dofs):
            for b, db in enumerate(dofs):
                if k[a, b] != 0:
                    rows.append(da); colsK.append(db); vals.append(k[a, b])

    L = hn
    kb = np.array([[12, 6 * L, -12, 6 * L], [6 * L, 4 * L * L, -6 * L, 2 * L * L],
                   [-12, -6 * L, 12, -6 * L], [6 * L, 2 * L * L, -6 * L, 4 * L * L]]) / L ** 3
    kt = np.array([[1, -1], [-1, 1]]) / L
    for (i, j), a in node.items():
        x, y = nodes[a]
        ci0 = int(round((x - x0) / ds))
        cj0 = int(round((y - y0) / ds))
        half = int(k_per / 2)
        if (i + 1, j) in node:                       # beam along x
            b = node[(i + 1, j)]
            EI, GJ = sec(range(ci0, ci0 + int(k_per)), range(cj0 - half, cj0 + half), True)
            if EI > 0:
                add([3 * a, 3 * a + 1, 3 * b, 3 * b + 1], EI * kb)
                add([3 * a + 2, 3 * b + 2], GJ * kt)
        if (i, j + 1) in node:                       # beam along y
            b = node[(i, j + 1)]
            EI, GJ = sec(range(ci0 - half, ci0 + half), range(cj0, cj0 + int(k_per)), False)
            if EI > 0:
                add([3 * a, 3 * a + 2, 3 * b, 3 * b + 2], EI * kb)
                add([3 * a + 1, 3 * b + 1], GJ * kt)
    K = sp.csr_matrix((vals, (rows, colsK)), shape=(3 * n, 3 * n))
    P = np.array(nodes)

    def nearest(x, y):
        return int(np.argmin((P[:, 0] - x) ** 2 + (P[:, 1] - y) ** 2))

    # gasket line points (1 mm)
    gl = h.groove_line
    npt = int(gl.length)
    gpts = [gl.position_at(k / npt) for k in range(npt)]
    gnode = np.array([nearest(p.X, p.Y) for p in gpts])
    seg = gl.length / npt
    fixed = set()
    for (sx_, sy_) in h.screws:
        for a in np.where((P[:, 0] - sx_) ** 2 + (P[:, 1] - sy_) ** 2 <= (hs.SCREW["washer_od"] / 2) ** 2)[0]:
            fixed.add(int(a))
        fixed.add(nearest(sx_, sy_))
    if land is None:
        land = set()
        for a, (x, y) in enumerate(P):
            if inside(h.base, (x, y, h.z_top - 0.05)) and inside(h.base, (x, y, h.z_top - 0.6)):
                land.add(a)
    land -= fixed
    F_ext = np.zeros(3 * n)
    for (nx_, ny_) in h.nubs:
        F_ext[3 * nearest(nx_, ny_)] += f_nub
    u = np.zeros(3 * n)
    contact = set()
    for it in range(60):
        w = u[0::3]
        fg = np.zeros(n); kg = np.zeros(n)
        for a in gnode:
            f, k = gasket(w[a])
            fg[a] += f * seg; kg[a] += k * seg
        Fg = np.zeros(3 * n); Fg[0::3] = fg
        R = K @ u - Fg - F_ext
        Kt = K + sp.diags(np.repeat(kg, 3) * np.tile([1, 0, 0], n))
        cons = sorted({3 * a for a in fixed} | {3 * a for a in contact})
        free = np.setdiff1d(np.arange(3 * n), cons)
        du = np.zeros(3 * n)
        Kff = Kt[free][:, free] + sp.identity(len(free)) * 1e-9
        du[free] = spla.spsolve(Kff.tocsc(), -R[free])
        u += du
        u[cons] = 0.0
        w = u[0::3]
        R = K @ u - Fg - F_ext
        changed = False
        for a in list(contact):
            if R[3 * a] < -1e-6:         # reaction = K u - F; < 0: land would pull the lid down
                contact.discard(a); changed = True
        for a in land:
            if w[a] < -1e-6 and a not in contact:
                contact.add(a); changed = True
        if not changed and np.abs(du).max() < 1e-7:
            break
    w = u[0::3]
    wl = np.array([w[a] for a in gnode])
    return dict(w_max=float(wl.max()), w_min=float(wl.min()), wl=wl, gpts=gpts, nodes=n,
                f_gasket=float(sum(gasket(x)[0] * seg for x in wl)), it=it,
                screws_load=None, cols=cols, land=land)


def gasket_cord(d, shore, D):
    E = shore_to_E(shore)
    s0 = d - D

    def f(w):
        dl = s0 - w
        if dl <= 0:
            return 0.0, 0.0
        e = dl / d
        return E * d * lindley(e), E * lindley_d(e)
    return f, s0


def gasket_tpu(b, h0, D, E=26.0, nu=0.48):
    Ep = E / (1 - nu * nu)
    s0 = h0 - D

    def f(w):
        dl = s0 - w
        if dl <= 0:
            return 0.0, 0.0
        return b * Ep * dl / h0, b * Ep / h0
    return f, s0


# ---------------------------------------------------------------- mass / time (est.)
def mass_time(shape, rho=1.07, walls=2.4, tops=1.2, infill=0.25, vox=0.4):
    bb = shape.bounding_box()
    nx, ny = int(bb.size.X / vox) + 3, int(bb.size.Y / vox) + 3
    cols = columns(shape, bb.min.X - vox, bb.min.Y - vox, nx, ny, vox, tol=0.05)
    nz = int(bb.size.Z / (vox / 2)) + 3
    vol = np.zeros((nx, ny, nz), bool)
    zc = bb.min.Z - vox / 2 + (np.arange(nz) + 0.5) * vox / 2
    for (i, j), iv in cols.items():
        for za, zb in iv:
            vol[i, j, (zc > za) & (zc < zb)] = True
    # anisotropic: z distances scaled so that 'tops' mm counts like 'walls' mm
    dist = ndimage.distance_transform_edt(vol, sampling=(vox, vox, vox / 2 * walls / tops))
    v_vox = vox * vox * vox / 2
    v_all = vol.sum() * v_vox
    v_core = (dist > walls).sum() * v_vox
    v_print = v_all - v_core * (1 - infill)
    layers = bb.size.Z / 0.2
    t_s = (v_all - v_core) / 4.0 + v_core * infill / 8.0 + layers * 2.0    # 4 / 8 mm3/s, 2 s/layer
    return dict(v_solid=shape.volume / 1000, v_vox=v_all / 1000, v_print=v_print / 1000,
                g=v_print / 1000 * rho, t_h=t_s / 3600)


# ---------------------------------------------------------------- overhangs (print orientation)
def overhangs(shape, flip=False, nz_lim=-0.75):
    V, T = tess(shape, 0.05)
    if flip:
        V = V * np.array([1, -1, -1])
    V[:, 2] -= V[:, 2].min()
    tri = V[T]
    n = np.cross(tri[:, 1] - tri[:, 0], tri[:, 2] - tri[:, 0])
    area = np.linalg.norm(n, axis=1) / 2
    nzu = n[:, 2] / (np.linalg.norm(n, axis=1) + 1e-12)
    zmin = tri[:, :, 2].min(axis=1)
    m = (nzu < nz_lim) & (zmin > 0.05) & (area > 0)
    cen = tri[m].mean(axis=1)
    clusters = {}
    for c, a in zip(cen, area[m]):
        k = (round(c[0] / 6) * 6, round(c[1] / 6) * 6, round(c[2], 1))
        clusters[k] = clusters.get(k, 0) + a
    return float(area[m].sum()), sorted(clusters.items(), key=lambda kv: -kv[1])


# ================================================================ main
def main():
    sweep = "--sweep" in sys.argv
    h = hs.build()
    board_solid, partsol = h.pcb_board, h.pcb_parts + h.extras
    pb = board_solid.bounding_box()
    row("0 board placed: PCB solid min corner vs ledge frame", abs(pb.min.X - h.bxy(0, 0)[0]) < 1e-6 and
        abs(pb.min.Z - h.z_board) < 1e-6, "(%.2f, %.2f, %.2f)" % (pb.min.X, pb.min.Y, pb.min.Z), "board origin")
    # J1 wires: vertical entry into the pads (2.3 mm OD, 4 mm above the board top)
    j1 = [p for p in hs.board["parts"] if p["ref"] == "J1"][0]
    wires = [Pos(*h.bxy(pd["x"], pd["y"]), h.z_btop) * Cylinder(1.15, 4.0, align=(Align.CENTER, Align.CENTER, Align.MIN))
             for pd in j1["pads"]]
    allparts = Compound(children=partsol)
    bb, lb = h.base.bounding_box(), h.lid.bounding_box()

    # 1 base on the bed
    row("1 base bbox min z (no geometry below the bed)", abs(bb.min.Z) < 1e-6, "%.3f" % bb.min.Z, "= 0")
    flat = sum(f.area for f in h.base.faces() if abs(f.center().Z) < 1e-6 and f.normal_at().Z < -0.99)
    row("1 base bed contact area (floor + ears)", flat > 3000, "%.0f mm2" % flat, "floor + 4 ears")

    # 2 groove width / depth from section probes, insert walls
    zprobe = h.z_top - hs.GROOVE_D / 2
    gl = h.groove_line
    ws, cw = [], []
    for k in range(120):
        t = k / 120
        p = gl.position_at(t)
        tg = gl.tangent_at(t)
        nrm = np.array([tg.Y, -tg.X, 0.0])
        c = (p.X, p.Y, zprobe)
        if inside(h.base, c):
            ws.append(0.0); continue
        wa = edge_along(h.base, c, nrm, 3, False) + edge_along(h.base, c, -nrm, 3, False)
        ws.append(wa)
        # corner probes: where the tangent is not axis aligned
        if 0.15 < abs(tg.X) < 0.99:
            cw.append(wa)
    depths = []
    for k in range(0, 120, 10):
        p = gl.position_at(k / 120)
        depths.append(edge_along(h.base, (p.X, p.Y, h.z_top + 0.01), (0, 0, -1), 3, False) - 0.01)
    row("2 groove width all around (120 section probes)", min(ws) > hs.GROOVE_W - 0.02,
        "%.3f..%.3f" % (min(ws), max(ws)), "= %.2f" % hs.GROOVE_W)
    row("2 groove width at the corners (%d probes)" % len(cw), bool(cw) and min(cw) > hs.GROOVE_W - 0.02,
        "%.3f..%.3f" % (min(cw), max(cw)) if cw else "none", "= %.2f" % hs.GROOVE_W)
    row("2 groove depth (12 probes)", max(abs(d - hs.GROOVE_D) for d in depths) < 0.02,
        "%.3f..%.3f" % (min(depths), max(depths)), "= %.2f" % hs.GROOVE_D)
    dmin_g = min(Cylinder(hs.INSERT["hole"] / 2, hs.INSERT["depth"], align=(Align.CENTER, Align.CENTER, Align.MAX))
                 .moved(Pos(x, y, h.z_top)).distance_to(h.groove) for x, y in h.screws)
    row("2 wall insert hole -> groove", dmin_g >= 1.6 + (hs.INSERT["od"] - hs.INSERT["hole"]) / 2 - 1e-3,
        "%.2f (%.2f from OD)" % (dmin_g, dmin_g - (hs.INSERT["od"] - hs.INSERT["hole"]) / 2), ">= 1.6 from OD")
    bad = 0
    rr = hs.INSERT["od"] / 2 + hs.INSERT_WALL - 0.05
    for x, y in h.screws:
        for a in range(36):
            for zz in (h.z_top - 0.3, h.z_top - 3, h.z_top - hs.INSERT["length"]):
                if not inside(h.base, (x + rr * math.cos(a * math.pi / 18), y + rr * math.sin(a * math.pi / 18), zz)):
                    bad += 1
    row("2 solid ring 1.6 mm around every insert OD", bad == 0, "%d of %d probe pts open" % (bad, 36 * 3 * len(h.screws)),
        "0")

    # 3 closed assembly interference
    v_lb = (h.lid & h.base).volume
    row("3 lid vs base, closed: intersection volume", v_lb < 0.01, "%.4f mm3" % v_lb, "0")
    v_lp = (h.lid & allparts).volume
    row("3 lid vs board parts (STEP + relay envelope)", v_lp < 0.01, "%.4f mm3" % v_lp, "0")
    v_lpcb = (h.lid & board_solid).volume
    nub_v = len(h.nubs) * math.pi * (hs.TIP_D / 2) ** 2 * (h.z_btop - h.z_tip)
    # STEP PCB solid is 1.51 thick (no copper/mask); nominal top is z_btop = z_board + 1.6
    row("3 lid vs PCB solid (only crush nubs expected)", v_lpcb <= nub_v + 0.01,
        "%.3f mm3" % v_lpcb, "<= %.3f (nubs)" % nub_v)
    v_bp = (h.base & allparts).volume + (h.base & board_solid).volume
    row("3 base vs board + parts", v_bp < 0.01, "%.4f mm3" % v_bp, "0")
    v_gl = (h.gl_out & h.base).volume + (h.gl_out & h.lid).volume
    glb = h.gl_out.bounding_box()
    row("3 gland body envelope vs base/lid", v_gl < 0.01, "%.4f mm3" % v_gl, "0")
    row("3 gland hex corners above mounting plane", glb.min.Z >= 0.5 - 1e-6, "%.2f" % glb.min.Z, ">= 0.5")
    seal_ok = sum(inside(h.base, (h.fx0 + 0.05, h.gy + r * math.cos(a), h.z_g + r * math.sin(a)))
                  for a in np.linspace(0, 2 * math.pi, 72, endpoint=False)
                  for r in (h.G["thread"] / 2 + 0.3, h.G["af"] / 2 - 0.2))
    row("3 gland seal face on the base end face (144 pts)", seal_ok == 144, "%d/144 solid" % seal_ok, "144")
    thr = Plane.YZ.offset(h.fx0 - 1) * Pos(h.gy, h.z_g) * Cylinder(h.G["thread"] / 2, h.END_T + 1,
                                                                   align=(Align.CENTER, Align.CENTER, Align.MIN))
    dthr = thr.distance_to(h.groove)
    row("3 tapped thread (major dia) -> groove bottom", dthr >= 1.6 - 1e-3, "%.2f" % dthr, ">= 1.6")
    v_cab = (h.cable_in & h.lid).volume
    row("3 cable (max OD) inside vs lid/skirt", v_cab < 0.01, "%.4f mm3" % v_cab, "0")

    # 4 lid stiffness: FE (est.)
    d_cord = hs.CORD_D
    D = hs.GROOVE_D
    fe_rows = []
    cache = {}
    for label, (d, sh) in [("nominal 1.5 mm / 60 ShA", (1.5, 60)), ("stiff 1.6 mm / 70 ShA", (1.6, 70)),
                           ("soft 1.4 mm / 50 ShA", (1.4, 50)), ("nominal, nubs relaxed (0 N)", (1.5, 60))]:
        g, s0 = gasket_cord(d, sh, D)
        r = lid_fe(h, g, cols=cache.get("cols"), land=cache.get("land"), f_nub=0.0 if "relaxed" in label else 14.0)
        cache["cols"], cache["land"] = r["cols"], r["land"]
        sq = (s0 - r["wl"]) / d
        low = float((sq < 0.10).mean())
        fe_rows.append((label, r, sq, s0, d))
        need = 0.15 if label.startswith("nominal") else 0.10
        row("4 lid FE: min squeeze, %s" % label, sq.min() >= need,
            "%.1f %% (lift %.3f, %.0f N)" % (100 * sq.min(), r["w_max"], r["f_gasket"]), ">= %d %%" % (100 * need), "est.")
    gt, s0t = gasket_tpu(1.2, 1.6, D)
    r = lid_fe(h, gt, cols=cache["cols"], land=cache["land"])
    sqt = (s0t - r["wl"]) / 1.6
    row("4 lid FE: TPU 95A bead fallback (info)", True,
        "%.1f %% (lift %.3f, %.0f N)" % (100 * sqt.min(), r["w_max"], r["f_gasket"]), "info", "est.")
    row("4 screw count / lid thickness / skirt", len(h.screws) >= 8 and h.lid_t >= 3.0,
        "%d / %.1f / %.1f deep" % (len(h.screws), h.lid_t, h.skirt_d), ">= 8 / >= 3.0")

    # metal vs antenna
    ant = h.antenna
    d_ins = min(s.distance_to(ant) for s in h.inserts)
    d_scr = min(s.distance_to(ant) for s in h.screw_sol)
    d_ear = min((Pos(x, y, 0) * Cylinder(7.5, hs.EAR["t"] + 6, align=(Align.CENTER, Align.CENTER, Align.MIN))).distance_to(ant)
                for x, y in h.ears)
    row("4 inserts / screws to antenna (module overhang)", min(d_ins, d_scr) >= 15.0,
        "%.1f / %.1f" % (d_ins, d_scr), ">= 15")
    row("   M5 bolt + washer to antenna", d_ear >= 15.0, "%.1f" % d_ear, ">= 15")

    # 5 groove fill
    A = math.pi * d_cord ** 2 / 4
    Wm, Dm = float(np.median(ws)), float(np.median(depths))
    fills = {lab: A * sw / ((Wm - nar) * Dm) for lab, nar, sw in
             [("CAD", 0, 1.0), ("print -0.1", 0.1, 1.0), ("print -0.2", 0.2, 1.0),
              ("print -0.1 hot", 0.1, 1.07), ("print -0.2 hot", 0.2, 1.07)]}
    row("5 cord fill, as printed (W -0.1..-0.2)", 0.70 <= fills["print -0.1"] and fills["print -0.2"] <= 0.80,
        "%.0f..%.0f %%" % (100 * fills["print -0.1"], 100 * fills["print -0.2"]), "70-80 %")
    row("5 cord fill, printed + 7 % swell at 100 C", fills["print -0.2 hot"] <= 0.85,
        "%.0f..%.0f %%" % (100 * fills["print -0.1 hot"], 100 * fills["print -0.2 hot"]), "<= 85 %")
    row("5 cord fill, CAD / squeeze nominal", True, "%.0f %% / %.0f %%" % (100 * fills["CAD"], 100 * (d_cord - Dm) / d_cord),
        "info (20 %)")
    big = math.pi * 1.6 ** 2 / 4 * 1.07 / ((Wm - 0.2) * Dm)
    row("5 cord 1.6 (+tol), print -0.2, hot", big <= 0.95, "%.0f %%" % (100 * big), "<= 95 %")
    tpu_a = h.gasket.volume / gl.length
    row("5 TPU bead fill (printed groove -0.1..-0.2)", 0.70 <= tpu_a / ((Wm - 0.1) * Dm) and tpu_a / ((Wm - 0.2) * Dm) <= 0.85,
        "%.0f..%.0f %%" % (100 * tpu_a / ((Wm - 0.1) * Dm), 100 * tpu_a / ((Wm - 0.2) * Dm)), "75-80 % (<= 85)")

    # 6 screws vs inserts
    sx, sy = h.screws[0]
    lid_t_meas = edge_along(h.lid, (sx + hs.SCREW["clear"] / 2 + 0.6, sy, h.z_top + 0.01), (0, 0, 1), 10, True) + 0.01
    hole_depth = edge_along(h.base, (sx, sy, h.z_top + 0.01), (0, 0, -1), 20, False) - 0.01
    tip = hs.SCREW["length"] - lid_t_meas - hs.SCREW["washer_t"]
    eng = min(tip, hs.INSERT["length"])
    row("6 screw M3x%g: thread in insert" % hs.SCREW["length"], eng >= 4.0 and tip <= hs.INSERT["length"] + 0.3,
        "%.2f of %.1f insert" % (eng, hs.INSERT["length"]), ">= 4.0 (1.3 D)")
    row("6 screw tip to hole bottom", hole_depth - tip >= 1.0, "%.2f (hole %.2f deep)" % (hole_depth - tip, hole_depth),
        ">= 1.0")
    v_sb = sum((s & h.base).volume for s in h.screw_sol) + sum((s & h.lid).volume for s in h.screw_sol)
    row("6 screws vs plastic (base + lid)", v_sb < 0.01, "%.4f mm3" % v_sb, "0")
    row("6 plastic under the screw head (lid, no counterbore)", lid_t_meas >= 3.0 - 1e-3, "%.2f + washer" % lid_t_meas,
        ">= 3.0")

    # 7 skirt / lid vs parts
    skirt_zone = h.lid & (Pos(0, 0, h.z_top) * Box(500, 500, h.skirt_d + 0.001, align=(Align.CENTER, Align.CENTER, Align.MAX)))
    pins_c = Compound(children=h.pins)
    skirt_only = skirt_zone - pins_c
    d_sk = min(skirt_only.distance_to(p) for p in partsol)
    tallest = max(p.bounding_box().max.Z for p in partsol)
    row("7 skirt to nearest part (3D)", d_sk >= 1.0, "%.2f (skirt %.1f deep)" % (d_sk, h.skirt_d), ">= 1.0")
    row("7 lid underside to tallest part top", h.z_top - tallest >= hs.TOP_CLEAR, "%.2f" % (h.z_top - tallest),
        ">= %.1f" % hs.TOP_CLEAR)
    d_wire = min(s.distance_to(w) for s in [skirt_only] + h.pins for w in wires)
    row("7 lid features to J1 wire stubs", d_wire >= 1.0, "%.2f" % d_wire, ">= 1.0")

    # 8 end stops
    zmid = h.z_board + hs.bt / 2
    ok = []
    for (bxv, byv) in [(-hs.GAP_X - 0.1, 2.1), (-hs.GAP_X - 0.1, hs.bh - 2.1), (hs.bw + hs.GAP_X + 0.1, 4.5),
                       (hs.bw + hs.GAP_X + 0.1, hs.bh - 4.5)]:
        ok.append(inside(h.base, (*h.bxy(bxv, byv), zmid)))
    row("8 board end stops (4 probes at the board edges)", all(ok), "%d/4" % sum(ok), "4/4")
    st = max(s.bounding_box().max.Z for s in h.stops)
    mod_v = (h.base & h.antenna).volume
    row("8 stop top vs board top / base vs module overhang", st <= h.z_btop + 1e-6 and mod_v < 0.01,
        "%.2f vs %.2f / %.3f mm3" % (st, h.z_btop, mod_v), "<= board top / 0")
    row("8 board play x / y", True, "%.2f / %.2f" % (2 * hs.GAP_X, 2 * hs.GAP_Y), "info")

    # 9 press fingers
    lows = [edge_along(h.lid, (x, y, h.z_btop + 2), (0, 0, -1), 4, True) for x, y in h.nubs]
    pre = [h.z_btop - (h.z_btop + 2 - lo) for lo in lows]
    row("9 nub preload (interference with board top)", all(0.15 <= p <= 0.25 for p in pre),
        "%.2f..%.2f (%d nubs)" % (min(pre), max(pre), len(pre)), "~0.1-0.2")
    tipd = [edge_along(h.lid, (x, y, h.z_tip + 0.2), (1, 0, 0), 2, True) * 2 for x, y in h.nubs]
    row("9 crush nub diameter", all(abs(t - hs.TIP_D) < 0.03 for t in tipd), "%.2f" % min(tipd), "~0.6")
    d_pin = min(p.distance_to(q) for p in h.pins for q in partsol)
    row("9 fingers to every part body (3D)", d_pin >= 1.0, "%.2f" % d_pin, ">= 1.0")
    row("9 fingers sit over the support ledge", all(inside(h.base, (x, y, h.z_board - 0.1)) for x, y in h.nubs),
        "%d/%d" % (sum(inside(h.base, (x, y, h.z_board - 0.1)) for x, y in h.nubs), len(h.nubs)), "all")

    # vent, layer grid, wire path
    vx, vy = -h.L_in / 2 + hs.CHAMBER / 2, hs.VENT["y"]
    seat = inside(h.base, (vx + 5, vy, hs.FLOOR + hs.VENT["seat_h"] - 0.05)) and \
        not inside(h.base, (vx + 5, vy, hs.FLOOR + hs.VENT["seat_h"] + 0.05))
    through = not inside(h.base, (vx, vy, 0.5)) and not inside(h.base, (vx, vy, hs.FLOOR + 0.5))
    exitf = not inside(h.base, (h.fx0 + 0.2, h.ey, 0.5))
    row("vent: seat top face, hole, channel out of the end face", seat and through and exitf,
        "%s/%s/%s" % (seat, through, exitf), "all")
    zs = dict(z_top=h.z_top, floor=hs.FLOOR, board=h.z_board, board_top=h.z_btop, lid=h.lid_t, groove=hs.GROOVE_D,
              skirt=h.skirt_d, nub=h.z_tip, stop=st, insert=hs.INSERT["depth"], ear=hs.EAR["t"],
              flange=hs.FLANGE_T, seat=hs.VENT["seat_h"], chan=hs.VENT["chan_d"], f_up=h.z_up)
    off = [k for k, v in zs.items() if abs(v / 0.2 - round(v / 0.2)) > 1e-6]
    row("layer grid: critical z values on 0.2 mm", not off, "off: %s" % (",".join(off) or "none"), "none")
    row("wire path: gland axis above board top / chamber", True,
        "%.1f / %.1f mm, %.1f mm to lid" % (h.z_g - h.z_btop, hs.CHAMBER, h.z_top - h.z_btop), "info")

    # print: overhangs
    oa_b, cl_b = overhangs(h.base)
    oa_l, cl_l = overhangs(h.lid, flip=True)
    row("print: base downward faces > 41 deg (upright)", oa_b < 120, "%.0f mm2" % oa_b, "bridges only", "V")
    row("print: lid downward faces > 41 deg (face down)", oa_l < 60, "%.0f mm2" % oa_l, "bridges only", "V")

    # ---------------------------------------------------------- report
    w = max(len(r[0]) for r in ROWS)
    print("\n%-*s  %-4s  %-28s  %-18s %s" % (w, "check", "res", "measured", "required", "kind"))
    for name, ok, meas, req, kind in ROWS:
        print("%-*s  %-4s  %-28s  %-18s %s" % (w, name, "PASS" if ok else "FAIL", meas, req, kind))
    print("\nbase overhang clusters (x, y, z: mm2):", [(k, round(v, 1)) for k, v in cl_b[:8]])
    print("lid overhang clusters (print coords):", [(k, round(v, 1)) for k, v in cl_l[:8]])
    for label, r, sq, s0, d in fe_rows:
        i = int(np.argmin(sq))
        p = r["gpts"][i]
        print("FE %-24s nodes %d, it %d, lift max %.3f at (%.1f, %.1f), squeeze %.1f..%.1f %%, <10 %%: %.0f %%" %
              (label, r["nodes"], r["it"], r["w_max"], p.X, p.Y, 100 * sq.min(), 100 * sq.max(), 100 * (sq < 0.1).mean()))
    mb, ml = mass_time(h.base), mass_time(h.lid)
    print("\nouter: flange %.1f x %.1f, body %.1f x %.1f, height %.1f, over ears %.1f x %.1f" %
          (h.fL, h.fW, h.L_in + h.END_T + hs.WALL, h.W_out, h.z_top + h.lid_t, bb.size.X, bb.size.Y))
    for nm, m in (("base", mb), ("lid", ml)):
        print("%s: solid %.1f cm3; printed (6 walls, 6 top/bottom, 25 %% gyroid) est. %.1f cm3 = %.0f g ASA, "
              "est. %.1f h at 4/8 mm3/s" % (nm, m["v_solid"], m["v_print"], m["g"], m["t_h"]))
    print("cord: %.0f mm of 1.5 mm silicone; TPU bead %.2f cm3" % (gl.length, h.gasket.volume / 1000))

    if sweep:
        print("\nlid FE sweep (nominal cord):")
        g, s0 = gasket_cord(1.5, 60, D)
        for kw in [dict(n_mid=1), dict(n_mid=2, skirt_end_t=(1.6, 1.6)), dict(n_mid=2), dict(n_mid=2, skirt_end_t=(4.0, 3.2)),
                   dict(n_mid=2, lid_t=3.6), dict(n_mid=2, n_ant=1), dict(n_mid=2, lid_t=3.6, skirt_end_t=(4.0, 3.2))]:
            hh = hs.build(**kw)
            r = lid_fe(hh, g)
            sq = (s0 - r["wl"]) / 1.5
            print("  %-32s screws %d: lift %.3f, min squeeze %.1f %%, <10 %%: %.0f %%" %
                  (kw, len(hh.screws), r["w_max"], 100 * sq.min(), 100 * (sq < 0.1).mean()))
    return 0 if all(r[1] for r in ROWS) else 1


if __name__ == "__main__":
    sys.exit(main())
