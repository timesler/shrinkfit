"""Per-case features for the labelled set: path costs, heuristic scores (shrinkfit --explain, with/without arguments)."""
import json, re, subprocess, sys
from concurrent.futures import ThreadPoolExecutor
import os
BIN = os.environ.get("SHRINKFIT_BIN", "shrinkfit")
pool = json.load(open("work/pool_all.json")); cases = json.load(open("data/lab_cases.json"))

def explain(text, args=None, n=4000):
    cmd = [BIN, "-n", str(n), "-i", "0", "--array-strategy", "condense", "--explain", "--no-notice"] + (["--arguments", args] if args else [])
    p = subprocess.run(cmd, input=text.encode(), capture_output=True); res = {}
    for line in p.stderr.decode("utf-8", "replace").splitlines():
        m = re.match(r"\s*([0-9.]+) (keep|drop) (/\S*)", line)
        if m: res[m.group(3)] = float(m.group(1))
    return res

def values(o, p, out):
    if isinstance(o, dict):
        for k, v in o.items(): values(v, p + "/" + k, out)
    elif isinstance(o, list):
        for v in o: values(v, p + "/*", out)
    elif o is not None: out.setdefault(p or "/", []).append(o)

def one(cid):
    c = cases[str(cid)]; src = pool[cid]; vals = {}; values(json.loads(src["text"]), "", vals)
    h0 = explain(src["text"]); h1 = explain(src["text"], src["args"])
    for p in c["paths"]:
        v = vals.get(p["path"], [])
        p["cost"] = sum(len(str(x)) for x in v); p["h"] = h0.get(p["path"]); p["h_args"] = h1.get(p["path"])
    return c
with ThreadPoolExecutor(8) as ex: out = list(ex.map(one, range(len(pool))))
json.dump({str(c["case"]): c for c in out}, open("data/lab_feats.json", "w"))
sc = sum(p["h"] is not None for c in out for p in c["paths"]); tot = sum(len(c["paths"]) for c in out)
print(f"{len(out)} cases, {tot} paths, {sc} ({sc/tot:.0%}) scored by the heuristic")
