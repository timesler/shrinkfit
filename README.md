# shrinkfit

[![CI](https://github.com/timesler/shrinkfit/actions/workflows/ci.yml/badge.svg)](https://github.com/timesler/shrinkfit/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/shrinkfit.svg)](https://crates.io/crates/shrinkfit)
[![PyPI](https://img.shields.io/pypi/v/shrinkfit.svg)](https://pypi.org/project/shrinkfit/)
[![npm](https://img.shields.io/npm/v/shrinkfit.svg)](https://www.npmjs.com/package/shrinkfit)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)

**Fit any large JSON or YAML document into a character budget without losing its shape.**

shrinkfit cuts a document down to the size you choose and keeps what a reader needs most:
every field name, the structure, a representative sample of the data, and clear markers
showing exactly what was left out. The output is always valid JSON or YAML.

It is built for **AI agents and the tools that serve them**. A tool that returns 200 KB of
JSON either floods the model's context or gets rejected. With shrinkfit it can always answer
inline, and the agent can see what it got and ask a narrower follow-up if it needs more.

Rust core with no dependencies, plus Python, TypeScript/JavaScript and a command-line tool.

## See it work

A Jira-style search result: 60 issues, 69 KB. Shrink it to 5,000 characters:

```bash
shrinkfit --max-chars 5000 --indent 0 --array-strategy condense issues.json
```

```json
{"startAt":0,"maxResults":60,"total":214,"issues":[
  {"key":"OPS-100","fields":{"summary":"Crash cache search webhook onboarding"}},
  {"key":"OPS-101","fields":{"summary":"Timeout sync search login crash"}},
  {"key":"OPS-102","fields":{"summary":"Onboarding login crash latency invoice"}},
  ... all 60 issues, one line each ...
]}
```

All 60 issues survive with their key and summary, and the avatars, icon URLs, empty custom
fields and long descriptions are gone. The default strategy would instead keep 5 of the issues
and replace the other 55 with `"…[55 items omitted]"`. Either way, the document stays valid and
nothing is cut silently. (Line breaks above added for reading; the real output is one line.)

## Install

| | |
|---|---|
| **Python** 3.8+ | `pip install shrinkfit` |
| **TypeScript / JavaScript** (Node 18+, Deno, browsers) | `npm install shrinkfit` |
| **Rust** library | `cargo add shrinkfit` |
| **Command line** | `cargo install shrinkfit-cli` (installs the `shrinkfit` command) |

## Quick start

**Python**

```python
from shrinkfit import shrink

text = shrink(big_dict, max_chars=4000)            # a JSON string of at most 4000 characters
text = shrink(json_or_yaml_text, 4000)             # text in, same format out
```

**TypeScript / JavaScript**

```ts
import { init, shrink } from "shrinkfit";

await init();                                      // loads the WebAssembly module once
const text = shrink(bigObject, 4000);
```

**Rust**

```rust
use shrinkfit::{shrink_json, Options};

let out = shrink_json(&big_json_text, &Options::new(4000))?;
println!("{}", out.text);   // out.report says what was cut
```

**Command line**

```bash
shrinkfit --max-chars 4000 response.json           # or:  curl ... | shrinkfit -n 4000
```

Every call also returns a **report** (`fits`, how many items and keys were omitted, and more).
See [all options](docs/options.md) and the [command-line guide](docs/cli.md).

## What you get

* **A hard budget.** The output is never longer than `max_chars`, as long as the budget can hold
  the document's most compact form. JSON sizes are predicted exactly.
* **Always valid, never silent.** The output parses in the requested format, and every cut is
  marked in place: `"…[118 items omitted]"`, `"…": "12 keys omitted"`, `…[+2133 chars]`.
* **Schema first.** Keys and types are kept before content. Array sampling prefers records with
  *new shapes*, and always keeps the last item so you can see where a list ends.
* **Record-aware.** For lists of records, the `condense` strategy keeps every record and drops the
  same low-value fields from all of them. Pass the tool call's `arguments` and fields the caller
  asked about are favoured.
* **Exact identifiers.** IDs, handles and cursors are never truncated. Name any others with `protect`.
* **Smart about text.** Long strings keep their outline (headings, paragraphs, lines) and show a
  preview of every section instead of just a prefix.
* **Fast.** A 41 MB payload shrinks in about half a second.

### New: let a decision model choose the fields

> shrinkfit can ask a hosted **decision model, [Jev](https://openrouter.ai/docs/guides/community/jev)**,
> which fields of a response actually matter, and blend its answer with the built-in heuristic.
> You switch it on with one option and never call the model yourself:
>
> ```bash
> export OPENROUTER_API_KEY=sk-or-...
> shrinkfit -n 4000 --array-strategy condense --jev --tool-name search_issues response.json
> ```
> ```python
> shrink(data, 4000, array_strategy="condense", jev=True)               # Python
> ```
> ```ts
> await shrinkAsync(data, 4000, { arrayStrategy: "condense", jev: true }); // TypeScript
> ```
>
> On 238 labelled tool responses (tested on servers the model had no examples from), Jev raised the share of useful
> content kept at a 25% budget from 0.72 to 0.79, and the share of essential fields kept from 0.77 to 0.85. It is
> shown four similar labelled examples with each request, which sharpens its choices. It is **opt-in** (it needs an
> OpenRouter API key and sends field names and a few example values to OpenRouter) and **best effort**: if the
> call fails for any reason, shrinkfit uses the built-in scoring and tells you why.
> [Read the full guide](docs/model-scoring.md).
>
> *Model scoring needs shrinkfit 0.3.0 or later; the labelled examples ship with the release after 0.3.0.*

## Documentation

| I want to... | Read |
|---|---|
| see every option and the report fields | [Options and report](docs/options.md) |
| use the command line, the stderr notice or the playground | [Command-line guide](docs/cli.md) |
| wire shrinkfit into an agent or an MCP tool | [Using shrinkfit in an agent](docs/agents.md) |
| use Jev (or any other source) to pick fields | [Model scoring](docs/model-scoring.md) |
| understand how the budget is spent | [How it works](docs/ALGORITHM.md) |
| add a scorer or an array strategy | [Extending shrinkfit](docs/EXTENDING.md) |
| see how the code is organised, build it and test it | [Architecture and development](docs/architecture.md) |
| see the evidence behind model scoring | [Path-selection experiment](experiments/path-selection) |
| cut a release | [RELEASING.md](RELEASING.md) |

## Status

shrinkfit is at v0.2: the API is small and stable, and new capabilities arrive as options. Planned next:

* more input formats (TOML, JSON Lines; YAML input is already native);
* token budgets: a pluggable cost function (for example a tokenizer) instead of characters;
* optionally listing the names of omitted keys in the object marker.

## License

Apache-2.0. See [LICENSE](LICENSE).
