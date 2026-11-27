"""Merge the per-extraction files the patched illuminate writes into one {path: {sha256: facts}} JSON document."""
import glob
import json
import sys

out = {}
for f in sorted(glob.glob(sys.argv[1] + "/*.json")):
    with open(f) as fh:
        r = json.load(fh)
    out.setdefault(r["path"], {})[r["sha256"]] = r["facts"]
json.dump(out, sys.stdout, indent=1, sort_keys=True)
