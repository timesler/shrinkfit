# Model scoring with Jev

shrinkfit decides which fields of a large response to keep. By default it uses a built-in heuristic.
With one option, it can instead ask a **decision model** that has seen the tool call and the field names,
and blend the model's judgement with the heuristic.

> **Availability.** Model scoring needs shrinkfit **0.3.0 or later**. It is opt-in and best effort, and
> shrinkfit behaves exactly as before unless you turn it on.

* [Try it](#try-it)
* [Why use it](#why-use-it)
* [How it works](#how-it-works)
* [What is sent](#what-is-sent)
* [Cost and speed](#cost-and-speed)
* [When it cannot answer](#when-it-cannot-answer)
* [Who sends the request](#who-sends-the-request)
* [Other hosts](#other-hosts)
* [Bring your own scores](#bring-your-own-scores)
* [FAQ](#faq)

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
r.report["model_scoring"]    # 'scored 24 fields with typesafe/jev-1.13 in 231 ms'
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

**How well fields are ranked** (AUC for essential fields, record-array fields only; 1.0 is perfect):

| Scorer | AUC |
|---|---|
| shrinkfit heuristic | 0.79 |
| heuristic + the call's arguments | 0.87 |
| Jev alone | 0.88 |
| **heuristic + arguments, blended 50/50 with Jev** | **0.94** |

**What survives a real shrink** (the actual command-line tool, `condense` with arguments, scores fed in at weight 0.5):

| Budget (share of the response) | Labelled value kept: built-in | with Jev | Essential fields kept: built-in | with Jev |
|---|---|---|---|---|
| 25% | 0.72 | **0.78** | 0.77 | **0.83** |
| 10% | 0.66 | **0.69** | 0.74 | **0.78** |

The 25% gain is outside the noise (+0.055 for labelled value, 95% interval +0.035 to +0.083; +0.068 for essential
fields); the 10% gain is smaller (+0.027 and +0.038). Precision (the share of kept fields that were worth keeping)
did not change.

These figures were measured with the released command-line tool calling the live service on all 238 responses
(474 requests at the two budgets; every one was answered). An earlier run that fed in recorded scores gave the
same result, so the numbers do not depend on the research scripts.

Read these numbers with care:

* The labels come from Claude agents, not humans, and the agents saw the tool arguments.
* The blend weight (0.5) was picked after trying a few values on the same data.
* The data covers about 175 distinct response shapes, and one server (oxylabs) is 63 of the 238 cases.
* The model has not been compared with every alternative; small general-purpose language models
  (up to 7B parameters) did **not** beat the heuristic on the same task, which is why a purpose-built
  decision model is used.

The data, labelling protocol, scripts and the full set of results are in
[experiments/path-selection](../experiments/path-selection).

## How it works

1. **Decide.** shrinkfit parses the document and only continues if it does not fit the budget.
2. **List the fields.** Every distinct leaf path, with arrays written as `*` (`/issues/*/fields/summary`),
   and up to two short example values for each. At most 120 fields are sent.
3. **Ask.** One request to Jev's Decisions API with the tool name, the arguments, and one yes/no
   question per field: *is this field worth keeping when the response is truncated, so that an agent can still
   scan what it contains and make targeted follow-up calls?*
4. **Read the answers.** Jev returns a probability of "yes" for each field.
5. **Blend.** The probabilities become [`field_scores`](options.md#scores-from-outside-and-model-scoring) and are
   mixed with the built-in score by `field_score_weight` (default 0.5). Protected and exact fields are unaffected.
6. **Shrink as usual.** The rest of the algorithm is unchanged.

## What is sent

Exactly this, to OpenRouter (and on to TypeSafe, who run the model):

* the tool name (`jev.tool`), if you gave one;
* the call's arguments, the first 400 characters;
* every distinct field path;
* up to two example values per path, each cut to 60 characters.

Not sent: the rest of the response, or any other value. Still, example values *are* content from the
response, so treat model scoring like any other third-party call: do not enable it for data that must not
leave your environment. The API key travels only in the request's `Authorization` header and is never printed,
logged or included in the report or in error messages.

## Cost and speed

* About 1,000 to 4,000 input tokens per request. Jev charges for input tokens only (currently $0.042 per
  million), which is about **$0.0001 per request**.
* Measured latency from a laptop: median about **240 ms**, 90th percentile about 320 ms. The default
  timeout is 3 seconds (`timeout_ms`).
* One request per document that needs cutting. If your tool returns the same shape repeatedly, the cost of an
  extra 240 ms per large result is usually small next to the model call that follows; there is no result cache yet.

## When it cannot answer

Model scoring never makes a shrink fail. If anything goes wrong, shrinkfit uses the built-in scoring (the output
is what you would have got without `jev`) and `report.model_scoring` says why. The command line also prints a
one-line note to stderr.

| `report.model_scoring` says | Meaning and what to do |
|---|---|
| `scored 24 fields with typesafe/jev-1.13 in 231 ms` | It worked. |
| `unavailable: no OpenRouter API key (set OPENROUTER_API_KEY or jev.api_key); used the built-in scoring` | No key found. |
| `unavailable: the service answered HTTP 401` | The key was rejected. |
| `unavailable: the service answered HTTP 402` | The OpenRouter account has no credit. |
| `unavailable: the service answered HTTP 429` | Rate limited. Retry later. |
| `unavailable: the request timed out` | Slower than `timeout_ms`. Raise it, or ignore it. |
| `unavailable: the request failed (...)` | No network, a TLS failure or a proxy problem. |
| `unavailable: too few fields to score` | The document has fewer than two fields. |
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
never includes it (it stays under 1 MB). With the feature, the command-line tool is about 2.3 MB and the
Python library about 2.2 MB, against about 1 MB without it, because of the bundled TLS stack.

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

`jev` is one source of `field_scores`. If you have another (your own model, a table of field usefulness
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

**Do I need an OpenRouter account?** Yes, an API key with credit. Jev has no separate sign-up.

**Can I use a different model?** The `model` and `base_url` options let you point at another endpoint, but
the request and reply formats are those of Jev's Decisions API (yes/no questions with probabilities).

**Is the output deterministic?** Not strictly: the model's probabilities can vary slightly between calls. The
built-in scoring is fully deterministic and is what you get whenever the model cannot answer.

**Will it slow my tool down?** By one extra round trip (about 240 ms) for each response that has to be cut.

**How do I switch it off?** Leave `jev` unset. Nothing else changes.
