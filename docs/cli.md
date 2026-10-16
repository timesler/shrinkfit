# Command-line guide

The `shrinkfit` command reads a JSON or YAML document from a file or standard input and writes the
shrunk document to standard output. Standard output is **always just the document**, so it is safe to
pipe onward; everything else goes to standard error.

```bash
cargo install shrinkfit-cli        # installs the `shrinkfit` command
```

```text
shrinkfit [options] [FILE|-]
shrinkfit serve [--port N]         # the local playground, see below
```

* [Everyday use](#everyday-use)
* [Telling the caller it was shrunk](#telling-the-caller-it-was-shrunk)
* [All flags](#all-flags)
* [Environment variables](#environment-variables)
* [Exit status](#exit-status)
* [Model scoring from the command line](#model-scoring-from-the-command-line)
* [The playground](#the-playground)

## Everyday use

```bash
shrinkfit response.json                          # 10,000 characters, indented JSON
shrinkfit -n 4000 response.json                  # a 4,000-character budget
curl -s https://api.example.com/items | shrinkfit -n 4000 -i 0     # compact, from a pipe
shrinkfit -n 3000 --format yaml response.json    # JSON in, YAML out
shrinkfit -n 4000 --report response.json         # also print a JSON report to stderr
```

The input format is detected automatically (JSON first, then YAML), and the output uses the same
format unless `--format` says otherwise.

For lists of records, use the `condense` strategy and tell shrinkfit what the call asked for:

```bash
shrinkfit -n 4000 --array-strategy condense \
          --arguments '{"jql": "project = OPS AND status = Done"}' \
          --protect '/issues/*/key' response.json
```

## Telling the caller it was shrunk

When something was cut, the CLI writes a one-line **notice to stderr** saying what was cut and where
the full document is:

```text
Output was truncated to fit 400 chars: 392 of 81,717 chars kept (298 array items
omitted, 2 object keys omitted). Truncated parts are marked in place with "…".
Full document: /tmp/full-outputs-alice/full-output-1791773177649-41772.json
```

* If you passed a file, the full document is **that file**.
* For piped input, shrinkfit saves a private (mode 0600) temp copy and names it. The file name does not
  contain the tool's name, so an agent reading the notice never sees it. Copies older than 24 hours
  are cleaned up.
* Agents that read only standard output can switch the notice off and rely on the in-place `…` markers.

| To... | Use |
|---|---|
| silence the notice | `--no-notice` or `SHRINKFIT_NOTICE=0` |
| choose where the full copy goes | `--save-full PATH` |
| keep no copy of piped input | `--no-save-full` or `SHRINKFIT_SAVE_FULL=0` |

## All flags

| Flag | Meaning | See |
|---|---|---|
| `-n, --max-chars N` | Output budget in characters (default 10000). | [options](options.md#budget-and-formats) |
| `-f, --format auto\|json\|yaml` | Output format (default: same as the input). | |
| `-F, --input-format auto\|json\|yaml` | Input format (default: auto-detect). | |
| `-i, --indent N` | Indent width; `0` gives compact JSON (default 2). | |
| `--head N` | String characters shown whole before previews begin (default 500). | [fine tuning](options.md#fine-tuning) |
| `--section N` | Characters shown per later string section (default 100). | |
| `--no-keep-last` | Do not prefer an array's last item when sampling. | |
| `--array-strategy sample\|condense` | How big arrays shrink. | [choosing fields](options.md#choosing-which-fields-survive) |
| `--field-scorer NAME` | Built-in field ranking for `condense`: `heuristic`, `compact`, `key_order`. | |
| `--arguments TEXT\|JSON` | The tool call's arguments. Only their values are used. | |
| `--protect PATTERN` | Never alter values at this path. Repeatable. | [protect patterns](options.md#protect-patterns) |
| `--field-scores JSON\|FILE` | Scores from outside: an object of path pattern to 0..1, inline or a file. | [model scoring](model-scoring.md) |
| `--field-score-weight W` | Pull of those scores against the built-in score, 0..1 (default 0.5). | |
| `--jev` | Also score fields with the hosted decision model Jev. | |
| `--tool-name NAME` | The tool that produced the document (context for `--jev`). | |
| `--jev-timeout MS` | Give up on `--jev` after this long (default 3000). | |
| `--jev-base-url URL` | Decisions API base (default `https://openrouter.ai/api`). | |
| `--jev-examples N` | Labelled examples shown with each `--jev` request: the N most similar from the bundled bank (default 4; 0 for none). | [the bank](model-scoring.md#labelled-examples-and-the-bank) |
| `--jev-weight W` | How far `--jev` scores replace the built-in ones, 0..1 (default 1). | |
| `--jev-bank FILE` | Your own bank of labelled examples (JSON) instead of the bundled one. | |
| `--explain` | Print every field's score and keep or drop decision to stderr (`condense`). | |
| `--report` | Print the [report](options.md#the-report) as JSON to stderr. | |
| `--no-notice`, `--save-full PATH`, `--no-save-full` | The stderr notice, see above. | |
| `-h, --help` | Show help. | |

## Environment variables

| Variable | Effect |
|---|---|
| `SHRINKFIT_NOTICE=0` | Same as `--no-notice`. |
| `SHRINKFIT_SAVE_FULL=0` | Same as `--no-save-full`. |
| `OPENROUTER_API_KEY` | The API key `--jev` uses. The CLI reads the key only from here, never from a flag, so it cannot show up in a process listing. |

## Exit status

* `0`: the output fits the budget.
* `1`: the budget was smaller than the document's most compact form. The most compact form is still
  written to standard output, with `fits: false` in the report.
* `2`: invalid input or options; the error is on stderr and nothing is written to standard output.

## Model scoring from the command line

```bash
export OPENROUTER_API_KEY=sk-or-...
shrinkfit -n 4000 --array-strategy condense --jev --tool-name search_issues \
          --arguments '{"jql": "project = OPS"}' response.json
```

If the model cannot be reached, the output is the same as without `--jev` and a one-line note on stderr says
why (for example `note: model scoring unavailable: no OpenRouter API key ...`). What is sent, what it costs
and how it fails are in the [model scoring guide](model-scoring.md).

## The playground

```bash
shrinkfit serve [--port 8765]      # then open http://127.0.0.1:8765/
```

A dependency-free local web page for exploring how the options behave: paste JSON or YAML, change any
option, and watch the output and the report update live. It runs the same Rust code, through the same options
object, as the bindings. It listens on loopback by default and is a development tool, not a production server.

Under **Large TWG CLI outputs**, eight examples reproduce the shapes of oversized TWG responses (Jira,
Confluence, teams, projects, goals, and boards). All business content is replaced with synthetic values for
public demos. Each button displays its size and loads a suggested budget; the source command and provenance
appear above the editor. See the [fixture guide](../crates/shrinkfit-cli/web/twg/README.md) for a demo
walkthrough and validation commands.

The **Jev scoring** checkbox uses the server's `OPENROUTER_API_KEY` environment variable. It is enabled by
default with `condense`; disable it to compare built-in scorers without model calls. The API key is never
sent to the browser.
