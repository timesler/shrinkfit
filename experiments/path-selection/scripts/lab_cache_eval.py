"""Is it safe to reuse Jev's scores across calls to the same tool with the same field shape (ignoring arguments)?
For every pair of responses with the same tool and field set, shrink the second with the first's scores and compare with
its own scores. Run from experiments/path-selection with PYTHONPATH=scripts."""
import collections
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

feats, final = E.feats, E.final
pool = json.load(open("work/pool_all.json", encoding="utf-8"))
S = json.load(open("data/jev_scores/jev3_fs_retr4.json"))      # the shipped configuration's scores (examples from other servers)
BIN = os.environ.get("SHRINKFIT_BIN", "shrinkfit")

groups = collections.defaultdict(list)
for cid in range(len(pool)):
    c = feats[str(cid)]
    groups[(c["tool"], tuple(p["path"] for p in c["paths"]))].append(cid)
pairs = [(g[0], g[1]) for g in groups.values() if len(g) >= 2]
print(f"{len(pairs)} pairs of responses with the same tool and field set (of {len(groups)} distinct shapes)")


def rank(s):
    order = sorted(range(len(s)), key=lambda i: s[i])
    r = [0.0] * len(s)
    for k, i in enumerate(order):
        r[i] = k / max(len(s) - 1, 1)
    return r


def run(cid, scores, frac):
    c, src = feats[str(cid)], pool[cid]
    n = max(400, int(len(src["text"]) * frac))
    cmd = [BIN, "-n", str(n), "-i", "0", "--array-strategy", "condense", "--no-notice", "--no-save-full", "--arguments", src["args"],
           "--field-scores", json.dumps({p["path"]: round(scores[i], 4) for i, p in enumerate(c["paths"][:70])}), "--field-score-weight", "1.0"]
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


def same_args(a, b):
    return pool[a]["args"] == pool[b]["args"]


print(f"pairs whose arguments are identical: {sum(same_args(a, b) for a, b in pairs)}; different: {sum(not same_args(a, b) for a, b in pairs)}")
for frac in (0.25, 0.10):
    jobs = []
    for a, b in pairs:
        jobs.append(("own", b, frac, rank(S[str(b)])))
        jobs.append(("reused", b, frac, rank(S[str(a)])))      # b shrunk with a's scores
    with ThreadPoolExecutor(8) as ex:
        res = list(ex.map(lambda j: run(j[1], j[3], j[2]), jobs))
    own = [r for j, r in zip(jobs, res) if j[0] == "own"]
    reu = [r for j, r in zip(jobs, res) if j[0] == "reused"]
    ok = [(o, r) for o, r in zip(own, reu) if o and r]
    d = [r[0] - o[0] for o, r in ok]
    rng = random.Random(0)
    bt = sorted(statistics.mean(rng.choices(d, k=len(d))) for _ in range(800))
    print(f"{int(frac * 100)}% budget, {len(ok)} pairs: own scores {statistics.mean(o[0] for o, _ in ok):.3f}/{statistics.mean(o[1] for o, _ in ok):.3f}"
          f"  reused from the sibling {statistics.mean(r[0] for _, r in ok):.3f}/{statistics.mean(r[1] for _, r in ok):.3f}"
          f"  | change {statistics.mean(d):+.3f} [{bt[20]:+.3f},{bt[780]:+.3f}]")
