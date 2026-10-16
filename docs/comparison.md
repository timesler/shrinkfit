# How shrinkfit compares with other projects

*Surveyed 16 October 2026. Star counts and versions are from GitHub and PyPI on that day; they measure popularity, not
quality. Claims about other projects come from their READMEs and, for Headroom, from running it (below); they may have
changed since.*

Many projects try to stop large tool outputs from flooding an agent's context. They differ in **what they shrink**
(any text, command output, or JSON specifically), **how** (drop content, re-encode it, move it out of the context, or
summarise it with a model) and **what they promise** (a ratio, a token count, or a hard limit). shrinkfit's niche is narrow:
**structured JSON/YAML, a hard character limit, output that is always valid and says what was cut**.

* [At a glance](#at-a-glance)
* [The projects](#the-projects)
* [Measured: shrinkfit against Headroom's SmartCrusher](#measured-shrinkfit-against-headrooms-smartcrusher)
* [Where shrinkfit is stronger, and where it is weaker](#where-shrinkfit-is-stronger-and-where-it-is-weaker)
* [Which to use](#which-to-use)

## At a glance

| | what it shrinks | method | hard size limit | output stays valid JSON | keeps every key and record | recovers the original | licence |
|---|---|---|---|---|---|---|---|
| **shrinkfit** | JSON, YAML | drops fields, samples items, shortens strings; optional model scores fields | **yes, in characters** | **yes** | yes (`condense` keeps every record) | saves the input to a file and says where | Apache-2.0 |
| [Headroom](https://github.com/headroomlabs-ai/headroom) (75k stars) | tool outputs, logs, code, prose, images, history | content-routed compressors; `SmartCrusher` for JSON (row sampling, constant factoring, tabular re-encoding) | no: adaptive, token-based | mostly (see below) | no (drops rows) | **yes** (CCR cache, retrieve tool) | Apache-2.0 |
| [TOON](https://github.com/toon-format/toon) (25k) | JSON data going into prompts | lossless re-encoding (header plus rows) | no | no (a different format) | yes (lossless) | n/a (nothing is lost) | MIT |
| [LLMLingua](https://github.com/microsoft/LLMLingua) (6.8k) | prompts, prose | small language model prunes tokens | ratio target | no (not structure-aware) | no | no | MIT |
| [context-mode](https://github.com/mksglu/context-mode) (30k) | any tool output in coding agents | keeps output in a sandbox plus a SQLite full-text index; the model searches it | n/a (out of context) | n/a | n/a | **yes** | Elastic-2.0 (source-available, not OSI open source) |
| [RTK](https://github.com/rtk-ai/rtk) (84k) | shell command output | per-command filters in a proxy | no | n/a (text) | no | no | Apache-2.0 |
| [sqz](https://github.com/ojuschugh1/sqz) (0.6k) | coding-agent tool output | rule-based compression and de-duplication, Rust | no | not a goal | no | n/a | Apache-2.0 |
| [claw-compactor](https://github.com/open-compress/claw-compactor) (2k) | text, code, JSON | 14-stage pipeline (one stage samples JSON) | no | n/a | no | partly | MIT |
| [mcp-compressor](https://github.com/atlassian-labs/mcp-compressor) (0.1k) | MCP **tool schemas**, optionally responses | compresses the tool surface; uses shrinkfit for bounded responses | via shrinkfit | yes | via shrinkfit | file or `full=true` | Apache-2.0 |
| [MCP Slim Guard](https://github.com/lennney/mcp-slim-guard) (0.2k) | MCP results | first delivery shortened, exact result recoverable | n/a | n/a | n/a | **yes** | see repo |

## The projects

### Headroom: the nearest like-for-like (and much broader)

Headroom is a context-compression layer for agents: a library, an HTTP proxy, an MCP server and a `wrap` command for many
coding agents. Its JSON component, **SmartCrusher**, is now Rust-backed. Per its README, it keeps error items, statistical
outliers and the first and last rows, chosen from field-variance statistics, and it keeps the original in a local cache that the
model can call back (`ccr_retrieve`). It also reports accuracy runs (GSM8K, SQuAD v2, BFCL) at 19 to 32% compression, and
latency well under a millisecond for a 10k-token JSON result. It is the popular choice and does far more than shrinkfit:
logs, code (AST-aware), prose (its own model), images, provider cache alignment, cross-agent memory.

The difference that matters for this comparison is the contract. Headroom optimises *tokens saved, with the original
recoverable*; it does not promise a size. shrinkfit promises a size and shows what was cut, and it can keep every record by
dropping fields instead of rows.

### TOON: a different answer to the same cost problem

TOON does not drop anything. It rewrites uniform arrays as a header plus rows, which its benchmarks put at roughly 40% fewer
tokens than formatted JSON with accuracy on par (72.2% against 71.4% on its retrieval test). That is a good deal when everything
fits. It cannot meet a 10% budget on a 200 KB response, and its output is not JSON. The two compose: shrink to a budget with
shrinkfit, then (if your consumer is a model) encode what is left. mcp-compressor already offers TOON output next to its
shrinking.

### LLMLingua and other model-based compressors

LLMLingua prunes tokens with a small language model, which suits long prose (documents, retrieved passages, chat history). On
JSON it is the wrong tool: it has no notion of keys, nesting or validity. It also needs a model at run time, where shrinkfit's
default path is deterministic and dependency-free.

### Sandboxing and recovery: context-mode, MCP Slim Guard

These keep the full output *outside* the context and let the model fetch what it needs (full-text search in context-mode's
SQLite index; exact recovery in MCP Slim Guard). Nothing is lost, at the price of an extra tool call and a host that supports
the plugin or hook. shrinkfit's view is the first delivery: it still has to be small and useful on its own, and it saves the
input (CLI) so a follow-up can retrieve more. context-mode's licence is Elastic-2.0, which is source-available rather than
open source.

### Command-output filters: RTK, sqz

RTK (84k stars) and sqz filter what shell commands print (test runners, git, build tools) with per-command rules. They are not
JSON tools; they would not shrink an API response.

### mcp-compressor

Atlassian's mcp-compressor mainly shrinks the *tool catalogue* an agent sees (a small surface, with the full schema fetched for
the selected tool). Its optional `--shrinkfit` flag bounds tool *responses* using this library, so the two are complements. (It
depends on shrinkfit; this comparison is not independent of that.)

## Measured: shrinkfit against Headroom's SmartCrusher

We ran Headroom 0.41.0's `SmartCrusher` (default configuration, the engine behind its JSON compression) on 44 documents: the
36 public-API responses used in the [real-world check](../experiments/path-selection/README.md#real-world-check-public-apis-not-mcp-atlas),
plus 8 large Jira, Confluence, Teams, Projects, Goals and Boards fixtures (up to 770 KB of JSON) from the open playground
pull request ([#2](https://github.com/timesler/shrinkfit/pull/2)).

| | SmartCrusher (default) |
|---|---|
| output size as a share of input, median | 0.86 |
| documents it cut to 25% of the input or less | 2 of 44 |
| output still valid JSON | 44 of 44 (some arrays re-encoded as one string of rows) |
| time, slowest document | 285 ms |

On this data SmartCrusher mostly leaves documents alone or trims them gently. It was built to save tokens without losing
information, with the rest recoverable, and not to reach a size. A 770 KB Jira response came out at 629 KB. That is a
reasonable outcome for its purpose and the wrong one if the rule is "must fit in 16,000 characters". shrinkfit's output on the
same fixture is 16,000 characters with all 30 issues present.

Where Headroom did cut hard, we compared record counts with shrinkfit set to the same output size (`sample` keeps whole
records, `condense` keeps every record with fewer fields):

| document | Headroom: records kept | shrinkfit `sample` | shrinkfit `condense` |
|---|---|---|---|
| projects (73 KB to 27 KB) | 14 of 38 | 16 of 38 | 38 of 38 |
| earthquakes (136 KB to 11 KB) | 16 of 189 | 17 of 189 | 189 of 189 |
| posts (25 KB to 6 KB) | 23 of 100 | 24 of 100 | 100 of 100 |

Row sampling gives about the same result in both; keeping every record by dropping fields is the option shrinkfit adds.

**What this does not show.** It compares sizes and what is kept, not downstream answer quality. Headroom publishes
accuracy results of its own; we have measured only whether the fields a reader needs survive (our own labelled data), not
whether an agent answers better. It also uses SmartCrusher alone, not Headroom's full pipeline, token budgets or
retrieval tool, so it is not a verdict on Headroom as a product. The script is `experiments/path-selection/scripts/compare_headroom.py`
(`SmartCrusher().crush(...)` with default settings).

## Where shrinkfit is stronger, and where it is weaker

**Stronger**

* A **hard character budget**, met exactly for JSON, on a deterministic path with no model and no network.
* **Always valid output** with every cut marked in place, and a report of what was dropped.
* **Field-level condensing** (keep every record, drop the same low-value fields) and **schema-first** sampling.
* **Optional model scoring** of fields with a labelled evaluation behind it (see [model scoring](model-scoring.md)).
* **Small and embeddable**: a dependency-free Rust core, wheels for five platforms, WebAssembly (under 1 MB) for browsers and Deno.

**Weaker**

* **Characters, not tokens.** Budgets are in characters; a tokenizer-aware budget is not built in.
* **Narrow scope.** JSON and YAML only: no logs, code, prose or images, which Headroom, RTK and others cover.
* **Recovery is minimal.** The CLI saves the input and says where; there is no cache, retrieve tool or index like Headroom's
  or context-mode's.
* **No agent integration layer.** No proxy, hooks or `wrap` command; you call the library, the CLI, or go through
  mcp-compressor.
* **Young and small.** Few users and a short track record next to projects with tens of thousands of stars.
* **Evidence is about fields, not answers.** Our labelled evaluation measures whether the right fields survive, not task accuracy.

## Which to use

* The consumer needs **a response under N characters that still parses**, or you serve many tools and want one predictable rule:
  shrinkfit.
* You want **broad savings across everything an agent reads**, with the original retrievable: Headroom (and, for command
  output, RTK).
* Everything fits but **costs too much**: TOON, possibly after shrinkfit.
* The data is **prose**: LLMLingua-style compression or summarisation, not shrinkfit.
* You want the model to **search large output on demand** in a coding agent: context-mode or MCP Slim Guard, with a shrunken first
  view from shrinkfit if the host allows one.

## Sources

* Headroom: [repository and README](https://github.com/headroomlabs-ai/headroom), version 0.41.0 from PyPI, run locally
* TOON: [repository and README](https://github.com/toon-format/toon)
* LLMLingua: [repository](https://github.com/microsoft/LLMLingua)
* context-mode: [repository](https://github.com/mksglu/context-mode)
* RTK: [repository](https://github.com/rtk-ai/rtk); sqz: [repository](https://github.com/ojuschugh1/sqz)
* claw-compactor: [repository](https://github.com/open-compress/claw-compactor)
* mcp-compressor: [repository](https://github.com/atlassian-labs/mcp-compressor); MCP Slim Guard: [repository](https://github.com/lennney/mcp-slim-guard)
