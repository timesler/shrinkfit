# Long strings that are not prose

shrinkfit used to cut every long string the same way: the start of it, then the start of every line or paragraph. That is
a poor way to treat a build log (the error is in the middle), an HTML page (the words are buried in tags), a JSON
document or rich text stored as a string, or a base64 image. `bench.py` measures what survives now.

## What it measures

Sixteen synthetic cases, written to resemble real outputs, each a small JSON document whose long string carries a few
**facts** a reader needs: the error line and exception in a log, the changed files in a diff, a link in an HTML page, a value
deep in an XML list, a person mentioned in rich text. The score is the share of the facts still present after shrinking to
8%, 15% and 30% of the document's size (`array_strategy: condense`, compact output). `before` is the build just before this
change (v0.5.0 plus the uniformity fix); `after` is this change.

| kind | case | before 8/15/30% | after 8/15/30% |
|---|---|---|---|
| log | cargo build | 0 / 0 / 0 | 100 / 100 / 100 |
| log | pytest | 0 / 0 / 0 | 100 / 100 / 100 |
| log | app log with a traceback | 0 / 0 / 0 | 100 / 100 / 100 |
| shell | ANSI codes, progress bars, exit code | 0 / 0 / 0 | 100 / 100 / 100 |
| shell | npm error | 0 / 0 / 0 | 100 / 100 / 100 |
| diff | 14-file diff | 0 / 0 / 0 | 50 / 75 / 75 |
| html | article with links | 0 / 0 / 40 | 100 / 100 / 100 |
| html | product table | 33 / 33 / 33 | 33 / 33 / 100 |
| html | search snippet | 0 / 0 / 0 | 67 / 67 / 67 |
| xml | SOAP reply | 25 / 25 / 25 | 100 / 100 / 100 |
| xml | RSS feed | 33 / 33 / 33 | 67 / 67 / 67 |
| json text | compact | 25 / 25 / 25 | 75 / 75 / 75 |
| json text | pretty-printed | 25 / 25 / 25 | 75 / 75 / 100 |
| rich text | ADF object | 20 / 20 / 20 | 80 / 100 / 100 |
| rich text | ADF as JSON text | 20 / 20 / 20 | 80 / 100 / 100 |
| blob | inline images | 100 / 100 / 100 | 100 / 100 / 100 |
| **mean** | | **17.6 / 17.6 / 20.1** | **82.9 / 87.0 / 92.7** |

(The blob case is 100 for both here because `condense` already keeps the neighbours; with the default `sample` strategy
the old build kept 33% of them.)

## Reading it honestly

* The cases are written by the person who wrote the code, to resemble real outputs. They show that the facts survive,
  not how often such strings occur. In the 238 MCP-Atlas and 36 public-API responses used elsewhere, logs, diffs and
  traces are rare (see [path-selection](../path-selection/README.md)); the gain shows on responses that carry them.
* What is missing in the "after" column is mostly a needle in a long list at a tight budget (the one JSON record
  `payments-gateway` among 90, the odd row of a table): there is not room to show every record, and nothing says which one
  matters. A budget of 30% finds more of them.
* Field selection is unchanged: on the 273 labelled responses the labelled value kept is the same as before (the 0.004 and
  0.002 differences come from the earlier tie-break change, not from this one).

## Run it

```bash
cd experiments/strings
python bench.py before=PATH/TO/OLD/shrinkfit after=PATH/TO/NEW/shrinkfit
python peek.py "app log + traceback" 0.15      # one case: the shrunk output and the facts that are missing
```
