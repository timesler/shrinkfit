//! The `condense` array strategy: keep every record, drop the same
//! low-value fields from all of them.
//!
//! Where the default `sample` strategy picks which *records* to keep,
//! `condense` picks which *fields*, once, for the whole array:
//!
//! 1. **Discover fields.** Walk all records and collect statistics for every
//!    field path (`/author/name`): how many records have it, what kinds of
//!    values, how big, how varied. Nested objects are walked into (to depth
//!    4); anything else is a leaf field.
//! 2. **Check the array is uniform** (most records share most of their
//!    fields). If not, return `None` and the caller falls back to `sample`.
//! 3. **Score** each leaf field with the configured
//!    [`FieldScorer`](crate::fields::FieldScorer).
//! 4. **Select** fields greedily by score. A field costs its *floor* in every
//!    record that has it: scalars in full, long strings as a short preview,
//!    other containers as a one-line marker. Fields are added while the
//!    array still fits; low-scoring fields may only use part of the budget,
//!    so better fields have room to show more of their values.
//! 5. **Project and fill.** Build the reduced records, then share the
//!    remaining budget among all records and values by water-filling
//!    (shortest first), exactly as for objects.
//!
//! Every record and every kept field appears in order; the same fields are
//! kept in each record. What was dropped is reported in
//! [`Stats::dropped_fields`](crate::prune::Stats), not inside the data.

use crate::fields::{ColumnInfo, KindCounts};
use crate::lexical::{self, Query};
use crate::prune::{Meta, Pruner};
use crate::text;
use crate::value::Value;
use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};

/// Deepest field path (in keys) that is walked into.
const MAX_PATH_DEPTH: usize = 4;
/// Preview lengths tried, in order, for the floor of a long string.
const FLOOR_LEVELS: [usize; 2] = [24, 12];
/// Only fields scoring at least this try to evict weaker ones, and only fields
/// scoring below this share of theirs.
const SWAP_MIN_SCORE: f64 = 0.6;
const SWAP_RATIO: f64 = 0.85;
/// Spare budget is spent on extra fields only if they score at least this:
/// room is no reason to add junk.
const MIN_FILL_SCORE: f64 = 0.3;
/// A plan must keep at least one scalar field scoring this or more.
const INFORMATIVE_SCORE: f64 = 0.45;
/// Fields scoring at least this are the "core" a plan should include.
const CORE_SCORE: f64 = 0.8;
/// Budget a field may use, by score: weaker fields leave more of the budget
/// to show fuller values of the stronger ones.
fn budget_share(score: f64) -> f64 {
    if score >= 0.75 {
        1.0
    } else if score >= 0.45 {
        0.9
    } else {
        0.7
    }
}
/// Most dropped fields listed in the report.
const MAX_DROPPED_REPORTED: usize = 24;
/// Distinct-value tracking stops growing at this many values per field.
const DISTINCT_CAP: usize = 1024;
/// Minimum share of field occurrences that must be "common" fields for an
/// array to count as uniform.
const UNIFORM_RATIO: f64 = 0.7;

#[derive(Default)]
struct Column {
    path: Vec<String>,
    parent: Option<usize>,
    present: usize,
    recursed: usize,
    kinds: KindCounts,
    empty: usize,
    key_sum: usize,
    full_sum: usize,
    floor_sum: usize,
    pos_sum: usize,
    scalars: usize,
    distinct: HashSet<u64>,
    first_hash: Option<u64>,
    all_same: bool,
    /// Some value was not a number or a digits-only string.
    non_numeric: bool,
    /// Some value was not a string starting with an ISO date.
    non_iso: bool,
    /// Strings that look machine-generated.
    opaque: usize,
    /// Normalised informative words of the field's values (only collected
    /// when the call's arguments were given).
    tokens: HashSet<String>,
    /// Records whose value mentions a query word.
    matched: usize,
    /// Order-independent fingerprint of (record, value) pairs: equal for two
    /// fields that hold the same value in every record.
    seq: u64,
}

impl Column {
    fn is_branch(&self) -> bool {
        self.present > 0 && self.recursed == self.present
    }
}

fn hash_scalar(v: &Value) -> Option<u64> {
    let mut h = DefaultHasher::new();
    match v {
        Value::Null => 0u8.hash(&mut h),
        Value::Bool(b) => (1u8, *b).hash(&mut h),
        Value::Number(n) => (2u8, n).hash(&mut h),
        Value::String(s) => (3u8, s).hash(&mut h),
        _ => return None,
    }
    Some(h.finish())
}

/// `2024-05-01` or `2024-05-01T10:00:00Z` and the like.
fn iso_date_prefix(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() >= 10
        && b[..4].iter().all(u8::is_ascii_digit)
        && b[4] == b'-'
        && b[5..7].iter().all(u8::is_ascii_digit)
        && b[7] == b'-'
        && b[8..10].iter().all(u8::is_ascii_digit)
}

/// Words naming a field for argument matching: its last segments, minus
/// envelope keys that say nothing about the field (`properties`, `fields`).
fn name_words(path: &[String]) -> (Vec<String>, Vec<String>) {
    const ENVELOPE: &[&str] = &[
        "properties",
        "fields",
        "attributes",
        "data",
        "nodes",
        "items",
        "results",
        "records",
        "values",
        "value",
    ];
    // Leaf names that say nothing without their parent (`Assignee/name`).
    const GENERIC: &[&str] = &[
        "name", "value", "id", "type", "key", "label", "text", "code",
    ];
    let segs: Vec<Vec<String>> = path
        .iter()
        .map(|k| {
            lexical::words(k)
                .into_iter()
                .filter(|w| !ENVELOPE.contains(&w.as_str()))
                .collect()
        })
        .collect();
    let mut at = segs.len();
    let mut leaf: Vec<String> = Vec::new();
    while at > 0 {
        at -= 1;
        let own = &segs[at];
        let generic = own.iter().all(|w| GENERIC.contains(&w.as_str()));
        if own.is_empty() {
            continue;
        }
        if !generic || leaf.is_empty() && at == 0 {
            leaf.extend(own.iter().cloned());
            break;
        }
        // Generic: defer to the parent's words as the field's name.
        if at == 0 {
            leaf.extend(own.iter().cloned());
        }
    }
    if leaf.is_empty() {
        leaf = lexical::words(path.last().map_or("", String::as_str));
    }
    let ancestors: Vec<String> = segs[..at.min(segs.len())]
        .iter()
        .flatten()
        .cloned()
        .collect();
    (leaf, ancestors)
}

/// `123`, `-4.5`, `0.25`: a number written as text.
fn numeric_text(s: &str) -> bool {
    let t = s.strip_prefix('-').unwrap_or(s);
    !t.is_empty()
        && t.bytes().any(|b| b.is_ascii_digit())
        && t.bytes().all(|b| b.is_ascii_digit() || b == b'.')
        && t.bytes().filter(|&b| b == b'.').count() <= 1
}

/// Machine-generated looking: hex/UUID-like, or long mixed-case alphanumerics.
fn looks_opaque(s: &str) -> bool {
    if s.len() < 8 || s.bytes().any(|b| b.is_ascii_whitespace()) {
        return false;
    }
    let hexish = s
        .bytes()
        .all(|b| b.is_ascii_hexdigit() || b == b'-' || b == b'_');
    let mixed = s.len() >= 12
        && s.bytes().any(|b| b.is_ascii_digit())
        && s.bytes().any(|b| b.is_ascii_uppercase())
        && s.bytes().any(|b| b.is_ascii_lowercase());
    (hexish && s.bytes().any(|b| b.is_ascii_digit())) || mixed
}

fn mix(i: usize, h: u64) -> u64 {
    let mut x = h ^ (i as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15);
    x ^= x >> 33;
    x = x.wrapping_mul(0xff51_afd7_ed55_8ccd);
    x ^ (x >> 29)
}

fn is_empty_value(v: &Value) -> bool {
    match v {
        Value::Null => true,
        Value::String(s) => s.is_empty(),
        Value::Array(a) => a.is_empty(),
        Value::Object(o) => o.is_empty(),
        _ => false,
    }
}

struct Discovery {
    cols: Vec<Column>,
    index: HashMap<Vec<String>, usize>,
    /// The tool call's argument words, if given.
    query: Option<Query>,
}

/// Value words kept per field for argument matching.
const TOKEN_CAP: usize = 256;

impl<'a> Pruner<'a> {
    /// Size of the smallest form of `v` that keeps all of its object keys:
    /// scalars in full, long strings as a short preview, arrays collapsed.
    pub(crate) fn floor(&self, v: &Value, meta: &Meta, depth: usize) -> usize {
        match v {
            Value::Object(entries) if !entries.is_empty() => {
                self.model.overhead(entries.len(), depth)
                    + entries
                        .iter()
                        .zip(&meta.kids)
                        .map(|((k, x), km)| self.model.key(k) + self.floor(x, km, depth + 1))
                        .sum::<usize>()
            }
            Value::String(s) => {
                if s.chars().count() <= self.floor_chars + 16 {
                    meta.full
                } else {
                    let head: String = s.chars().take(self.floor_chars).collect();
                    let n = s.chars().count();
                    let preview = format!("{head}{}", text::chars_marker(n - self.floor_chars));
                    self.model.string(&preview).min(meta.full)
                }
            }
            _ => meta.min,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn visit(
        &self,
        d: &mut Discovery,
        entries: &[(String, Value)],
        meta: &Meta,
        parent: Option<usize>,
        prefix: &[String],
        obj_depth: usize,
        item: usize,
    ) {
        for (pos, ((k, v), km)) in entries.iter().zip(&meta.kids).enumerate() {
            let mut path = prefix.to_vec();
            path.push(k.clone());
            let idx = match d.index.get(&path) {
                Some(&i) => i,
                None => {
                    d.cols.push(Column {
                        path: path.clone(),
                        parent,
                        all_same: true,
                        ..Default::default()
                    });
                    d.index.insert(path.clone(), d.cols.len() - 1);
                    d.cols.len() - 1
                }
            };
            let floor = self.floor(v, km, obj_depth + 1);
            let key_cost = self.model.key(k);
            let c = &mut d.cols[idx];
            c.present += 1;
            c.key_sum += key_cost;
            c.full_sum += km.full;
            c.floor_sum += floor;
            c.pos_sum += pos;
            match v {
                Value::Null => c.kinds.null += 1,
                Value::Bool(_) => c.kinds.bool_ += 1,
                Value::Number(_) => c.kinds.number += 1,
                Value::String(_) => c.kinds.string += 1,
                Value::Array(_) => c.kinds.array += 1,
                Value::Object(_) => c.kinds.object += 1,
            }
            if is_empty_value(v) {
                c.empty += 1;
            }
            if let Some(q) = d.query.as_ref() {
                let mut here = HashSet::new();
                match v {
                    Value::String(s) => lexical::value_tokens(s, &mut here, TOKEN_CAP),
                    Value::Number(n) => lexical::value_tokens(n, &mut here, TOKEN_CAP),
                    _ => {}
                }
                if q.mentions(&here) {
                    c.matched += 1;
                }
                if c.tokens.len() < TOKEN_CAP {
                    c.tokens.extend(here);
                }
            }
            let digits = match v {
                Value::Number(_) => true,
                Value::String(s) => numeric_text(s),
                _ => false,
            };
            c.non_numeric |= !digits;
            c.non_iso |= !matches!(v, Value::String(s) if iso_date_prefix(s));
            let opaque = match v {
                Value::String(s) => looks_opaque(s),
                // Very long integers are surrogate keys, not quantities.
                Value::Number(n) => n.len() >= 9 && n.bytes().all(|b| b.is_ascii_digit()),
                _ => false,
            };
            if opaque {
                c.opaque += 1;
            }
            if let Some(h) = hash_scalar(v) {
                c.scalars += 1;
                c.seq = c.seq.wrapping_add(mix(item, h));
                if c.distinct.len() < DISTINCT_CAP {
                    c.distinct.insert(h);
                }
                match c.first_hash {
                    None => c.first_hash = Some(h),
                    Some(f) if f != h => c.all_same = false,
                    _ => {}
                }
            }
            if let Value::Object(sub) = v {
                if !sub.is_empty() && path.len() < MAX_PATH_DEPTH {
                    d.cols[idx].recursed += 1;
                    self.visit(d, sub, km, Some(idx), &path, obj_depth + 1, item);
                }
            }
        }
    }

    fn discover(&self, items: &[Value], meta: &Meta, depth: usize) -> Discovery {
        let mut d = Discovery {
            cols: Vec::new(),
            index: HashMap::new(),
            query: self.query(),
        };
        for (n, (item, im)) in items.iter().zip(&meta.kids).enumerate() {
            if let Value::Object(entries) = item {
                self.visit(&mut d, entries, im, None, &[], depth + 1, n);
            }
        }
        d
    }

    /// Lower the score of fields that repeat what a better field already says:
    /// identical values in every record, or the same declared role.
    fn discount_redundant(cols: &[Column], roles: &[Option<String>], leaves: &mut [(usize, f64)]) {
        let by_idx: HashMap<usize, usize> = leaves
            .iter()
            .enumerate()
            .map(|(p, &(i, _))| (i, p))
            .collect();
        let better = |a: usize, b: usize, leaves: &[(usize, f64)]| -> bool {
            let (sa, sb) = (leaves[by_idx[&a]].1, leaves[by_idx[&b]].1);
            sa > sb || (sa == sb && (cols[a].path.len(), a) < (cols[b].path.len(), b))
        };
        // Identical values everywhere: keep the best copy.
        let mut twins: HashMap<(usize, u64), Vec<usize>> = HashMap::new();
        for &(i, _) in leaves.iter() {
            let c = &cols[i];
            if c.present >= 3 && c.scalars == c.present && !c.all_same {
                twins.entry((c.present, c.seq)).or_default().push(i);
            }
        }
        for group in twins.values().filter(|g| g.len() > 1) {
            let best =
                group
                    .iter()
                    .copied()
                    .fold(group[0], |m, x| if better(x, m, leaves) { x } else { m });
            for &i in group.iter().filter(|&&x| x != best) {
                leaves[by_idx[&i]].1 *= 0.15;
            }
        }
        // Same role: each further field is worth less.
        let mut by_role: HashMap<&str, Vec<usize>> = HashMap::new();
        for &(i, _) in leaves.iter() {
            if let Some(r) = roles[i].as_deref() {
                by_role.entry(r).or_default().push(i);
            }
        }
        for mut group in by_role.into_values() {
            group.sort_by(|&a, &b| {
                leaves[by_idx[&b]]
                    .1
                    .partial_cmp(&leaves[by_idx[&a]].1)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then(a.cmp(&b))
            });
            for (rank, i) in group.into_iter().enumerate() {
                leaves[by_idx[&i]].1 *= 0.75f64.powi(rank as i32);
            }
        }
    }

    /// The call's arguments as a matcher, unless planning without them.
    fn query(&self) -> Option<Query> {
        if self.ignore_query {
            None
        } else {
            self.opts.arguments.as_deref().and_then(Query::new)
        }
    }

    /// A column is usable if every ancestor is a fully walked branch.
    fn usable(cols: &[Column], mut i: usize) -> bool {
        while let Some(p) = cols[i].parent {
            if !cols[p].is_branch() {
                return false;
            }
            i = p;
        }
        true
    }

    /// Reduce a record to the selected leaf paths (and the branches leading to them).
    fn project(
        entries: &[(String, Value)],
        prefix: &mut Vec<String>,
        keep: &HashSet<Vec<String>>,
        leaves: &HashSet<Vec<String>>,
    ) -> Vec<(String, Value)> {
        let mut out = Vec::new();
        for (k, v) in entries {
            prefix.push(k.clone());
            if leaves.contains(prefix.as_slice()) {
                out.push((k.clone(), v.clone()));
            } else if keep.contains(prefix.as_slice()) {
                if let Value::Object(sub) = v {
                    let inner = Self::project(sub, prefix, keep, leaves);
                    out.push((k.clone(), Value::Object(inner)));
                }
            }
            prefix.pop();
        }
        out
    }

    /// For an object that must lose keys: pick the keys worth keeping by
    /// score (not simply the first ones), in their original order. Returns the
    /// kept indices and the fixed layout cost, or `None` if nothing fits.
    pub(crate) fn select_keys(
        &self,
        entries: &[(String, Value)],
        meta: &Meta,
        key_costs: &[usize],
        budget: usize,
        depth: usize,
    ) -> Option<(Vec<usize>, usize)> {
        let m = self.model;
        let n = entries.len();
        let marker_cost = |omitted: usize| {
            m.key(crate::prune::MARKER_KEY) + m.string(&crate::prune::keys_marker(omitted))
        };
        let query = self.query();
        let mut scored: Vec<(usize, f64)> = entries
            .iter()
            .enumerate()
            .map(|(i, (k, v))| {
                let path = [k.clone()];
                let mut kinds = KindCounts::default();
                let (mut scalars, mut text) = (0, None);
                match v {
                    Value::Null => (kinds.null, scalars) = (1, 1),
                    Value::Bool(_) => (kinds.bool_, scalars) = (1, 1),
                    Value::Number(_) => (kinds.number, scalars) = (1, 1),
                    Value::String(s) => (kinds.string, scalars, text) = (1, 1, Some(s.as_str())),
                    Value::Array(_) => kinds.array = 1,
                    Value::Object(_) => kinds.object = 1,
                }
                let mut toks = HashSet::new();
                match v {
                    Value::String(s) => lexical::value_tokens(s, &mut toks, TOKEN_CAP),
                    Value::Number(n) => lexical::value_tokens(n, &mut toks, TOKEN_CAP),
                    _ => {}
                }
                let (name_m, value_m, mentioned) = query.as_ref().map_or((0.0, 0.0, false), |q| {
                    (
                        q.name_match(&lexical::words(k), &[]),
                        q.value_match(&toks),
                        q.mentions(&toks),
                    )
                });
                let info = ColumnInfo {
                    path: &path,
                    total: 1,
                    present: 1,
                    kinds,
                    empty: usize::from(is_empty_value(v)),
                    distinct: scalars,
                    scalars,
                    constant: false,
                    numeric_like: matches!(v, Value::Number(_)) || text.is_some_and(numeric_text),
                    timestamp_like: text.is_some_and(iso_date_prefix),
                    opaque_like: text.is_some_and(looks_opaque),
                    relevance: name_m.max(value_m),
                    name_match: name_m,
                    value_match: value_m,
                    match_fraction: if mentioned { 1.0 } else { 0.0 },
                    avg_chars: meta.kids[i].full as f64,
                    avg_position: i as f64,
                };
                (i, self.scorer.score(&info))
            })
            .collect();
        scored.sort_by(|a, b| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.0.cmp(&b.0))
        });
        let mut chosen: Vec<usize> = Vec::new();
        let mut acc = 0usize;
        for &(i, score) in &scored {
            let add = key_costs[i] + meta.kids[i].min;
            let k = chosen.len() + 1;
            let cost = m.overhead(k + usize::from(k < n), depth)
                + acc
                + add
                + if k < n { marker_cost(n - k) } else { 0 };
            // Weaker keys leave room for the stronger ones' values (as in arrays).
            if cost <= (budget as f64 * budget_share(score)) as usize {
                acc += add;
                chosen.push(i);
            }
        }
        if chosen.is_empty() {
            return None;
        }
        chosen.sort_unstable();
        let k = chosen.len();
        let base = m.overhead(k + usize::from(k < n), depth)
            + acc
            + if k < n { marker_cost(n - k) } else { 0 };
        Some((chosen, base))
    }

    /// Fit `v` into `budget` keeping every object key: values shrink (long
    /// strings become previews, arrays sample) but nothing is dropped.
    /// Requires `budget >= floor(v)`.
    pub(crate) fn fit_keep(
        &mut self,
        v: &Value,
        meta: &Meta,
        budget: usize,
        depth: usize,
    ) -> (Value, usize) {
        if meta.full <= budget {
            return (v.clone(), meta.full);
        }
        match v {
            Value::Object(entries) if !entries.is_empty() => {
                let floors: Vec<usize> = entries
                    .iter()
                    .zip(&meta.kids)
                    .map(|((_, x), km)| self.floor(x, km, depth + 1))
                    .collect();
                let keys: usize = entries.iter().map(|(k, _)| self.model.key(k)).sum();
                let skeleton =
                    self.model.overhead(entries.len(), depth) + keys + floors.iter().sum::<usize>();
                let kids: Vec<(&Value, &Meta)> =
                    entries.iter().map(|(_, x)| x).zip(&meta.kids).collect();
                let labels: Vec<String> = entries.iter().map(|(k, _)| k.clone()).collect();
                let (vals, kids_size) = self.fill(
                    &kids,
                    &floors,
                    budget.saturating_sub(skeleton),
                    depth + 1,
                    &labels,
                );
                let size = self.model.overhead(entries.len(), depth) + keys + kids_size;
                let out = entries.iter().map(|(k, _)| k.clone()).zip(vals).collect();
                (Value::Object(out), size)
            }
            _ => self.fit(v, meta, budget, depth),
        }
    }

    /// Water-fill `surplus` over children that each have a floor size.
    /// Returns the new values and the sum of their sizes.
    pub(crate) fn fill(
        &mut self,
        kids: &[(&Value, &Meta)],
        floors: &[usize],
        surplus: usize,
        depth: usize,
        labels: &[String],
    ) -> (Vec<Value>, usize) {
        let mut order: Vec<usize> = (0..kids.len()).collect();
        order.sort_by_key(|&i| kids[i].1.full.saturating_sub(floors[i]));
        let mut out: Vec<Option<Value>> = vec![None; kids.len()];
        let mut remaining = surplus as i64;
        let mut left = kids.len() as i64;
        let mut total = 0usize;
        for i in order {
            let (v, m) = kids[i];
            let demand = m.full.saturating_sub(floors[i]) as i64;
            let alloc = demand.min(remaining.max(0) / left.max(1));
            self.path
                .push(labels.get(i).cloned().unwrap_or_else(|| "*".into()));
            let (nv, size) = self.fit_keep(v, m, floors[i] + alloc as usize, depth);
            self.path.pop();
            remaining += floors[i] as i64 - size as i64;
            left -= 1;
            total += size;
            out[i] = Some(nv);
        }
        (out.into_iter().map(|x| x.unwrap()).collect(), total)
    }

    fn path_string(&self, rel: &[String]) -> String {
        let mut s = String::new();
        for p in self.path.iter() {
            s.push('/');
            s.push_str(p);
        }
        s.push_str("/*");
        for p in rel {
            s.push('/');
            s.push_str(p);
        }
        s
    }

    /// Try to fit an array of similar records by dropping the same fields
    /// from every record. `None` means "not applicable or cannot fit": the
    /// caller then uses the sampling strategy.
    ///
    /// Fields are first costed with the usual string preview floor; if the
    /// high-scoring "core" fields do not all fit, shorter previews are tried
    /// before giving up on a core field.
    pub(crate) fn try_condense(
        &mut self,
        items: &[Value],
        meta: &Meta,
        budget: usize,
        depth: usize,
    ) -> Option<(Value, usize)> {
        let n = items.len();
        if n < 2 || !items.iter().all(|x| matches!(x, Value::Object(_))) {
            return None;
        }
        let previous = self.floor_chars;
        let mut chosen: Option<Plan> = None;
        // If the plan boosted by the call's arguments is rejected, plan again
        // without them: the arguments may refine a result, never lose it.
        for attempt in 0..2 {
            if attempt == 1 {
                if self.opts.arguments.is_none() {
                    break;
                }
                self.ignore_query = true;
            }
            for (level, &chars) in FLOOR_LEVELS.iter().enumerate() {
                self.floor_chars = chars;
                let plan = self.plan(items, meta, budget, depth, level == 0);
                self.floor_chars = previous;
                match plan {
                    Planned::NotUniform => {
                        self.ignore_query = false;
                        return None;
                    }
                    Planned::NoFit => {}
                    Planned::Fit(p) => {
                        let done = p.core_complete;
                        chosen = Some(*p);
                        if done {
                            break;
                        }
                    }
                }
            }
            self.ignore_query = false;
            if chosen.is_some() {
                break;
            }
        }
        let plan = chosen?;
        self.floor_chars = plan.floor_chars;
        let out = self.execute(plan, depth);
        self.floor_chars = previous;
        Some(out)
    }

    /// Discover, score and select fields for the current string-preview
    /// floor. Pure with respect to the pruner's statistics.
    fn plan(
        &self,
        items: &[Value],
        meta: &Meta,
        budget: usize,
        depth: usize,
        check_uniform: bool,
    ) -> Planned {
        let n = items.len();
        let d = self.discover(items, meta, depth);
        let cols = &d.cols;

        if check_uniform {
            // Uniformity: most field occurrences belong to fields most
            // records have.
            let (mut occ, mut common) = (0usize, 0usize);
            for c in cols.iter().filter(|c| c.parent.is_none()) {
                occ += c.present;
                if c.present * 2 >= n {
                    common += c.present;
                }
            }
            if occ == 0 || (common as f64) < UNIFORM_RATIO * occ as f64 {
                return Planned::NotUniform;
            }
        }

        // Score the leaf fields.
        let m = self.model;
        let per = |dep: usize| m.overhead(2, dep) - m.overhead(1, dep);
        let base = |dep: usize| m.overhead(1, dep).saturating_sub(per(dep));
        let mut leaves: Vec<(usize, f64)> = Vec::new();
        let mut roles: Vec<Option<String>> = vec![None; cols.len()];
        let mut rels: Vec<f64> = vec![0.0; cols.len()];
        for (i, c) in cols.iter().enumerate() {
            if c.is_branch() || !Self::usable(cols, i) {
                continue;
            }
            let (name_m, value_m) = d.query.as_ref().map_or((0.0, 0.0), |q| {
                let (leaf, anc) = name_words(&c.path);
                (q.name_match(&leaf, &anc), q.value_match(&c.tokens))
            });
            let info = ColumnInfo {
                path: &c.path,
                total: n,
                present: c.present,
                kinds: c.kinds,
                empty: c.empty,
                distinct: c.distinct.len(),
                scalars: c.scalars,
                constant: c.scalars == c.present && c.all_same,
                numeric_like: c.scalars == c.present && !c.non_numeric,
                timestamp_like: c.scalars == c.present && !c.non_iso,
                opaque_like: c.present > 0 && c.opaque * 5 >= c.present * 4,
                relevance: name_m.max(value_m),
                name_match: name_m,
                value_match: value_m,
                match_fraction: c.matched as f64 / c.present.max(1) as f64,
                avg_chars: c.full_sum as f64 / c.present.max(1) as f64,
                avg_position: c.pos_sum as f64 / c.present.max(1) as f64,
            };
            roles[i] = self.scorer.role(&info);
            rels[i] = info.relevance;
            leaves.push((i, self.scorer.score(&info)));
        }
        Self::discount_redundant(cols, &roles, &mut leaves);
        leaves.sort_by(|a, b| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(cols[a.0].floor_sum.cmp(&cols[b.0].floor_sum))
                .then(a.0.cmp(&b.0))
        });

        // Greedy selection under the budget (estimated, then verified).
        let item_depth = depth + 1;
        let mut est = m.overhead(n, depth) + n * base(item_depth);
        let mut activated: HashSet<usize> = HashSet::new();
        let mut selected: Vec<usize> = Vec::new();
        let mut chosen: HashSet<usize> = HashSet::new();
        // Cost of adding field `i` (with the branches leading to it, if new).
        let cost_of = |i: usize, activated: &HashSet<usize>| -> (usize, Vec<usize>) {
            let c = &cols[i];
            let mut delta = c.key_sum + c.present * per(depth + c.path.len()) + c.floor_sum;
            let mut chain = Vec::new();
            let mut p = c.parent;
            while let Some(b) = p {
                if !activated.contains(&b) {
                    let bc = &cols[b];
                    let lb = bc.path.len();
                    delta += bc.key_sum + bc.present * (per(depth + lb) + base(depth + lb + 1));
                    chain.push(b);
                }
                p = cols[b].parent;
            }
            (delta, chain)
        };
        // Pass 1: best fields first; weaker fields may use only part of the
        // budget so that better ones have room to show fuller values.
        for &(i, score) in &leaves {
            let (delta, chain) = cost_of(i, &activated);
            let limit = (budget as f64 * budget_share(score)) as usize;
            if est + delta <= limit {
                est += delta;
                activated.extend(chain);
                selected.push(i);
                chosen.insert(i);
            }
        }
        // Swap step: a strong field that did not fit may evict several weaker
        // ones if that raises the total value (cubed scores, so one good field
        // outweighs a handful of mediocre ones).
        let value = |sel: &[usize]| -> f64 {
            sel.iter()
                .map(|i| {
                    leaves
                        .iter()
                        .find(|l| l.0 == *i)
                        .map_or(0.0, |l| l.1.powi(3))
                })
                .sum()
        };
        let estimate = |sel: &[usize]| -> usize {
            let mut act: HashSet<usize> = HashSet::new();
            let mut total = m.overhead(n, depth) + n * base(item_depth);
            for &i in sel {
                let (delta, chain) = cost_of(i, &act);
                total += delta;
                act.extend(chain);
            }
            total
        };
        for &(i, score) in &leaves {
            if chosen.contains(&i) || score < SWAP_MIN_SCORE {
                continue;
            }
            let limit = (budget as f64 * budget_share(score)) as usize;
            let mut trial: Vec<usize> = selected.clone();
            let score_of = |x: usize| leaves.iter().find(|l| l.0 == x).map_or(0.0, |l| l.1);
            trial.push(i);
            // Evict the weakest first, but only fields clearly weaker than this one.
            let mut order: Vec<usize> = selected
                .iter()
                .copied()
                .filter(|&x| score_of(x) < score * SWAP_RATIO)
                .collect();
            order.sort_by(|&x, &y| {
                score_of(x)
                    .partial_cmp(&score_of(y))
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            let mut evicted = false;
            while estimate(&trial) > limit {
                match order.first().copied() {
                    Some(x) => {
                        order.remove(0);
                        trial.retain(|&t| t != x);
                        evicted = true;
                    }
                    None => break,
                }
            }
            if evicted && estimate(&trial) <= limit && value(&trial) > value(&selected) {
                selected = trial;
                chosen = selected.iter().copied().collect();
                activated.clear();
                est = m.overhead(n, depth) + n * base(item_depth);
                for &x in &selected {
                    let (delta, chain) = cost_of(x, &activated);
                    est += delta;
                    activated.extend(chain);
                }
            }
        }
        // Pass 2: that room is only worth keeping for values that can still
        // grow (long strings). Spend whatever exceeds their growth on more fields.
        let growth = |i: usize| cols[i].full_sum.saturating_sub(cols[i].floor_sum);
        let mut demand: usize = selected.iter().map(|&i| growth(i)).sum();
        for &(i, score) in &leaves {
            if chosen.contains(&i) || score < MIN_FILL_SCORE {
                continue;
            }
            let (delta, chain) = cost_of(i, &activated);
            if est + delta + demand + growth(i) <= budget {
                est += delta;
                demand += growth(i);
                activated.extend(chain);
                selected.push(i);
                chosen.insert(i);
            }
        }

        // Project, then verify the exact floor; drop the weakest field until it fits.
        loop {
            if selected.is_empty() {
                return Planned::NoFit;
            }
            let leaf_set: HashSet<Vec<String>> =
                selected.iter().map(|&i| cols[i].path.clone()).collect();
            let mut keep: HashSet<Vec<String>> = HashSet::new();
            for p in &leaf_set {
                for l in 1..=p.len() {
                    keep.insert(p[..l].to_vec());
                }
            }
            let mut reduced = Vec::with_capacity(n);
            let mut metas = Vec::with_capacity(n);
            let mut floors = Vec::with_capacity(n);
            let mut prefix = Vec::new();
            for item in items {
                let Value::Object(entries) = item else {
                    return Planned::NotUniform;
                };
                let v = Value::Object(Self::project(entries, &mut prefix, &keep, &leaf_set));
                let mt = self.analyze(&v, item_depth);
                floors.push(self.floor(&v, &mt, item_depth));
                reduced.push(v);
                metas.push(mt);
            }
            let total_floor = m.overhead(n, depth) + floors.iter().sum::<usize>();
            if total_floor > budget {
                selected.pop();
                continue;
            }
            let kept: HashSet<usize> = selected.iter().copied().collect();
            // A plan that keeps nothing a reader can use (only junk, or only
            // containers shown as markers) is worse than sampling real items.
            let score_of: HashMap<usize, f64> = leaves.iter().copied().collect();
            let informative = selected.iter().any(|i| {
                let c = &cols[*i];
                let scalar = c.kinds.string + c.kinds.number + c.kinds.bool_;
                // Categories alone (a few distinct values) tell a reader nothing
                // about individual records: need identifiers or text.
                let varied = c.distinct.len() * 2 >= c.present || c.full_sum >= 20 * c.present;
                // Ids alone say how many records there are, not what they are.
                let identity = roles[*i].as_deref() == Some("identity");
                score_of[i] >= INFORMATIVE_SCORE && scalar * 2 > c.present && varied && !identity
            });
            // And the single best field must fit: if even that is too big for
            // every record, the budget per record is too small for this strategy.
            let top = leaves.iter().map(|l| l.1).fold(0.0f64, f64::max);
            let top_kept = leaves
                .iter()
                .any(|(i, sc)| *sc >= top - 1e-9 && kept.contains(i));
            if !informative || !top_kept {
                return Planned::NoFit;
            }
            let core_complete = leaves
                .iter()
                .filter(|&&(_, s)| s >= CORE_SCORE)
                .all(|(i, _)| kept.contains(i));
            return Planned::Fit(Box::new(Plan {
                d,
                leaves,
                kept,
                keep,
                reduced,
                metas,
                floors,
                rels,
                budget_left: budget - total_floor,
                core_complete,
                floor_chars: self.floor_chars,
            }));
        }
    }

    /// Fill the planned records, share the leftover budget, and report.
    fn execute(&mut self, plan: Plan, depth: usize) -> (Value, usize) {
        let n = plan.reduced.len();
        let item_depth = depth + 1;
        let kids: Vec<(&Value, &Meta)> = plan.reduced.iter().zip(&plan.metas).collect();
        let labels = vec!["*".to_string(); n];
        let (vals, kids_size) =
            self.fill(&kids, &plan.floors, plan.budget_left, item_depth, &labels);
        let size = self.model.overhead(n, depth) + kids_size;

        // A group (e.g. `assignee`) whose fields were all dropped is
        // reported once, as the group, not field by field.
        let cols = &plan.d.cols;
        let mut seen: HashSet<Vec<String>> = HashSet::new();
        let mut dropped: Vec<(String, usize)> = Vec::new();
        for &(i, _) in plan.leaves.iter().filter(|(i, _)| !plan.kept.contains(i)) {
            let path = &cols[i].path;
            let l = (1..=path.len())
                .find(|&l| !plan.keep.contains(&path[..l]))
                .unwrap_or(path.len());
            let top = path[..l].to_vec();
            if seen.insert(top.clone()) {
                let present = plan
                    .d
                    .index
                    .get(&top)
                    .map_or(cols[i].present, |&c| cols[c].present);
                dropped.push((self.path_string(&top), present));
            }
        }
        dropped.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        if self.opts.explain {
            let mut decisions: Vec<(String, u32, bool, u32)> = plan
                .leaves
                .iter()
                .map(|&(i, score)| {
                    (
                        self.path_string(&cols[i].path),
                        (score * 1000.0).round() as u32,
                        plan.kept.contains(&i),
                        (plan.rels[i] * 1000.0).round() as u32,
                    )
                })
                .collect();
            decisions.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
            self.stats.field_decisions.extend(decisions);
        }
        self.stats.arrays_condensed += 1;
        self.stats.fields_dropped += dropped.len();
        for entry in dropped {
            if self.stats.dropped_fields.len() < MAX_DROPPED_REPORTED {
                self.stats.dropped_fields.push(entry);
            }
        }
        (Value::Array(vals), size)
    }
}

enum Planned {
    NotUniform,
    NoFit,
    Fit(Box<Plan>),
}

/// A chosen set of fields, ready to be filled with values.
struct Plan {
    d: Discovery,
    leaves: Vec<(usize, f64)>,
    kept: HashSet<usize>,
    keep: HashSet<Vec<String>>,
    reduced: Vec<Value>,
    metas: Vec<Meta>,
    floors: Vec<usize>,
    /// Relevance of each column to the call's arguments (for `explain`).
    rels: Vec<f64>,
    /// Budget left over for expanding values beyond their floors.
    budget_left: usize,
    core_complete: bool,
    floor_chars: usize,
}

#[cfg(test)]
mod tests {
    use super::name_words;

    fn path(p: &[&str]) -> Vec<String> {
        p.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn name_words_split_leaf_from_ancestors() {
        // Generic leaves defer to their parent: `Assignee/name` is "assignee".
        let (leaf, anc) = name_words(&path(&["fields", "Assignee", "name"]));
        assert_eq!((leaf, anc.is_empty()), (vec!["assignee".to_string()], true));
        // Ordinary leaves keep their own words; ancestors are separate.
        let (leaf, anc) = name_words(&path(&["status", "podIP"]));
        assert_eq!(leaf, ["pod", "ip"]);
        assert_eq!(anc, ["status"]);
        // Envelope keys carry no meaning.
        let (leaf, _) = name_words(&path(&["properties", "Amount"]));
        assert_eq!(leaf, ["amount"]);
    }
}
