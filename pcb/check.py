#!/usr/bin/env python3
"""ERC + netlist proof: KiCad's connectivity must equal the nets declared in schematic.py."""
import json
import os
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, "..", "tools"))
from kicad.schematic import compare_netlist  # noqa: E402

B = os.path.join(HERE, "build")
os.makedirs(B, exist_ok=True)
sch = os.path.join(HERE, "probe-wink.kicad_sch")
erc = subprocess.run(["kicad-cli", "sch", "erc", "--severity-all", "-o", os.path.join(B, "erc.rpt"), sch],
                     capture_output=True, text=True).stdout
print([l for l in erc.splitlines() if "Found" in l][0])
subprocess.run(["kicad-cli", "sch", "export", "netlist", "--format", "kicadsexpr",
                "-o", os.path.join(B, "probe-wink.net"), sch], capture_output=True, check=True)
diffs = compare_netlist(json.load(open(os.path.join(HERE, "intended_netlist.json"))),
                        os.path.join(B, "probe-wink.net"))
print("netlist matches declared nets" if not diffs else "\n".join(diffs))
sys.exit(1 if diffs else 0)
