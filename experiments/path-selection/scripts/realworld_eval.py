"""Evaluate the built-in scoring and Jev on real responses from public APIs (not MCP-Atlas), labelled blind.

Needs work/realworld_pool.json, work/rw_out/batch_*.json (labels), data/realworld_cases.json, OPENROUTER_API_KEY in the
environment (never printed) and SHRINKFIT_BIN (the CLI built with default features). Jev uses the BUNDLED example bank:
none of these responses are in it, so there is no leakage.
Writes data/realworld_labels.json (the aggregated labels) and data/realworld_results.json.
Run from experiments/path-selection:  python scripts/realworld_eval.py
"""
import collections
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
pool = json.load(open("work/realworld_pool.json", encoding="utf-8"))
cases = json.load(open("data/realworld_cases.json", encoding="utf-8"))
anchors = json.load(open("work/rw_anchors.json"))["anchors"]

raw = collections.defaultdict(list)           # case -> [labels dict, ...]
for k in (0, 1):
    for cid, lab in json.load(open(f"work/rw_out/batch_{k}.json", encoding="utf-8")).items():
        raw[cid].append(lab)
labels, agree, pairs = {}, [], []
for cid, ls in raw.items():
    n = len(cases[cid]["paths"])
    labels[cid] = {str(i): statistics.mean(l[str(i)] for l in ls) for i in range(n)}
    if len(ls) == 2:
        a = [ls[0][str(i)] for i in range(n)]
        b = [ls[1][str(i)] for i in range(n)]
        pairs += list(zip(a, b))
        agree.append(sum(x == y for x, y in zip(a, b)) / n)
json.dump(labels, open("data/realworld_labels.json", "w"))
exact = sum(x == y for x, y in pairs) / len(pairs)
within = sum(abs(x - y) <= 1 for x, y in pairs) / len(pairs)
ess_both = sum(x == 2 and y == 2 for x, y in pairs)
ess_any = sum(x == 2 or y == 2 for x, y in pairs)
print(f"labeller agreement on {len(anchors)} shared cases ({len(pairs)} paths): exact {exact:.2f}, essential-essential/any-essential {ess_both}/{ess_any}")

FRACS = (0.25, 0.10)
CONFIGS = {"built-in": None, "jev-zero-shot": ["--jev-examples", "0"], "jev-default": ["--jev"]}


def run(args):
    cid, frac, name = args
    c, src = cases[str(cid)], pool[cid]
    n = max(400, int(len(src["text"]) * frac))
    cmd = [BIN, "-n", str(n), "-i", "0", "--array-strategy", "condense", "--no-notice", "--no-save-full", "--arguments", src["args"], "--report"]
    if CONFIGS[name] is not None:
        cmd += ["--jev", "--tool-name", src["tool"]] + CONFIGS[name]
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
    lab = labels[str(cid)]
    tot = sum(lab.values())
    ess = [i for i in range(len(c["paths"])) if lab[str(i)] >= 1.5]
    if tot <= 0 or not ess:
        return None
    kept = [q["path"] in present for q in c["paths"]]
    return (sum(lab[str(i)] for i, k in enumerate(kept) if k) / tot, sum(1 for i in ess if kept[i]) / len(ess), note, kept)


jobs = [(cid, f, name) for name in CONFIGS for cid in range(len(pool)) for f in FRACS]
with ThreadPoolExecutor(4) as ex:
    R = dict(zip(jobs, ex.map(run, jobs)))
ok = [cid for cid in range(len(pool)) if all(R[(cid, f, n)] for n in CONFIGS for f in FRACS)]
outc = collections.Counter(((r[2] or "none")[:60] if not (r[2] or "").startswith("scored") else "scored") for (c, f, n), r in R.items() if r and CONFIGS[n])
print(f"{len(ok)} responses; Jev outcomes: {dict(outc)}\n")
print(f"{'':16s} {'25% budget':>14s}    {'10% budget':>14s}    (labelled value kept / essential fields kept)")
for name in CONFIGS:
    print(f"{name:16s} " + "    ".join(f"{statistics.mean(R[(c, f, name)][0] for c in ok):.3f} / {statistics.mean(R[(c, f, name)][1] for c in ok):.3f}" for f in FRACS))


def paired(a, b, f, k):
    d = [R[(c, f, a)][k] - R[(c, f, b)][k] for c in ok]
    rng = random.Random(0)
    bt = sorted(statistics.mean(rng.choices(d, k=len(d))) for _ in range(2000))
    return f"{statistics.mean(d):+.3f} [{bt[50]:+.3f},{bt[1950]:+.3f}]"


for a, b in (("jev-default", "built-in"), ("jev-default", "jev-zero-shot"), ("jev-zero-shot", "built-in")):
    print(f"\n{a} minus {b}:")
    for f in FRACS:
        print(f"   {int(f * 100)}%: value {paired(a, b, f, 0)}   essential {paired(a, b, f, 1)}")

print("\nper tool family (25% budget, value kept: built-in -> jev-default):")
fam = collections.defaultdict(list)
for c in ok:
    fam[pool[c]["tool"]].append(c)
wins = ties = losses = 0
for t, cs in sorted(fam.items()):
    b = statistics.mean(R[(c, 0.25, "built-in")][0] for c in cs)
    j = statistics.mean(R[(c, 0.25, "jev-default")][0] for c in cs)
    wins += j > b + 1e-9
    losses += j < b - 1e-9
    ties += abs(j - b) <= 1e-9
    print(f"   {t:34s} n={len(cs)}  {b:.2f} -> {j:.2f}")
print(f"tools where Jev is better / equal / worse: {wins}/{ties}/{losses}")
json.dump({f"{c}|{f}|{n}": R[(c, f, n)][:3] for (c, f, n) in R if R[(c, f, n)]}, open("data/realworld_results.json", "w"))

# qualitative: what each system kept/dropped, at the 10% budget, for the reader to eyeball
with open("work/rw_report.md", "w", encoding="utf-8") as fh:
    for c in ok:
        cs = cases[str(c)]
        fh.write(f"## {c} {pool[c]['tool']} {pool[c]['args']} ({len(pool[c]['text'])} chars)\n")
        for name in ("built-in", "jev-default"):
            kept = R[(c, 0.10, name)][3]
            fh.write(f"- {name} keeps: " + ", ".join(f"{q['path']}[{labels[str(c)][str(q['i'])]:.0f}]" for q, k in zip(cs["paths"], kept) if k) + "\n")
        fh.write("- essential and dropped by jev: " + ", ".join(q["path"] for q, k in zip(cs["paths"], R[(c, 0.10, 'jev-default')][3]) if not k and labels[str(c)][str(q['i'])] >= 1.5) + "\n\n")
