import json, sys, collections
k = sys.argv[1]
cases = json.load(open(f"work/lab_in/batch_{k}.json"))
try: out = json.load(open(f"data/labels_raw/batch_{k}.json"))
except Exception as e: sys.exit(f"cannot read data/labels_raw/batch_{k}.json: {e}")
bad = []; hist = collections.Counter()
for c in cases:
    lab = out.get(str(c["case"]))
    if not isinstance(lab, dict): bad.append(f"case {c['case']}: missing"); continue
    for p in c["paths"]:
        v = lab.get(str(p["i"]))
        if v not in (0, 1, 2): bad.append(f"case {c['case']} path {p['i']}: need 0/1/2, got {v!r}")
        else: hist[v] += 1
if bad: print("INVALID:", len(bad), "problems; first:", bad[:8])
else: print(f"OK: {len(cases)} cases, labels 2/1/0 = {hist[2]}/{hist[1]}/{hist[0]}")
