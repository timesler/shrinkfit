"""Live check of the SHIPPED few-shot Jev path, without leakage.

Runs the real CLI (`--jev`: its own example retrieval, request and rank scoring) on every labelled response, with
a bank built only from the OTHER fold of servers (data/bank_fold0.json scores fold-1 responses and vice versa), at a
10% and a 25% budget, and compares with zero-shot Jev and with the built-in scoring.

Needs OPENROUTER_API_KEY in the environment (never printed) and SHRINKFIT_BIN (the CLI, built with its default features).
Run from experiments/path-selection:  python scripts/lab_live_fold.py
"""
import json
import os
import random
import statistics
import subprocess
import sys
from concurrent.futures import ThreadPoolExecutor

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from ml_paths import paths  # noqa: E402

BIN = os.environ.get("SHRINKFIT_BIN", "shrinkfit")
feats = json.load(open("data/lab_feats.json", encoding="utf-8"))
final = json.load(open("data/lab_labels.json", encoding="utf-8"))
pool = json.load(open("work/pool_all.json", encoding="utf-8"))
FOLD0 = {"oxylabs", "national-parks", "open-library", "wikipedia", "memory", "twelvedata", "arxiv", "google-workspace", "alchemy"}
FRACS = (0.25, 0.10)
CONFIGS = {
    "built-in scoring only": None,
    "Jev zero-shot (--jev-examples 0)": ["--jev-examples", "0"],
    "Jev with 4 retrieved examples": ["--jev-examples", "4"],
    "Jev with 4 examples, mixed (--jev-weight 0.5)": ["--jev-examples", "4", "--jev-weight", "0.5"],
}


def bank_for(cid):
    own_fold0 = pool[cid]["tool"].split("_")[0] in FOLD0
    return "data/bank_fold1.json" if own_fold0 else "data/bank_fold0.json"   # examples from the other fold


def run(args):
    cid, frac, name = args
    c, src = feats[str(cid)], pool[cid]
    n = max(400, int(len(src["text"]) * frac))
    cmd = [BIN, "-n", str(n), "-i", "0", "--array-strategy", "condense", "--no-notice", "--no-save-full", "--arguments", src["args"], "--report"]
    if CONFIGS[name] is not None:
        cmd += ["--jev", "--tool-name", src["tool"], "--jev-bank", bank_for(cid)] + CONFIGS[name]
    p = subprocess.run(cmd, input=src["text"].encode(), capture_output=True)
    try:
        out = json.loads(p.stdout.decode("utf-8"))
    except Exception:
        return None
    note = None
    for line in p.stderr.decode("utf-8", "replace").splitlines():
        if line.startswith("{"):
            note = json.loads(line).get("model_scoring")
    present = set(paths(out))
    lab = final[str(cid)]
    tot = sum(lab[str(i)] for i in range(len(c["paths"])))
    ess = [i for i in range(len(c["paths"])) if lab[str(i)] >= 1.5]
    if tot <= 0 or not ess:
        return None
    return (sum(lab[str(i)] for i, q in enumerate(c["paths"]) if q["path"] in present) / tot,
            sum(1 for i in ess if c["paths"][i]["path"] in present) / len(ess), note)


jobs = [(cid, f, name) for name in CONFIGS for cid in range(len(pool)) for f in FRACS]
with ThreadPoolExecutor(4) as ex:
    R = dict(zip(jobs, ex.map(run, jobs)))
ok = [cid for cid in range(len(pool)) if all(R[(cid, f, n)] for n in CONFIGS for f in FRACS)]
outcomes = {}
for (cid, f, name), r in R.items():
    if r and CONFIGS[name] is not None:
        key = "scored" if (r[2] or "").startswith("scored") else (r[2] or "none")[:80]
        outcomes[key] = outcomes.get(key, 0) + 1
print(f"{len(ok)} responses; Jev outcomes across {sum(outcomes.values())} live requests: {outcomes}\n")
print(f"{'':48s} {'25% budget':>14s}    {'10% budget':>14s}    (labelled value kept / essential fields kept)")
for name in CONFIGS:
    cells = [f"{statistics.mean(R[(c, f, name)][0] for c in ok):.3f} / {statistics.mean(R[(c, f, name)][1] for c in ok):.3f}" for f in FRACS]
    print(f"{name:48s} " + "    ".join(cells))


def paired(a, b, f, k=0):
    d = [R[(c, f, a)][k] - R[(c, f, b)][k] for c in ok]
    rng = random.Random(0)
    bt = sorted(statistics.mean(rng.choices(d, k=len(d))) for _ in range(800))
    return f"{statistics.mean(d):+.3f} [{bt[20]:+.3f},{bt[780]:+.3f}]"


best = "Jev with 4 retrieved examples"
for ref in ("Jev zero-shot (--jev-examples 0)", "built-in scoring only"):
    print(f"\n{best} minus {ref}:")
    for f in FRACS:
        print(f"   {int(f * 100)}% budget: value {paired(best, ref, f)}   essential {paired(best, ref, f, 1)}")
