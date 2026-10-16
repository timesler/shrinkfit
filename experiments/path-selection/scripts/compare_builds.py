"""Does a change to the built-in scoring or planning help or hurt on the labelled data? Runs two or more CLI builds,
Jev off, on the 238 labelled MCP-Atlas responses and the 36 real-world ones, at 25% and 10% budgets.

usage: python scripts/compare_builds.py NAME=PATH_TO_CLI [NAME=PATH ...]   (the first is the baseline)
Run from experiments/path-selection."""
import json
import os
import random
import statistics
import subprocess
import sys
from concurrent.futures import ThreadPoolExecutor

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from ml_paths import paths  # noqa: E402

builds = dict(a.split("=", 1) for a in sys.argv[1:])
sets = {
    "MCP-Atlas (238)": (json.load(open("work/pool_all.json", encoding="utf-8")), json.load(open("data/lab_feats.json", encoding="utf-8")),
                        json.load(open("data/lab_labels.json", encoding="utf-8"))),
    "public APIs (36)": (json.load(open("work/realworld_pool.json", encoding="utf-8")), json.load(open("data/realworld_cases.json", encoding="utf-8")),
                         json.load(open("data/realworld_labels.json", encoding="utf-8"))),
}


def run(a):
    name, sname, cid, frac = a
    pool, feats, labels = sets[sname]
    c, src, lab = feats[str(cid)], pool[cid], labels[str(cid)]
    n = max(400, int(len(src["text"]) * frac))
    cmd = [builds[name], "-n", str(n), "-i", "0", "--array-strategy", "condense", "--no-notice", "--no-save-full", "--arguments", src["args"]]
    p = subprocess.run(cmd, input=src["text"].encode(), capture_output=True)
    try:
        present = set(paths(json.loads(p.stdout.decode("utf-8"))))
    except Exception:
        return None
    k = len(c["paths"])
    tot = sum(lab[str(i)] for i in range(k))
    ess = [i for i in range(k) if lab[str(i)] >= 1.5]
    if tot <= 0 or not ess:
        return None
    return (sum(lab[str(i)] for i, q in enumerate(c["paths"]) if q["path"] in present) / tot, sum(1 for i in ess if c["paths"][i]["path"] in present) / len(ess))


for sname, (pool, feats, labels) in sets.items():
    jobs = [(b, sname, cid, f) for b in builds for cid in range(len(pool)) for f in (0.25, 0.10)]
    with ThreadPoolExecutor(8) as ex:
        R = dict(zip(jobs, ex.map(run, jobs)))
    ok = [cid for cid in range(len(pool)) if all(R[(b, sname, cid, f)] for b in builds for f in (0.25, 0.10))]
    base = next(iter(builds))
    print(f"\n{sname}: {len(ok)} responses (labelled value kept / essential fields kept)")
    for b in builds:
        cells = []
        for f in (0.25, 0.10):
            cells.append(f"{int(f * 100)}%: {statistics.mean(R[(b, sname, c, f)][0] for c in ok):.3f} / {statistics.mean(R[(b, sname, c, f)][1] for c in ok):.3f}")
            if b != base:
                d = [R[(b, sname, c, f)][0] - R[(base, sname, c, f)][0] for c in ok]
                rng = random.Random(0)
                bt = sorted(statistics.mean(rng.choices(d, k=len(d))) for _ in range(1000))
                cells[-1] += f" ({statistics.mean(d):+.4f} [{bt[25]:+.4f},{bt[975]:+.4f}], {sum(x != 0 for x in d)} changed)"
        print(f"  {b:12s} " + "   ".join(cells))
