//! Budget-driven pruning of a value tree.
//!
//! `fit(v, budget)` returns a reduced copy of `v` whose rendered size is at
//! most `budget` (whenever `budget` is at least the value's minimum
//! representation). The strategy, applied recursively:
//!
//! * **Anything that fits is kept verbatim.**
//! * **Objects keep every key** while their *skeleton* (all keys, each value
//!   at its minimum representation) fits. Spare budget is shared among the
//!   values by *water-filling*: small values are kept whole and the rest is
//!   split equally among the larger ones (max-min fairness). Only when even
//!   the skeleton is too big are trailing keys dropped behind a
//!   `"…": "N keys omitted"` entry.
//! * **Arrays keep whole items** in a priority order: the first item, then
//!   the first item of every *new shape* (distinct key set or type), then
//!   the last item, then the rest in order. Gaps get a
//!   `"…[N items omitted]"` marker in place. If budget remains, one more
//!   item is included in reduced form. If not even the first item fits
//!   whole, up to `max_samples` shape-distinct items are kept, all reduced,
//!   sharing the budget by water-filling.
//! * **Strings** are truncated structure-first (see [`crate::text`]).
//! * A container that cannot fit at all collapses to a one-element marker
//!   of the same type: `["…[N items omitted]"]` or `{"…": "N keys omitted"}`.

use crate::fields::FieldScorer;
use crate::options::{ArrayStrategy, Options};
use crate::protect::Pattern;
use crate::size::Model;
use crate::text;
use crate::value::Value;
use std::cell::RefCell;
use std::collections::hash_map::DefaultHasher;
use std::collections::{BTreeSet, HashSet};
use std::hash::{Hash, Hasher};

/// Key used for the "omitted keys" marker entry.
pub const MARKER_KEY: &str = "…";

pub fn items_marker(n: usize) -> String {
    format!("…[{n} items omitted]")
}

pub fn keys_marker(n: usize) -> String {
    format!("{n} keys omitted")
}

/// Counts of what was cut, reported back to the caller.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    pub strings_truncated: usize,
    pub arrays_sampled: usize,
    pub items_omitted: usize,
    pub objects_trimmed: usize,
    pub keys_omitted: usize,
    pub containers_collapsed: usize,
    /// Arrays handled by the `condense` strategy.
    pub arrays_condensed: usize,
    /// Distinct fields dropped from every record by `condense`.
    pub fields_dropped: usize,
    /// The first of those, as `(path, records that had it)`; `*` stands for
    /// any array index, e.g. `/results/*/avatar_url`.
    pub dropped_fields: Vec<(String, usize)>,
    /// With `Options::explain`: every scored field as `(path, score x 1000,
    /// kept, relevance x 1000)`, best first, for each condensed array.
    pub field_decisions: Vec<(String, u32, bool, u32)>,
    /// Protected values missing from the output because the record, item or key
    /// holding them was omitted. Check this, not just `fits`.
    pub protected_omitted: usize,
}

/// Precomputed sizes for a subtree, mirroring the value tree.
pub struct Meta {
    /// Rendered size of the untouched subtree.
    pub full: usize,
    /// Size of the smallest representation `fit` will ever produce.
    pub min: usize,
    /// Shape signature, used to pick diverse array samples.
    pub shape: u64,
    pub kids: Vec<Meta>,
    /// Protected: never altered (`min == full`).
    pub pinned: bool,
    /// Protected, or an object holding something protected that it must not
    /// drop: such entries survive key trimming and collapsing.
    pub must_keep: bool,
    /// Protected values (leaves) in this subtree; omitted ones are counted in
    /// [`Stats::protected_omitted`].
    pub pinned_leaves: usize,
}

pub struct Pruner<'a> {
    pub model: Model,
    pub opts: &'a Options,
    pub stats: Stats,
    /// Scores fields for the `condense` strategy.
    pub scorer: &'a dyn FieldScorer,
    /// Keys leading to the node being fitted (`*` for array elements).
    pub path: Vec<String>,
    /// Preview length used when costing long strings in `condense`.
    pub floor_chars: usize,
    /// Plan as if no arguments were given (a fallback, see `try_condense`).
    pub ignore_query: bool,
    /// Parsed `protect` patterns.
    pub(crate) protect: Vec<Pattern>,
    /// Absolute paths `condense` has marked exact for the array being planned.
    pub(crate) exact_extra: RefCell<std::collections::HashSet<Vec<String>>>,
}

fn shape_of(v: &Value) -> u64 {
    let mut h = DefaultHasher::new();
    v.kind().hash(&mut h);
    if let Value::Object(entries) = v {
        let mut keys: Vec<&str> = entries.iter().map(|(k, _)| k.as_str()).collect();
        keys.sort_unstable();
        keys.dedup();
        keys.hash(&mut h);
    }
    h.finish()
}

impl<'a> Pruner<'a> {
    pub fn new(model: Model, opts: &'a Options) -> Self {
        Self::with_scorer(model, opts, opts.field_scorer.get())
    }

    /// Like [`Pruner::new`] with a custom field scorer.
    pub fn with_scorer(model: Model, opts: &'a Options, scorer: &'a dyn FieldScorer) -> Self {
        Pruner {
            model,
            opts,
            stats: Stats::default(),
            scorer,
            path: Vec::new(),
            floor_chars: 24,
            ignore_query: false,
            protect: opts
                .protect
                .iter()
                .filter_map(|p| Pattern::parse(p))
                .collect(),
            exact_extra: RefCell::new(std::collections::HashSet::new()),
        }
    }

    fn collapsed_array_size(&self, n: usize, depth: usize) -> usize {
        self.model.overhead(1, depth) + self.model.string(&items_marker(n))
    }

    fn collapsed_object_size(&self, n: usize, depth: usize) -> usize {
        self.model.overhead(1, depth)
            + self.model.key(MARKER_KEY)
            + self.model.string(&keys_marker(n))
    }

    /// Compute full/min sizes and shapes for every node.
    pub fn analyze(&self, v: &Value, depth: usize) -> Meta {
        self.analyze_at(v, depth, &mut Vec::new(), false)
    }

    /// Is the node at this path protected (never altered)? Either named by the
    /// `protect` option, or marked exact by `condense` for the current array.
    pub(crate) fn is_pinned(&self, path: &[String]) -> bool {
        self.protect.iter().any(|p| p.matches(path)) || self.exact_extra.borrow().contains(path)
    }

    /// Size of the smallest form of an object that still keeps the entries
    /// at `kept` (each at its own minimum), behind an omission marker if
    /// others are dropped.
    fn pin_form_size(
        &self,
        entries: &[(String, Value)],
        kids: &[Meta],
        kept: &[usize],
        depth: usize,
    ) -> usize {
        let m = &self.model;
        let omitted = entries.len() - kept.len();
        m.overhead(kept.len() + usize::from(omitted > 0), depth)
            + kept
                .iter()
                .map(|&i| m.key(&entries[i].0) + kids[i].min)
                .sum::<usize>()
            + if omitted > 0 {
                m.key(MARKER_KEY) + m.string(&keys_marker(omitted))
            } else {
                0
            }
    }

    /// `analyze` for a node at `path` (keys, `*` for array positions);
    /// `forced` when an ancestor is protected, which protects everything below.
    pub(crate) fn analyze_at(
        &self,
        v: &Value,
        depth: usize,
        path: &mut Vec<String>,
        forced: bool,
    ) -> Meta {
        let m = &self.model;
        let pinned = forced || self.is_pinned(path);
        match v {
            Value::Array(items) => {
                let kids: Vec<Meta> = items
                    .iter()
                    .map(|x| {
                        path.push("*".to_string());
                        let k = self.analyze_at(x, depth + 1, path, pinned);
                        path.pop();
                        k
                    })
                    .collect();
                let full =
                    m.overhead(items.len(), depth) + kids.iter().map(|k| k.full).sum::<usize>();
                let min = if items.is_empty() || pinned {
                    full
                } else {
                    full.min(self.collapsed_array_size(items.len(), depth))
                };
                let pinned_leaves = kids.iter().map(|k| k.pinned_leaves).sum();
                Meta {
                    full,
                    min,
                    shape: shape_of(v),
                    kids,
                    pinned,
                    must_keep: pinned,
                    pinned_leaves,
                }
            }
            Value::Object(entries) => {
                let kids: Vec<Meta> = entries
                    .iter()
                    .map(|(k, x)| {
                        path.push(k.clone());
                        let km = self.analyze_at(x, depth + 1, path, pinned);
                        path.pop();
                        km
                    })
                    .collect();
                let full = m.overhead(entries.len(), depth)
                    + entries
                        .iter()
                        .zip(&kids)
                        .map(|((k, _), km)| m.key(k) + km.full)
                        .sum::<usize>();
                let must: Vec<usize> = (0..kids.len()).filter(|&i| kids[i].must_keep).collect();
                let min = if entries.is_empty() || pinned {
                    full
                } else if must.is_empty() {
                    full.min(self.collapsed_object_size(entries.len(), depth))
                } else {
                    full.min(self.pin_form_size(entries, &kids, &must, depth))
                };
                let pinned_leaves = kids.iter().map(|k| k.pinned_leaves).sum();
                Meta {
                    full,
                    min,
                    shape: shape_of(v),
                    kids,
                    pinned,
                    must_keep: pinned || !must.is_empty(),
                    pinned_leaves,
                }
            }
            Value::String(s) => {
                let full = m.string(s);
                let min = if pinned {
                    full
                } else {
                    full.min(m.string(&text::chars_marker(s.chars().count())))
                };
                Meta {
                    full,
                    min,
                    shape: shape_of(v),
                    kids: Vec::new(),
                    pinned,
                    must_keep: pinned,
                    pinned_leaves: usize::from(pinned),
                }
            }
            other => {
                let full = m.scalar(other);
                Meta {
                    full,
                    min: full,
                    shape: shape_of(v),
                    kids: Vec::new(),
                    pinned,
                    must_keep: pinned,
                    pinned_leaves: usize::from(pinned),
                }
            }
        }
    }

    /// The smallest representation of `v`, and its size.
    fn minimal(&mut self, v: &Value, meta: &Meta, depth: usize) -> (Value, usize) {
        if meta.min >= meta.full {
            return (v.clone(), meta.full);
        }
        match v {
            Value::Array(items) => {
                self.stats.containers_collapsed += 1;
                self.stats.items_omitted += items.len();
                self.stats.protected_omitted += meta.pinned_leaves;
                (
                    Value::Array(vec![Value::String(items_marker(items.len()))]),
                    self.collapsed_array_size(items.len(), depth),
                )
            }
            Value::Object(entries) => {
                self.stats.containers_collapsed += 1;
                let must: Vec<usize> = (0..entries.len())
                    .filter(|&i| meta.kids[i].must_keep)
                    .collect();
                if must.is_empty() {
                    self.stats.keys_omitted += entries.len();
                    self.stats.protected_omitted += meta.pinned_leaves;
                    return (
                        Value::Object(vec![(
                            MARKER_KEY.to_string(),
                            Value::String(keys_marker(entries.len())),
                        )]),
                        self.collapsed_object_size(entries.len(), depth),
                    );
                }
                // Keep what must survive (in its own minimal form), drop the rest.
                let m = self.model;
                let omitted = entries.len() - must.len();
                let mut out: Vec<(String, Value)> = Vec::with_capacity(must.len() + 1);
                let mut size = m.overhead(must.len() + usize::from(omitted > 0), depth);
                for &i in &must {
                    let (k, x) = &entries[i];
                    let (nv, sz) = self.minimal(x, &meta.kids[i], depth + 1);
                    size += m.key(k) + sz;
                    out.push((k.clone(), nv));
                }
                if omitted > 0 {
                    self.stats.keys_omitted += omitted;
                    let kept_pins: usize = must.iter().map(|&i| meta.kids[i].pinned_leaves).sum();
                    self.stats.protected_omitted += meta.pinned_leaves.saturating_sub(kept_pins);
                    size += m.key(MARKER_KEY) + m.string(&keys_marker(omitted));
                    out.push((MARKER_KEY.to_string(), Value::String(keys_marker(omitted))));
                }
                (Value::Object(out), size)
            }
            Value::String(s) => {
                self.stats.strings_truncated += 1;
                (
                    Value::String(text::chars_marker(s.chars().count())),
                    meta.min,
                )
            }
            _ => (v.clone(), meta.full),
        }
    }

    /// Reduce `v` to at most `budget` rendered characters. Returns the new
    /// value and its exact size under the model.
    pub fn fit(&mut self, v: &Value, meta: &Meta, budget: usize, depth: usize) -> (Value, usize) {
        if meta.full <= budget {
            return (v.clone(), meta.full);
        }
        if budget <= meta.min {
            return self.minimal(v, meta, depth);
        }
        match v {
            Value::String(s) => {
                let model = self.model;
                let out = text::fit_text(s, budget, &self.opts.text, &|x: &str| model.string(x));
                self.stats.strings_truncated += 1;
                let size = model.string(&out);
                (Value::String(out), size)
            }
            Value::Array(items) => self.fit_array(v, items, meta, budget, depth),
            Value::Object(entries) => self.fit_object(v, entries, meta, budget, depth),
            _ => (v.clone(), meta.full),
        }
    }

    /// Fit a set of children that share `surplus` characters above their
    /// minimum sizes, using sequential water-filling: children are visited
    /// from least to most demanding; each gets an equal share of what is
    /// left, and any share it does not use flows on to the rest.
    fn water_fill(
        &mut self,
        kids: &[(&Value, &Meta)],
        surplus: usize,
        depth: usize,
        labels: &[String],
    ) -> (Vec<Value>, usize) {
        let mut order: Vec<usize> = (0..kids.len()).collect();
        order.sort_by_key(|&i| kids[i].1.full - kids[i].1.min);
        let mut out: Vec<Option<Value>> = vec![None; kids.len()];
        let mut remaining = surplus;
        let mut left = kids.len();
        let mut total = 0;
        for i in order {
            let (v, m) = kids[i];
            let extra = m.full - m.min;
            let alloc = extra.min(remaining / left.max(1));
            self.path
                .push(labels.get(i).cloned().unwrap_or_else(|| "*".into()));
            let (nv, size) = self.fit(v, m, m.min + alloc, depth);
            self.path.pop();
            remaining = remaining.saturating_sub(size.saturating_sub(m.min));
            left -= 1;
            total += size;
            out[i] = Some(nv);
        }
        (out.into_iter().map(|x| x.unwrap()).collect(), total)
    }

    fn fit_object(
        &mut self,
        whole: &Value,
        entries: &[(String, Value)],
        meta: &Meta,
        budget: usize,
        depth: usize,
    ) -> (Value, usize) {
        let m = self.model;
        let n = entries.len();
        let key_costs: Vec<usize> = entries.iter().map(|(k, _)| m.key(k)).collect();
        let skeleton: usize = m.overhead(n, depth)
            + key_costs
                .iter()
                .zip(&meta.kids)
                .map(|(k, km)| k + km.min)
                .sum::<usize>();

        // Which entries to keep, and the fixed cost of that layout
        // (overhead + keys + children at minimum + marker entry if any).
        let (idx, base): (Vec<usize>, usize) = if skeleton <= budget {
            ((0..n).collect(), skeleton)
        } else if meta.kids.iter().any(|k| k.must_keep)
            && self.opts.array_strategy != ArrayStrategy::Condense
        {
            // Protected entries first, then as many leading ones as fit.
            let marker_cost = |omitted: usize| m.key(MARKER_KEY) + m.string(&keys_marker(omitted));
            let mut order: Vec<usize> = (0..n).filter(|&i| meta.kids[i].must_keep).collect();
            let pins = order.len();
            order.extend((0..n).filter(|&i| !meta.kids[i].must_keep));
            let (mut chosen, mut acc) = (Vec::new(), 0usize);
            for (rank, &i) in order.iter().enumerate() {
                let add = key_costs[i] + meta.kids[i].min;
                let k = chosen.len() + 1;
                let cost = m.overhead(k + usize::from(k < n), depth)
                    + acc
                    + add
                    + if k < n { marker_cost(n - k) } else { 0 };
                if cost <= budget || rank < pins {
                    acc += add;
                    chosen.push(i);
                } else {
                    break;
                }
            }
            chosen.sort_unstable();
            let k = chosen.len();
            let base = m.overhead(k + usize::from(k < n), depth)
                + acc
                + if k < n { marker_cost(n - k) } else { 0 };
            (chosen, base)
        } else if self.opts.array_strategy == ArrayStrategy::Condense {
            // Keep the most valuable keys, not merely the first ones.
            match self.select_keys(entries, meta, &key_costs, budget, depth) {
                Some(r) => r,
                None => return self.minimal(whole, meta, depth),
            }
        } else {
            let marker_cost = |omitted: usize| m.key(MARKER_KEY) + m.string(&keys_marker(omitted));
            let mut acc = 0;
            let mut best: Option<(usize, usize)> = None;
            for (j, (kc, km)) in key_costs.iter().zip(&meta.kids).enumerate() {
                acc += kc + km.min;
                let c = m.overhead(j + 2, depth) + acc + marker_cost(n - j - 1);
                if c <= budget {
                    best = Some((j + 1, c));
                } else {
                    break;
                }
            }
            match best {
                Some((kept, c)) => ((0..kept).collect(), c),
                None => return self.minimal(whole, meta, depth),
            }
        };
        let kept = idx.len();

        let kids: Vec<(&Value, &Meta)> = idx
            .iter()
            .map(|&i| (&entries[i].1, &meta.kids[i]))
            .collect();
        let fixed = base - idx.iter().map(|&i| meta.kids[i].min).sum::<usize>();
        let labels: Vec<String> = idx.iter().map(|&i| entries[i].0.clone()).collect();
        let (values, kids_size) = self.water_fill(&kids, budget - base, depth + 1, &labels);
        let mut out: Vec<(String, Value)> = idx
            .iter()
            .map(|&i| entries[i].0.clone())
            .zip(values)
            .collect();
        if kept < n {
            self.stats.objects_trimmed += 1;
            self.stats.keys_omitted += n - kept;
            let kept_pins: usize = idx.iter().map(|&i| meta.kids[i].pinned_leaves).sum();
            self.stats.protected_omitted += meta.pinned_leaves.saturating_sub(kept_pins);
            out.push((MARKER_KEY.to_string(), Value::String(keys_marker(n - kept))));
        }
        (Value::Object(out), fixed + kids_size)
    }

    fn fit_array(
        &mut self,
        whole: &Value,
        items: &[Value],
        meta: &Meta,
        budget: usize,
        depth: usize,
    ) -> (Value, usize) {
        let n = items.len();
        if self.opts.array_strategy == ArrayStrategy::Condense {
            if let Some(done) = self.try_condense(items, meta, budget, depth) {
                return done;
            }
        }
        let order = self.priority(items, meta);

        // Phase A: whole items in priority order.
        let mut sel = Selection::new(n, self.model, depth);
        let mut next = None;
        for &i in &order {
            if sel.cost_if_add(i, meta.kids[i].full) <= budget {
                sel.add(i, meta.kids[i].full);
            } else {
                next = Some(i);
                break;
            }
        }

        let mut chosen: Vec<(usize, Option<Value>)>; // None = keep original whole
        let size;
        if !sel.is_empty() {
            chosen = sel.indices().map(|i| (i, None)).collect();
            let mut total = sel.cost();
            // One more item, reduced, if the leftover budget is worth it.
            if let Some(i) = next {
                let km = &meta.kids[i];
                let with_min = sel.cost_if_add(i, km.min);
                if with_min <= budget {
                    let item_budget = km.min + (budget - with_min);
                    if item_budget >= km.full.min(self.opts.min_partial_chars).max(km.min) {
                        let (v, s) = self.fit(&items[i], km, item_budget, depth + 1);
                        total = sel.cost_if_add(i, s);
                        chosen.push((i, Some(v)));
                        sel.add(i, s);
                    }
                }
            }
            size = total;
        } else {
            // Phase B: nothing fits whole. Take shape-distinct samples at
            // their minimum size, then share the surplus.
            let mut reps: Vec<usize> = Vec::new();
            let mut seen = HashSet::new();
            for &i in &order {
                if reps.len() >= self.opts.max_samples.max(1) {
                    break;
                }
                if seen.insert(meta.kids[i].shape) {
                    if sel.cost_if_add(i, meta.kids[i].min) <= budget {
                        sel.add(i, meta.kids[i].min);
                        reps.push(i);
                    } else {
                        break;
                    }
                }
            }
            if reps.is_empty() {
                return self.minimal(whole, meta, depth);
            }
            let base = sel.cost();
            let kids: Vec<(&Value, &Meta)> =
                reps.iter().map(|&i| (&items[i], &meta.kids[i])).collect();
            let labels = vec!["*".to_string(); reps.len()];
            let (values, kids_size) = self.water_fill(&kids, budget - base, depth + 1, &labels);
            let mins: usize = reps.iter().map(|&i| meta.kids[i].min).sum();
            size = base - mins + kids_size;
            chosen = reps.into_iter().zip(values.into_iter().map(Some)).collect();
        }

        chosen.sort_by_key(|(i, _)| *i);
        let mut out = Vec::with_capacity(chosen.len() * 2 + 1);
        let mut prev: Option<usize> = None;
        for (i, v) in chosen {
            let gap = match prev {
                None => i,
                Some(p) => i - p - 1,
            };
            if gap > 0 {
                out.push(Value::String(items_marker(gap)));
            }
            out.push(v.unwrap_or_else(|| items[i].clone()));
            prev = Some(i);
        }
        let tail = n - 1 - prev.unwrap();
        if tail > 0 {
            out.push(Value::String(items_marker(tail)));
        }
        self.stats.arrays_sampled += 1;
        self.stats.items_omitted += n - sel.len();
        let kept_pins: usize = sel.indices().map(|i| meta.kids[i].pinned_leaves).sum();
        self.stats.protected_omitted += meta.pinned_leaves.saturating_sub(kept_pins);
        (Value::Array(out), size)
    }

    /// Priority order for array items: first, new shapes, last, the rest.
    fn priority(&self, items: &[Value], meta: &Meta) -> Vec<usize> {
        let n = items.len();
        let mut order = Vec::with_capacity(n);
        let mut taken = vec![false; n];
        let push = |i: usize, order: &mut Vec<usize>, taken: &mut Vec<bool>| {
            if !taken[i] {
                taken[i] = true;
                order.push(i);
            }
        };
        push(0, &mut order, &mut taken);
        let mut seen = HashSet::new();
        seen.insert(meta.kids[0].shape);
        for i in 1..n {
            if seen.len() >= 64 {
                break;
            }
            if seen.insert(meta.kids[i].shape) {
                push(i, &mut order, &mut taken);
            }
        }
        if self.opts.keep_last_item && n > 1 {
            push(n - 1, &mut order, &mut taken);
        }
        for i in 1..n {
            push(i, &mut order, &mut taken);
        }
        order
    }
}

/// An incrementally maintained selection of array indices, with the exact
/// rendered cost of the resulting array (kept items + gap markers).
struct Selection {
    n: usize,
    model: Model,
    depth: usize,
    set: BTreeSet<usize>,
    sizes_sum: usize,
    /// Number of gap markers and their total size.
    gaps: usize,
    gaps_cost: usize,
}

impl Selection {
    fn new(n: usize, model: Model, depth: usize) -> Self {
        // Empty selection is modelled as one gap covering everything, so the
        // first insertion splits it like any other.
        let mut s = Selection {
            n,
            model,
            depth,
            set: BTreeSet::new(),
            sizes_sum: 0,
            gaps: 0,
            gaps_cost: 0,
        };
        s.gaps = 1;
        s.gaps_cost = s.gap_cost(n);
        s
    }

    fn gap_cost(&self, len: usize) -> usize {
        if len == 0 {
            0
        } else {
            self.model.string(&items_marker(len))
        }
    }

    fn neighbours(&self, i: usize) -> (isize, usize) {
        let prev = self
            .set
            .range(..i)
            .next_back()
            .map(|&p| p as isize)
            .unwrap_or(-1);
        let next = self.set.range(i + 1..).next().copied().unwrap_or(self.n);
        (prev, next)
    }

    /// (new gap count, new gaps cost) if `i` were added.
    fn split(&self, i: usize) -> (usize, usize) {
        let (p, s) = self.neighbours(i);
        let old = (s as isize - p - 1) as usize;
        let left = (i as isize - p - 1) as usize;
        let right = s - i - 1;
        let mut gaps = self.gaps - usize::from(old > 0);
        gaps += usize::from(left > 0) + usize::from(right > 0);
        let cost = self.gaps_cost - self.gap_cost(old) + self.gap_cost(left) + self.gap_cost(right);
        (gaps, cost)
    }

    fn cost_with(&self, count: usize, gaps: usize, gaps_cost: usize, sizes: usize) -> usize {
        self.model.overhead(count + gaps, self.depth) + gaps_cost + sizes
    }

    fn cost_if_add(&self, i: usize, size: usize) -> usize {
        let (g, gc) = self.split(i);
        self.cost_with(self.set.len() + 1, g, gc, self.sizes_sum + size)
    }

    fn add(&mut self, i: usize, size: usize) {
        let (g, gc) = self.split(i);
        self.gaps = g;
        self.gaps_cost = gc;
        self.sizes_sum += size;
        self.set.insert(i);
    }

    fn cost(&self) -> usize {
        self.cost_with(self.set.len(), self.gaps, self.gaps_cost, self.sizes_sum)
    }

    fn is_empty(&self) -> bool {
        self.set.is_empty()
    }

    fn len(&self) -> usize {
        self.set.len()
    }

    fn indices(&self) -> impl Iterator<Item = usize> + '_ {
        self.set.iter().copied()
    }
}
