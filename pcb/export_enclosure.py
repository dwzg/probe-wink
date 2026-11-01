#!/usr/bin/env python3
"""Board data for the housing (enclosure/housing.py): 3D model as STEP and geometry as JSON.

Run after board.py:  python3 export_enclosure.py
"""
import os
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, "..", "tools", "hw-design", "src"))
from hwdesign.kicad.geometry import export  # noqa: E402

PCB = os.path.join(HERE, "probe-wink.kicad_pcb")
OUT = os.path.join(HERE, "..", "enclosure", "build")
os.makedirs(OUT, exist_ok=True)
subprocess.run(["kicad-cli", "pcb", "export", "step", "--subst-models", "--force", "-o",
                os.path.join(OUT, "probe-wink-board.step"), PCB], check=True, capture_output=True)
# Heights from the 3D models / datasheets; overhang is the module's real overhang past the
# board edge (the courtyard reaches 11.5 mm, the module itself 6.24 mm).
d = export(PCB, os.path.join(OUT, "board.json"),
           extra={"tallest_above_board": 5.85, "relay_height": 5.65,
                  "antenna_overhang": 6.24, "antenna_keepout": 11.48})
print("board %.1f x %.1f mm, %d parts -> %s" % (d["board"]["w"], d["board"]["h"], len(d["parts"]), OUT))
