"""End-to-end: run the real CLI (condense + arguments) with and without external field scores and measure what survives.
Metrics per case: weighted recall = sum(mean label of surviving paths) / sum(all); essential recall = share of essential
(mean label >= 1.5) paths still present; precision = share of surviving paths that are keep-worthy (mean >= 1)."""
import json, subprocess, statistics, sys, random
from concurrent.futures import ThreadPoolExecutor
from ml_paths import paths
import os
BIN = os.environ.get("SHRINKFIT_BIN", "shrinkfit")
feats = json.load(open("data/lab_feats.json")); final = json.load(open("data/lab_labels.json")); pool = json.load(open("work/pool_all.json"))
SC = {t: json.load(open(f"data/jev_scores/{t}.json")) for t in ("jev3_compact_noul", "jev3_full_noul", "jev3_compact_choice")}
CONFIGS = [("heuristic + args (baseline)", None, 0), ("Jev compact yes/no, weight 0.3", "jev3_compact_noul", 0.3), ("Jev compact yes/no, weight 0.5", "jev3_compact_noul", 0.5),
           ("Jev compact yes/no, weight 0.7", "jev3_compact_noul", 0.7), ("Jev compact yes/no, weight 1.0", "jev3_compact_noul", 1.0),
           ("Jev full-response yes/no, weight 0.5", "jev3_full_noul", 0.5), ("Jev compact choice, weight 0.5", "jev3_compact_choice", 0.5)]
FRACS = (0.10, 0.25)

def run(cid, cfg, frac):
    c = feats[str(cid)]; src = pool[cid]; n = max(400, int(len(src["text"]) * frac))
    cmd = [BIN, "-n", str(n), "-i", "0", "--array-strategy", "condense", "--no-notice", "--no-save-full", "--arguments", src["args"]]
    if cfg[1]:
        s = SC[cfg[1]][str(cid)]; fs = {p["path"]: round(s[i], 4) for i, p in enumerate(c["paths"])}
        cmd += ["--field-scores", json.dumps(fs), "--field-score-weight", str(cfg[2])]
    p = subprocess.run(cmd, input=src["text"].encode(), capture_output=True)
    try: out = json.loads(p.stdout.decode("utf-8"))
    except Exception: return None
    present = set(paths(out)); lab = final[str(cid)]
    allp = [(i, p["path"]) for i, p in enumerate(c["paths"])]
    tot = sum(lab[str(i)] for i, _ in allp); kept = [(i, q) for i, q in allp if q in present]
    ess = [i for i, _ in allp if lab[str(i)] >= 1.5]
    if tot <= 0 or not ess: return None
    return {"w": sum(lab[str(i)] for i, _ in kept) / tot, "e": sum(1 for i in ess if c["paths"][i]["path"] in present) / len(ess),
            "p": (sum(lab[str(i)] >= 1 for i, _ in kept) / len(kept)) if kept else 0.0, "size": len(p.stdout) / max(len(src["text"]), 1)}

jobs = [(cid, k, f) for cid in range(len(pool)) for k in range(len(CONFIGS)) for f in FRACS]
with ThreadPoolExecutor(8) as ex: res = list(ex.map(lambda j: (j, run(j[0], CONFIGS[j[1]], j[2])), jobs))
def ci(v, B=400):
    rng = random.Random(0); bs = sorted(statistics.mean(rng.choices(v, k=len(v))) for _ in range(B)); return statistics.mean(v), bs[int(.025 * B)], bs[int(.975 * B)]
for f in FRACS:
    print(f"\nbudget = {int(f*100)}% of the response size (min 400 chars)")
    base = {j[0]: r for j, r in res if j[1] == 0 and j[2] == f and r}
    for k, cfg in enumerate(CONFIGS):
        rows = {j[0]: r for j, r in res if j[1] == k and j[2] == f and r}; ids = [i for i in rows if i in base]
        w = ci([rows[i]["w"] for i in ids]); e = ci([rows[i]["e"] for i in ids]); pr = statistics.mean(rows[i]["p"] for i in ids); sz = statistics.mean(rows[i]["size"] for i in ids)
        d = [rows[i]["w"] - base[i]["w"] for i in ids]; dm = ci(d) if k else (0, 0, 0)
        print(f"  {cfg[0]:38s} weighted recall {w[0]:.3f} [{w[1]:.3f},{w[2]:.3f}]  essential recall {e[0]:.3f} [{e[1]:.3f},{e[2]:.3f}]  precision {pr:.3f}  out/in {sz:.2f}  n={len(ids)}" + (f"  | vs baseline: weighted {dm[0]:+.3f} [{dm[1]:+.3f},{dm[2]:+.3f}]" if k else ""))
