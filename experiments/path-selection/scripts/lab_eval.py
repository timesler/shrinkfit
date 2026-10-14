"""Evaluate scorers against the Claude-labelled set (238 cases, all unique JSON paths).
Targets:  essential = mean label >= 1.5   |   keep = mean label >= 1
Metrics (macro over cases, 95% bootstrap CI over cases): AUC, R-precision, and
retention@f: share of total label weight (mean label value) captured when the top-ranked paths
are kept until f of the total value characters is used (always >= 1 path)."""
import json, random, re, statistics, sys
feats = json.load(open("data/lab_feats.json")); final = json.load(open("data/lab_labels.json"))
CASES = sorted(int(k) for k in feats)
IDRE = re.compile(r"(^|_|-)(id|ids|key|code|uuid|slug)$|^(id|key|code|uuid|pageid|nctid)$|url|uri|href|link|name|title|summary", re.I)
IMG = re.compile(r"image|avatar|icon|favicon|thumb|photo|picture|logo|cover|img|color", re.I)

def auc(s, l):
    pos = [x for x, y in zip(s, l) if y]; neg = [x for x, y in zip(s, l) if not y]
    if not pos or not neg: return None
    return sum((p > q) + 0.5 * (p == q) for p in pos for q in neg) / (len(pos) * len(neg))

def rprec(s, l):
    r = sum(l)
    if not r: return None
    top = sorted(range(len(s)), key=lambda i: -s[i])[:r]; return sum(l[i] for i in top) / r

def retention(s, w, cost, f):
    tot = sum(w)
    if tot <= 0: return None
    T = sum(cost) or 1; used = 0; got = 0; first = True
    for i in sorted(range(len(s)), key=lambda i: -s[i]):
        if not first and used + cost[i] > f * T: continue
        used += cost[i]; got += w[i]; first = False
    return got / tot

def per_case(score, cid, idx=None):
    c = feats[str(cid)]; lab = final[str(cid)]
    P = [i for i in range(len(c["paths"])) if (idx is None or idx(c["paths"][i]))]
    if len(P) < 3: return None
    s = [score(c, i) for i in P]; w = [lab[str(i)] for i in P]; cost = [c["paths"][i]["cost"] for i in P]
    ess = [x >= 1.5 for x in w]; keep = [x >= 1 for x in w]
    return {"auc_e": auc(s, ess), "rp_e": rprec(s, ess), "auc_k": auc(s, keep), "rp_k": rprec(s, keep),
            "ret10": retention(s, w, cost, 0.10), "ret25": retention(s, w, cost, 0.25)}

def summarize(score, cases=CASES, idx=None, boots=400):
    rows = [r for r in (per_case(score, c, idx) for c in cases) if r]
    out = {}
    for k in rows[0]:
        v = [r[k] for r in rows if r[k] is not None]; m = statistics.mean(v)
        rng = random.Random(0); bs = sorted(statistics.mean(rng.choices(v, k=len(v))) for _ in range(boots))
        out[k] = (m, bs[int(.025 * boots)], bs[int(.975 * boots)], len(v))
    return out

def line(name, s):
    f = lambda k: f"{s[k][0]:.3f} [{s[k][1]:.2f},{s[k][2]:.2f}]"
    return f"{name:38s} AUC(ess) {f('auc_e')}  Rprec(ess) {f('rp_e')}  AUC(keep) {f('auc_k')}  ret@10% {s['ret10'][0]:.3f}  ret@25% {s['ret25'][0]:.3f}  (n={s['auc_e'][3]})"

def loadj(tag):
    try: d = json.load(open(f"data/jev_scores/{tag}.json"))
    except FileNotFoundError: return None
    return lambda c, i, d=d: d[str(c["case"])][i]

if __name__ == "__main__":
    rng = random.Random(3); rnd = {}
    def r_(c, i): return rnd.setdefault((c["case"], i), rng.random())
    base = {"random": r_,
            "key order (earlier = better)": lambda c, i: -i,
            "shorter values first": lambda c, i: -c["paths"][i]["cost"] / max(c["paths"][i]["count"], 1),
            "name regex (id/url/name/title)": lambda c, i: (1.0 if IDRE.search(c["paths"][i]["path"].rsplit("/", 1)[-1]) and not IMG.search(c["paths"][i]["path"]) else 0.0)}
    sub = lambda p: p.get("h") is not None
    print("ALL PATHS (whole JSON)")
    for n, f in base.items(): print(line(n, summarize(f)))
    h = lambda c, i: (c["paths"][i]["h"] if c["paths"][i]["h"] is not None else -1)
    ha = lambda c, i: (c["paths"][i]["h_args"] if c["paths"][i]["h_args"] is not None else -1)
    print(line("shrinkfit heuristic (unscored=lowest)", summarize(h))); print(line("shrinkfit heuristic+args (unscored=low)", summarize(ha)))
    for tag, name in (("jev3_compact_noul", "Jev yes/no, compact"), ("jev3_compact_choice", "Jev choice, compact"),
                      ("jev3_full_noul", "Jev yes/no, full response"), ("jev3_full_choice", "Jev choice, full response")):
        f = loadj(tag)
        if f: print(line(name, summarize(f)))
    print("\nARRAY FIELDS ONLY (paths the heuristic scores; like-for-like)")
    for n, f in base.items(): print(line(n, summarize(f, idx=sub)))
    print(line("shrinkfit heuristic", summarize(h, idx=sub))); print(line("shrinkfit heuristic + args", summarize(ha, idx=sub)))
    for tag, name in (("jev3_compact_noul", "Jev yes/no, compact"), ("jev3_compact_choice", "Jev choice, compact"),
                      ("jev3_full_noul", "Jev yes/no, full response"), ("jev3_full_choice", "Jev choice, full response")):
        f = loadj(tag)
        if f: print(line(name, summarize(f, idx=sub)))
