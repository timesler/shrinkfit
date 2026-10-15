"""Nested evaluation: pick (score scaling, blend weight) on one fold of servers, score on the other.
Standalone = Jev alone (weight 1.0); mixed = blended with the built-in score (weight < 1).
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

CFGS = ["k0", "fixed2", "fixed4", "fixed8", "retr2", "retr4"]
HOW = ("raw", "rank")
WS = (0.25, 0.5, 0.75, 1.0)
FRACS = (0.10, 0.25)
S = {c: json.load(open(f"data/jev_scores/jev3_fs_{c}.json")) for c in CFGS}
feats, final = E.feats, E.final
pool = json.load(open("work/pool_all.json"))
BIN = os.environ.get("SHRINKFIT_BIN", "shrinkfit")
FOLD0 = {"oxylabs", "national-parks", "open-library", "wikipedia", "memory", "twelvedata", "arxiv", "google-workspace", "alchemy"}
fold = lambda cid: 0 if feats[str(cid)]["tool"].split("_")[0] in FOLD0 else 1


def transform(s, how):
    if how == "raw":
        return s
    order = sorted(range(len(s)), key=lambda i: s[i])
    r = [0.0] * len(s)
    for rank, i in enumerate(order):
        r[i] = rank / max(len(s) - 1, 1)
    return r


def e2e(args):
    cid, frac, cfg, how, w = args
    c, src = feats[str(cid)], pool[cid]
    n = max(400, int(len(src["text"]) * frac))
    cmd = [BIN, "-n", str(n), "-i", "0", "--array-strategy", "condense", "--no-notice", "--no-save-full", "--arguments", src["args"]]
    if cfg:
        s = transform(S[cfg][str(cid)], how)
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


grid = [(cfg, how, w) for cfg in CFGS for how in HOW for w in WS]
jobs = [(cid, f, None, "raw", 0) for cid in range(len(pool)) for f in FRACS]
jobs += [(cid, f, cfg, how, w) for (cfg, how, w) in grid for cid in range(len(pool)) for f in FRACS]
with ThreadPoolExecutor(8) as ex:
    R = dict(zip(jobs, ex.map(e2e, jobs)))

ok = [cid for cid in range(len(pool)) if all(R[(cid, f, None, "raw", 0)] for f in FRACS)]
for cfg, how, w in grid:
    ok = [cid for cid in ok if all(R[(cid, f, cfg, how, w)] for f in FRACS)]
print(f"{len(ok)} cases evaluated\n")


def val(cid, cfg, how, w, which=0):
    """Average over the two budgets of the chosen metric (0 = labelled value kept, 1 = essential fields kept)."""
    if cfg is None:
        return statistics.mean(R[(cid, f, None, "raw", 0)][which] for f in FRACS)
    return statistics.mean(R[(cid, f, cfg, how, w)][which] for f in FRACS)


def nested(cfg, weights):
    """Choose (how, w) on the other fold, apply on this fold. Returns {cid: (value, essential)} and the choices."""
    out, choice = {}, {}
    for f in (0, 1):
        train = [c for c in ok if fold(c) != f]
        test = [c for c in ok if fold(c) == f]
        best = max(((how, w) for how in HOW for w in weights), key=lambda hw: statistics.mean(val(c, cfg, hw[0], hw[1]) for c in train))
        choice[f] = best
        for c in test:
            out[c] = (val(c, cfg, best[0], best[1]), val(c, cfg, best[0], best[1], 1))
    return out, choice


base = {c: (val(c, None, "raw", 0), val(c, None, "raw", 0, 1)) for c in ok}
bs = lambda d: (statistics.mean(x[0] for x in d.values()), statistics.mean(x[1] for x in d.values()))


def paired(a, b, which=0):
    d = [a[c][which] - b[c][which] for c in ok]
    rng = random.Random(0)
    bt = sorted(statistics.mean(rng.choices(d, k=len(d))) for _ in range(800))
    return statistics.mean(d), bt[20], bt[780]


print("Averaged over the 10% and 25% budgets. value = labelled value kept, ess = essential fields kept.")
print(f"{'':34s} {'value':>6s} {'ess':>6s}   {'value vs built-in':>26s}   {'value vs zero-shot k0':>26s}   chosen (fold0 / fold1)")
print(f"{'built-in scoring only':34s} {bs(base)[0]:.3f}  {bs(base)[1]:.3f}")
for mode, weights in (("standalone (Jev alone)", (1.0,)), ("mixed (with built-in)", (0.25, 0.5, 0.75))):
    print(f"-- {mode}")
    ref, _ = nested("k0", weights)
    for cfg in CFGS:
        res, ch = nested(cfg, weights)
        v, e = bs(res)
        d1 = paired(res, base)
        d2 = paired(res, ref) if cfg != "k0" else (0, 0, 0)
        flag = "" if cfg == "k0" else ("  *" if d2[1] > 0 else ("  (worse)" if d2[2] < 0 else ""))
        print(f"  {cfg:32s} {v:.3f}  {e:.3f}   {d1[0]:+.3f} [{d1[1]:+.3f},{d1[2]:+.3f}]   {d2[0]:+.3f} [{d2[1]:+.3f},{d2[2]:+.3f}]  {ch[0]} / {ch[1]}{flag}")
print("\n(* = interval for the gain over zero-shot excludes zero)")
