# shrinkfit

Shrink a JSON (or YAML) document to fit a character budget while keeping as
much of its schema and structure as possible.

Built for AI-agent tools that return large structured payloads. Instead of
failing, or spilling the result to a file, a tool can always answer inline
with a faithful, clearly marked sample of the data:

```text
$ shrinkfit --max-chars 900 --format yaml api_response.json   # 25 KB in, 895 chars out
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

shrink(big_dict, max_chars=4000)                 # -> str (compact JSON)
shrink(big_dict, 4000, format="yaml")            # YAML output
shrink(json_text, 4000)                          # str/bytes are parsed as JSON
shrink(yaml_text, 4000, input_format="yaml")     # parsed natively

r = shrink_with_report(big_dict, 4000, string_section_chars=60)
r.text, r.report["items_omitted"], r.report["fits"]
```

### TypeScript / JavaScript (Node, Deno, browsers)

```ts
import { init, shrink, shrinkWithReport } from "shrinkfit";

await init();                      // loads shrinkfit.wasm next to the module
shrink(bigObject, 4000);
shrink(bigObject, 4000, { format: "yaml", stringSectionChars: 60 });
```

### Rust

```rust
use shrinkfit::{shrink_json, Options};

let out = shrink_json(&big_json_text, &Options::new(4000).yaml())?;
println!("{}", out.text);
// YAML text in (feature `yaml`): set `opts.input_format = InputFormat::Yaml`
// and call `shrink_str`.
assert!(out.report.fits);
```

### CLI

```sh
cargo install --path crates/shrinkfit-cli
shrinkfit --max-chars 4000 [--format yaml] [--input-format yaml] [--indent 2] [--report] payload.json
```

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
| `max_chars` | 2nd arg | required | output ceiling, in Unicode characters |
| `format` | `format` | `"json"` | `"json"` or `"yaml"` output |
| `input_format` | `inputFormat` | `"json"` | `"json"` or `"yaml"` for text input (Rust: needs the `yaml` cargo feature) |
| `indent` | `indent` | 0 (JSON), 2 (YAML) | JSON: 0 = compact |
| `string_head_chars` | `stringHeadChars` | 500 | text shown whole before section previews begin |
| `string_section_chars` | `stringSectionChars` | 100 | preview length for each later section |
| `string_min_section_chars` | `stringMinSectionChars` | 16 | smallest preview before sections are dropped |
| `keep_last_item` | `keepLastItem` | true | prefer keeping an array's last item |
| `max_samples` | `maxSamples` | 3 | shape-distinct samples when no array item fits whole |
| `min_partial_chars` | `minPartialChars` | 40 | smallest budget worth spending on an extra, reduced item |

The report includes `input_chars`, `output_chars`, `fits`, `pruned`,
`strings_truncated`, `arrays_sampled`, `items_omitted`, `objects_trimmed`,
`keys_omitted` and `containers_collapsed`.

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
- **Hierarchical string previews:** when a paragraph is cut, preview its
  lines rather than only its opening.
- **Token budgets:** a pluggable cost function (e.g. a tokenizer) instead of
  characters.
- Optionally listing omitted key names in the object marker.
- Published wheels (manylinux/macOS/Windows via cibuildwheel), crate and npm
  package.

## License

Apache-2.0. See [LICENSE](LICENSE).
