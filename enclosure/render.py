#!/usr/bin/env python3
"""f3d renders of the housing: closed, open, section (through the gland axis), lid underside, base.

  enclosure/.venv/bin/python enclosure/render.py        -> build/r-*.png
Scenes are written as OBJ + MTL (one material per part) and rendered headless with f3d.
"""
import os
import sys

import housing as hs
from build123d import Align, Box, Pos

B = hs.B
sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "tools", "hw-design", "src"))
from hwdesign.mcad.render import f3d, write_obj  # noqa: E402


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
