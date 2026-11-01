#!/usr/bin/env python3
"""Probe wink module: JLCPCB fabrication + assembly files into build/fab/."""
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, "..", "tools", "hw-design", "src"))
from hwdesign.kicad.fab import export  # noqa: E402

# JLC's placement rotation can differ from KiCad's per footprint. Check every part in
# JLC's placement preview; add corrections here as found. Degrees, counter-clockwise,
# keyed by footprint name. Found in JLC's preview on 2026-11-01: their SOT-23 and
# TSOT-23-6 models have pin 1 opposite KiCad's, and their IM06 relay model lies 90 degrees
# off (body across the pad columns).
ROT_OFFSET = {"SOT-23": 180, "TSOT-23-6": 180, "Relay_2P2T_10x6mm_TE_IMxxG": -90}

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
