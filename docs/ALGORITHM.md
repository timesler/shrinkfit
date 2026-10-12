# How shrinkfit works

## Problem

Given a document `D` and a budget `B` in characters, produce a document `D'`
such that:

1. `render(D')` is at most `B` characters and is valid in the output format;
2. `D'` is visibly a *sample* of `D`: every cut is marked in-band;
3. `D'` preserves as much of `D`'s **schema** as possible (keys, types,
   nesting, distinct record shapes), and as much **content** as the budget
   allows after that.

The goal ranks schema above content because an agent reading a truncated
payload mostly needs to know what fields exist and what values look like.
It can always make a narrower follow-up call for the specific data it needs.

## Value model

Every input format is parsed into one tree: null, bool, number (the original
literal text is kept, so there is no float round-tripping), string, array, and
object (entries in order). Output formats are renderers of the same tree.
Adding a format therefore means writing a parser and/or a renderer, plus
optionally a size model.

## Size model

The pruner needs a subtree's rendered cost before it renders anything.
`Model` is compositional:

```
size(scalar)            = rendered length (strings: quotes + escapes)
size(container, depth)  = overhead(n, depth) + Σ key costs + Σ size(child, depth+1)
```

with `overhead` capturing brackets, separators, newlines and indentation.
For compact and pretty JSON it is exact; this is checked by tests against the
emitter. For YAML the pretty-JSON model is used as an estimate (see
*Verification*).

A single bottom-up pass annotates each node with:

- `full`: size of the untouched subtree;
- `min`: size of the smallest form `fit` will produce. For a container this
  is a one-element marker of the same type, `["…[N items omitted]"]` or
  `{"…": "N keys omitted"}`. For a string it is `"…[+N chars]"`. Scalars
  are their own minimum.
- `shape`: a hash of the type, plus the sorted key set for objects.

## `fit(node, budget)`

The contract: if `budget >= min`, the result's size is at most `budget`. If
`budget >= full`, the node is returned unchanged.

### Objects: keep the skeleton, then water-fill

The **skeleton** is the object with every key kept and every value at its
`min`. If the skeleton fits, all keys are kept and the surplus
`budget - skeleton` is distributed among the values.

The distribution is **sequential water-filling**, a form of max-min
fairness:

```
sort children by demand (full - min), ascending
for each child:
    share = remaining / children_left
    give  = min(demand, share)
    fit child with min + give; subtract what it actually used
```

Small values (ids, flags, short strings) are satisfied completely. The rest
is split evenly among the large values, and anything a child does not use,
such as a truncated string ending a little early at a word boundary, flows
on to the children after it. No large field can starve its siblings, and
no budget is left on the table.

If even the skeleton does not fit, the longest prefix of entries whose
skeleton fits is kept, followed by a `"…": "N keys omitted"` entry. If not
even one entry fits, the object collapses to its `min` form.

### Arrays: whole items in priority order

Items are ranked:

1. the first item;
2. the first item of every **new shape** (e.g. records with an optional field
   that earlier records lack, or a different type), so every record layout in
   the array is represented;
3. the last item (`keep_last_item`), which shows where the sequence ends;
4. the remaining items in order.

**Phase A** adds whole items in this order while the array's exact cost,
including a `"…[N items omitted]"` marker in every gap, fits. The cost is
maintained incrementally: adding an index splits at most one gap into two,
so each step costs O(log n). If budget remains after the last whole item,
the next item in priority is added in reduced form, provided the leftover is
at least `min_partial_chars`.

**Phase B** handles the case where not even the first item fits whole. Up
to `max_samples` shape-distinct items are kept at their minimum, and the
surplus is water-filled among them. A large array of large records thus
becomes one or a few reduced records, which show the schema, plus counts.

Whole items are preferred over many reduced items. One complete record is
usually more informative to a reader than ten records with every field cut
short.

### Strings: structure first

Long text is split into **sections** at its coarsest natural boundary:
paragraphs (blank lines), else lines, else sentences. The separators are
kept verbatim. With parameters `head` and `section`:

```
whole sections while total <= head          (shown verbatim)
for each later section:
    first `section` chars (cut at a word boundary) + "…[+N chars]" + separator
```

The two parameters scale together, keeping their ratio (defaults 500:100). A
binary search finds the largest scale that fits, so a generous budget shows
long previews and a tight one short previews. This also means the result
uses the budget fully instead of stopping at fixed defaults. Below
`min_section` characters per preview, trailing sections are dropped behind
`…[+K sections, N chars]`. Text with no internal boundaries (an id, base64)
is cut at the head with `…[+N chars]`.

**Hierarchical previews.** A section that is itself multi-line (a paragraph
holding a log excerpt) is not reduced to one opaque prefix. Its preview is
the first line (up to `section` chars), then each following line shortened to
`section / 4` chars (minimum 8) while the preview stays within `2 * section`
chars, then a `…[+K lines, N chars]` marker. The preview is still monotone in
`section`, so the same binary search applies.

The string cost function is additive (`cost(a + b) = cost(a) + cost(b) - 2`),
so candidates are evaluated per piece without re-escaping whole strings.

## Verification and the outer loop

`shrink()` always renders the candidate and counts its characters. For JSON
the model is exact, so the first attempt with internal budget `B` is final.
For YAML, which is usually more compact than the pretty-JSON estimate (fewer
quotes, no braces) but occasionally larger (quoting, block indentation), the
internal budget is searched:

- if the output overshoots, scale the internal budget down by the observed
  ratio, then bisect;
- if it fits with more than 0.5% slack, scale it up;
- stop on convergence or after 24 attempts, and return the best output that
  fits.

The loop assumes output size grows with the internal budget, which holds in
practice but is not strictly guaranteed, so the best fitting attempt is
tracked rather than the last one.

If the budget is smaller than the document's minimum form, that minimum form
is returned with `fits: false`, so callers always get something to show.

## Complexity

Parsing and the size pass are O(n). `fit` visits each kept node once;
sorting children for water-filling adds O(k log k) per container. Array
selection is O(m log n) for m kept items. String fitting is
O(log L · output) per truncated string. A 41 MB payload shrinks in about
0.5 s, parsing included.

## Known limitations

- Shapes are compared by exact key set, so arrays whose records each have
  different optional keys produce many "new shapes". The cap of 64 shapes in
  the priority order, and `max_samples` in Phase B, bound the effect.
- Only paragraph-level previews are shown when the text has paragraphs.
  Lines inside a long paragraph are not previewed individually (planned:
  hierarchical previews).
- Budgets are in Unicode scalar values. Token budgets need a pluggable cost
  function (planned).
- Duplicate object keys are preserved as-is. The `"…"` marker key is not
  deduplicated against a real `"…"` key.
- Nesting deeper than 512 levels is rejected at parse time.

## Condensing arrays of records (`array_strategy: "condense"`)

The default array strategy samples whole items. For arrays of similar records
(search results, issue lists, table rows) it is often more useful to keep
**every** record and drop the same low-value **fields** from all of them.
`condense` does this (module `condense.rs`; scoring in `fields.rs`):

1. **Discover fields.** One pass collects, for every field path
   (`/fields/assignee/displayName`) across all records: presence, value
   kinds, empties, distinctness, average size, and its exact cost. Nested
   objects are walked to depth 4; other values are leaf fields.
2. **Uniformity check.** If most field occurrences do not belong to fields
   most records share, or any element is not an object, fall back to `sample`.
3. **Score** each leaf with a `FieldScorer`. The default heuristic combines a
   *name tier* (identifiers and titles high, links/avatars/expansion hints
   low, envelope keys like `fields` ignored) with *value statistics* (always
   empty or identical in every record: nearly worthless; short and all-distinct:
   identifier-like; low variety: category-like; opaque long ids and `*_id`
   references: lower).
4. **Discount redundancy.** Fields holding identical values in every record
   keep only their best copy (score x 0.15 for the rest). Fields sharing a
   role (identity, title, time) lose 25% per rank after the best one.
5. **Select greedily** by score. A field costs its *floor* in every record
   that has it (scalars in full, long strings as a short preview, arrays as a
   one-line marker). Weaker fields may use only 70-90% of the budget so the
   best fields have room to show fuller values. If the high-scoring "core"
   fields do not all fit, the string-preview floor is shortened (24, 12, 6
   characters) before giving up on one.
6. **Project and fill.** Build the reduced records, verify their exact floor,
   then share the remaining budget by water-filling over records and values.

The same fields are kept in every record. What was dropped is reported in
`Stats` (`fields_dropped`, `dropped_fields` as paths such as
`/issues/*/avatarUrls`, with the number of records that had each), not
inserted into the data, so typed arrays stay homogeneous.

Quality gates keep `condense` from doing worse than sampling: the plan must
include at least one informative scalar field, and the single best-scoring
field must fit (otherwise the budget per record is too small and the array is
sampled instead). Spare budget beyond what the chosen fields' values can still
grow into is spent on further fields, but only ones scoring at least 0.3.
`explain: true` (CLI `--explain`) lists every field's score and keep/drop
decision, which is how the heuristic is tuned.

Under `condense` an object that must lose keys also uses the scorer to decide
which keys survive (rather than keeping the first ones), and plans made only of
categories or identifiers fall back to sampling.

Known limitations: rich-text structures such as Atlassian Document Format
keep their text several levels deep, so a body field is expensive and is often
dropped; a plain-text projection of such fields is the main planned
improvement. Redundancy detection only catches identical values (not, say,
`name` versus `full_name`). The heuristic is untuned: it has been spot-checked
on Jira, Confluence, GitHub, Notion, Airtable, arXiv and other tool outputs,
not optimised against a dataset.
