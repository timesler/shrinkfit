# Model scoring with Jev

shrinkfit decides which fields of a large response to keep. By default it uses a built-in heuristic.
With one option, it can instead ask a **decision model** that has seen the tool call, the field names and
a few labelled examples of good choices, and rank the fields by the model's judgement.

> **Availability.** Model scoring needs shrinkfit **0.3.0 or later**. The labelled examples, the `examples`,
> `weight` and `bank` options, and using Jev's scores on their own by default need **0.4.0 or later** (0.3.0 sends
> no examples and blends the scores 50/50 with the built-in ones); the score cache is on `main` and ships after 0.4.0. Model scoring is
> opt-in and best effort, and shrinkfit behaves exactly as before unless you turn it on.

* [Try it](#try-it)
* [Why use it](#why-use-it)
* [How it works](#how-it-works)
* [Labelled examples and the bank](#labelled-examples-and-the-bank)
* [What is sent](#what-is-sent)
* [Cost and speed](#cost-and-speed)
* [When it cannot answer](#when-it-cannot-answer)
* [Who sends the request](#who-sends-the-request)
* [Other hosts](#other-hosts)
* [Bring your own scores](#bring-your-own-scores)
* [FAQ](#faq)

## TWG CLI evaluation

For wide TWG responses, see the [reproducible TWG evaluation](../experiments/twg/README.md).
The measured recommendation is `condense` with JEV weight `0.7` and a 6K–12K+ character budget,
not a change to the global default. The evaluation includes identity-retention regressions,
metadata-heavy stress cases, privacy constraints and model-cost limitations.

When more than 120 leaf paths exist, populated, shallower fields get model coverage before empty
or deeply nested fields. This is coverage selection, not a replacement for JEV ranking. The
remaining fields still use built-in scoring.

## Try it

You need an [OpenRouter](https://openrouter.ai) API key with credit. Jev is the TypeSafe decision model
`typesafe/jev-1.13`, [served through OpenRouter](https://openrouter.ai/docs/guides/community/jev). You
never call it yourself.

```bash
export OPENROUTER_API_KEY=sk-or-...
shrinkfit -n 4000 --array-strategy condense --jev --tool-name search_issues \
          --arguments '{"jql": "project = OPS"}' response.json
```

```python
from shrinkfit import shrink_with_report

r = shrink_with_report(data, 4000, array_strategy="condense",
                       arguments={"jql": "project = OPS"},
                       jev={"tool": "search_issues"})        # or jev=True
r.report["model_scoring"]    # 'scored 24 fields with typesafe/jev-1.13 in 318 ms'
```

```ts
import { init, shrinkWithReportAsync } from "shrinkfit";

await init();
const { text, report } = await shrinkWithReportAsync(data, 4000, {
  arrayStrategy: "condense",
  arguments: { jql: "project = OPS" },
  jev: { tool: "search_issues" },        // or jev: true
});
report.model_scoring;
```

```rust
use shrinkfit::{shrink_json, ArrayStrategy, JevOptions, Options};

let opts = Options {
    array_strategy: ArrayStrategy::Condense,
    jev: Some(JevOptions { tool: Some("search_issues".into()), ..Default::default() }),
    ..Options::new(4000)
};                                       // needs the `jev` cargo feature
let out = shrink_json(&text, &opts)?;
```

Things to know straight away:

* It only runs when the document **has to be cut** and the strategy is `condense`. A document that already
  fits is never sent anywhere.
* The key comes from `jev.api_key` (`apiKey` in TypeScript) or the `OPENROUTER_API_KEY` environment variable.
* **TypeScript uses the async functions** (`shrinkAsync`, `shrinkWithReportAsync`), because a network
  reply cannot be waited for synchronously. See [who sends the request](#who-sends-the-request).
* The tool name and arguments are context for the model. Passing them is optional but helps.

## Why use it

The built-in heuristic is good, but it only sees the document. A decision model can tell that `summary` and
`key` matter to someone scanning search results while `avatarUrls` do not, and it can score fields the
heuristic cannot rank at all (anything outside a list of records).

We measured it on **238 real tool responses** from the
[MCP-Atlas](https://huggingface.co/datasets/ScaleAI/MCP-Atlas) dataset (19 servers, CC-BY-4.0). Claude agents
labelled each field as essential, useful or droppable for a reader who has to scan the result and make
follow-up calls. Twenty-four responses were labelled twice, and the labellers agreed on essential-or-not for 96%
of fields (Cohen's kappa 0.88).

**What survives a real shrink.** The actual command-line tool, `condense` with arguments, live against the
service (237 responses; every request but one was answered, and that one fell back to the built-in scoring).
The labelled examples shown with each request came only from servers *other than* the one being shrunk, so this
measures how it behaves on tools it has never seen:

| | 25% budget: labelled value kept | essential fields kept | 10% budget: labelled value kept | essential fields kept |
|---|---|---|---|---|
| built-in scoring only | 0.720 | 0.766 | 0.661 | 0.742 |
| Jev with no examples | 0.778 | 0.843 | 0.682 | 0.771 |
| **Jev with 4 labelled examples (the default)** | **0.793** | **0.851** | **0.713** | **0.794** |
| same, blended 50/50 with the built-in score | 0.750 | 0.798 | 0.694 | 0.772 |

Against the built-in scoring the default keeps **+0.073 more labelled value at a 25% budget** (95% interval
+0.049 to +0.099) and +0.053 at 10% (+0.033 to +0.073). The examples are worth +0.015 at 25% (just inside the
noise) and +0.031 at 10% (outside it) on top of Jev alone. Blending the scores with the built-in ones does
**worse** than using them alone, because the model's ranking is better than the heuristic's and the blend dilutes it.

**How well fields are ranked** (AUC for essential fields on record-array fields; 1.0 is perfect):

| Scorer | AUC |
|---|---|
| shrinkfit heuristic | 0.79 |
| heuristic + the call's arguments | 0.87 |
| Jev with no examples | 0.88 |
| **Jev with 4 labelled examples** | **0.94** |

Read these numbers with care:

* The labels come from Claude agents, not humans, and the agents saw the tool arguments.
* The data covers about 175 distinct response shapes from 19 servers, and one server (oxylabs) is 63 of the 238 cases.
* Real-world transfer is not measured: the examples and the test responses come from one dataset.
* Small general-purpose language models (up to 7B parameters) did **not** beat the heuristic on the same task,
  which is why a purpose-built decision model is used.

The data, labelling protocol, scripts and every result, including the experiments that led to these defaults, are in
[experiments/path-selection](../experiments/path-selection).

## How it works

1. **Decide.** shrinkfit parses the document and only continues if it does not fit the budget. If the scores for this
   tool's field set are already remembered (see [cost and speed](#cost-and-speed)), it skips to step 7.
2. **List the fields.** Every distinct leaf path, with arrays written as `*` (`/issues/*/fields/summary`),
   and up to two short example values for each. At most 120 fields are sent.
3. **Pick examples.** The four labelled examples from the [bank](#labelled-examples-and-the-bank) whose field names
   best match this document (at most one per tool).
4. **Ask.** One request to Jev's Decisions API with the examples, the tool name, the arguments, and one yes/no
   question per field: *is this field worth keeping when the response is truncated, so that an agent can still
   scan what it contains and make targeted follow-up calls?* All the questions travel in the one request.
5. **Read the answers.** Jev returns a probability of "yes" for each field.
6. **Rank.** The probabilities are replaced by each field's rank within the response (0 for the lowest, 1 for the
   highest). Jev's probabilities order fields well but sit close together, so ranks combine more predictably with
   the rest of the algorithm.
7. **Shrink as usual.** The ranks become the fields' scores (see `weight` below), and the rest of the algorithm is
   unchanged. Protected and exact fields are unaffected.

`jev.weight` (default `1`) sets how far the model's scores replace the built-in ones: `1` ranks the fields it
scored by the model alone, `0` ignores it, and in between blends the two. Fields the model did not score (beyond the
first 120) keep their built-in score. This is separate from [`field_score_weight`](options.md#scores-from-outside-and-model-scoring),
which applies to scores you supply yourself.

## Labelled examples and the bank

Showing the model a few examples of good choices sharpens its answers. Each example is a labelled response: the
tool call and its fields, with a verdict of `essential`, `useful` or `drop` for each. shrinkfit ships a **bank of
39 examples** from 20 tools, built from the labelled responses in the experiment, and sends the `examples` (default 4)
most similar to the document. Similarity is the overlap of field names and tool name, and no tool appears twice.

| Option | Default | Meaning |
|---|---|---|
| `examples` | `4` | How many examples to send. `0` sends none; the maximum is 12. Two capture most of the gain. |
| `bank` | bundled | Your own examples, as an array (Python and TypeScript: pass the list; CLI: `--jev-bank FILE`). Use it to show the model what *your* tools' good answers look like. |
| `weight` | `1` | See above. |

A bank entry looks like this:

```json
{"tool": "search_issues",
 "arguments": "{\"jql\": \"project = OPS\"}",
 "fields": [
   {"path": "/issues/*/key",             "example_values": ["OPS-1", "OPS-2"], "verdict": "essential"},
   {"path": "/issues/*/fields/summary",  "example_values": ["Crash on login"], "verdict": "essential"},
   {"path": "/issues/*/fields/created",  "example_values": ["2026-03-01T10:00:00.000+0000"], "verdict": "useful"},
   {"path": "/issues/*/fields/avatarUrls/48x48", "example_values": ["https://avatar.example/48"], "verdict": "drop"}]}
```

Eight to forty fields per example work best, and a mix of verdicts. The bundled bank is
[`crates/shrinkfit/src/jev_bank.json`](../crates/shrinkfit/src/jev_bank.json), generated by
[`experiments/path-selection/scripts/build_bank.py`](../experiments/path-selection/scripts/build_bank.py).

## What is sent

Exactly this, to OpenRouter (and on to TypeSafe, who run the model):

* the tool name (`jev.tool`), if you gave one;
* the call's arguments, the first 400 characters;
* every distinct field path;
* up to two example values per path, each cut to 60 characters;
* the labelled examples from the bank (generic, not your data).

Not sent: the rest of the response, or any other value. Still, example values *are* content from the
response, so treat model scoring like any other third-party call: do not enable it for data that must not
leave your environment. The API key travels only in the request's `Authorization` header and is never printed,
logged or included in the report or in error messages.

## Cost and speed

* About 5,000 to 8,000 input tokens per request with the default four examples (about 3,700 without). Jev charges
  for input tokens only (currently $0.042 per million), which is about **$0.0003 per request** ($0.0001 with
  `examples: 0`).
* Measured latency from a laptop: median about **320 ms** with four examples (270 ms without), 90th percentile
  about 450 ms. The default timeout is 3 seconds (`timeout_ms`).
* One request per document that needs cutting, **the first time**. The scores are remembered per tool and field
  set (the model, endpoint, `tool`, example settings and the exact list of field paths; up to 256 sets, in memory
  for the life of the process). The next response with the same fields makes no request and costs nothing, and the
  report says `(cached)`. Reusing scores costs a little accuracy, because the call's arguments are not part of the
  key: measured on 59 pairs of responses from the same tool, it was -0.011 labelled value kept at a 25% budget
  (interval -0.035 to +0.001) and -0.006 at 10%, against a gain of about +0.07 from the model. Set `cache: false`
  to ask every time. The command line starts a new process for each document, so it has nothing to remember; the
  cache is for long-running Python, Rust and TypeScript processes such as a proxy.

## When it cannot answer

Model scoring never makes a shrink fail. If anything goes wrong, shrinkfit uses the built-in scoring (the output
is what you would have got without `jev`) and `report.model_scoring` says why. The command line also prints a
one-line note to stderr.

| `report.model_scoring` says | Meaning and what to do |
|---|---|
| `scored 24 fields with typesafe/jev-1.13 in 318 ms` | It worked. |
| `scored 24 fields with typesafe/jev-1.13 (cached)` | The scores for this tool's field set were remembered; no request was made. |
| `unavailable: no OpenRouter API key (set OPENROUTER_API_KEY or jev.api_key); used the built-in scoring` | No key found. |
| `unavailable: the service answered HTTP 401` | The key was rejected. |
| `unavailable: the service answered HTTP 402` | The OpenRouter account has no credit. |
| `unavailable: the service answered HTTP 429` | Rate limited. Retry later. |
| `unavailable: the request timed out` | Slower than `timeout_ms`. Raise it, or ignore it. |
| `unavailable: the request failed (...)` | No network, a TLS failure or a proxy problem. |
| `unavailable: too few fields to score` | The document has fewer than two fields. |
| `unavailable: jev.bank: ...` | Your example bank is malformed; the message says where. |
| `unavailable: the reply had no answer for field N` / `the reply was not JSON` | The service returned something unexpected. |
| `skipped: model scoring needs array_strategy=condense` | Switch the strategy to `condense`. |
| `unavailable: this build cannot send the request itself ...` | WebAssembly or a build without the `jev` feature: use the async API, see below. |
| `null` | Not requested, or the document already fits. |

## Who sends the request

The core builds the request and reads the reply. The only question is who moves the bytes, and the answer
depends on where shrinkfit runs:

| Where | Who sends the request |
|---|---|
| **Command line**, **Python**, **Rust** | The core, with a small built-in HTTPS client. |
| **TypeScript / JavaScript** | Your runtime's `fetch`, called by `shrinkAsync` and `shrinkWithReportAsync`. |

WebAssembly cannot open network connections, so in TypeScript the core builds the request, `fetch` carries it,
and the core reads the reply. You do not do any of that yourself. If you need a proxy, extra headers or a
runtime without a global `fetch`, pass your own:

```ts
const text = await shrinkAsync(data, 4000, {
  arrayStrategy: "condense",
  jev: { tool: "search_issues", fetch: (url, init) => myFetch(url, init) },
});
```

The synchronous `shrink` and `shrinkWithReport` cannot wait for a reply, so with `jev` set they report
`unavailable` and use the built-in scoring.

**Build note (Rust):** the HTTP client sits behind the `jev` cargo feature, which is off by default so the
core crate has no dependencies. The command-line tool and the Python library enable it; the WebAssembly build
never includes it (it is under 1 MB, including the example bank). With the feature, the command-line tool is about
2.4 MB and the Python library about 2.3 MB, against about 1.1 MB without it, because of the bundled TLS stack.

## Other hosts

Anything that can call the C ABI but does its own networking (Go, Ruby, a service with a shared HTTP client)
can do the same two steps TypeScript does:

1. Call `shrinkfit_model_request(input, options)` (Rust: `shrinkfit::model_request`). It returns
   `{"ok": true, "request": null}` when no request is needed, or
   `{"ok": true, "request": {"url", "headers", "body", "timeout_ms"}}`, or `{"ok": false, "error": "..."}`
   when scoring is not possible (for example, no key).
2. Send the request, then call `shrinkfit_shrink` again with the **same options plus** `jev_reply` set to the
   response body, or `jev_error` set to a short description of what went wrong. With either set, the core makes no
   request of its own and a failure falls back to the built-in scoring.

## Bring your own scores

`jev` is one source of external scores. If you have another (your own model, a table of field usefulness
learned from earlier calls, a per-tool allow-list), pass the scores directly and skip the network:

```python
shrink(data, 4000, array_strategy="condense",
       field_scores={"/issues/*/fields/summary": 1.0, "/issues/*/fields/avatarUrls": 0.0})
```

Patterns use the same syntax as `protect`; a score is 0 to 1 and is blended with the built-in score by
`field_score_weight`. Both options are described in [options](options.md#scores-from-outside-and-model-scoring).

## FAQ

**Do I need to call Jev myself?** No. Set `jev`; shrinkfit builds the request, sends it (or has your
`fetch` send it) and uses the reply.

**Is it one request per field?** No: one request per document. The per-field part is only the list of yes/no
questions inside it.

**Why yes/no questions rather than one "which field matters most?" question?** We tested both. Without examples
the single-question form does about as well and sends a smaller request (about 2,600 tokens against 3,700). But it does
not improve with examples (it gets slightly worse), while the yes/no form improves clearly, so yes/no is the default. See
the [experiment](../experiments/path-selection/README.md#few-shot-examples-in-jevs-input).

**Do I need an OpenRouter account?** Yes, an API key with credit. Jev has no separate sign-up.

**Can I use a different model?** The `model` and `base_url` options let you point at another endpoint, but
the request and reply formats are those of Jev's Decisions API (yes/no questions with probabilities).

**Is the output deterministic?** Not strictly: the model's probabilities can vary slightly between calls. The
built-in scoring is fully deterministic and is what you get whenever the model cannot answer.

**Will it slow my tool down?** By one extra round trip (about 320 ms) the first time a response of a given shape has to be cut. After that the scores are remembered and there is no delay.

**How do I switch it off?** Leave `jev` unset. Nothing else changes.
