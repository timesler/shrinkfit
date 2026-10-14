"""Jev over the labelled set (238 cases, ALL unique JSON paths as options).
usage: ml_jev3.py <compact|full> <choice|noul> [--n N]
compact: tool call + path list with 2 sample values each (the format used in the first Jev run)
full   : the user's template - intro, tool call JSON, the full tool response; options are bare paths"""
import json, os, sys, time, statistics, urllib.request, urllib.error
from concurrent.futures import ThreadPoolExecutor
fmt_, mode = sys.argv[1], sys.argv[2]
lim = int(sys.argv[sys.argv.index("--n") + 1]) if "--n" in sys.argv else None
KEY = os.environ["OPENROUTER_API_KEY"]; MODEL = "typesafe/jev-1.13"
feats = json.load(open("data/lab_feats.json")); pool = json.load(open("work/pool_all.json"))
INTRO = ("Below is a tool call and its response. Due to the size of the response, it needs to be truncated. "
         "Fields that are needed to answer questions or make more targeted follow up tool calls (such as identifiers or URLs) are important.")
GOAL = "one of the fields worth keeping when this response is truncated so that an agent can still scan what it contains (some descriptive content) and make targeted follow-up calls (identifiers, URLs)"

def build(c, cap):
    try: args = json.loads(json.dumps(c["arguments"]))
    except Exception: args = c["arguments"]
    call = json.dumps({"name": c["tool"], "arguments": args}, ensure_ascii=False)
    P = c["paths"]
    if fmt_ == "full":
        resp = pool[c["case"]]["text"]
        if len(resp) > cap: resp = resp[:cap] + f"...[{len(resp) - cap} more characters not shown]"
        state = f"{INTRO}\n\nHere is the tool call JSON:\n{call}\n\nHere is the full tool response:\n{resp}"
        crit = {p["path"]: None for p in P}
        ins_choice = "Which JSON path in the tool response is the most important to keep when the response is truncated?"
        ins_noul = lambda p: f"Is the JSON path `{p}` important to keep when this response is truncated?"
    else:
        state = json.dumps({"tool_call": {"name": c["tool"], "arguments": c["arguments"] if isinstance(c["arguments"], str) else json.dumps(c["arguments"])[:400]},
                            "output_fields": [{"path": p["path"], "example_values": [s[:60] for s in p["samples"][:2]]} for p in P]}, ensure_ascii=False)
        crit = {p["path"]: ", ".join(s[:30] for s in p["samples"][:2]) or None for p in P}
        ins_choice = f"Which output field is most important to keep? A field is important when it is {GOAL}."
        ins_noul = lambda p: f"Is the output field `{p}` {GOAL}?"
    if mode == "noul": qs = {f"p{i}": {"type": "noul", "instructions": ins_noul(p["path"])} for i, p in enumerate(P)}
    else: qs = {"key": {"type": "choice", "instructions": ins_choice, "criteria": crit}}
    return state, qs

def call(state, qs):
    body = json.dumps({"model": MODEL, "state": state, "questions": qs}).encode()
    for a in range(4):
        req = urllib.request.Request("https://openrouter.ai/api/alpha/decisions", body, {"Authorization": f"Bearer {KEY}", "Content-Type": "application/json"})
        t = time.time()
        try:
            with urllib.request.urlopen(req, timeout=180) as r: return json.load(r), time.time() - t
        except urllib.error.HTTPError as e:
            err = e.read().decode()[:300]
            if "max_tokens_exceeded" in err: raise OverflowError
            if e.code in (429, 500, 502, 503, 504) and a < 3: time.sleep(2 * (a + 1)); continue
            raise RuntimeError(f"HTTP {e.code}: {err}")

def one(c):
    cap = 90000
    while True:
        st, qs = build(c, cap)
        try: r, dt = call(st, qs); break
        except OverflowError: cap //= 2
    a = r["answers"]; P = c["paths"]
    s = [float(a[f"p{i}"]["noul"]) for i in range(len(P))] if mode == "noul" else [float(a["key"]["probabilities"].get(p["path"], 0.0)) for p in P]
    u = r.get("usage", {})
    return c["case"], s, dt, u.get("input_tokens", 0), u.get("cost", 0) or 0, (cap if fmt_ == "full" and len(pool[c["case"]]["text"]) > cap else None)

cs = [feats[str(i)] for i in range(len(feats))][:lim]
with ThreadPoolExecutor(4) as ex: res = list(ex.map(one, cs))
tag = f"jev3_{fmt_}_{mode}"
json.dump({str(r[0]): r[1] for r in res}, open(f"data/jev_scores/{tag}.json", "w"))
lat = sorted(r[2] for r in res); n = len(lat)
print(f"{tag}: {n} calls, latency median {statistics.median(lat)*1000:.0f} ms p90 {lat[int(n*.9)-1]*1000:.0f} ms max {lat[-1]*1000:.0f} ms; avg input tokens {sum(r[3] for r in res)//n}; cost ${sum(r[4] for r in res):.4f}; truncated responses {sum(r[5] is not None for r in res)}")
