"""Build the bank of labelled examples that is shown to Jev with every request.

Reads the labelled responses (data/lab_feats.json, data/lab_labels.json) and writes
  crates/shrinkfit/src/jev_bank.json   the bank compiled into shrinkfit (all servers)
  data/bank_fold0.json, bank_fold1.json  banks from one fold of servers each, used to score the OTHER fold,
                                         so the shipped retrieval can be evaluated without leakage

An entry is one labelled response: the tool call and up to 40 fields, each with up to two short example values and a
verdict (essential, useful or drop). Entries are distinct (tool, response shape) pairs, at most three per tool.

Run from experiments/path-selection:  python scripts/build_bank.py
"""
import json
import os

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
feats = json.load(open(os.path.join(ROOT, "data", "lab_feats.json"), encoding="utf-8"))
labels = json.load(open(os.path.join(ROOT, "data", "lab_labels.json"), encoding="utf-8"))
cases = [feats[str(i)] for i in range(len(feats))]

FOLD0 = {"oxylabs", "national-parks", "open-library", "wikipedia", "memory", "twelvedata", "arxiv", "google-workspace", "alchemy"}
server = lambda c: c["tool"].split("_")[0]


def verdict(case, i):
    m = labels[str(case["case"])][str(i)]
    return "essential" if m >= 1.5 else ("useful" if m >= 0.5 else "drop")


def eligible(c):
    n = len(c["paths"])
    ess = sum(labels[str(c["case"])][str(i)] >= 1.5 for i in range(n))
    return 8 <= n <= 40 and ess >= 2 and n - ess >= 3


def entry(c):
    args = c["arguments"] if isinstance(c["arguments"], str) else json.dumps(c["arguments"])
    return {
        "tool": c["tool"],
        "arguments": args[:200],
        "fields": [
            {"path": p["path"], "example_values": [s[:40] for s in p["samples"][:2]], "verdict": verdict(c, i)}
            for i, p in enumerate(c["paths"][:40])
        ],
    }


def select(pool, per_tool=3):
    distinct = {}
    for c in pool:
        if eligible(c):
            distinct.setdefault((c["tool"], tuple(p["path"] for p in c["paths"])), c)
    by_tool = {}
    for c in sorted(distinct.values(), key=lambda c: (c["tool"], -len(c["paths"]), c["case"])):
        by_tool.setdefault(c["tool"], []).append(c)
    return [entry(c) for t in sorted(by_tool) for c in by_tool[t][:per_tool]]


def write(path, bank):
    text = json.dumps(bank, ensure_ascii=False, separators=(",", ":"))
    open(path, "w", encoding="utf-8", newline="\n").write(text)
    return len(bank), len(text.encode("utf-8"))


n, size = write(os.path.join(ROOT, "..", "..", "crates", "shrinkfit", "src", "jev_bank.json"), select(cases))
print(f"built-in bank: {n} examples, {size / 1024:.0f} KiB")
for fold, keep in ((0, lambda c: server(c) in FOLD0), (1, lambda c: server(c) not in FOLD0)):
    n, size = write(os.path.join(ROOT, "data", f"bank_fold{fold}.json"), select([c for c in cases if keep(c)]))
    print(f"bank_fold{fold}.json (servers of fold {fold}): {n} examples, {size / 1024:.0f} KiB")
