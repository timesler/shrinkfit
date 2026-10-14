# shrinkfit for Python

**Fit any large JSON or YAML document into a character budget without losing its shape.**

shrinkfit cuts a document down to the size you choose and keeps what a reader needs most: every field
name, the structure, a representative sample of the data, and clear markers showing what was left out. The
output is always valid JSON or YAML. It is built for AI agents and the tools that serve them: a tool that
returns 200 KB of JSON can always answer inline, and the agent can see what it got and ask a narrower
follow-up.

The core is written in Rust and bundled in the wheel (no dependencies, no compiler needed).

```bash
pip install shrinkfit        # Python 3.8+
```

## Quick start

```python
from shrinkfit import shrink, shrink_with_report

text = shrink(big_dict, max_chars=4000)          # a JSON string of at most 4000 characters
text = shrink(big_dict, 4000, indent=0)          # compact one-line JSON: most information per character
text = shrink(big_dict, 4000, format="yaml")     # YAML output
text = shrink(json_or_yaml_text, 4000)           # str or bytes in: format detected, same format out

result = shrink_with_report(big_dict, 4000)
result.text                                      # the shrunk document
result.report["fits"], result.report["items_omitted"]
```

Cuts are marked in place so nothing is hidden: `"…[118 items omitted]"`, `"…": "12 keys omitted"`,
`…[+2133 chars]`.

## Lists of records

For search results, rows or issues, keep **every** record and drop the same low-value fields from all
of them, and tell shrinkfit what the call asked for:

```python
shrink(
    result,
    4000,
    array_strategy="condense",
    arguments={"jql": "project = OPS AND status = Done"},   # fields it mentions are favoured
    protect=["/issues/*/key"],                               # never truncate these values
    indent=0,
)
```

## Let a decision model choose the fields

shrinkfit can ask the hosted decision model **Jev** which fields matter, and blend its answer with the
built-in heuristic. One option, no extra code:

```python
# Set OPENROUTER_API_KEY in the environment (or pass jev={"api_key": ...} from your secrets store).
r = shrink_with_report(data, 4000, array_strategy="condense", jev={"tool": "search_issues"})
r.report["model_scoring"]       # 'scored 24 fields with typesafe/jev-1.13 in 231 ms'
```

It is opt-in and best effort: if the call cannot be made, shrinkfit uses the built-in scoring and
`model_scoring` tells you why. It sends field names and a few example values to OpenRouter. In an evaluation
on 238 tool responses it raised the share of essential fields kept at a 25% budget from 0.77 to 0.83. See the
[model scoring guide](https://github.com/timesler/shrinkfit/blob/main/docs/model-scoring.md).
*(Ships with the release after 0.2.2.)*

## More

* [All options and the report](https://github.com/timesler/shrinkfit/blob/main/docs/options.md)
* [Using shrinkfit in an agent or MCP tool](https://github.com/timesler/shrinkfit/blob/main/docs/agents.md)
* [How it works](https://github.com/timesler/shrinkfit/blob/main/docs/ALGORITHM.md)
* [Project home](https://github.com/timesler/shrinkfit)

License: Apache-2.0
