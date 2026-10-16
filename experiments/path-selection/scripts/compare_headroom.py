"""Headroom's SmartCrusher (the JSON compressor behind Headroom, `pip install headroom-ai`) against shrinkfit, on the 36
public-API responses and, optionally, a directory of larger fixtures with a manifest.json (as in the playground PR).

SmartCrusher is run with its default configuration. It has no size target, so shrinkfit is given the size SmartCrusher
reached (for the record counts). Needs the shrinkfit CLI in SHRINKFIT_BIN. Prints the table used in docs/comparison.md.
usage: python scripts/compare_headroom.py [FIXTURE_DIR]      Run from experiments/path-selection."""
import json
import os
import statistics
import subprocess
import sys
import time

from headroom.transforms.smart_crusher import SmartCrusher

BIN = os.environ.get("SHRINKFIT_BIN", "shrinkfit")
docs = []
if len(sys.argv) > 1:
    d = sys.argv[1]
    docs += [("fixture " + m["id"], open(f"{d}/{m['id']}.json", encoding="utf-8").read(), "") for m in json.load(open(f"{d}/manifest.json"))]
docs += [(c["tool"], c["text"], c["args"]) for c in json.load(open("work/realworld_pool.json", encoding="utf-8"))]
sc = SmartCrusher()


def biggest(o):
    best = (0, "")

    def rec(o, p):
        nonlocal best
        if isinstance(o, list):
            if o and all(isinstance(x, dict) for x in o) and len(o) > best[0]:
                best = (len(o), p)
            for x in o[:3]:
                rec(x, p + "/*")
        elif isinstance(o, dict):
            for k, v in o.items():
                rec(v, p + "/" + k)
    rec(o, "")
    return best


def count_at(o, path):
    cur = [o]
    for seg in [s for s in path.split("/") if s]:
        nxt = []
        for c in cur:
            if seg == "*":
                nxt += [x for x in c if isinstance(x, dict)] if isinstance(c, list) else []
            elif isinstance(c, dict) and seg in c:
                nxt.append(c[seg])
        cur = nxt
    return sum(len(c) for c in cur if isinstance(c, list))


ratios, valid, slowest, rows = [], 0, 0.0, []
for name, text, args in docs:
    t = json.dumps(json.loads(text), separators=(",", ":"), ensure_ascii=False)
    t0 = time.time()
    r = sc.crush(t, query=args)
    slowest = max(slowest, time.time() - t0)
    ratios.append(len(r.compressed) / len(t))
    try:
        json.loads(r.compressed)
        valid += 1
    except ValueError:
        pass
    if len(r.compressed) <= 0.6 * len(t):
        n, path = biggest(json.loads(t))
        if n:
            kept = {}
            for s in ("sample", "condense"):
                p = subprocess.run([BIN, "-n", str(len(r.compressed)), "-i", "0", "--array-strategy", s, "--no-notice", "--no-save-full",
                                    "--arguments", args or "{}"], input=t.encode(), capture_output=True)
                kept[s] = count_at(json.loads(p.stdout), path)
            rows.append((name, len(t), len(r.compressed), count_at(json.loads(r.compressed), path), n, kept["sample"], kept["condense"]))
print(f"{len(docs)} documents: median output/input {statistics.median(ratios):.2f}, cut to <=25%: {sum(x <= 0.25 for x in ratios)}, "
      f"valid JSON {valid}, slowest {slowest * 1000:.0f} ms")
print("document, input, headroom output, records kept by headroom / shrinkfit sample / shrinkfit condense (same size), of n")
for r in rows:
    print("  ", *r)
