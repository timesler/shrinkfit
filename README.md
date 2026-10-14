# shrinkfit

Shrink a JSON (or YAML) document to fit a character budget while keeping as
much of its schema and structure as possible.

Built for AI-agent tools that return large structured payloads. Instead of
failing, or spilling the result to a file, a tool can always answer inline
with a faithful, clearly marked sample of the data:

```text
$ shrinkfit --max-chars 900 api_response.yaml   # 25 KB in, 895 chars out
status: ok
total: 250
users:
  - id: 0
    name: User 0
    email: user0@example.com
    roles:
      - dev
      - admin
  - id: 1
    name: User 1
    email: user1@example.com
    roles:
      - dev
  - id: 2
    name: User 2
    email: user2@example.com
    roles:
      - dev
  - …[246 items omitted]
  - id: 249
    name: User 249
    email: user249@example.com
    roles:
      - dev
changelog: |-
  ## v1.12
  Fixed a number of bugs and improved performance across the board. Fixed a number of bugs…[+176 chars]

  ## v1.11
  Fixed a…[+257 chars]

  ## v1.10
  Fixed a…[+257 chars]

  … (one heading + preview per remaining release, down to v1.1)
```

The input was 250 user records plus a 12-release markdown changelog. Every field
survives. The first records are kept whole, and so is the last one, so you can see
where the list ends. Every changelog heading is still present, with a preview of its
section. (If some records had different keys, the first record of each distinct
shape would be preferred too.)

* **Hard budget.** The output is always at most `max_chars` characters, as
  long as the budget can hold the document's most compact form. JSON sizes
  are predicted exactly; for YAML the real output is measured and the budget
  corrected.
* **Always valid.** Output parses in the requested format. Cuts are marked
  in-band with `…` markers, so a reader never mistakes a sample for the whole:
  `"…[118 items omitted]"`, `"…": "12 keys omitted"`, `…[+2133 chars]`.
* **Schema first.** Every key of an object is kept while it can be, scalars
  survive intact, and array sampling prefers items with *new shapes*.
* **Structure-aware strings.** Long text keeps its outline (headings, lines,
  paragraphs) and shows a preview of every section rather than just a prefix.
* **Fast and dependency-free.** Rust core with no dependencies; a 41 MB
  payload shrinks in about 0.5 s. Thin bindings for Python and TypeScript (WASM).

## Usage

### Python

```python
from shrinkfit import shrink, shrink_with_report

shrink(big_dict)                                 # -> str: indented JSON, 10000-char budget
shrink(big_dict, max_chars=4000, indent=0)       # compact one-line JSON
shrink(big_dict, 4000, format="yaml")            # YAML output
shrink(json_or_yaml_text, 4000)                  # str/bytes: format auto-detected,
                                                 # output in the same format

r = shrink_with_report(big_dict, 4000, string_section_chars=60)
r.text, r.report["items_omitted"], r.report["fits"]
```

### TypeScript / JavaScript (Node, Deno, browsers)

```ts
import { init, shrink, shrinkWithReport } from "shrinkfit";

await init();                      // loads shrinkfit.wasm next to the module
shrink(bigObject);                 // 10000-char budget, indented JSON
shrink(bigObject, 4000);
shrink(bigObject, 4000, { format: "yaml", stringSectionChars: 60 });
```

### Rust

```rust
use shrinkfit::{shrink_json, Options};

let out = shrink_json(&big_json_text, &Options::new(4000))?; // or Options::default()
println!("{}", out.text);
// YAML text in (feature `yaml`): set `opts.input_format = InputFormat::Yaml`
// and call `shrink_str`.
assert!(out.report.fits);
```

### CLI

```sh
cargo install --path crates/shrinkfit-cli
shrinkfit [--max-chars 10000] [--format auto|json|yaml] [--input-format auto|json|yaml] \
          [--indent 2] [--report] [payload.json|-]
```

The input format is auto-detected (JSON first, then YAML) and the output uses
the same format unless `--format` says otherwise.

#### Telling the caller it was shrunk

stdout is always just the (valid) JSON/YAML. When something was cut, the CLI
also writes a one-line notice to **stderr** saying what was cut and where the
full document is:

```text
Output was truncated to fit 400 chars: 392 of 81,717 chars kept (298 array items
omitted, 2 object keys omitted). Truncated parts are marked in place with "…".
Full document: /tmp/full-outputs-alice/full-output-1791773177649-41772.json
```

The full document is the input file itself when you passed one; for piped input
a private (0600) temp copy is saved (named without the tool's name, so the agent never sees it) and old copies (over 24 h) are cleaned up.
Configure with `--no-notice` / `SHRINKFIT_NOTICE=0`, `--no-save-full` /
`SHRINKFIT_SAVE_FULL=0`, or `--save-full PATH`. Agents that only read stdout
can turn the notice off and rely on the in-place `…` markers.

### Playground

```sh
shrinkfit serve [--port 8765]    # then open http://127.0.0.1:8765/
```

A dependency-free local web page for exploring the behaviour while
developing: paste JSON or YAML, change any option, and see the output and
report update live. It talks to the same Rust code through the same options
object as the bindings. It binds to loopback only; it is a dev tool, not a
production server.

## How it works

In brief (see [docs/ALGORITHM.md](docs/ALGORITHM.md) for the details):

1. **Size every subtree** once. Both its full size and its *minimum* size
   (the smallest form it could be reduced to) are recorded.
2. **Allocate the budget top-down.** An object keeps all its keys while its
   skeleton (every key, each value at its minimum) fits. Spare budget is
   shared among its values by **water-filling**: small values are kept whole,
   and the rest is divided equally among the larger ones, with any unused
   share flowing on to the others.
3. **Sample arrays by priority:** the first item, then the first item of each
   distinct shape, then the last item, then the rest in order. As many whole
   items as fit are kept, plus one reduced item if budget remains. When not
   even one item fits whole, a few shape-distinct items are kept in reduced
   form. Each gap is replaced by a `"…[N items omitted]"` marker.
4. **Truncate strings by structure.** Text is split at its coarsest natural
   boundary (blank lines, then lines, then sentences). Whole sections are
   shown up to a head of `string_head_chars`. After that, each section shows
   its first `string_section_chars` and a `…[+N chars]` marker. Both limits
   scale together to use the budget fully. Only under severe pressure are
   trailing sections dropped behind `…[+K sections, N chars]`.
5. **Verify.** The result is rendered and measured. If the format's size
   model was only an estimate (YAML), the internal budget is searched until
   the real output fits as tightly as possible.

## Options

| option (Python / Rust field) | TS | default | meaning |
|---|---|---|---|
| `max_chars` | 2nd arg | 10000 | output ceiling, in Unicode characters |
| `format` | `format` | `"auto"` | `"auto"` (same as the input; JSON for in-memory values), `"json"` or `"yaml"` output |
| `input_format` | `inputFormat` | `"auto"` | `"auto"` (JSON, else YAML), `"json"` or `"yaml"` for text input (Rust: YAML needs the `yaml` cargo feature) |
| `indent` | `indent` | 2 | JSON: 0 = compact single-line; YAML: at least 2 |
| `string_head_chars` | `stringHeadChars` | 500 | text shown whole before section previews begin |
| `string_section_chars` | `stringSectionChars` | 100 | preview length for each later section |
| `string_min_section_chars` | `stringMinSectionChars` | 16 | smallest preview before sections are dropped |
| `array_strategy` | `arrayStrategy` | `"sample"` | how oversized arrays shrink: `"sample"` keeps a few whole items; `"condense"` keeps every item and drops the same low-value fields from all of them (see [docs/ALGORITHM.md](docs/ALGORITHM.md)) |
| `arguments` | `arguments` | none | the tool call's arguments (a string, or an object/array whose *values* are used; names are ignored). `condense` boosts fields whose names or values mention them, e.g. the columns in a `SELECT` or the matched field of a search |
| `protect` | `protect` | `[]` | paths whose values are never altered: `/results/*/id`, `ari`, `content/id` (CLI: `--protect`, repeatable). Kept exactly, or the result reports `fits: false` |
| `field_scores` | `fieldScores` | `{}` | field scores from outside, for example from a model that has read the response: an object of path pattern (as for `protect`) to a score from 0 to 1. A field they name is scored `(1 - field_score_weight) * built-in + field_score_weight * external`; others keep the built-in score. Used by `condense` (array columns and object keys). CLI: `--field-scores JSON|FILE` |
| `field_score_weight` | `fieldScoreWeight` | 0.5 | how strongly `field_scores` pull against the built-in score (0 ignores them, 1 uses them alone) |
| `jev` | `jev` | off | score fields with a hosted decision model (Jev via OpenRouter) and blend it in, see [Model scoring](#model-scoring-optional). `true`, or `{api_key, tool, model, base_url, timeout_ms}` (CLI: `--jev`, `--tool-name`, `--jev-timeout`, `--jev-base-url`). TypeScript: use `shrinkAsync` |
| `field_scorer` | `fieldScorer` | `"heuristic"` | field ranking for `condense`: `"heuristic"`, `"compact"` or `"key_order"` |
| `keep_last_item` | `keepLastItem` | true | prefer keeping an array's last item |
| `max_samples` | `maxSamples` | 3 | shape-distinct samples when no array item fits whole |
| `min_partial_chars` | `minPartialChars` | 40 | smallest budget worth spending on an extra, reduced item |

### Model scoring (optional)

`condense` ranks fields with a built-in heuristic. A decision model that has read the response can do
better: on 238 labelled tool responses, blending in scores from Jev raised the share of labelled value
kept at a 25% budget from 0.72 to 0.77 (see [experiments/path-selection](experiments/path-selection)).
You never call the model yourself. Set `jev`, and when the document has to be cut shrinkfit builds the
request, gets the reply and blends it in:

```bash
export OPENROUTER_API_KEY=sk-or-...
shrinkfit -n 4000 --array-strategy condense --jev --tool-name search_issues \
  --arguments '{"jql": "project = OPS"}' response.json
```

```python
shrink(data, 4000, array_strategy="condense", jev={"tool": "search_issues"}, arguments={"jql": "project = OPS"})
```

```ts
const text = await shrinkAsync(data, 4000, { arrayStrategy: "condense", jev: { tool: "search_issues" }, arguments: { jql: "project = OPS" } });
```

* **Opt-in and best effort.** Nothing is sent unless `jev` is set, the document has to be cut, and the
  strategy is `condense`. The key comes from `jev.api_key` (`apiKey` in TypeScript) or `OPENROUTER_API_KEY`.
  If there is no key, no network, a timeout (default 3 s), an error status or an unreadable reply, the
  built-in scoring is used and `report.model_scoring` says why (the CLI prints a one-line note).
* **What leaves the machine:** the tool name, the arguments (first 400 characters), every field path
  and up to two example values (60 characters each) per path. Not the rest of the response.
* **Cost and speed:** about 1,000 to 4,000 input tokens (about $0.0001) and 0.2 to 0.4 s per call.
  Scores are blended with the built-in ones by `field_score_weight` (default 0.5).
* **Who sends the request.** The core does it, with a small built-in HTTPS client, in the CLI, the Python
  package and the Rust crate (with its `jev` cargo feature, which the CLI and the Python library turn on; it adds
  about 1.2 MB, and the default crate stays dependency-free). The one exception is WebAssembly, which has no
  sockets: in TypeScript the core builds the request and reads the reply, and `fetch` carries it in between.
  Use `shrinkAsync` / `shrinkWithReportAsync` (the synchronous `shrink` cannot wait for a network reply and
  reports `unavailable`); they use the global `fetch`, or `jev: { fetch }` if you pass one (a proxy, auth
  middleware, a runtime without a global `fetch`). Nothing else needs a transport.
* **Other hosts.** Anything with a C ABI that cannot or would rather not send the request itself can do the
  same two steps: call `shrinkfit_model_request` (Rust: `shrinkfit::model_request`) to get the request, send it,
  and pass the response body back as the `jev_reply` option (or the failure as `jev_error`).

The report includes `format` (the output format actually used), `input_chars`, `output_chars`, `fits`, `pruned`,
`strings_truncated`, `arrays_sampled`, `items_omitted`, `objects_trimmed`,
`keys_omitted` and `containers_collapsed`.

## Integrating with a tool-result adapter

* Prune the tool's result **document**, then build the MCP text block; shrinking
  the already-serialized envelope breaks the embedded JSON.
* Budget for escaping: a 10,000-character JSON body can exceed 10,000 characters
  once escaped into a text block, so leave headroom.
* Identifiers, URLs used as handles, cursors and snapshot tokens are kept exact
  automatically when they are recognisable; name any others with `protect`.
* Check `report.protected_omitted` (not just `fits`) when you use `protect`: it
  counts protected values that were left out with their records. Prefer anchored
  paths (`/data/issues/*/key`); a bare suffix like `key` matches at every depth.
* `condense` records what it dropped in the report (`dropped_fields`,
  `fields_dropped`), not in the data. Surface a short note to the model (for
  example `Report::summary()`), bounded in size, so field loss is not silent.

## Repository layout

```text
crates/shrinkfit       core library (Rust API), zero dependencies
crates/shrinkfit-cli   command-line tool
crates/shrinkfit-ffi   C ABI (cdylib/staticlib + include/shrinkfit.h), used by all bindings
bindings/python        ctypes wrapper + wheel build (no PyO3 needed)
bindings/typescript    WASM loader, no wasm-bindgen needed
```

Every binding calls one function, `shrinkfit_shrink(input_json, options_json)
-> envelope_json`. Options pass through as JSON, so adding an option never
requires binding changes.

## Status and roadmap

v0.1 is a first working cut. Planned next:

- **More input formats:** TOML and JSON Lines are natural next steps
  (YAML input is already native; see `input_format`).
- **Token budgets:** a pluggable cost function (e.g. a tokenizer) instead of
  characters.
- Optionally listing omitted key names in the object marker.
- Published wheels (manylinux/macOS/Windows via cibuildwheel), crate and npm
  package.

## License

Apache-2.0. See [LICENSE](LICENSE).
