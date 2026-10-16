# Options and report

Every way of calling shrinkfit (Rust, Python, TypeScript, the command line, the C ABI) takes the
same options and returns the same report. This page is the reference for both.

* [How options are passed](#how-options-are-passed)
* [Budget and formats](#budget-and-formats)
* [Choosing which fields survive](#choosing-which-fields-survive)
* [Scores from outside, and model scoring](#scores-from-outside-and-model-scoring)
* [Fine tuning](#fine-tuning)
* [The report](#the-report)

## How options are passed

The core defines each option once, as a JSON key such as `max_chars`. The bindings only spell it
the way their language does:

| Language | How | Example |
|---|---|---|
| **Python** | keyword arguments, same names as the JSON keys | `shrink(data, 4000, array_strategy="condense")` |
| **TypeScript** | an options object, camelCase | `shrink(data, 4000, { arrayStrategy: "condense" })` |
| **Rust** | fields of `Options` | `Options { array_strategy: ArrayStrategy::Condense, ..Options::new(4000) }` |
| **Command line** | flags, see the [command-line guide](cli.md) | `shrinkfit -n 4000 --array-strategy condense` |
| **C ABI / anything else** | a JSON object | `{"max_chars": 4000, "array_strategy": "condense"}` |

Unknown option names are rejected, so a typo never silently does nothing. In the tables below, the
first column is the JSON/Python name and the second is the TypeScript name.

## Budget and formats

| Option | TypeScript | Default | Meaning |
|---|---|---|---|
| `max_chars` | 2nd argument | `10000` | The output ceiling, in Unicode characters. |
| `format` | `format` | `"auto"` | Output format: `"auto"` (same as the input; JSON for in-memory values), `"json"` or `"yaml"`. |
| `input_format` | `inputFormat` | `"auto"` | For text input: `"auto"` (JSON first, then YAML), `"json"` or `"yaml"`. In Rust, YAML input needs the `yaml` cargo feature. |
| `indent` | `indent` | `2` | JSON: `0` gives compact single-line output. YAML: at least 2. |

Notes:

* The budget is a **hard ceiling** whenever it can hold the document's most compact form. Below
  that, shrinkfit returns that most compact form and reports `fits: false`.
* JSON sizes are predicted exactly. For YAML the real output is measured and the internal budget
  corrected, so YAML lands a little under the limit.
* Compact output (`indent: 0`) packs the most information per character. Use it when the budget is tight.

## Choosing which fields survive

These matter most for lists of records (search results, rows, issues).

| Option | TypeScript | Default | Meaning |
|---|---|---|---|
| `array_strategy` | `arrayStrategy` | `"sample"` | How oversized arrays shrink. `"sample"` keeps a few whole items and marks the gap. `"condense"` keeps **every** item and drops the same low-value fields from all of them. |
| `arguments` | `arguments` | none | The tool call's arguments: a string, or an object or array whose *values* are used (names are ignored). Under `condense`, fields whose name or values mention them are favoured, such as the columns of a `SELECT` or the matched field of a search. |
| `protect` | `protect` | `[]` | Paths whose values must never be altered. They are kept exactly or, if they cannot fit, the result reports `fits: false`. See [protect patterns](#protect-patterns). CLI: `--protect`, repeatable. |
| `field_scorer` | `fieldScorer` | `"heuristic"` | The built-in ranking of fields for `condense`: `"heuristic"` (the default), or two baselines for comparison, `"compact"` (value per character) and `"key_order"` (earlier keys win). |

**Which strategy?** Use `sample` when records are heterogeneous or you only need a taste of the
data. Use `condense` when the records share a shape and the agent should see *all* of them, with
fewer fields each. `condense` falls back to sampling when it cannot beat it, so it is safe to leave on.
The [algorithm guide](ALGORITHM.md#condensing-arrays-of-records-array_strategy-condense) explains the scoring.

### Protect patterns

A pattern is a `/`-separated list of keys. `*` matches any one key or array position.

| Pattern | Matches |
|---|---|
| `/pagination/next_cursor` | exactly that path (a leading `/` anchors it at the root) |
| `/results/*/id` | the `id` of every element of `results` |
| `ari` | any key named `ari`, at any depth (no leading `/` means "ends with") |
| `content/id` | any `id` directly inside a `content` object |

Prefer anchored patterns when you know the schema: a bare suffix such as `key` matches that name at
every depth and can change how the whole response is budgeted. A protected container is protected as
a whole. Identifiers, handles and cursors that are recognisable (by name or by looking machine-generated)
are kept exact automatically, so you only need `protect` for the ones shrinkfit cannot recognise.

## Scores from outside, and model scoring

`field_scores`, `field_score_weight` and `jev` need shrinkfit 0.3.0 or later.

| Option | TypeScript | Default | Meaning |
|---|---|---|---|
| `field_scores` | `fieldScores` | `{}` | Scores you supply: an object mapping a path pattern (same syntax as `protect`) to a number from 0 to 1. The first matching pattern wins. CLI: `--field-scores JSON\|FILE`. |
| `field_score_weight` | `fieldScoreWeight` | `0.5` | How strongly your `field_scores` pull against the built-in score: a named field scores `(1 - w) * built-in + w * external`. `0` ignores them, `1` uses them alone. (The model's scores have their own `jev.weight`.) |
| `jev` | `jev` | off | Score fields with the hosted decision model Jev and blend that in. `true`, or an object (see below). CLI: `--jev`. |

`jev` accepts `true` or an object with these keys (TypeScript uses camelCase):

| Key | Default | Meaning |
|---|---|---|
| `api_key` | `OPENROUTER_API_KEY` | OpenRouter API key. |
| `tool` | none | Name of the tool that produced the document (context for the model). |
| `examples` | `4` | How many labelled examples to send with the request: the most similar from the bundled bank. `0` sends none. |
| `weight` | `1` | How far the model's scores replace the built-in ones, 0 to 1. At `1` the fields it scores are ranked by the model alone. |
| `bank` | bundled | Your own labelled examples instead of the bundled bank, as an array. See [the bank](model-scoring.md#labelled-examples-and-the-bank). |
| `cache` | `true` | Remember the scores per tool and field set, so the next response of the same shape needs no request. See [cost and speed](model-scoring.md#cost-and-speed). |
| `model` | `typesafe/jev-1.13` | The model asked. |
| `base_url` | `https://openrouter.ai/api` | The Decisions API base (a gateway or proxy can go here). |
| `timeout_ms` | `3000` | Give up after this long and use the built-in scoring. |
| `fetch` (TypeScript only) | global `fetch` | Your own `fetch`, for proxies or auth middleware. |

`field_scores` and `jev` only affect the `condense` strategy. Everything about Jev, including what is
sent, what it costs and what happens on failure, is in the [model scoring guide](model-scoring.md).

## Fine tuning

| Option | TypeScript | Default | Meaning |
|---|---|---|---|
| `string_head_chars` | `stringHeadChars` | `500` | Text shown whole before section previews begin. |
| `string_section_chars` | `stringSectionChars` | `100` | Preview length for each later section of a long string. |
| `string_min_section_chars` | `stringMinSectionChars` | `16` | The smallest preview before whole sections are dropped. |
| `keep_last_item` | `keepLastItem` | `true` | Prefer keeping an array's last item, so you can see where a list ends. |
| `max_samples` | `maxSamples` | `3` | When not even one array item fits whole, how many items of distinct shape to show in reduced form. |
| `min_partial_chars` | `minPartialChars` | `40` | The smallest budget worth spending on one extra, partly pruned item. |
| `explain` | n/a | `false` | Record every field's score and keep or drop decision in the report (`condense` only). For tuning and debugging. CLI: `--explain`. |

The two string limits scale together to use the budget fully; see
[strings](ALGORITHM.md#strings-structure-first).

## The report

Every call returns a report next to the text (Python: `result.report`, a dict; TypeScript:
`result.report`; Rust: `out.report`; command line: `--report` prints it as JSON on stderr).

| Field | Meaning |
|---|---|
| `input_chars`, `output_chars`, `max_chars` | Size of the input rendered in the output format, size of the result, and the budget. |
| `format`, `input_format` | The formats actually used (never `"auto"`). |
| `pruned` | Whether anything was removed at all. |
| `fits` | Whether the output is within `max_chars`. Only `false` when the budget is below the most compact form. |
| `items_omitted`, `arrays_sampled` | Array items dropped, and arrays that were sampled. |
| `keys_omitted`, `objects_trimmed` | Object keys dropped, and objects that lost keys. |
| `strings_truncated` | Strings shortened. |
| `strings_reshaped` | Of those, strings treated by what they hold: HTML or XML turned into text or an outline, JSON or rich text stored as text, logs and command output, diffs, base64 blobs. |
| `containers_collapsed` | Containers replaced by a single marker. |
| `arrays_condensed` | Arrays handled by `condense`. |
| `fields_dropped`, `dropped_fields` | Under `condense`: how many fields were dropped from all records, and which (`path` and the number of `items` affected). The data itself carries no marker for these, so surface them to the model. |
| `protected_omitted` | Protected values that were left out because their record, item or key was omitted. Check this, not just `fits`, when you use `protect`. |
| `model_scoring` | What happened when `jev` was requested: how many fields were scored, or why the built-in scoring was used instead. `null` when not requested or not needed. |
| `field_decisions` | With `explain`: every field's score and whether it was kept. |

`report.summary()` (Rust) renders the counts as one readable line, for example
`9,981 of 197,236 chars kept (180 array items omitted, 12 strings truncated)`.

## In Rust

```rust
use shrinkfit::{shrink_json, shrink_str, ArrayStrategy, InputFormat, Options};

let opts = Options {
    array_strategy: ArrayStrategy::Condense,
    protect: vec!["/results/*/id".into()],
    ..Options::new(4000)
};
let out = shrink_json(&text, &opts)?;           // JSON text in
// YAML text in (cargo feature `yaml`): set opts.input_format = InputFormat::Yaml; shrink_str(&text, &opts)
```

Other entry points: `shrink(&Value, &opts)` for a parsed document, `shrink_with(&Value, &opts, &scorer)`
to pass your own `FieldScorer` (see [extending](EXTENDING.md)), and `model_request(...)` for hosts that send
the model-scoring request themselves (see [model scoring](model-scoring.md#other-hosts)).
