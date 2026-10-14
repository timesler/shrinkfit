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

   The 25% gain (+0.052, CI +0.032 to +0.079) is clear of noise; the 10% gain (+0.025) is smaller.
   Precision is unchanged. Weights between 0.5 and 0.7 work best.

**Now built in.** shrinkfit fetches these scores itself (`jev` option, `--jev` in the CLI,
`shrinkAsync` in TypeScript); see "Model scoring" in the top-level README. `scripts/ml_jev3.py` is the
research version of the same call.

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
