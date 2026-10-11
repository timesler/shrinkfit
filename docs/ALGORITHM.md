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
