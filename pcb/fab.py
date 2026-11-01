#!/usr/bin/env python3
"""Probe wink module: JLCPCB fabrication + assembly files into build/fab/."""
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, "..", "tools"))
from kicad.fab import export  # noqa: E402

# JLC's placement rotation can differ from KiCad's per footprint. Leave at 0 and
# check every part in JLC's placement preview; add corrections here as found.
ROT_OFFSET = {}

r = export(os.path.join(HERE, "probe-wink.kicad_pcb"), os.path.join(HERE, "build", "probe-wink.net"),
           os.path.join(HERE, "build", "fab"), "probe-wink", ROT_OFFSET)
for k, v in r.items():
    print(f"{k}: {v}")

# Assembly drawings: every reference designator (most do not fit on the silkscreen).
import subprocess  # noqa: E402
for side, layers in (("top", "F.Fab,Edge.Cuts"), ("bottom", "B.Fab,Edge.Cuts")):
    out = os.path.join(HERE, "build", "fab", "probe-wink-assembly-%s.pdf" % side)
    cmd = ["kicad-cli", "pcb", "export", "pdf", "--layers", layers, "-o", out]
    if side == "bottom":
        cmd.append("--mirror")
    subprocess.run(cmd + [os.path.join(HERE, "probe-wink.kicad_pcb")], check=True, capture_output=True)
    print("assembly drawing:", out)
