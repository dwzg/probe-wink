"""Build a KiCad PCB from a schematic netlist with the pcbnew Python API.

Flow: read the KiCad netlist (kicad-cli sch export netlist --format kicadsexpr),
load and place footprints, assign nets, draw the outline, autoroute with
Freerouting (Specctra DSN/SES), pour zones, save, then DRC with kicad-cli.
"""
import json
import hashlib
import heapq
import math
import os
import subprocess

import pcbnew

from .sexp import find, find1, parse

FP_DIR = os.environ.get("KICAD_FOOTPRINT_DIR", "/usr/share/kicad/footprints")
MM = pcbnew.FromMM


def read_netlist(path):
    with open(path) as f:
        t = parse(f.read())
    comps = {}
    for c in find(find1(t, "components"), "comp"):
        ref = str(find1(c, "ref")[1])
        fields = {}
        fl = find1(c, "fields")
        if fl is not None:
            for fld in find(fl, "field"):
                name = str(find1(fld, "name")[1])
                fields[name] = str(fld[-1]) if not isinstance(fld[-1], list) else ""
        ts = find1(c, "tstamps")
        props = {str(find1(p, "name")[1]) for p in find(c, "property")}
        comps[ref] = dict(value=str(find1(c, "value")[1]),
                          footprint=str(find1(c, "footprint")[1]) if find1(c, "footprint") is not None else "",
                          uuid=str(ts[1]) if ts is not None else "",
                          fields=fields, exclude_from_bom="exclude_from_bom" in props,
                          dnp="dnp" in props)
    nets = {}
    for n in find(find1(t, "nets"), "net"):
        name = str(find1(n, "name")[1])
        nets[name] = [(str(find1(x, "ref")[1]), str(find1(x, "pin")[1])) for x in find(n, "node")]
    return comps, nets


class Board:
    def __init__(self, netlist_path, track=0.3, clearance=0.2, via=(0.6, 0.3), min_drill=0.3,
                 declared=None, route_margin=0.02, fp_dirs=()):
        self.fp_dirs = list(fp_dirs)
        self.comps, self.nets = read_netlist(netlist_path)
        self.declared = {}
        if declared:
            with open(declared) as f:
                self.declared = json.load(f)
        self.board = pcbnew.BOARD()
        ds = self.board.GetDesignSettings()
        ds.SetCopperLayerCount(2)
        ds.m_MinThroughDrill = MM(min_drill)
        self.rules = dict(track=track, clearance=clearance, via=via, min_drill=min_drill)
        self.classes = {}
        self.ns = ds.m_NetSettings
        d = self.ns.GetDefaultNetclass()
        d.SetTrackWidth(MM(track))
        d.SetClearance(MM(clearance + route_margin))   # route a hair wider than the DRC rule
        d.SetViaDiameter(MM(via[0]))
        d.SetViaDrill(MM(via[1]))
        self.fps = {}
        self.netinfo = {}
        self._stitch_vias = []
        self.placed = set()
        self._load()

    # ---- setup ------------------------------------------------------------
    def resolve(self, name):
        """Map a plain net name to the board's net name (KiCad prefixes local nets with the sheet path)."""
        if name in self.nets:
            return name
        for cand in ("/" + name,):
            if cand in self.nets:
                return cand
        if name in self.declared:                   # unlabeled net: find it by its pins
            pins = {tuple(p) for p in self.declared[name]}
            for net, nodes in self.nets.items():
                if pins == {n for n in nodes if not n[0].startswith("#")}:
                    return net
        raise KeyError("no such net: " + name)

    def _net(self, name):
        if name not in self.netinfo:
            ni = pcbnew.NETINFO_ITEM(self.board, name)
            self.board.Add(ni)
            self.netinfo[name] = ni
        return self.netinfo[name]

    def _load(self):
        pin_net = {}
        for net, nodes in self.nets.items():
            for ref, pin in nodes:
                pin_net[(ref, pin)] = net
        for ref, c in self.comps.items():
            if ":" not in c["footprint"]:
                continue
            lib, name = c["footprint"].split(":", 1)
            fp = None
            for d in self.fp_dirs + [FP_DIR]:          # project libraries first
                if os.path.isdir(os.path.join(d, lib + ".pretty")):
                    fp = pcbnew.FootprintLoad(os.path.join(d, lib + ".pretty"), name)
                    break
            if fp is None:
                raise RuntimeError("footprint not found: " + c["footprint"])
            fp.SetReference(ref)
            fp.SetValue(c["value"])
            fp.SetFPIDAsString(c["footprint"])
            if c["uuid"]:
                fp.SetPath(pcbnew.KIID_PATH("/" + c["uuid"]))
            self.board.Add(fp)
            for pad in fp.Pads():
                net = pin_net.get((ref, pad.GetNumber()))
                if net:
                    pad.SetNet(self._net(net))
            fp.SetExcludedFromBOM(c["exclude_from_bom"])
            if hasattr(fp, "SetDNP"):
                fp.SetDNP(c["dnp"])
            for name, val in c["fields"].items():
                if name in ("Footprint",):
                    continue
                fp.SetField(name, val)
                f = fp.GetField(name)
                if f is not None:
                    f.SetVisible(False)
            self.fps[ref] = fp

    def netclass(self, name, nets, track, clearance=None):
        nc = pcbnew.NETCLASS(name)
        d = self.ns.GetDefaultNetclass()
        nc.SetTrackWidth(MM(track))
        nc.SetClearance(MM(clearance + 0.02) if clearance else d.GetClearance())
        nc.SetViaDiameter(d.GetViaDiameter())
        nc.SetViaDrill(d.GetViaDrill())
        nets = [self.resolve(n) for n in nets]
        self.classes[name] = dict(nets=list(nets), track=track,
                                  clearance=clearance or self.rules["clearance"])
        self.ns.SetNetclass(name, nc)
        for n in nets:
            self.ns.SetNetclassPatternAssignment(n, name)
        self.ns.RecomputeEffectiveNetclasses()

    # ---- geometry ---------------------------------------------------------
    def place(self, ref, x, y, rot=0, back=False):
        """Fixed placement (marks the part as placed for later place_near calls)."""
        self.placed.add(ref)
        fp = self.fps[ref]
        if back and not fp.IsFlipped():
            fp.Flip(fp.GetPosition(), pcbnew.FLIP_DIRECTION_LEFT_RIGHT)
        fp.SetPosition(pcbnew.VECTOR2I_MM(x, y))
        fp.SetOrientationDegrees(rot)

    def _crt(self, fp):
        cy = fp.GetCourtyard(pcbnew.F_CrtYd if not fp.IsFlipped() else pcbnew.B_CrtYd)
        return cy if cy.OutlineCount() else None

    def fits(self, ref, edge=0.3):
        """True if ref's courtyard is inside the board (with edge margin) and clear of all others."""
        me = self._crt(self.fps[ref])
        if me is None:
            return True
        bb = me.BBox()
        x, y, w, h = self.box
        if (pcbnew.ToMM(bb.GetLeft()) < x + edge or pcbnew.ToMM(bb.GetTop()) < y + edge or
                pcbnew.ToMM(bb.GetRight()) > x + w - edge or pcbnew.ToMM(bb.GetBottom()) > y + h - edge):
            return False
        for other, fp in self.fps.items():
            if other == ref or other not in self.placed:
                continue
            o = self._crt(fp)
            if o is not None and me.Collide(o):
                return False
        return True

    def place_near(self, ref, x, y, rots=(0, 90, 180, 270), step=0.25, max_r=20.0, edge=0.3):
        """Put ref at the free spot closest to (x, y), trying the given rotations at each radius."""
        r = 0.0
        while r <= max_r:
            n = max(1, int(2 * math.pi * r / step)) if r else 1
            for i in range(n):
                a = 2 * math.pi * i / n
                px, py = x + r * math.cos(a), y + r * math.sin(a)
                for rot in rots:
                    self.place(ref, px, py, rot)
                    if self.fits(ref, edge):
                        self.placed.add(ref)
                        return px, py, rot
            r += step
        raise RuntimeError("no free spot for %s near (%.1f, %.1f)" % (ref, x, y))

    def pad_xy(self, ref, num):
        for p in self.fps[ref].Pads():
            if p.GetNumber() == str(num):
                v = p.GetPosition()
                return pcbnew.ToMM(v.x), pcbnew.ToMM(v.y)
        raise KeyError((ref, num))

    def outline(self, x, y, w, h):
        self.box = (x, y, w, h)
        pts = [(x, y), (x + w, y), (x + w, y + h), (x, y + h)]
        for (ax, ay), (bx, by) in zip(pts, pts[1:] + pts[:1]):
            s = pcbnew.PCB_SHAPE(self.board)
            s.SetShape(pcbnew.SHAPE_T_SEGMENT)
            s.SetStart(pcbnew.VECTOR2I_MM(ax, ay))
            s.SetEnd(pcbnew.VECTOR2I_MM(bx, by))
            s.SetLayer(pcbnew.Edge_Cuts)
            s.SetWidth(MM(0.1))
            self.board.Add(s)

    def outline_rounded(self, x, y, w, h, r=1.5):
        """Rectangular board outline with rounded corners."""
        self.box = (x, y, w, h)

        def add(shape):
            shape.SetLayer(pcbnew.Edge_Cuts)
            shape.SetWidth(MM(0.1))
            self.board.Add(shape)

        V = pcbnew.VECTOR2I_MM
        for (ax, ay), (bx, by) in [((x + r, y), (x + w - r, y)), ((x + w, y + r), (x + w, y + h - r)),
                                   ((x + w - r, y + h), (x + r, y + h)), ((x, y + h - r), (x, y + r))]:
            s = pcbnew.PCB_SHAPE(self.board)
            s.SetShape(pcbnew.SHAPE_T_SEGMENT)
            s.SetStart(V(ax, ay))
            s.SetEnd(V(bx, by))
            add(s)
        k = r * (1 - math.sqrt(0.5))
        for (sx, sy), (mx, my), (ex, ey) in [
                ((x, y + r), (x + k, y + k), (x + r, y)),
                ((x + w - r, y), (x + w - k, y + k), (x + w, y + r)),
                ((x + w, y + h - r), (x + w - k, y + h - k), (x + w - r, y + h)),
                ((x + r, y + h), (x + k, y + h - k), (x, y + h - r))]:
            s = pcbnew.PCB_SHAPE(self.board)
            s.SetShape(pcbnew.SHAPE_T_ARC)
            s.SetArcGeometry(V(sx, sy), V(mx, my), V(ex, ey))
            add(s)

    def text(self, s, x, y, size=1.0, thickness=0.15, layer=pcbnew.F_SilkS, angle=0, justify="center"):
        t = pcbnew.PCB_TEXT(self.board)
        t.SetText(s)
        t.SetLayer(layer)
        t.SetTextSize(pcbnew.VECTOR2I(MM(size), MM(size)))
        t.SetTextThickness(MM(thickness))
        t.SetTextAngleDegrees(angle)
        t.SetHorizJustify({"left": pcbnew.GR_TEXT_H_ALIGN_LEFT, "right": pcbnew.GR_TEXT_H_ALIGN_RIGHT,
                           "center": pcbnew.GR_TEXT_H_ALIGN_CENTER}[justify])
        t.SetPosition(pcbnew.VECTOR2I_MM(x, y))
        self.board.Add(t)
        return t

    def _silk_obstacles(self, gap):
        def inflate(bb, d):
            bb = pcbnew.BOX2I(bb.GetOrigin(), bb.GetSize())
            bb.Inflate(MM(d))
            return bb
        obs = []
        for fp in self.board.GetFootprints():
            for pad in fp.Pads():
                obs.append(inflate(pad.GetBoundingBox(), gap))
            for g in fp.GraphicalItems():
                if g.GetLayer() == pcbnew.F_SilkS and g.GetClass() != "PCB_FIELD":
                    obs.append(inflate(g.GetBoundingBox(), gap / 2))
            r = fp.Reference()
            if r.IsVisible() and r.GetLayer() == pcbnew.F_SilkS:
                obs.append(inflate(r.GetBoundingBox(), gap / 2))
        for item in self.board.GetDrawings():
            if item.GetLayer() == pcbnew.F_SilkS:
                obs.append(inflate(item.GetBoundingBox(), gap))
        return obs

    def label_near(self, ref, pad, text, size=1.0, thickness=0.15, gap=0.2, edge=0.5, dist=(1.4, 2.2, 3.0)):
        """Silkscreen text next to a pad, at the first clear spot around it. Returns the text or None."""
        x, y = self.pad_xy(ref, pad)
        obs = self._silk_obstacles(gap)
        bx, by, bw, bh = self.box
        board = pcbnew.BOX2I(pcbnew.VECTOR2I_MM(bx + edge, by + edge), pcbnew.VECTOR2I_MM(bw - 2 * edge, bh - 2 * edge))
        t = self.text(text, x, y, size=size, thickness=thickness)
        for d in dist:
            for dx, dy in ((0, -d), (0, d), (d + 0.3 * len(text), 0), (-d - 0.3 * len(text), 0),
                           (d, -d), (-d, -d), (d, d), (-d, d)):
                t.SetPosition(pcbnew.VECTOR2I_MM(x + dx, y + dy))
                tb = t.GetBoundingBox()
                if board.Contains(tb) and not any(tb.Intersects(o) for o in obs):
                    return t
        self.board.Remove(t)
        return None

    def clip_silk(self, ref):
        """Drop a footprint's silkscreen graphics that leave the board outline (e.g. an overhanging antenna)."""
        x, y, w, h = self.box
        board = pcbnew.BOX2I(pcbnew.VECTOR2I_MM(x + 0.3, y + 0.3), pcbnew.VECTOR2I_MM(w - 0.6, h - 0.6))
        fp = self.fps[ref]
        for g in list(fp.GraphicalItems()):
            if g.GetLayer() == pcbnew.F_SilkS and g.GetClass() != "PCB_FIELD" and not board.Contains(g.GetBoundingBox()):
                fp.Remove(g)

    def place_refs(self, size=0.8, thickness=0.12, gap=0.2, edge=0.5, keep=()):
        """Put every reference label just outside its courtyard, clear of pads, silk and other labels.

        Tries above, below, right, left; falls back to the courtyard centre. Refs in
        `keep` are left where they are. Returns refs that found no clear spot.
        """
        def inflate(bb, d):
            bb = pcbnew.BOX2I(bb.GetOrigin(), bb.GetSize())
            bb.Inflate(MM(d))
            return bb

        obstacles = []
        for fp in self.board.GetFootprints():
            for pad in fp.Pads():
                obstacles.append((fp.GetReference(), inflate(pad.GetBoundingBox(), gap)))
            for g in fp.GraphicalItems():
                if g.GetLayer() == pcbnew.F_SilkS and g.GetClass() != "PCB_FIELD":
                    obstacles.append((fp.GetReference(), inflate(g.GetBoundingBox(), gap / 2)))
        for item in self.board.GetDrawings():
            if item.GetLayer() == pcbnew.F_SilkS:
                obstacles.append(("", inflate(item.GetBoundingBox(), gap)))
        x, y, w, h = self.box
        board = pcbnew.BOX2I(pcbnew.VECTOR2I_MM(x + edge, y + edge), pcbnew.VECTOR2I_MM(w - 2 * edge, h - 2 * edge))
        placed, failed = [], []
        fps = sorted(self.board.GetFootprints(), key=lambda f: -f.GetBoundingBox(False).GetArea())
        for fp in fps:
            ref = fp.GetReference()
            r = fp.Reference()
            if not r.IsVisible() or ref in keep:
                continue
            r.SetTextSize(pcbnew.VECTOR2I(MM(size), MM(size)))
            r.SetTextThickness(MM(thickness))
            r.SetTextAngleDegrees(0)
            r.SetKeepUpright(True)
            cy = fp.GetCourtyard(pcbnew.F_CrtYd)
            bb = cy.BBox() if cy.OutlineCount() else fp.GetBoundingBox(False)
            cx, cyy = bb.Centre().x, bb.Centre().y
            th = MM(size)
            tw = MM(size * 0.75 * len(ref)) + MM(thickness)
            d = MM(gap)
            cands = [(cx, bb.GetTop() - d - th // 2), (cx, bb.GetBottom() + d + th // 2),
                     (bb.GetRight() + d + tw // 2, cyy), (bb.GetLeft() - d - tw // 2, cyy)]
            ok_pos = None
            for px, py in cands:
                r.SetPosition(pcbnew.VECTOR2I(int(px), int(py)))
                tb = r.GetBoundingBox()
                if not board.Contains(tb):
                    continue
                if any(tb.Intersects(o) for _, o in obstacles):
                    continue
                if any(tb.Intersects(p) for p in placed):
                    continue
                ok_pos = (px, py)
                break
            if ok_pos is None:
                r.SetPosition(pcbnew.VECTOR2I(int(cx), int(cyy)))
                r.SetLayer(pcbnew.F_Fab)       # no clear silk spot: keep it on the fab layer only
                failed.append(ref)
            else:
                placed.append(inflate(r.GetBoundingBox(), gap / 2))
        return failed

    def zone(self, net, layer, inset=0.3, clearance=0.3, min_width=0.2, thermal_gap=0.3, spoke=0.4):
        x, y, w, h = self.box
        z = pcbnew.ZONE(self.board)
        z.SetLayer(layer)
        z.SetNet(self._net(self.resolve(net)))
        o = z.Outline()
        o.NewOutline()
        for px, py in [(x + inset, y + inset), (x + w - inset, y + inset),
                       (x + w - inset, y + h - inset), (x + inset, y + h - inset)]:
            o.Append(MM(px), MM(py))
        z.SetLocalClearance(MM(clearance))
        z.SetMinThickness(MM(min_width))
        z.SetPadConnection(pcbnew.ZONE_CONNECTION_THT_THERMAL)   # SMD solid (reflow), THT thermal
        z.SetThermalReliefGap(MM(thermal_gap))
        z.SetThermalReliefSpokeWidth(MM(spoke))
        z.SetIslandRemovalMode(pcbnew.ISLAND_REMOVAL_MODE_ALWAYS)
        self.board.Add(z)
        return z

    def keepout(self, x, y, w, h, layers=(pcbnew.F_Cu, pcbnew.B_Cu), name="keepout",
                tracks=True, vias=True, fills=True):
        """Rule area; by default no copper pour, tracks or vias (footprint pads still allowed).

        tracks=True, vias=False, fills=False keeps a plane solid: no routing, pour and vias allowed.
        """
        z = pcbnew.ZONE(self.board)
        z.SetIsRuleArea(True)
        z.SetZoneName(name)
        ls = pcbnew.LSET()
        for l in layers:
            ls.AddLayer(l)
        z.SetLayerSet(ls)
        z.SetDoNotAllowZoneFills(fills)
        z.SetDoNotAllowTracks(tracks)
        z.SetDoNotAllowVias(vias)
        z.SetDoNotAllowPads(False)
        z.SetDoNotAllowFootprints(False)
        o = z.Outline()
        o.NewOutline()
        for px, py in [(x, y), (x + w, y), (x + w, y + h), (x, y + h)]:
            o.Append(MM(px), MM(py))
        self.board.Add(z)
        return z

    def courtyard_overlaps(self):
        boxes = {}
        for ref, fp in self.fps.items():
            cy = fp.GetCourtyard(pcbnew.F_CrtYd if not fp.IsFlipped() else pcbnew.B_CrtYd)
            if cy.OutlineCount():
                boxes[ref] = cy
        out = []
        refs = sorted(boxes)
        for i, a in enumerate(refs):
            for b in refs[i + 1:]:
                if (self.fps[a].IsFlipped() == self.fps[b].IsFlipped()) and boxes[a].Collide(boxes[b]):
                    out.append((a, b))
        x, y, w, h = self.box
        edge = []
        for ref, cy in boxes.items():
            bb = cy.BBox()
            if (pcbnew.ToMM(bb.GetLeft()) < x - 0.01 or pcbnew.ToMM(bb.GetTop()) < y - 0.01 or
                    pcbnew.ToMM(bb.GetRight()) > x + w + 0.01 or pcbnew.ToMM(bb.GetBottom()) > y + h + 0.01):
                edge.append(ref)
        return out, edge

    # ---- hand routing (locked, so the autorouter keeps it) --------------------
    def _xy(self, p):
        return self.pad_xy(*p) if isinstance(p[0], str) else p

    def _width(self, net):
        for c in self.classes.values():
            if net in c["nets"]:
                return c["track"]
        return self.rules["track"]

    def track(self, net, pts, width=None, layer=pcbnew.F_Cu, locked=True):
        """Polyline of locked segments; points are (x, y) or (ref, pad)."""
        net = self.resolve(net)
        w = width or self._width(net)
        pts = [self._xy(p) for p in pts]
        out = []
        for (x1, y1), (x2, y2) in zip(pts, pts[1:]):
            s = pcbnew.PCB_TRACK(self.board)
            s.SetStart(pcbnew.VECTOR2I_MM(x1, y1))
            s.SetEnd(pcbnew.VECTOR2I_MM(x2, y2))
            s.SetWidth(MM(w))
            s.SetLayer(layer)
            s.SetNet(self._net(net))
            s.SetLocked(locked)
            self.board.Add(s)
            out.append(s)
        return out

    def via(self, net, x, y, size=None, locked=True):
        size = size or self.rules["via"]
        v = pcbnew.PCB_VIA(self.board)
        v.SetPosition(pcbnew.VECTOR2I_MM(x, y))
        v.SetWidth(MM(size[0]))
        v.SetDrill(MM(size[1]))
        v.SetNet(self._net(self.resolve(net)))
        v.SetLocked(locked)
        self.board.Add(v)
        return v

    def via_near(self, net, ref, pad, rmin=0.9, rmax=3.0, clearance=None, size=None):
        """Drop a via on `net` at the closest legal spot around a pad (after routing).

        Legal: clear of every other net's pads, tracks and vias by the clearance on
        both layers, and of the board edge. Returns (x, y) or None.
        """
        size = size or self.rules["via"]
        clr = clearance if clearance is not None else self.rules["clearance"] + 0.05
        net = self.resolve(net)
        px, py = self.pad_xy(ref, pad)
        rv = size[0] / 2
        x0, y0, w, h = self.box
        items = []                                   # (kind, geometry, net)
        for fp in self.board.GetFootprints():
            for p in fp.Pads():
                if p.GetNetname() == net and not p.GetDrillSize().x:
                    continue
                bb = p.GetBoundingBox()
                items.append(("box", (pcbnew.ToMM(bb.GetLeft()), pcbnew.ToMM(bb.GetTop()),
                                      pcbnew.ToMM(bb.GetRight()), pcbnew.ToMM(bb.GetBottom())), p.GetNetname()))
        for tr in self.board.GetTracks():
            if tr.GetNetname() == net and tr.GetClass() != "PCB_VIA":
                continue
            if tr.GetClass() == "PCB_VIA":
                c = tr.GetPosition()
                items.append(("circ", (pcbnew.ToMM(c.x), pcbnew.ToMM(c.y), pcbnew.ToMM(tr.GetWidth(pcbnew.F_Cu)) / 2),
                              tr.GetNetname()))
            else:
                s, e = tr.GetStart(), tr.GetEnd()
                items.append(("seg", (pcbnew.ToMM(s.x), pcbnew.ToMM(s.y), pcbnew.ToMM(e.x), pcbnew.ToMM(e.y),
                                      pcbnew.ToMM(tr.GetWidth()) / 2), tr.GetNetname()))

        def free(x, y):
            if not (x0 + rv + 0.4 <= x <= x0 + w - rv - 0.4 and y0 + rv + 0.4 <= y <= y0 + h - rv - 0.4):
                return False
            for kind, g, n in items:
                if kind == "box":
                    dx = max(g[0] - x, 0, x - g[2])
                    dy = max(g[1] - y, 0, y - g[3])
                    d = math.hypot(dx, dy)
                    gap = 0.5 if n == net else clr        # same-net pads: keep the via off the pad
                elif kind == "circ":
                    d = math.hypot(x - g[0], y - g[1]) - g[2]
                    gap = 0.3 if n == net else clr        # drill-to-drill spacing for same-net vias
                else:
                    ax, ay, bx, by, hw = g
                    vx, vy = bx - ax, by - ay
                    L2 = vx * vx + vy * vy or 1e-12
                    u = max(0.0, min(1.0, ((x - ax) * vx + (y - ay) * vy) / L2))
                    d = math.hypot(x - ax - u * vx, y - ay - u * vy) - hw
                    gap = clr
                if d < rv + gap:
                    return False
            return True
        r = rmin
        while r <= rmax:
            n = max(8, int(2 * math.pi * r / 0.2))
            for i in range(n):
                a = 2 * math.pi * i / n
                x, y = px + r * math.cos(a), py + r * math.sin(a)
                if free(x, y):
                    self.via(net, x, y, size)
                    self.track(net, [(ref, pad), (x, y)], 0.3, locked=False)
                    return round(x, 2), round(y, 2)
            r += 0.1
        return None

    # ---- routing ----------------------------------------------------------
    def autoroute(self, workdir, passes=30, timeout=900, strategies=None, parallel=True, cache=True):
        """Route with Freerouting, trying several strategies; import the best session.

        All strategies run at the same time, each with its share of the CPU threads, so
        the wall time is one run, not one per strategy. Results are cached by a hash of
        the DSN and the options: an unchanged placement re-imports its session instantly.
        Each result is scored on a fresh copy of the unrouted board (loaded from disk), so
        the live board is only touched once, by the final import.
        """
        strategies = strategies or [[], ["-us", "global"], ["-is", "random"],
                                    ["-us", "hybrid", "-hr", "1:1"], ["-is", "sequential"]]
        dsn = os.path.join(workdir, "route.dsn")
        pre = os.path.join(workdir, "preroute.kicad_pcb")
        pcbnew.SaveBoard(pre, self.board)
        if not pcbnew.ExportSpecctraDSN(self.board, dsn):
            raise RuntimeError("DSN export failed")
        with open(dsn, "rb") as f:
            dsn_hash = hashlib.sha256(f.read()).hexdigest()[:16]
        cdir = os.path.join(workdir, "route-cache")
        os.makedirs(cdir, exist_ok=True)
        threads = max(1, (os.cpu_count() or 2) // (len(strategies) if parallel else 1))
        jobs = []
        for i, extra in enumerate(strategies):
            key = hashlib.sha256(("%s|%d|%s" % (dsn_hash, passes, " ".join(extra))).encode()).hexdigest()[:16]
            ses = os.path.join(cdir, key + ".ses")
            log = ses[:-4] + ".log"
            if cache and os.path.exists(ses):
                jobs.append((i, extra, ses, log, None))
                continue
            cmd = ["freerouting", "-de", dsn, "-do", ses[:-4] + ".part.ses", "-mp", str(passes), "-mt", str(threads),
                   "--gui.enabled=false"] + extra
            lf = open(log, "w")
            proc = subprocess.Popen(cmd, stdout=lf, stderr=subprocess.STDOUT, text=True)
            jobs.append((i, extra, ses, log, (proc, lf)))
            if not parallel:
                self._finish(proc, lf, ses, timeout)
                jobs[-1] = (i, extra, ses, log, None)
        for i, extra, ses, log, run in jobs:
            if run:
                self._finish(run[0], run[1], ses, timeout)
        best = None
        for i, extra, ses, log, run in jobs:
            if not os.path.exists(ses):
                continue
            trial = pcbnew.LoadBoard(pre)
            if not pcbnew.ImportSpecctraSES(trial, ses):
                continue
            trial.BuildConnectivity()
            left = trial.GetConnectivity().GetUnconnectedCount(False)
            print("  route attempt %d (%s)%s: %d unrouted" % (i + 1, " ".join(extra) or "default",
                                                           " [cached]" if run is None and cache else "", left))
            if best is None or left < best[0]:
                best = (left, ses, log)
        if best is None:
            raise RuntimeError("Freerouting produced no usable session")
        if not pcbnew.ImportSpecctraSES(self.board, best[1]):
            raise RuntimeError("SES import failed")
        with open(best[2]) as f:
            return f.read()

    @staticmethod
    def _finish(proc, logfile, ses, timeout):
        try:
            proc.wait(timeout=timeout)
        except subprocess.TimeoutExpired:
            proc.kill()
        logfile.close()
        part = ses[:-4] + ".part.ses"
        if os.path.exists(part):
            os.replace(part, ses)

    def ref_text(self, ref, visible=True, dx=None, dy=None):
        """Hide a reference label or move it relative to the footprint origin."""
        fp = self.fps[ref]
        r = fp.Reference()
        r.SetVisible(visible)
        if dx is not None or dy is not None:
            pos = fp.GetPosition()
            r.SetPosition(pcbnew.VECTOR2I(pos.x + MM(dx or 0), pos.y + MM(dy or 0)))

    def stitch(self, net="GND", pitch=4.0, via=(0.6, 0.3), margin=0.45, edge=1.5, hole_gap=0.5):
        """Drop vias on a grid wherever the net's pour covers the via on every copper layer."""
        net = self.resolve(net)
        self.fill_zones()
        polys = []
        for z in self.board.Zones():
            if z.GetNetname() != net or z.GetIsRuleArea():
                continue
            for layer in z.GetLayerSet().Seq():
                polys.append((layer, z.GetFilledPolysList(layer)))
        layers = {l for l, _ in polys}
        holes = []   # (x, y, radius) in mm
        for fp in self.board.GetFootprints():
            for pad in fp.Pads():
                d = pad.GetDrillSize()
                if d.x > 0:
                    p = pad.GetPosition()
                    holes.append((pcbnew.ToMM(p.x), pcbnew.ToMM(p.y), pcbnew.ToMM(max(d.x, d.y)) / 2))
        for tr in self.board.GetTracks():
            if tr.GetClass() == "PCB_VIA":
                p = tr.GetPosition()
                holes.append((pcbnew.ToMM(p.x), pcbnew.ToMM(p.y), pcbnew.ToMM(tr.GetDrillValue()) / 2))
        banned = [z.Outline() for z in self.board.Zones() if z.GetIsRuleArea() and z.GetDoNotAllowVias()]
        x, y, w, h = self.box
        r = via[0] / 2 + margin
        ring = [(0, 0)] + [(r * math.cos(a * math.pi / 4), r * math.sin(a * math.pi / 4)) for a in range(8)]
        n = 0
        gy = y + edge
        while gy <= y + h - edge:
            gx = x + edge
            while gx <= x + w - edge:
                ok = all(any(p.Contains(pcbnew.VECTOR2I_MM(gx + dx, gy + dy)) for l, p in polys if l == layer)
                         for layer in layers for dx, dy in ring)
                if ok and any(o.Collide(pcbnew.VECTOR2I_MM(gx, gy), MM(via[0] / 2)) for o in banned):
                    ok = False
                if ok and any(math.hypot(gx - hx, gy - hy) < hr + via[1] / 2 + hole_gap
                              for hx, hy, hr in holes):
                    ok = False
                if ok:
                    holes.append((gx, gy, via[1] / 2))
                    v = pcbnew.PCB_VIA(self.board)
                    v.SetPosition(pcbnew.VECTOR2I_MM(gx, gy))
                    v.SetWidth(MM(via[0]))
                    v.SetDrill(MM(via[1]))
                    v.SetNet(self._net(net))
                    self.board.Add(v)
                    self._stitch_vias.append(v)
                    n += 1
                gx += pitch
            gy += pitch
        self.fill_zones()
        n -= self._prune_floating_vias(net)
        return n

    def _reaches_pad(self, item, limit=400):
        """Breadth-first search over connected items; True once any pad is reached."""
        conn = self.board.GetConnectivity()
        seen, todo = {item.m_Uuid.AsString()}, [item]
        while todo and len(seen) < limit:
            cur = todo.pop()
            for nxt in conn.GetConnectedItems(cur):
                if nxt.GetClass() == "PAD":
                    return True
                key = nxt.m_Uuid.AsString()
                if key not in seen:
                    seen.add(key)
                    todo.append(nxt)
        return False

    def _prune_floating_vias(self, net):
        """Remove stitching vias whose copper cluster never reaches a pad of the net."""
        removed_total = 0
        for _ in range(10):
            removed = 0
            for v in list(self._stitch_vias):
                if not self._reaches_pad(v):
                    self.board.Remove(v)
                    self._stitch_vias.remove(v)
                    removed += 1
            removed_total += removed
            self.fill_zones()
            if removed == 0:
                break
        return removed_total

    def write_project(self, pro_path, min_track=0.15, min_via=0.5, min_annular=0.1, silk_clearance=0.15):
        """Store design rules and net classes in the .kicad_pro (KiCad keeps them there)."""
        data = {}
        if os.path.exists(pro_path):
            with open(pro_path) as f:
                data = json.load(f)
        data.setdefault("meta", {"filename": os.path.basename(pro_path), "version": 3})
        rules = data.setdefault("board", {}).setdefault("design_settings", {}).setdefault("rules", {})
        rules.update({"min_track_width": min_track, "min_clearance": self.rules["clearance"],
                      "min_through_hole_diameter": self.rules["min_drill"], "min_via_diameter": min_via,
                      "min_via_annular_width": min_annular, "min_copper_edge_clearance": 0.3,
                      "min_silk_clearance": silk_clearance})
        v_d, v_h = self.rules["via"]

        def cls(name, track, clearance, prio):
            return {"name": name, "track_width": track, "clearance": clearance, "via_diameter": v_d,
                    "via_drill": v_h, "microvia_diameter": 0.3, "microvia_drill": 0.1,
                    "diff_pair_width": 0.2, "diff_pair_gap": 0.25, "diff_pair_via_gap": 0.25,
                    "wire_width": 6, "bus_width": 12, "line_style": 0, "pcb_color": "rgba(0, 0, 0, 0.000)",
                    "schematic_color": "rgba(0, 0, 0, 0.000)", "priority": prio}
        classes = [cls("Default", self.rules["track"], self.rules["clearance"], 2147483647)]
        patterns = []
        for i, (name, c) in enumerate(self.classes.items()):
            classes.append(cls(name, c["track"], c["clearance"], i))
            patterns += [{"netclass": name, "pattern": n} for n in c["nets"]]
        data["net_settings"] = {"classes": classes, "meta": {"version": 4},
                                "netclass_patterns": patterns}
        with open(pro_path, "w") as f:
            json.dump(data, f, indent=2)

    def prune_dangling(self, rounds=10):
        """Remove unlocked vias and track stubs that lead nowhere (autorouter leftovers).

        A track end counts as connected if it lands on a pad, a via, another track or a
        filled zone of its net on its layer; a via needs connections on two layers.
        Returns the number of items removed.
        """
        self.fill_zones()
        removed = 0
        for _ in range(rounds):
            tracks = list(self.board.GetTracks())
            pads = [p for fp in self.board.GetFootprints() for p in fp.Pads()]
            zones = [z for z in self.board.Zones() if not z.GetIsRuleArea()]

            def touches(pt, layer, net, me):
                for p in pads:
                    if p.GetNetname() == net and p.IsOnLayer(layer) and p.HitTest(pt):
                        return True
                for o in tracks:
                    if o is me or o.GetNetname() != net:
                        continue
                    if o.GetClass() == "PCB_VIA":
                        if o.HitTest(pt):
                            return True
                    elif o.GetLayer() == layer and o.HitTest(pt, 0):
                        return True
                for z in zones:
                    if z.GetNetname() == net and z.IsOnLayer(layer) and z.HitTestFilledArea(layer, pt):
                        return True
                return False
            gone = []
            for tr in tracks:
                if tr.IsLocked():
                    continue
                net = tr.GetNetname()
                if tr.GetClass() == "PCB_VIA":
                    pt = tr.GetPosition()
                    layers = sum(1 for l in (pcbnew.F_Cu, pcbnew.B_Cu) if touches(pt, l, net, tr))
                    if layers < 2 and tr not in self._stitch_vias:
                        gone.append(tr)
                else:
                    l = tr.GetLayer()
                    if not (touches(tr.GetStart(), l, net, tr) and touches(tr.GetEnd(), l, net, tr)):
                        gone.append(tr)
            if not gone:
                break
            for tr in gone:
                self.board.Remove(tr)
            removed += len(gone)
            self.fill_zones()
        return removed

    def fill_zones(self):
        self.board.BuildConnectivity()
        pcbnew.ZONE_FILLER(self.board).Fill(self.board.Zones())
        self.board.BuildConnectivity()

    def unrouted(self):
        self.board.BuildConnectivity()
        return self.board.GetConnectivity().GetUnconnectedCount(False)

    def save(self, path, mask_min_web=0.1):
        self.board.GetDesignSettings().m_SolderMaskMinWidth = MM(mask_min_web)   # JLC: 0.1 mm web
        self.board.SetFileName(path)
        pcbnew.SaveBoard(path, self.board)


def path_length(board, net, a, b):
    """Shortest copper path between two pads along tracks and vias: (mm, vias) or None.

    Pours are ignored on purpose: this measures what was routed. DRC passes a buck whose
    switch node wanders 30 mm around the board; this check does not.
    a, b: "REF.PAD" strings.
    """
    mm = pcbnew.ToMM
    adj, pads, segs = {}, {}, []

    def key(x, y, l):
        return (round(x, 3), round(y, 3), l)

    def link(u, v, w, via=0):
        adj.setdefault(u, []).append((v, w, via))
        adj.setdefault(v, []).append((u, w, via))
    for t in board.GetTracks():
        if t.GetNetname() != net:
            continue
        if t.GetClass() == "PCB_VIA":
            p = t.GetPosition()
            link(key(mm(p.x), mm(p.y), pcbnew.F_Cu), key(mm(p.x), mm(p.y), pcbnew.B_Cu), 0, 1)
        else:
            s, e = t.GetStart(), t.GetEnd()
            u, v = key(mm(s.x), mm(s.y), t.GetLayer()), key(mm(e.x), mm(e.y), t.GetLayer())
            link(u, v, mm(t.GetLength()))
            segs.append((t, u, v))
    nodes = [k for k in adj]
    for t, u, v in segs:                       # endpoints that land mid-segment
        for k in nodes:
            if k[2] == u[2] and k not in (u, v) and t.HitTest(pcbnew.VECTOR2I_MM(k[0], k[1]), 0):
                link(k, u, math.hypot(k[0] - u[0], k[1] - u[1]))
                link(k, v, math.hypot(k[0] - v[0], k[1] - v[1]))
    for fp in board.GetFootprints():
        for p in fp.Pads():
            if p.GetNetname() != net:
                continue
            name = "%s.%s" % (fp.GetReference(), p.GetNumber())
            q = p.GetPosition()
            for layer in (pcbnew.F_Cu, pcbnew.B_Cu):
                if not p.IsOnLayer(layer):
                    continue
                pk = ("PAD", name, layer)
                pads.setdefault(name, []).append(pk)
                for k in nodes:
                    if k[2] == layer and p.HitTest(pcbnew.VECTOR2I_MM(k[0], k[1])):
                        link(pk, k, math.hypot(k[0] - mm(q.x), k[1] - mm(q.y)))
                for tr, u, v in segs:                  # a track that crosses the pad without ending on it
                    if u[2] == layer and tr.HitTest(q, 0):
                        link(pk, u, math.hypot(u[0] - mm(q.x), u[1] - mm(q.y)))
                        link(pk, v, math.hypot(v[0] - mm(q.x), v[1] - mm(q.y)))
            if len(pads[name]) == 2:
                link(pads[name][0], pads[name][1], 0, 1)
    if a not in pads or b not in pads:
        return None
    dst = set(pads[b])
    dist = {s: (0.0, 0) for s in pads[a]}
    pq = [(0.0, 0, i, s) for i, s in enumerate(pads[a])]
    tick = len(pq)
    while pq:
        d, nv, _, u = heapq.heappop(pq)
        if u in dst:
            return round(d, 2), nv
        if (d, nv) > dist.get(u, (1e9, 0)):
            continue
        for v, w, via in adj.get(u, []):
            nd = (d + w, nv + via)
            if nd < dist.get(v, (1e9, 0)):
                dist[v] = nd
                tick += 1
                heapq.heappush(pq, (nd[0], nd[1], tick, v))
    return None


def foreign_copper(board, rect, allowed, layers=(pcbnew.F_Cu, pcbnew.B_Cu)):
    """Tracks and vias inside rect (x0, y0, x1, y1) on nets not in `allowed`.

    Guards an area such as a buck's hot loop against the autorouter threading an
    unrelated net through it. Returns [(net, layer name, x, y)].
    """
    x0, y0, x1, y1 = rect
    box = pcbnew.BOX2I(pcbnew.VECTOR2I_MM(x0, y0), pcbnew.VECTOR2I_MM(x1 - x0, y1 - y0))
    out = []
    for t in board.GetTracks():
        if t.GetNetname() in allowed:
            continue
        if t.GetClass() == "PCB_VIA":
            hit = box.Contains(t.GetPosition())
            lname = "via"
        else:
            if t.GetLayer() not in layers:
                continue
            hit = box.Intersects(t.GetBoundingBox())
            lname = board.GetLayerName(t.GetLayer())
        if hit:
            c = t.GetPosition() if t.GetClass() == "PCB_VIA" else t.GetStart()
            out.append((t.GetNetname(), lname, round(pcbnew.ToMM(c.x), 2), round(pcbnew.ToMM(c.y), 2)))
    return out


def drc(pcb_path, report_path, schematic_parity=True):
    cmd = ["kicad-cli", "pcb", "drc", "--severity-all", "-o", report_path]
    if schematic_parity:
        cmd.append("--schematic-parity")
    cmd.append(pcb_path)
    r = subprocess.run(cmd, capture_output=True, text=True)
    return r.stdout + r.stderr


def render(pcb_path, png_path, layers="F.Cu,B.Cu,F.SilkS,F.Fab,Edge.Cuts,F.CrtYd", width=2400):
    svg = png_path[:-4] + ".svg"
    subprocess.run(["kicad-cli", "pcb", "export", "svg", "--mode-single", "--fit-page-to-board",
                    "--exclude-drawing-sheet", "-l", layers, "-o", svg, pcb_path],
                   capture_output=True, text=True, check=True)
    subprocess.run(["rsvg-convert", "-w", str(width), "-b", "white", svg, "-o", png_path], check=True)
    return png_path
