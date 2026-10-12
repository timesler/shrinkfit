# Extending shrinkfit

The core is organised so that new array strategies and field scorers can be
added without touching the rest of the pruner.

## Array strategies

`Options::array_strategy` (`array_strategy` in JSON options) picks how an
array that does not fit is reduced:

| strategy | file | idea |
|---|---|---|
| `sample` (default) | `prune.rs` (`fit_array`) | keep a few whole items, mark the gaps |
| `condense` | `condense.rs` | keep every item, drop the same low-value fields from all of them |

A strategy is a method on `Pruner` that takes the array's items, their
`Meta` (precomputed full/min sizes), the budget and the depth, and returns
`Some((value, exact_size))`, or `None` to let the caller fall back to
`sample`. To add one:

1. Add a variant to `ArrayStrategy` in `options.rs` and parse its name in
   `Options::from_value`.
2. Implement it in its own module (`impl Pruner { pub(crate) fn try_xyz(..) }`).
3. Dispatch to it at the top of `Pruner::fit_array` in `prune.rs`.
4. Keep the invariants checked by `tests/fit.rs`: valid output, within budget
   whenever the budget is at least the root's minimum size, rendered text equals
   the returned value. Extend `condense_output_always_parses_and_fits`.

Strategies report what they cut through `Stats`, never by inserting
strings into typed arrays (`dropped_fields`, `fields_dropped`, ...).

## Field scorers

`condense` asks a `FieldScorer` (`fields.rs`) how worth keeping each field is.
The scorer sees statistics about the field across all records (`ColumnInfo`:
path, presence, value kinds, empties, distinctness, average size, ...) and
returns a score in `0..=1`. It may also declare a `role` (such as `"identity"`)
so that redundant fields of the same role are discounted after the best one.

Built in: `heuristic` (default), and two baselines, `compact` (value per
character) and `key_order` (earlier keys win), useful as floors when comparing
scorers.

To add a built-in scorer implement `FieldScorer`, add a `ScorerKind` variant,
and register it in `ScorerKind::get`, `from_name` and `NAMES`. Rust callers can
also pass any scorer directly with `shrinkfit::shrink_with(&value, &opts,
&my_scorer)`.

## Guidelines

* Scorers are pure and cheap: they run once per field, not per value.
* Prefer signals that generalise (value statistics, structure) over per-API
  special cases; use name cues as a prior, not as the only evidence.
* Add a spot-check fixture for any new heuristic and describe what it fixed.
