"""Few-shot examples in Jev's input, evaluated on unseen servers.

Two folds, split by server. Every case is scored with examples taken ONLY from the other fold, so the examples never
share a server with the case they help judge.

usage: lab_fewshot.py <config> [<config> ...]
config: k0 | k0b (repeat of k0, for the noise floor) | fixed<K>[_s<seed>] | retr<K>
        add _choice for ONE choice question over the fields (instead of a yes/no question per field), e.g. retr4_choice
Needs OPENROUTER_API_KEY in the environment (never printed)."""
import json
import os
import random
import re
import sys
import time
import urllib.error
import urllib.request
from concurrent.futures import ThreadPoolExecutor

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
OUT = os.path.join(ROOT, "data", "jev_scores")
feats = json.load(open(os.path.join(ROOT, "data", "lab_feats.json")))
labels = json.load(open(os.path.join(ROOT, "data", "lab_labels.json")))
CASES = [feats[str(i)] for i in range(len(feats))]
KEY = os.environ["OPENROUTER_API_KEY"]
MODEL = "typesafe/jev-1.13"
GOAL = ("one of the fields worth keeping when this response is truncated so that an agent can still scan what it "
        "contains (some descriptive content) and make targeted follow-up calls (identifiers, URLs)")

srv = lambda c: c["tool"].split("_")[0]
FOLD0 = {"oxylabs", "national-parks", "open-library", "wikipedia", "memory", "twelvedata", "arxiv", "google-workspace", "alchemy"}
fold_of = lambda c: 0 if srv(c) in FOLD0 else 1


def args_text(c):
    a = c["arguments"]
    return a if isinstance(a, str) else json.dumps(a)


def fields_view(c, with_verdict=False, cap=40):
    out = []
    for i, p in enumerate(c["paths"][:cap]):
        f = {"path": p["path"], "example_values": [s[:40 if with_verdict else 60] for s in p["samples"][:2]]}
        if with_verdict:
            m = labels[str(c["case"])][str(i)]
            f["verdict"] = "essential" if m >= 1.5 else ("useful" if m >= 0.5 else "drop")
        out.append(f)
    return out


def example(c):
    return {"tool_call": {"name": c["tool"], "arguments": args_text(c)[:200]}, "fields": fields_view(c, True)}


def eligible(c):
    n = len(c["paths"])
    ess = sum(labels[str(c["case"])][str(i)] >= 1.5 for i in range(n))
    return 8 <= n <= 40 and ess >= 2 and n - ess >= 3


def pick_fixed(fold, k, seed):
    pool = [c for c in CASES if fold_of(c) == fold and eligible(c)]
    random.Random(seed).shuffle(pool)
    chosen, servers = [], set()
    for c in pool:                      # one example per server first, for diversity
        if srv(c) not in servers:
            chosen.append(c)
            servers.add(srv(c))
        if len(chosen) == k:
            return chosen
    for c in pool:
        if c not in chosen:
            chosen.append(c)
        if len(chosen) == k:
            break
    return chosen


def tokens(c):
    t = set()
    for p in c["paths"]:
        t |= set(w for w in re.split(r"[^a-z0-9]+", p["path"].rsplit("/", 1)[-1].lower()) if w)
    t |= set(w for w in re.split(r"[^a-z0-9]+", c["tool"].lower()) if w)
    return t


def pick_retrieved(c, k):
    other = [x for x in CASES if fold_of(x) != fold_of(c) and eligible(x)]
    mine = tokens(c)
    scored = sorted(other, key=lambda x: -len(mine & tokens(x)) / max(len(mine | tokens(x)), 1))
    chosen, seen = [], set()
    for x in scored:                    # distinct tools, so the examples are not near-duplicates
        if x["tool"] not in seen:
            chosen.append(x)
            seen.add(x["tool"])
        if len(chosen) == k:
            break
    return chosen


def examples_for(cfg, c):
    cfg = cfg[:-len("_choice")] if cfg.endswith("_choice") else cfg
    m = re.fullmatch(r"fixed(\d+)(?:_s(\d+))?", cfg)
    if m:
        return pick_fixed(1 - fold_of(c), int(m.group(1)), int(m.group(2) or 0))
    m = re.fullmatch(r"retr(\d+)", cfg)
    if m:
        return pick_retrieved(c, int(m.group(1)))
    return []


def build(c, exs, choice=False):
    state = {}
    if exs:
        state["task"] = ("Judge each output field of the LAST tool call the way the labelled examples do. "
                         "A verdict of essential or useful means worth keeping; drop means not worth keeping.")
        state["labelled_examples"] = [example(x) for x in exs]
    state["tool_call"] = {"name": c["tool"], "arguments": args_text(c)[:400]}
    state["output_fields"] = fields_view(c, False, cap=70)
    if choice:
        questions = {"key": {"type": "choice",
                             "instructions": f"Which output field is most important to keep? A field is important when it is {GOAL}.",
                             "criteria": {p["path"]: ", ".join(v[:30] for v in p["samples"][:2]) or None
                                          for p in c["paths"][:70]}}}
    else:
        questions = {f"p{i}": {"type": "noul", "instructions": f"Is the output field `{p['path']}` {GOAL}?"}
                     for i, p in enumerate(c["paths"][:70])}
    return json.dumps(state, ensure_ascii=False), questions


def call(state, questions):
    body = json.dumps({"model": MODEL, "state": state, "questions": questions}).encode()
    for a in range(5):
        req = urllib.request.Request("https://openrouter.ai/api/alpha/decisions", body,
                                     {"Authorization": f"Bearer {KEY}", "Content-Type": "application/json"})
        try:
            with urllib.request.urlopen(req, timeout=180) as r:
                return json.load(r)
        except urllib.error.HTTPError as e:
            err = e.read().decode()[:200]
            if "max_tokens_exceeded" in err:
                raise OverflowError
            if e.code in (429, 500, 502, 503, 504) and a < 4:
                time.sleep(2 * (a + 1))
                continue
            raise RuntimeError(f"HTTP {e.code}: {err}")


def score_case(cfg, c):
    choice = cfg.endswith("_choice")
    exs = examples_for(cfg, c)
    while True:
        state, qs = build(c, exs, choice)
        try:
            r = call(state, qs)
            break
        except OverflowError:
            exs = exs[:-1]               # too long: drop one example and retry
    a = r["answers"]
    n = len(c["paths"])
    if choice:
        pr = a["key"]["probabilities"]
        s = [float(pr.get(p["path"], 0.0)) if i < 70 else 0.0 for i, p in enumerate(c["paths"])]
    else:
        s = [float(a[f"p{i}"]["noul"]) if i < 70 else 0.0 for i in range(n)]
    return c["case"], s, r.get("usage", {}).get("input_tokens", 0), r.get("usage", {}).get("cost", 0) or 0, len(exs)


for cfg in sys.argv[1:]:
    t0 = time.time()
    with ThreadPoolExecutor(4) as ex:
        res = list(ex.map(lambda c: score_case(cfg, c), CASES))
    json.dump({str(cid): s for cid, s, *_ in res}, open(os.path.join(OUT, f"jev3_fs_{cfg}.json"), "w"))
    toks = sum(r[2] for r in res)
    print(f"{cfg}: {len(res)} cases, avg input tokens {toks // len(res)}, cost ${sum(r[3] for r in res):.3f}, "
          f"avg examples {sum(r[4] for r in res) / len(res):.1f}, {time.time() - t0:.0f}s", flush=True)
