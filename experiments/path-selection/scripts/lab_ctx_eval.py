"""Head-to-head, per budget: what the released version does today versus few-shot Jev.
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

CFGS = ["k0", "retr4"]
CTX = ["ctx0", "ctx4", "ctx4p", "ctx0s", "ctx4s"]
FRACS = (0.25, 0.10)
S = {c: json.load(open(f"data/jev_scores/jev3_fs_{c}.json")) for c in CFGS}
S.update({c: json.load(open(f"data/jev_scores/jev3_ctx_{c}.json")) for c in CTX})
feats, final = E.feats, E.final
pool = json.load(open("work/pool_all.json"))
BIN = os.environ.get("SHRINKFIT_BIN", "shrinkfit")


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


combos = {"built-in scoring only": (None, "raw", 0), "SHIPPED: 4 examples, fields only": ("retr4", "rank", 1.0), "zero-shot, fields only": ("k0", "rank", 1.0)}
for c in CTX:
    combos[c] = (c, "rank", 1.0)
jobs = [(cid, f) + v for v in combos.values() for cid in range(len(pool)) for f in FRACS]
with ThreadPoolExecutor(8) as ex:
    R = dict(zip(jobs, ex.map(e2e, jobs)))
ok = [cid for cid in range(len(pool)) if all(R[(cid, f) + v] for v in combos.values() for f in FRACS)]
print(len(ok), "cases")
print()
get = lambda v, f, cid, k: R[(cid, f) + v][k]
print(f"{'':44s} {'25% budget':>14s}    {'10% budget':>14s}      (labelled value kept / essential fields kept)")
for name, v in combos.items():
    print(f"{name:44s} " + "    ".join(f"{statistics.mean(get(v, f, c, 0) for c in ok):.3f} / {statistics.mean(get(v, f, c, 1) for c in ok):.3f}" for f in FRACS))


def paired(a, b, f, k=0):
    d = [get(a, f, c, k) - get(b, f, c, k) for c in ok]
    rng = random.Random(0)
    bt = sorted(statistics.mean(rng.choices(d, k=len(d))) for _ in range(800))
    return f"{statistics.mean(d):+.3f} [{bt[20]:+.3f},{bt[780]:+.3f}]"


for best_name in CTX:
    best = combos[best_name]
    for ref in ("SHIPPED: 4 examples, fields only", "built-in scoring only"):
        print()
        print(f"{best_name} minus {ref}:")
        for f in FRACS:
            print(f"   {int(f * 100)}% budget: value {paired(best, combos[ref], f)}   essential {paired(best, combos[ref], f, 1)}")
