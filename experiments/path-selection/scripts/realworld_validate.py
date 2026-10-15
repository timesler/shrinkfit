"""Check one labelling batch: python scripts/realworld_validate.py <k>  (run from experiments/path-selection)"""
import collections
import json
import sys

k = sys.argv[1]
cases = json.load(open(f"work/rw_in/batch_{k}.json", encoding="utf-8"))
try:
    out = json.load(open(f"work/rw_out/batch_{k}.json", encoding="utf-8"))
except Exception as e:
    sys.exit(f"cannot read work/rw_out/batch_{k}.json: {e}")
bad, hist = [], collections.Counter()
for c in cases:
    lab = out.get(str(c["case"]))
    if not isinstance(lab, dict):
        bad.append(f"case {c['case']}: missing")
        continue
    for p in c["paths"]:
        v = lab.get(str(p["i"]))
        if v not in (0, 1, 2):
            bad.append(f"case {c['case']} path {p['i']}: need 0/1/2, got {v!r}")
        else:
            hist[v] += 1
print(("INVALID: %d problems; first: %s" % (len(bad), bad[:8])) if bad else f"OK: {len(cases)} cases, labels 2/1/0 = {hist[2]}/{hist[1]}/{hist[0]}")
