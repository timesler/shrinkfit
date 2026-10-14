# Using shrinkfit in an agent or an MCP tool

The usual job: a tool calls an API, gets a document far bigger than the model should see, and has
to return something useful anyway. This guide is the checklist for doing that well.

* [The pattern](#the-pattern)
* [Choose a strategy](#choose-a-strategy)
* [Keep identifiers exact](#keep-identifiers-exact)
* [Leave headroom for escaping](#leave-headroom-for-escaping)
* [Tell the model what happened](#tell-the-model-what-happened)
* [Check the report](#check-the-report)
* [A complete example](#a-complete-example)

## The pattern

1. Get the tool's result as a **document** (parsed JSON, or the JSON text).
2. Shrink the document with a budget.
3. Only then build the tool-result message (for MCP, the text block).

Shrink the document, **not** the serialized message. If you shrink a text block that already contains
JSON escaped inside a string, the cut lands inside the escaping and the embedded JSON breaks.

## Choose a strategy

| The result is... | Use |
|---|---|
| a list of similar records (search hits, issues, rows) | `array_strategy="condense"`, plus `arguments` |
| an object or document with some arrays | the default `sample`, or `condense` (it falls back to sampling when it cannot do better) |
| long text (an article, a log) | either; strings keep their outline |

With `condense`, pass the tool call's **arguments** so fields the caller asked about are favoured. A
search for `status = Done` keeps the status field; `SELECT amount, region` keeps those columns. Only the
argument *values* are used.

```python
shrink(result, 4000, array_strategy="condense", arguments=tool_args, indent=0)
```

If you can afford one extra request per large result, `jev=True` lets a
[decision model choose the fields](model-scoring.md), which measurably keeps more of what matters.

## Keep identifiers exact

An identifier cut to a prefix cannot be used to fetch anything, so identifiers must come out whole.
shrinkfit never truncates a value it recognises as an identifier or handle: fields named like ids, keys,
codes or slugs, unique machine-generated ids, and unique URLs. If such a field cannot fit, it is dropped
rather than corrupted.

Name any others yourself with `protect`:

```python
shrink(result, 4000, array_strategy="condense", protect=["/data/issues/*/key", "/pagination/next_cursor"])
```

Prefer anchored patterns (a leading `/`) when you know the schema. Details in
[protect patterns](options.md#protect-patterns).

## Leave headroom for escaping

The budget counts characters of the document. Once a 10,000-character JSON body is placed inside a
message, quotes and newlines are escaped and it can grow past 10,000. Set the budget a little below the
limit you must respect. How much depends on how many quotes, backslashes and newlines the document has,
so measure with a representative result.

## Tell the model what happened

The output already marks every cut in place: `"…[118 items omitted]"`, `"…": "12 keys omitted"`, `…[+2133 chars]`.
Two kinds of loss are not visible in the data and are worth a short note to the model:

* **Dropped fields.** `condense` removes fields from *every* record, so the records look complete. The
  report lists them (`dropped_fields`, `fields_dropped`). Surface a short, bounded note, for example the
  names of the fields that were dropped.
* **How to get more.** Say where the full result is, or how to ask for it (a narrower query, a field list,
  the next page). The [command-line notice](cli.md#telling-the-caller-it-was-shrunk) does this for the CLI:
  one stderr line with the counts and the path to the full copy.

In Rust, `report.summary()` gives a one-line description of the counts.

## Check the report

| Field | Why check it |
|---|---|
| `fits` | `false` means the budget was below the most compact form; the result is that form, over budget. |
| `protected_omitted` | Protected values that were left out because their whole record was omitted. A `fits: true` result can still be missing protected values. |
| `model_scoring` | If you asked for `jev`, whether it was used or why not. |

The full list is in [the report](options.md#the-report).

## A complete example

An MCP-style tool wrapper in Python that returns a size-bounded text block and a note:

```python
import json
from shrinkfit import shrink_with_report

MAX_CHARS = 8000          # leave headroom under the client's limit

def to_tool_result(document, tool_args):
    out = shrink_with_report(
        document,
        MAX_CHARS,
        array_strategy="condense",
        arguments=tool_args,                # favour the fields the caller asked about
        protect=["/pagination/next_cursor"],
        indent=0,                           # compact JSON: most information per character
    )
    notes = []
    dropped = out.report["dropped_fields"]
    if dropped:
        names = sorted({d["path"].rsplit("/", 1)[-1] for d in dropped})[:10]
        notes.append("Fields left out of every record: " + ", ".join(names))
    if out.report["pruned"]:
        notes.append("The result was shortened; cuts are marked with '…'. Ask for a narrower result for more.")
    return [{"type": "text", "text": out.text}] + [{"type": "text", "text": n} for n in notes]
```
