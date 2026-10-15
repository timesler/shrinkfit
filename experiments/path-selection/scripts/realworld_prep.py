"""Package the real-world responses for blind labelling, exactly like the MCP-Atlas set (see ml_lab_prep2.py).

Reads work/realworld_pool.json. Writes
  data/realworld_cases.json   every case: tool, arguments, first 1,500 characters of the response, every unique path with samples
  work/rw_in/batch_<k>.json   the labelling batches (two labellers; some cases go to both, for an agreement check)

Run from experiments/path-selection:  python scripts/realworld_prep.py
"""
import json
import os
import random
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from ml_paths import paths  # noqa: E402

pool = json.load(open("work/realworld_pool.json", encoding="utf-8"))


def values(o, p, out):
    if isinstance(o, dict):
        for k, v in o.items():
            values(v, p + "/" + k, out)
    elif isinstance(o, list):
        for v in o:
            values(v, p + "/*", out)
    elif o is not None:
        out.setdefault(p or "/", []).append(o)


cases = {}
for cid, c in enumerate(pool):
    doc = json.loads(c["text"])
    vals = {}
    values(doc, "", vals)
    P = list(paths(doc))[:70]
    ps = []
    for i, p in enumerate(P):
        ss = []
        for x in vals.get(p, []):
            x = str(x)[:60]
            if x not in ss:
                ss.append(x)
            if len(ss) >= 2:
                break
        ps.append({"i": i, "path": p, "count": len(vals.get(p, [])), "samples": ss})
    cases[cid] = {"case": cid, "tool": c["tool"], "arguments": json.loads(c["args"]), "response_chars": len(c["text"]),
                  "response_preview": c["text"][:1500] + ("..." if len(c["text"]) > 1500 else ""), "paths": ps}
os.makedirs("data", exist_ok=True)
json.dump({str(k): v for k, v in cases.items()}, open("data/realworld_cases.json", "w", encoding="utf-8"), ensure_ascii=False)

ids = list(cases)
random.Random(9).shuffle(ids)
anchors = ids[:6]                         # labelled by both
rest = ids[6:]
os.makedirs("work/rw_in", exist_ok=True)
halves = [rest[: len(rest) // 2], rest[len(rest) // 2:]]
for k in range(2):
    mine = halves[k] + anchors
    json.dump([cases[i] for i in mine], open(f"work/rw_in/batch_{k}.json", "w", encoding="utf-8"), ensure_ascii=False)
    print(f"batch {k}: {len(mine)} cases, {sum(len(cases[i]['paths']) for i in mine)} paths")
json.dump({"anchors": anchors}, open("work/rw_anchors.json", "w"))
