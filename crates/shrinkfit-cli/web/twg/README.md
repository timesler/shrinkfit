# Large TWG playground examples

Eight publication-safe fixtures derived from **actual full JSON outputs** automatically written to temporary files by the TWG CLI. These are the saved `output_files.stdout` payloads, not TWG's compact summaries or the YAML notice that points to those files.

The fixtures preserve captured response envelopes, nested field structure, array lengths, nulls, repeated values and long-text pressure. **They are not original business records.** All names, prose, identifiers, dates, URLs, opaque tokens, numeric values (apart from zero and structural version values), and custom-field IDs were replaced with synthetic values. A small allowlist retains protocol literals such as ADF node types. Object property names and structural booleans are retained. Replacement values are deterministic; repeated original values have repeated synthetic counterparts. Counts and query metadata inside fixtures are synthetic, not authoritative or replayable.

Only the transformed fixtures are committed. Raw captures, local file paths, credentials, and the original tenant's details are not included. The example commands in `manifest.json` omit the original site and use demo issue keys. They document the command shape, not a way to retrieve these fictional records from a real tenant.

| Example | Original saved payload | Demonstrates |
| --- | ---: | --- |
| Jira search | 541,319 bytes | 50 issues, descriptions, repeated user/project metadata |
| Jira all fields | 1,424,791 bytes | 30 issues, custom fields, rich text, nested metadata |
| Jira batch details | 57,764 bytes | Three full detail envelopes and supplemental data |
| Confluence search | 99,301 bytes | 50 results, excerpts, duplicate titles and coverage |
| Teams | 163,392 bytes | 49 teams, nested member lists and avatar links |
| Projects | 107,355 bytes | 38 projects, descriptions, owners and goals |
| Goals | 60,844 bytes | 27 goals, parent and contributing-project metadata |
| Boards | 22,369 bytes | 50 scrum boards with mixed location shapes |

## Demo

```sh
cargo run --release -p shrinkfit-cli -- serve
```

Open http://127.0.0.1:8765 and select a button under **Large TWG CLI outputs**. Each button shows the transformed fixture's size. Selecting it loads JSON, selects `condense`, clears arguments from a previous example, and applies a suggested output budget. The command, provenance, and input/budget sizes appear above the editor. The selected JEV checkbox state is preserved.

The JEV checkbox uses `OPENROUTER_API_KEY` from the **server process environment**, never from browser code. It sends field summaries to the hosted model. Disable it to compare built-in scorers without network access. With JEV enabled, inspect `model_scoring` in the report to distinguish successful scoring from fallback.

Try **Jira: all fields** first, then compare `sample` with `condense` at 16,000 characters. Follow with teams or Confluence search to show different nesting patterns. These synthetic fixtures demonstrate compression behavior, not semantic benchmark scores.

## Assets and validation

Fixtures and their manifest are compiled into the binary and exposed through an explicit route allowlist. The browser fetches only the selected payload, so initial page load does not download all fixtures. No running TWG installation or original temporary files are required.

`cargo test -p shrinkfit-cli` checks manifest/asset agreement, exact byte/character counts, JSON validity, route allowlisting, and compression of every fixture under both strategies without model calls. `node --test crates/shrinkfit-cli/web/playground.test.cjs` checks the picker, failed fetches, and selection races.

When adding a fixture: transform source content before placing it here, audit property names as well as values, register its asset in `src/serve.rs`, and update the manifest's size, budget, command and provenance. Never commit raw internal outputs.
