import json, collections, random, sys
import pyarrow.parquet as pq
sys.path.insert(0, ".")
from ml_paths import paths
rows = pq.read_table("work/atlas.parquet").to_pylist()
out = []; seen = collections.Counter()
for ri, row in enumerate(rows):
    tr = json.loads(row["TRAJECTORY"]); names = {}; args = {}
    for m in tr:
        for tc in m.get("tool_calls") or []:
            names[tc["id"]] = tc["function"]["name"]; args[tc["id"]] = tc["function"]["arguments"]
        if m.get("role") != "tool": continue
        c = m["content"]; text = "".join(x.get("text", "") for x in c) if isinstance(c, list) else str(c)
        if len(text) < 2000: continue
        try: j = json.loads(text)
        except Exception: continue
        if not isinstance(j, (dict, list)): continue
        P = paths(j)
        if len(P) < 3: continue
        cid = m.get("tool_call_id"); tool = names.get(cid, "?")
        sig = (tool, tuple(sorted(P)))
        seen[sig] += 1
        if seen[sig] > 2: continue
        out.append({"row": ri, "tool": tool, "args": args.get(cid, ""), "text": json.dumps(j, ensure_ascii=False, separators=(",", ":")), "npaths": len(P), "sig": hash(sig) % 10**9})
print(len(out), "cases;", len({o["sig"] for o in out}), "distinct shapes;", collections.Counter(o["tool"].split("_")[0] for o in out).most_common())
print("responses with no array at all:", sum('[' not in o["text"] for o in out))
json.dump(out, open("work/pool_all.json", "w"))
