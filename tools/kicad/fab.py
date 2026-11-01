"""Fabrication outputs for JLCPCB: Gerbers + drill zip, assembly BOM and CPL.

Parts are included in assembly when their schematic field Assembly == "SMT" and
they have an LCSC field. Everything else (hand-soldered THT, jumpers, holes) is
left out of BOM/CPL.
"""
import csv
import glob
import os
import subprocess
import zipfile

from .board import read_netlist

LAYERS = "F.Cu,B.Cu,F.Paste,B.Paste,F.SilkS,B.SilkS,F.Mask,B.Mask,Edge.Cuts"


def _run(*args):
    r = subprocess.run(list(args), capture_output=True, text=True)
    if r.returncode != 0:
        raise RuntimeError(" ".join(args) + "\n" + r.stdout + r.stderr)
    return r.stdout


def export(pcb, netlist, outdir, name, rot_offset=None):
    rot_offset = rot_offset or {}
    gdir = os.path.join(outdir, "gerbers")
    os.makedirs(gdir, exist_ok=True)
    for f in glob.glob(os.path.join(gdir, "*")):
        os.remove(f)
    _run("kicad-cli", "pcb", "export", "gerbers", "--layers", LAYERS, "--no-x2", "--use-drill-file-origin",
         "-o", gdir + "/", pcb)
    _run("kicad-cli", "pcb", "export", "drill", "--format", "excellon", "--drill-origin", "absolute",
         "--excellon-units", "mm", "-o", gdir + "/", pcb)
    zpath = os.path.join(outdir, name + "-gerbers.zip")
    with zipfile.ZipFile(zpath, "w", zipfile.ZIP_DEFLATED) as z:
        for f in sorted(glob.glob(os.path.join(gdir, "*"))):
            z.write(f, os.path.basename(f))

    comps, _ = read_netlist(netlist)
    smt = {r: c for r, c in comps.items()
           if c["fields"].get("Assembly") == "SMT" and c["fields"].get("LCSC")}

    groups = {}
    for ref, c in smt.items():
        key = (c["value"], c["footprint"].split(":")[-1], c["fields"]["LCSC"])
        groups.setdefault(key, []).append(ref)
    bom = os.path.join(outdir, name + "-bom-jlc.csv")
    with open(bom, "w", newline="") as f:
        w = csv.writer(f)
        w.writerow(["Comment", "Designator", "Footprint", "LCSC Part #"])
        for (val, fp, lcsc), refs in sorted(groups.items(), key=lambda kv: kv[1][0]):
            refs.sort(key=lambda r: (r.rstrip("0123456789"), int("0" + r[len(r.rstrip("0123456789")):])))
            w.writerow([val, ",".join(refs), fp, lcsc])

    pos = os.path.join(outdir, "pos-kicad.csv")
    _run("kicad-cli", "pcb", "export", "pos", "--format", "csv", "--units", "mm", "--side", "both",
         "--use-drill-file-origin", "-o", pos, pcb)
    cpl = os.path.join(outdir, name + "-cpl-jlc.csv")
    placed = set()
    with open(pos) as fi, open(cpl, "w", newline="") as fo:
        w = csv.writer(fo)
        w.writerow(["Designator", "Mid X", "Mid Y", "Layer", "Rotation"])
        for row in csv.DictReader(fi):
            ref = row["Ref"]
            if ref not in smt:
                continue
            fp = smt[ref]["footprint"].split(":")[-1]
            rot = (float(row["Rot"]) + rot_offset.get(fp, 0)) % 360
            w.writerow([ref, row["PosX"] + "mm", row["PosY"] + "mm",
                        "Top" if row["Side"].lower().startswith("top") else "Bottom", "%g" % rot])
            placed.add(ref)
    missing = sorted(set(smt) - placed)
    return dict(zip=zpath, bom=bom, cpl=cpl, parts=len(smt), lines=len(groups), missing=missing)
