#!/usr/bin/env python3
"""ERC + netlist proof: KiCad's connectivity must equal the nets declared in schematic.py."""
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, "..", "tools", "hw-design", "src"))
from hwdesign.kicad.verify import erc_and_parity  # noqa: E402

summary, _, diffs = erc_and_parity(os.path.join(HERE, "probe-wink.kicad_sch"),
                                   os.path.join(HERE, "intended_netlist.json"), os.path.join(HERE, "build"))
print(summary)
print("netlist matches declared nets" if not diffs else "\n".join(diffs))
sys.exit(1 if diffs else 0)
