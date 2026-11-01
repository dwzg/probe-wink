#!/usr/bin/env python3
"""f3d renders of the housing: closed, open, section (through the gland axis), lid underside, base.

  enclosure/.venv/bin/python enclosure/render.py        -> build/r-*.png
Scenes are written as OBJ + MTL (one material per part) and rendered headless with f3d.
"""
import os
import subprocess

import numpy as np

import housing as hs
from build123d import Align, Box, Pos

B = hs.B
COL = {"base": (0.357, 0.498, 0.647), "lid": (0.627, 0.769, 0.886), "pcb": (0.18, 0.49, 0.2),
       "part": (0.25, 0.25, 0.27), "module": (0.75, 0.75, 0.78), "gland": (0.15, 0.15, 0.15),
       "cable": (0.1, 0.1, 0.1)}


def write_obj(items, path):
    """items: [(name, shape, colour key)] -> OBJ + MTL."""
    mtl = path[:-4] + ".mtl"
    with open(mtl, "w") as f:
        for k, (r, g, b) in COL.items():
            f.write("newmtl %s\nKd %.3f %.3f %.3f\nKa 0 0 0\nKs 0.1 0.1 0.1\nNs 20\n\n" % (k, r, g, b))
    off = 1
    with open(path, "w") as f:
        f.write("mtllib %s\n" % os.path.basename(mtl))
        for name, shape, col in items:
            v, t = shape.tessellate(0.03, 0.2)
            if not t:
                continue
            V = np.array([(q.X, q.Y, q.Z) for q in v])
            f.write("o %s\nusemtl %s\n" % (name, col))
            f.write("".join("v %.4f %.4f %.4f\n" % tuple(p) for p in V))
            f.write("".join("f %d %d %d\n" % (a + off, b + off, c + off) for a, b, c in t))
            off += len(V)


def f3d(obj, png, pos, focal, up="0,0,1", res="1600,1000"):
    subprocess.run(["f3d", obj, "--output", png, "--resolution", res, "--camera-position", pos,
                    "--camera-focal-point", focal, "--camera-view-up", up, "--background-color", "1,1,1",
                    "--up", "+Z", "--no-config", "--coloring-mode", "material"],
                   check=False, capture_output=True)


def main():
    h = hs.build()
    parts = [(("module" if "ESP32" in str(p.label) else "part"), p) for p in h.pcb_parts] + \
        [("part", e) for e in h.extras]
    board = [("pcb", "pcb", h.pcb_board)] + [("p%d" % i, c, p) for i, (c, p) in enumerate(parts)]
    board = [(n, s, c) for n, c, s in board]
    hw = [("gland", h.gl_out, "gland"), ("cable", h.cable_in, "cable")]
    s = os.path.join(B, "scene-%s.obj")
    z = h.z_top / 2
    # closed
    write_obj([("base", h.base, "base"), ("lid", h.lid, "lid")] + hw, s % "closed")
    f3d(s % "closed", os.path.join(B, "r-closed.png"), "-170,-210,160", "0,0,%g" % z)
    # open: base + board
    write_obj([("base", h.base, "base")] + board + hw, s % "open")
    f3d(s % "open", os.path.join(B, "r-open.png"), "-90,-160,210", "0,0,%g" % z)
    # section through the gland axis (keep y < gy)
    cut = Pos(0, h.gy, 0) * Box(500, 500, 500, align=(Align.CENTER, Align.MIN, Align.CENTER))
    sec = [("base", h.base - cut, "base"), ("lid", h.lid - cut, "lid"), ("pcb", h.pcb_board - cut, "pcb")]
    sec += [("p%d" % i, p - cut, c) for i, (c, p) in enumerate(parts) if p.bounding_box().min.Y < h.gy]
    sec += [("gland", h.gl_out - cut, "gland"), ("cable", h.cable_in - cut, "cable")]
    write_obj(sec, s % "section")
    f3d(s % "section", os.path.join(B, "r-section.png"), "0,150,%g" % z, "0,0,%g" % z, res="1800,800")
    # lid underside
    write_obj([("lid", h.lid, "lid")], s % "lid")
    f3d(s % "lid", os.path.join(B, "r-lid-under.png"), "-80,-140,-170", "0,0,%g" % h.z_top)
    # base from above (groove, ledge, stops, vent seat)
    write_obj([("base", h.base, "base")], s % "base")
    f3d(s % "base", os.path.join(B, "r-base.png"), "-60,-110,210", "0,0,%g" % z)
    # base from below (vent channel, flat bed face)
    f3d(s % "base", os.path.join(B, "r-base-under.png"), "-90,-120,-160", "0,0,0")
    print("rendered:", ", ".join(n for n in sorted(os.listdir(B)) if n.startswith("r-")))


if __name__ == "__main__":
    main()
