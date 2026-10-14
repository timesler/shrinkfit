import json, statistics
from ml_all import CASES
EX = json.load(open("work/ml_excerpts.json"))

def paths(o, p="", out=None):
    """All unique leaf paths; arrays wildcarded (/a/*/b); same format as shrinkfit's candidates."""
    out = {} if out is None else out
    if isinstance(o, dict):
        if not o and p: out.setdefault(p, 0)
        for k, v in o.items(): paths(v, p + "/" + k, out)
    elif isinstance(o, list):
        if not o and p: out.setdefault(p, 0)
        for v in o: paths(v, p + "/*", out)
    else:
        out[p or "/"] = out.get(p or "/", 0) + 1
    return out

if __name__ == "__main__":
    sizes, npaths, sub = [], [], 0
    for c in CASES:
        j = json.loads(EX[str(c["case"])]["text"]); P = paths(j)
        sizes.append(len(EX[str(c["case"])]["text"])); npaths.append(len(P))
        sub += all(d["path"] in P for d in c["cands"])
    q = lambda v, p: sorted(v)[int(len(v) * p) - 1]
    print("cases", len(CASES), "| response chars: median", int(statistics.median(sizes)), "p90", q(sizes, .9), "max", max(sizes))
    print("unique leaf paths: median", int(statistics.median(npaths)), "p90", q(npaths, .9), "max", max(npaths))
    print("gold candidates all present among the full path set:", sub, "/", len(CASES))
    print("cases whose response exceeds ~90k chars (won't fit Jev's 32k-token window):", sum(s > 90000 for s in sizes))
