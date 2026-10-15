"""Evaluate the few-shot configs: standalone, mixed with the heuristic, and end to end (real CLI).
Run from experiments/path-selection with PYTHONPATH=scripts."""
import json
import os
import math
import random
import statistics
import subprocess
import sys
from concurrent.futures import ThreadPoolExecutor

import numpy as np

sys.path.insert(0, "scripts")
import lab_eval as E  # noqa: E402
from ml_paths import paths as leaf_paths  # noqa: E402

CONFIGS = sys.argv[1:] or ["k0", "k0b", "fixed2", "fixed4", "fixed4_s1", "fixed8", "retr2", "retr4"]
CASES = E.CASES
sub = lambda p: p.get("h") is not None
S = {c: json.load(open(f"data/jev_scores/jev3_fs_{c}.json")) for c in CONFIGS}
score = lambda cfg: (lambda c, i, d=S[cfg]: d[str(c["case"])][i])


def logit(x):
    x = min(max(x, 1e-6), 1 - 1e-6)
    return math.log(x / (1 - x))


def z(v):
    v = np.array(v, float)
    s = v.std()
    return (v - v.mean()) / (s if s > 1e-9 else 1)


cache = {}


def blended(cfg, w=0.5):
    """z-score blend of Jev with the heuristic + arguments; Jev alone where the heuristic has no score."""
    def f(c, i):
        k = (cfg, w, c["case"])
        if k not in cache:
            zj = z([logit(S[cfg][str(c["case"])][j]) for j in range(len(c["paths"]))])
            hv = [p["h_args"] for p in c["paths"]]
            have = [x for x in hv if x is not None]
            mh = statistics.mean(have) if have else 0
            sh = (statistics.pstdev(have) or 1) if have else 1
            cache[k] = [(w * zj[j] + (1 - w) * (hv[j] - mh) / sh) if hv[j] is not None else zj[j] for j in range(len(c["paths"]))]
        return cache[k][i]
    return f


def paired(a, b, idx=None, metric="auc_e", B=600):
    """Mean per-case difference a - b with a bootstrap interval, over cases where both are defined."""
    d = []
    for cid in CASES:
        x = E.per_case(a, cid, idx)
        y = E.per_case(b, cid, idx)
        if x and y and x[metric] is not None and y[metric] is not None:
            d.append(x[metric] - y[metric])
    rng = random.Random(0)
    bs = sorted(statistics.mean(rng.choices(d, k=len(d))) for _ in range(B))
    return statistics.mean(d), bs[int(.025 * B)], bs[int(.975 * B)], len(d)


def row(name, fn, idx, base):
    s = E.summarize(fn, idx=idx)
    dm = paired(fn, base, idx) if base is not fn else (0, 0, 0, 0)
    dr = paired(fn, base, idx, "rp_e") if base is not fn else (0, 0, 0, 0)
    flag = "" if base is fn else ("  <- beyond noise" if dm[1] > 0 or dm[2] < 0 else "")
    print(f"  {name:34s} AUC {s['auc_e'][0]:.3f}  R-prec {s['rp_e'][0]:.3f}  AUC(keep) {s['auc_k'][0]:.3f} | vs k0: AUC {dm[0]:+.3f} [{dm[1]:+.3f},{dm[2]:+.3f}]  R-prec {dr[0]:+.3f}{flag}")


base = score("k0")
print("STANDALONE, array fields only (n={}), unseen servers".format(E.summarize(base, idx=sub)["auc_e"][3]))
for cfg in CONFIGS:
    row(cfg, score(cfg), sub, base)
print("\nSTANDALONE, all paths (whole JSON)")
for cfg in CONFIGS:
    row(cfg, score(cfg), None, base)
bb = blended("k0")
print("\nMIXED (z-blend 50/50 with heuristic + arguments), array fields only")
for cfg in CONFIGS:
    row("blend " + cfg, blended(cfg), sub, bb)
print("\nMIXED, all paths (Jev alone where the heuristic has no score)")
for cfg in CONFIGS:
    row("blend " + cfg, blended(cfg), None, bb)

# ---- end to end through the real CLI (--field-scores, weight 0.5), as the product does it
BIN = os.environ.get("SHRINKFIT_BIN", "shrinkfit")
pool = json.load(open("work/pool_all.json"))
final = E.final


def e2e(cid, frac, cfg):
    c = E.feats[str(cid)]
    src = pool[cid]
    n = max(400, int(len(src["text"]) * frac))
    cmd = [BIN, "-n", str(n), "-i", "0", "--array-strategy", "condense", "--no-notice", "--no-save-full", "--arguments", src["args"]]
    if cfg:
        s = S[cfg][str(cid)]
        cmd += ["--field-scores", json.dumps({p["path"]: round(s[i], 4) for i, p in enumerate(c["paths"][:70])}), "--field-score-weight", "0.5"]
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


print("\nEND TO END (real CLI, condense + arguments, scores blended at weight 0.5)")
for frac in (0.10, 0.25):
    jobs = [(cid, frac, cfg) for cfg in [None] + CONFIGS for cid in range(len(pool))]
    with ThreadPoolExecutor(8) as ex:
        res = dict(zip(jobs, ex.map(lambda a: e2e(*a), jobs)))
    print(f"  budget {int(frac * 100)}% of the response")
    ref = {cid: res[(cid, frac, "k0")] for cid in range(len(pool)) if res[(cid, frac, "k0")]}
    for cfg in [None] + CONFIGS:
        ids = [cid for cid in range(len(pool)) if res[(cid, frac, cfg)] and cid in ref]
        w = [res[(cid, frac, cfg)][0] for cid in ids]
        e = [res[(cid, frac, cfg)][1] for cid in ids]
        dw = [a - ref[cid][0] for a, cid in zip(w, ids)]
        rng = random.Random(0)
        bs = sorted(statistics.mean(rng.choices(dw, k=len(dw))) for _ in range(600))
        tag = "built-in only" if cfg is None else cfg
        print(f"    {tag:16s} labelled value kept {statistics.mean(w):.3f}  essential kept {statistics.mean(e):.3f} | vs k0: value {statistics.mean(dw):+.3f} [{bs[15]:+.3f},{bs[585]:+.3f}]")
