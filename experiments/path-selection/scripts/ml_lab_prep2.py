import json, random
from ml_paths import paths
pool = json.load(open("work/pool_all.json"))
def values(o, p, out):
    if isinstance(o, dict):
        for k, v in o.items(): values(v, p + "/" + k, out)
    elif isinstance(o, list):
        for v in o: values(v, p + "/*", out)
    elif o is not None: out.setdefault(p or "/", []).append(o)
cases = {}
for cid, c in enumerate(pool):
    j = json.loads(c["text"]); vals = {}; values(j, "", vals); P = list(paths(j))[:70]
    ps = []
    for i, p in enumerate(P):
        ss = []
        for x in vals.get(p, []):
            x = str(x)[:60]
            if x not in ss: ss.append(x)
            if len(ss) >= 2: break
        ps.append({"i": i, "path": p, "count": len(vals.get(p, [])), "samples": ss})
    try: args = json.loads(c["args"])
    except Exception: args = c["args"]
    cases[cid] = {"case": cid, "tool": c["tool"], "arguments": args, "response_chars": len(c["text"]),
                  "response_preview": c["text"][:1500] + ("..." if len(c["text"]) > 1500 else ""), "paths": ps}
json.dump({str(k): v for k, v in cases.items()}, open("data/lab_cases.json", "w"))
ids = list(cases); random.Random(5).shuffle(ids)
K = 12; anchors = ids[:24]; rest = ids[24:]
per = (len(rest) + K - 1) // K
for k in range(K):
    mine = rest[k * per:(k + 1) * per] + [anchors[(2 * k + j) % 24] for j in range(2)] + [anchors[(2 * ((k - 1) % K) + j) % 24] for j in range(2)]
    json.dump([cases[i] for i in mine], open(f"work/lab_in/batch_{k}.json", "w"))
    print(k, len(mine), "cases,", sum(len(cases[i]["paths"]) for i in mine), "paths,", sum(len(json.dumps(cases[i])) for i in mine) // 1000, "KB")
json.dump({"anchors": anchors}, open("data/lab_anchors.json", "w"))
