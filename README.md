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
| `field_scorer` | `fieldScorer` | `"heuristic"` | field ranking for `condense`: `"heuristic"`, `"compact"` or `"key_order"` |
| `keep_last_item` | `keepLastItem` | true | prefer keeping an array's last item |
| `max_samples` | `maxSamples` | 3 | shape-distinct samples when no array item fits whole |
| `min_partial_chars` | `minPartialChars` | 40 | smallest budget worth spending on an extra, reduced item |

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
