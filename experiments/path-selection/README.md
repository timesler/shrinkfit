# Path selection experiment

Can a model decide which JSON paths of a large tool response are worth keeping, better than
shrinkfit's built-in heuristic? This directory holds the labelled data, the scoring and evaluation
scripts, and the findings that led to the `field_scores` option.

**Source data.** Tool outputs come from [ScaleAI/MCP-Atlas](https://huggingface.co/datasets/ScaleAI/MCP-Atlas)
(CC-BY-4.0). `data/lab_cases.json` redistributes a derived subset (paths, sample values, the first
1,500 characters of each response) under that licence. Labels are by Claude agents, not humans.

## Data

| file | content |
|---|---|
| `data/lab_cases.json` | 238 cases (175 distinct tool/response shapes, 19 servers): tool, arguments, response preview, every unique JSON path with sample values |
| `data/lab_labels.json` | final labels, `{case: {path index: mean level 0..2}}` (see `LABELLING.md`) |
| `data/labels_raw/` | each labeller's raw output (12 batches; anchor cases appear in two) |
| `data/lab_feats.json` | per-path value size and the heuristic's score with and without `--arguments` |
| `data/jev_scores/` | Jev scores per path: compact/full-response prompt, yes/no and choice forms |

## Findings

All numbers are from this data unless stated.

1. **General-purpose small LLMs cannot do this.** On a 55-case hand-labelled pilot, per-field yes/no
   scoring with SmolLM2 (135M to 1.7B), Qwen2.5/3 (0.5B to 3B), Flan-T5 and embedding/cross-encoder
   models gave AUC 0.44 to 0.70 against 0.72 for the heuristic. Listwise selection by Qwen2.5 up to 7B
   (4-bit) tied the heuristic at best. Fine-tuning MiniLM/bge-small/SmolLM2-135M on ~400 labelled
   examples dropped to the heuristic's level once whole servers were held out.
2. **Decision models do better** (vllm-sr d3-edge/lite/nano; TypeSafe Jev). They return a probability
   per option in one forward pass. On the pilot, d3-nano reached 0.78 AUC, Jev 0.86.
3. **On the 238 labelled cases** (AUC for essential fields, 95% CI over cases), like-for-like on the
   array fields the heuristic can score: heuristic 0.79, heuristic + arguments 0.87, Jev yes/no 0.88,
   Jev full-response format 0.87; a 50/50 z-score blend of heuristic + arguments and Jev reaches 0.94.
   Jev also scores the 69% of paths (outside record arrays) that the heuristic cannot.
4. **The full response in the prompt does not help.** Giving Jev the entire tool response with bare
   paths as options scored no better than tool call + paths + two sample values, at ~9,000 vs ~3,700
   input tokens per call.
5. **End to end** (`scripts/lab_e2e.py`: the real CLI, condense + arguments, `--field-scores` from
   Jev compact yes/no), share of labelled value kept / share of essential paths kept:

   | budget | baseline | weight 0.5 | weight 1.0 |
   |---|---|---|---|
   | 10% of response | 0.661 / 0.742 | 0.685 / 0.778 | 0.668 / 0.747 |
   | 25% of response | 0.720 / 0.766 | 0.772 / 0.830 | 0.756 / 0.819 |

   Re-measured with the released CLI calling the live service (474 requests, all answered), the 25% row is
   0.720 / 0.766 to 0.775 / 0.834 (+0.055 and +0.068) and the 10% row 0.661 / 0.742 to 0.688 / 0.780.
   The 25% gain (+0.052, CI +0.032 to +0.079) is clear of noise; the 10% gain (+0.025) is smaller.
   Precision is unchanged. Weights between 0.5 and 0.7 work best.

**Now built in.** shrinkfit fetches these scores itself (`jev` option, `--jev` in the CLI; in
TypeScript `shrinkAsync` passes the request to `fetch`); see "Model scoring" in the top-level README. `scripts/ml_jev3.py` is the
research version of the same call.

## Few-shot examples in Jev's input

*Question: does putting labelled examples in Jev's input improve its field choices, alone or blended with the built-in
scoring?* Not shipped; this is what the experiment found.

**Method.** The 19 servers are split into two balanced folds. Every response is scored with examples taken **only from
the other fold**, so the examples never share a server with the response they help judge: the numbers describe unseen
tools. An example is a labelled response (the tool call and its fields, each with a verdict of `essential`, `useful` or
`drop`, from the Claude labels) added to the state before the response being scored. Two ways of choosing them:
*fixed* (a set of 2, 4 or 8 responses, one per server where possible) and *retrieved* (the 2 or 4 most similar labelled
responses, by overlap of field names, one per tool). One run of zero-shot Jev is repeated to measure run-to-run noise.
About 1,900 live requests, roughly $0.50.

**Ranking quality improves a lot, standalone** (AUC for essential fields, record-array fields; 95% intervals are paired
differences against zero-shot):

| Examples | AUC | R-precision | AUC gain over zero-shot |
|---|---|---|---|
| none (zero-shot) | 0.879 | 0.635 | |
| none, repeated | 0.881 | 0.627 | +0.001 (noise: ±0.005) |
| 2 fixed | 0.936 | 0.761 | +0.057 [+0.046, +0.068] |
| 4 fixed | 0.940 | 0.771 | +0.061 [+0.050, +0.071] |
| 8 fixed | 0.945 | 0.774 | +0.065 [+0.053, +0.077] |
| 2 retrieved | 0.944 | 0.795 | +0.065 [+0.053, +0.077] |
| 4 retrieved | 0.944 | 0.783 | +0.065 [+0.055, +0.076] |

Over all paths (the whole JSON) the AUC rises from 0.903 to 0.944 to 0.952. Two examples capture most of the gain, and
which examples (fixed with another seed) barely matters. Blended 50/50 with the built-in score the ranking gain is
smaller but still real: AUC 0.942 to 0.957 to 0.964.

**But a better ranking did not give a better shrink at the default settings.** With the real CLI at the default blend
weight of 0.5, few-shot scores were *worse* than zero-shot at a 25% budget (about -0.03 labelled value kept, interval
excluding zero) and no better at 10%. The reason is calibration: the examples make Jev's scores sharper (the share of
scores in the uncertain 0.3 to 0.7 band falls from 65% to 49%), so the right blend weight moves.

**With the weight and scaling chosen honestly** (picked on one fold of servers, scored on the other; options were
the raw score or its rank within the response, at weights 0.25 to 1.0), the picture is:

* *Standalone* (Jev alone, scores replaced by their within-response rank): few-shot helps in every variant. Labelled
  value kept, averaged over the 10% and 25% budgets: zero-shot 0.729, 4 retrieved examples **0.755**
  (+0.027 [+0.016, +0.037]), 8 fixed 0.753, 2 retrieved 0.745.
* *Mixed* (blended with the built-in score): few-shot does **not** help (0.721 to 0.729 against 0.737 for zero-shot).
  The built-in score dilutes the sharper signal.

**Head to head against the released behaviour** (zero-shot, blended at 0.5), 237 responses, labelled value kept /
essential fields kept:

| | 25% budget | 10% budget |
|---|---|---|
| built-in scoring only | 0.720 / 0.766 | 0.661 / 0.742 |
| released: zero-shot, mixed | 0.769 / 0.824 | 0.686 / 0.781 |
| best zero-shot (mixed, rank, weight 0.75) | 0.772 / 0.835 | 0.701 / 0.798 |
| **4 retrieved examples, Jev alone, rank** | **0.798 / 0.858** | **0.712 / 0.798** |

Against the released behaviour that is +0.029 [+0.013, +0.046] at 25% and +0.026 [+0.009, +0.044] at 10% for labelled
value (essential fields: +0.033 and +0.016, the second inside the noise); against the built-in scoring, +0.078 and
+0.052.

**Cost.** Two to four examples add 1,700 to 3,400 input tokens (the request grows from about 3,700 to 5,300 or
7,100 tokens), about $0.0002 per request instead of $0.0001, and a small increase in latency.

**Caveats.** As before the labels come from Claude agents who saw the tool arguments, and the data is 19 servers. The
weight and scaling were chosen with a nested split, but the examples and the test responses come from one dataset, so
real-world transfer is not measured. A production bank could use all 238 labelled responses, not half.

**Reproduce.** `scripts/lab_fewshot.py <config>...` (live requests; configs `k0`, `fixed4`, `retr4`, ...) writes
`data/jev_scores/jev3_fs_*.json`, which are committed. `lab_fewshot_eval.py` (ranking and the end-to-end run at the
default weight), `lab_fewshot_diag.py` (score distributions, weight and scaling sweep), `lab_fewshot_cv.py` (nested
selection) and `lab_fewshot_final.py` (the table above) need no API key.

## Caveats

* Labels come from Claude agents who saw the tool arguments; a scorer that shares their taste is
  favoured, and so is anything that uses the arguments.
* The blend weight was chosen after trying a few values on the same data.
* Atlas covers few distinct response shapes and 63 of the 238 cases are one server (oxylabs).
* Jev is a hosted alpha endpoint (`/api/alpha/decisions` on OpenRouter): median ~240 ms per call,
  about $0.0001 per call at $0.042 per million input tokens. It sends the tool call, field paths and
  two sample values per path to a third party. Keep it opt-in, with the heuristic as the fallback.

## Reproduce

Run from this directory with `PYTHONPATH=scripts`.

```bash
python scripts/lab_agg.py            # labeller agreement, rebuilds data/lab_labels.json
python scripts/lab_eval.py           # ranking metrics for every scorer in data/jev_scores
python scripts/lab_blend.py          # heuristic + Jev blends
```

Rebuilding from scratch needs the Atlas parquet in `work/atlas.parquet`: `ml_pool.py` builds the
case pool, `ml_lab_prep2.py` the labelling batches, `ml_lab_feats.py` the heuristic features
(`SHRINKFIT_BIN` points at the CLI), `ml_jev3.py <compact|full> <noul|choice>` scores with Jev
(`OPENROUTER_API_KEY`), and `lab_e2e.py` runs the end-to-end comparison.
