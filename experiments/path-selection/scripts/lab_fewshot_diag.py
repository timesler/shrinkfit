"""Why does better ranking not give better shrinks? Distribution diagnostics, weight sweep, rank normalisation.
Run from experiments/path-selection with PYTHONPATH=scripts."""
import json
import os
import random
import statistics
import subprocess
import sys
from concurrent.futures import ThreadPoolExecutor

sys.path.insert(0, "scripts")
import lab_eval as E  # noqa: E402
from ml_paths import paths as leaf_paths  # noqa: E402

CFGS = ["k0", "fixed4", "retr4"]
S = {c: json.load(open(f"data/jev_scores/jev3_fs_{c}.json")) for c in CFGS}
feats, final = E.feats, E.final
pool = json.load(open("work/pool_all.json"))
BIN = os.environ.get("SHRINKFIT_BIN", "shrinkfit")

# ---- 1. what the scores look like
print("SCORE DISTRIBUTIONS (all fields, P(yes))")
for cfg in CFGS:
    v = [x for c in S[cfg].values() for x in c]
    print(f"  {cfg:7s} mean {statistics.mean(v):.2f}  share >0.9: {sum(x > .9 for x in v) / len(v):.0%}  share <0.1: {sum(x < .1 for x in v) / len(v):.0%}  share in 0.3-0.7: {sum(.3 <= x <= .7 for x in v) / len(v):.0%}")

# how much of a case's value characters do the 'yes' fields cost, and do scores favour expensive fields?
print("\nCOST OF WHAT SCORES HIGH (per case: share of total value characters held by fields with P>0.5)")
for cfg in CFGS:
    sh, hi_ess = [], []
    for cid in range(len(feats)):
        c = feats[str(cid)]
        tot = sum(p["cost"] for p in c["paths"]) or 1
        sel = [p["cost"] for i, p in enumerate(c["paths"]) if S[cfg][str(cid)][i] > 0.5]
        sh.append(sum(sel) / tot)
    print(f"  {cfg:7s} fields with P>0.5 hold on average {statistics.mean(sh):.0%} of a response's value characters")
print("  (labelled essential fields hold:", f"{statistics.mean(sum(p['cost'] for i, p in enumerate(feats[str(c)]['paths']) if final[str(c)][str(i)] >= 1.5) / (sum(p['cost'] for p in feats[str(c)]['paths']) or 1) for c in range(len(feats))):.0%}", ")")


# ---- 2. end to end with different weights and a rank-normalised version of the scores
def transformed(cfg, how):
    out = {}
    for cid, s in S[cfg].items():
        if how == "raw":
            out[cid] = s
        else:                                   # rank within the case, scaled to 0..1
            order = sorted(range(len(s)), key=lambda i: s[i])
            r = [0.0] * len(s)
            for rank, i in enumerate(order):
                r[i] = rank / max(len(s) - 1, 1)
            out[cid] = r
    return out


def e2e(cid, frac, scores, w):
    c = feats[str(cid)]
    src = pool[cid]
    n = max(400, int(len(src["text"]) * frac))
    cmd = [BIN, "-n", str(n), "-i", "0", "--array-strategy", "condense", "--no-notice", "--no-save-full", "--arguments", src["args"]]
    if scores is not None:
        s = scores[str(cid)]
        cmd += ["--field-scores", json.dumps({p["path"]: round(s[i], 4) for i, p in enumerate(c["paths"][:70])}), "--field-score-weight", str(w)]
    p = subprocess.run(cmd, input=src["text"].encode(), capture_output=True)
    try:
        out = json.loads(p.stdout.decode("utf-8"))
    except Exception:
        return None
    present = set(leaf_paths(out))
    lab = final[str(cid)]
    tot = sum(lab[str(i)] for i in range(len(c["paths"])))
    ess = [i for i in range(len(c["paths"])) if lab[str(i)] >= 1.5]
    if tot <= 0 or not ess:
        return None
    return (sum(lab[str(i)] for i, q in enumerate(c["paths"]) if q["path"] in present) / tot,
            sum(1 for i in ess if c["paths"][i]["path"] in present) / len(ess))


def run(label, scores, w, frac):
    with ThreadPoolExecutor(8) as ex:
        res = list(ex.map(lambda cid: e2e(cid, frac, scores, w), range(len(pool))))
    ok = [r for r in res if r]
    return statistics.mean(r[0] for r in ok), statistics.mean(r[1] for r in ok)


for frac in (0.25, 0.10):
    print(f"\nEND TO END at {int(frac * 100)}% budget: labelled value kept / essential fields kept")
    print(f"  {'built-in only':28s} {run('b', None, 0, frac)[0]:.3f} / {run('b', None, 0, frac)[1]:.3f}")
    for how in ("raw", "rank"):
        for cfg in CFGS:
            line = []
            for w in (0.25, 0.5, 0.75, 1.0):
                v, e = run(cfg, transformed(cfg, how), w, frac)
                line.append(f"w={w}: {v:.3f}/{e:.3f}")
            print(f"  {cfg + ' ' + how:28s} " + "   ".join(line))
