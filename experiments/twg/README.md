# TWG CLI evaluation: JEV-first shrinking

## Scope and verdict

Evaluated 2026-10-15 against baseline `e2b997f`, using `typesafe/jev-1.13` and the eight **publication-safe synthetic TWG fixtures** in `crates/shrinkfit-cli/web/twg`. No internal content was fetched or submitted in this evaluation.

**Recommendation:** use `condense`, JEV weight **0.7**, the real tool name, and a 6,000–12,000+ character budget. JEV remains the primary scorer. Keep the global default unchanged: this small, task-specific evaluation does not establish a universally best weight or prompt.

The candidate improves structural task-detail retention, particularly at 6K–12K budgets, and speeds up discovery on very wide responses. It is **not** a universal win: identity coverage falls slightly at weight 0.7, more noticeably at weight 1, and the team-directory cases regress. Tight 2K budgets still lose most useful detail.

## Method

- Eight response shapes: Jira search, Jira all-fields search, Jira batch details, Confluence search, teams, projects, goals, boards.
- Three variants: original fixtures; recursively reversed object-key order; an added envelope containing 40 synthetic diagnostic strings of roughly 1,350 characters each. Record order is not changed.
- Budgets: 2,000, 6,000, 12,000 characters plus each fixture's suggested demo budget, deduplicated.
- Four modes: sample, heuristic condense, pure JEV condense, and 70% JEV / 30% heuristic condense.
- **348 case/mode/budget observations per revision**, each replayed three times for local timing. Baseline, candidate, and a no-coverage-selection ablation were evaluated. Each successful output must parse as JSON and fit its budget. JEV runs explicitly reject fallback.
- Fetch a real model reply once per distinct complete request, cache it by SHA256 of the request, then replay it across budgets. This isolates compression behavior from repeated network calls. Before/after prompts and selected fields differ, so their model replies necessarily differ too.
- Task contracts are explicit in `evaluate.py`. Identity recall credits either valid ID or key. Usable-record recall requires an exact identity and exact readable title. Task-value recall gives equal weight to four useful fields per record, excludes absent source values, accepts equivalent status-label/value and title paths, and gives truncated prose credit only for the fraction of original prefix preserved. No credit for fabricated values or omission markers.
- All aggregate percentages below weight **case-budget observations equally**, not fixtures, individual fields across the entire corpus, or raw characters. Cases with four unique budgets contribute more than those with three. Variants are correlated, not independent samples.

The initial metric incorrectly penalized alternate IDs/titles/status representations. It was corrected, tested, and **both revisions rerun** with the same final metrics. Contract choices were informed by inspection, so the reported improvements are exploratory, not a blinded held-out evaluation.

## Results: all three variants

| Mode | Identity recall | Usable record recall | Task-value recall | Local replay mean |
|---|---:|---:|---:|---:|
| Sample | 13.1% | 10.4% | 9.2% | 2.8 ms |
| Heuristic condense | 51.9% | 51.4% | 20.4% | 4.0 ms |
| Baseline pure JEV | 58.7% | 45.7% | 20.6% | 7.5 ms |
| Candidate pure JEV | 48.4% | 47.6% | 27.1% | 4.6 ms |
| Baseline 70% JEV | 56.5% | 54.8% | 21.2% | 7.3 ms |
| **Candidate 70% JEV** | **55.3%** | **54.8%** | **29.3%** | **4.6 ms** |

At weight 0.7: task-value recall increases **8.1 percentage points** (~38% relative), identity recall falls **1.2 points**, and usable-record recall is approximately unchanged. Local replay mean falls ~37%; this is **not** an end-to-end network latency reduction.

### Budget sensitivity (candidate, all variants)

| Budget | Heuristic task value | Pure JEV task value | 70% JEV task value | 70% JEV usable records |
|---|---:|---:|---:|---:|
| 2,000 | 5.0% | 7.0% | 5.1% | 15.2% |
| 6,000 | 15.5% | 27.4% | 27.6% | 60.3% |
| 12,000 | 37.0% | 44.1% | 48.4% | 80.3% |

### Variant sensitivity (70% JEV task value)

| Variant | Baseline | Candidate |
|---|---:|---:|
| Original | 27.3% | 35.9% |
| Reversed keys | 23.1% | 37.2% |
| Metadata-heavy | 13.3% | 14.8% |

Metadata-heavy envelopes remain a major weakness: object water-filling still allocates budget to diagnostics instead of exclusively to actionable records. This is not solved by changing the scorer alone.

### Per-fixture task value (70% JEV, across variants/budgets)

| Fixture | Baseline | Candidate |
|---|---:|---:|
| Jira search | 14.3% | 19.9% |
| Jira all fields | 4.1% | 4.4% |
| Jira batch details | 45.8% | 60.4% |
| Confluence search | 7.1% | 7.2% |
| Teams | 19.9% | **17.6%** |
| Projects | 22.6% | 41.9% |
| Goals | 23.5% | 42.3% |
| Boards | 29.0% | 34.0% |

The positive aggregate is driven mostly by projects/goals/details, not every product. Jira all-fields and Confluence still need work.

## Changes retained

1. **JEV objective:** explicitly prefer primary identity, readable titles, useful content, status and responsible people. Avoid redundant identifiers, secondary links, avatars, transport metadata, and empty fields crowding out useful content. The old prompt strongly emphasized identifiers/URLs and often kept several links per record.
2. **Wide-schema coverage:** when more than 120 paths are discovered, prioritize populated and shallower paths before applying the existing cap. JEV still decides importance. Null/empty custom fields no longer automatically consume the request simply because they appeared first. Ordinary documents retain discovery order. The all-fields fixture has 3,081 distinct leaf paths; the cap still means most are not model-scored.
3. **Indexed discovery:** replace repeated linear searches for known field paths with a hash index while retaining deterministic output order and the first two distinct sample values. This removes quadratic path lookup in wide schemas. Median local replay for Jira all-fields at weight 0.7 fell from **37.6 ms to 17.8 ms**; smaller fixtures have much smaller gains.
4. Add a no-network JSON-line Rust adapter, repeatable Python evaluator, explicit metric tests, and wide-schema Rust regression tests.

### Rejected experiment and ablation

- **Rejected:** propagate each model leaf score to its parent containers using the maximum descendant score. With identical model replies it reduced early task-value recall, so it was removed. A high-scored descendant can cause an expensive parent to displace useful peer fields; max propagation is too crude.
- **Ablation:** revised objective + indexed discovery, but original first-120 selection: 70% JEV task-value recall **27.7%**. Adding populated/shallow selection raises it to **29.3%**, with identity recall unchanged (55.3%). The coverage change has a modest independent contribution.
- Global JEV weight, field cap, cache semantics, timeout, protected-field behavior, and non-JEV scoring are unchanged.

## Latency, cost and privacy

- Recorded model-request median: baseline ~388 ms; candidate ~402 ms. Maximum in this run: baseline 1,357 ms; candidate 628 ms. These are single-request observations per shape, **not stable latency percentiles or proof of faster networking**.
- Median serialized request size increases from ~52 KB to ~75 KB because the more detailed objective is repeated per decision question. Token billing was not measured. Improving prompt placement without harming quality is future work.
- Local replay timings include JSON-line IPC, Python serialization, parsing, and compression. The three-trial median is noisy and hardware-specific; it is not an isolated Rust microbenchmark.
- Only synthetic fixtures and synthetic variants are sent to JEV. API keys remain in the environment. Saved result files contain numeric metrics, hashes, fixture IDs and protocol names, not credentials.
- Real TWG deployment needs an explicit privacy policy: JEV transmits field paths, example values, tool name, and supplied arguments to a hosted service. Do not assume a synthetic-fixture evaluation authorizes sending real tenant data.

## Reproduce

```sh
cargo build --release -p shrinkfit --example evaluate
# Set OPENROUTER_API_KEY in your environment without writing it to a command/log.
python3 experiments/twg/evaluate.py \
  --variants original reversed metadata-heavy \
  --output target/twg-evaluation.json

# Same model decisions, no network calls:
python3 experiments/twg/evaluate.py --offline \
  --variants original reversed metadata-heavy \
  --output target/twg-replay.json

cargo test --workspace --locked
python3 -m unittest discover -s experiments/twg -p 'test_*.py'
node --test crates/shrinkfit-cli/web/*.test.cjs
cargo clippy --workspace --all-targets -- -D warnings
```

`--binary` accepts a separately built baseline adapter. Use identical fixtures, evaluator and build optimization settings. The baseline is `e2b997f`'s `jev.rs`; other core code was unchanged. The cache under `target/twg-model-cache` is local and ignored. Missing cache entries in offline mode fail instead of falling back. Do not commit credentials or generalize the cached synthetic replies to real inputs.

Detailed numeric observations are in `results/baseline.json`, `results/candidate.json`, and `results/no-coverage-selection.json`. They preserve the complete measured matrix; individual shrunk outputs are not committed.

## Suggested TWG integration

Use the **full JSON payload referenced by TWG's `output_files.stdout`**, not the YAML file-spill notice or compact projection:

```sh
shrinkfit saved-full-twg-output.json \
  --array-strategy condense --jev --jev-weight 0.7 \
  --tool-name jira.workitem.query --max-chars 12000 --report
```

Keep TWG's original file as the authoritative source and retain the compression report/omission notice. Do not silently replace a complete response with compressed data. If a cursor or identifier must survive exactly, configure `--protect` for that known response path, but measure the impact: protecting every ID can consume a tight budget. A single CLI invocation does not share the process-local JEV cache with subsequent invocations; an embedded long-running integration can.

## Remaining work / limits

- Need real task-answering labels, non-synthetic content under approved handling, and a broader held-out corpus before changing defaults or claiming production gains.
- Eight base fixtures and derived variants are small and correlated. Model outputs vary; no confidence interval, token-cost study, concurrency study or downstream agent success rate is claimed.
- Values are deliberately synthetic; status, description and identity statistics differ from real tenants. Original object/array shapes alone do not recreate real semantic utility.
- No TWG CLI integration was changed: this work optimizes shrinkfit and provides a tested integration recipe.
- Wide-field selection still depends on key order for equal-depth fields; deeply nested rich text can be underrepresented. Metadata-heavy envelopes and 2K budgets remain poor.
- The process-local cache intentionally ignores arguments and example values; do not assume query-specific model relevance is refreshed on a cache hit. Existing behavior is unchanged.
- Explore bounded schema-aware payload allocation, model-scored field groups, richer content previews, better duplicate-link handling, and preserving at least one actionable identity without forcing all identifiers. Validate on held-out cases before shipping those changes.
