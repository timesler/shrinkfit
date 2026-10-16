"""Does showing Jev the tool call as formatted JSON plus a schema-preserving preview of the response help?

The preview keeps every key, truncates long strings and keeps only the first item of each array. Examples come only from
the other fold of servers, exactly as in lab_fewshot.py (this reuses its cases, retrieval and API call).

usage: lab_ctx.py <config> [...]
config: ctx0        no examples; the call and the preview
        ctx4        4 retrieved examples (fields + verdicts, as shipped); the call and the preview for the case being judged
        ctx4p       as ctx4, and each example also carries the call as JSON and its preview
        ctx0s/ctx4s the same with strings cut to 40 characters and the preview capped at 3,000 characters
Needs OPENROUTER_API_KEY. Run from experiments/path-selection:  python scripts/lab_ctx.py ctx0 ctx4"""
import json
import os
import sys
import time
from concurrent.futures import ThreadPoolExecutor

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
sys.argv, argv = sys.argv[:1], sys.argv[1:]          # lab_fewshot runs its CLI loop on import; give it nothing to do
import lab_fewshot as F  # noqa: E402

pool = json.load(open(os.path.join(F.ROOT, "work", "pool_all.json"), encoding="utf-8"))
NOTE = ("The tool response above has been truncated for brevity by trimming long strings and showing only the first item "
        "in each array, but is included to demonstrate the approximate schema of the response.")


def prune(o, s=80):
    if isinstance(o, dict):
        return {k: prune(v, s) for k, v in o.items()}
    if isinstance(o, list):
        return [prune(o[0], s)] if o else []
    if isinstance(o, str) and len(o) > s:
        return o[:s] + "..."
    return o


def preview(cid, s=80, cap=None):
    p = prune(json.loads(pool[cid]["text"]), s)
    t = json.dumps(p, ensure_ascii=False, indent=2)
    if cap and len(t) > cap:
        t = json.dumps(prune(json.loads(pool[cid]["text"]), 30), ensure_ascii=False, indent=1)[:cap] + "\n..."
    return t


def call_json(cid):
    try:
        return json.loads(pool[cid]["args"])
    except Exception:
        return pool[cid]["args"]


def build_ctx(c, exs, with_ex_preview, short):
    s, cap = (40, 3000) if short else (80, None)
    st = {}
    if exs:
        st["task"] = ("Judge each output field of the LAST tool call the way the labelled examples do. "
                      "A verdict of essential or useful means worth keeping; drop means not worth keeping.")
        lab = []
        for x in exs:
            e = F.example(x)
            if with_ex_preview:
                e["tool_call"]["arguments"] = call_json(x["case"])
                e["tool_response_preview"] = preview(x["case"], s, cap)
            lab.append(e)
        st["labelled_examples"] = lab
    st["tool_call"] = {"name": c["tool"], "arguments": call_json(c["case"])}
    st["tool_response_preview"] = preview(c["case"], s, cap)
    st["tool_response_preview_note"] = NOTE
    st["output_fields"] = F.fields_view(c, False, cap=70)
    qs = {f"p{i}": {"type": "noul", "instructions": f"Is the output field `{p['path']}` {F.GOAL}?"} for i, p in enumerate(c["paths"][:70])}
    return json.dumps(st, ensure_ascii=False, indent=1), qs


def retry(state, qs):
    for a in range(5):
        try:
            return F.call(state, qs)
        except OverflowError:
            raise
        except Exception:                       # dropped connections on large requests: try again
            if a == 4:
                raise
            time.sleep(3 * (a + 1))


def score(cfg, c):
    base = cfg.rstrip("s") if cfg.endswith("s") else cfg
    short = cfg.endswith("s")
    k = 4 if base.startswith("ctx4") else 0
    exs = F.pick_retrieved(c, k) if k else []
    while True:
        state, qs = build_ctx(c, exs, base == "ctx4p", short)
        try:
            r = retry(state, qs)
            break
        except OverflowError:
            if not exs:
                raise
            exs = exs[:-1]
    n = len(c["paths"])
    a = r["answers"]
    return c["case"], [float(a[f"p{i}"]["noul"]) if i < 70 else 0.0 for i in range(n)], r.get("usage", {}).get("input_tokens", 0), r.get("usage", {}).get("cost", 0) or 0, len(exs)


for cfg in argv:
    t0 = time.time()
    with ThreadPoolExecutor(4) as ex:
        res = list(ex.map(lambda c: score(cfg, c), F.CASES))
    json.dump({str(cid): s for cid, s, *_ in res}, open(os.path.join(F.OUT, f"jev3_ctx_{cfg}.json"), "w"))
    print(f"{cfg}: {len(res)} cases, avg input tokens {sum(r[2] for r in res) // len(res)}, cost ${sum(r[3] for r in res):.3f}, "
          f"avg examples {sum(r[4] for r in res) / len(res):.1f}, {time.time() - t0:.0f}s", flush=True)
