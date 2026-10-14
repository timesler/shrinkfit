import json, math, statistics
from lab_eval import summarize, line, feats, loadj
jn = loadj("jev3_compact_noul"); jf = loadj("jev3_full_noul"); jc = loadj("jev3_compact_choice")
sub = lambda p: p.get("h") is not None
def logit(x): x = min(max(x, 1e-6), 1 - 1e-6); return math.log(x / (1 - x))
def zs(c, f):  # z-score of a scorer within one case, over all its paths
    v = [f(c, i) for i in range(len(c["paths"]))]; m = statistics.mean(v); s = statistics.pstdev(v) or 1
    return lambda i: (v[i] - m) / s
cache = {}
def blend(jev, w, key):
    def f(c, i):
        k = (key, c["case"])
        if k not in cache:
            zj = zs(c, lambda cc, ii: logit(jev(cc, ii)))
            hv = [p["h_args"] for p in c["paths"]]; have = [x for x in hv if x is not None]
            mh = statistics.mean(have) if have else 0; sh = statistics.pstdev(have) or 1 if have else 1
            cache[k] = [(w * zj(j) + (1 - w) * ((hv[j] - mh) / sh)) if hv[j] is not None else zj(j) for j in range(len(c["paths"]))]
        return cache[k][i]
    return f
print("ARRAY FIELDS ONLY"); h = lambda c, i: (c["paths"][i]["h_args"] if c["paths"][i]["h_args"] is not None else -1)
print(line("heuristic + args", summarize(h, idx=sub))); print(line("Jev yes/no compact", summarize(jn, idx=sub)))
for w in (0.3, 0.5, 0.7): print(line(f"blend heuristic+args & Jev compact w={w}", summarize(blend(jn, w, f"c{w}"), idx=sub)))
print(line("blend heuristic+args & Jev full w=0.5", summarize(blend(jf, 0.5, "f5"), idx=sub)))
print("\nALL PATHS (hybrid: blend where the heuristic has a score, Jev alone elsewhere)")
print(line("Jev yes/no compact", summarize(jn))); print(line("Jev yes/no full response", summarize(jf)))
print(line("hybrid compact w=0.5", summarize(blend(jn, 0.5, "c5")))); print(line("hybrid full w=0.5", summarize(blend(jf, 0.5, "f5"))))
