//! Lexical matching between a tool call's arguments and a response's fields.
//!
//! Tool arguments say what the caller asked for: `status = 'In Progress'`,
//! `SELECT Amount, CloseDate`, a search term. Fields whose *names* or *values*
//! mention those words are the ones the caller is likely to care about. This
//! module scores that overlap, cheaply and without a model:
//!
//! * the query is only the argument **values** (argument names such as
//!   `query` or `jql` and the tool name add noise, not information), split
//!   into lowercase words with stop words and query-language keywords removed;
//! * a field matches by **name** (its name words appear in the query, as in a
//!   `SELECT` list) or by **value** (query words appear among its values,
//!   allowing plurals and a one-letter typo in longer words).
//!
//! The result is a relevance in `0.0..=1.0`, used by the heuristic scorer as
//! a boost.

use std::collections::HashSet;

/// Words that carry no information about what the caller wants.
const STOP: &[&str] = &[
    "a", "an", "the", "of", "in", "on", "at", "to", "for", "with", "and", "or", "not", "is", "are",
    "by", "from", "as", "be", "me", "my", "all", "any", "select", "where", "order", "asc", "desc",
    "limit", "offset", "like", "null", "true", "false", "between", "group", "having", "distinct",
    "count", "query", "search", "find", "get", "list", "show", "what", "which", "who", "when",
    "how", "that", "this", "it", "user", "http", "https", "www", "com",
];

/// Split text into lowercase words: letters/digits runs, splitting camelCase.
pub(crate) fn words(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut prev_lower = false;
    for ch in text.chars() {
        if ch.is_alphanumeric() {
            if ch.is_uppercase() && prev_lower && !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            cur.extend(ch.to_lowercase());
            prev_lower = ch.is_lowercase() || ch.is_numeric();
        } else {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            prev_lower = false;
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// Normalise a word for matching: strip a plural `s`.
pub(crate) fn norm(w: &str) -> String {
    if w.len() > 3 && w.ends_with('s') && !w.ends_with("ss") {
        w[..w.len() - 1].to_string()
    } else {
        w.to_string()
    }
}

fn informative(w: &str) -> bool {
    if w.chars().count() < 2 || STOP.contains(&w) {
        return false;
    }
    // Short bare numbers (day, month) match everything.
    !(w.chars().all(|c| c.is_ascii_digit()) && w.len() < 4)
}

/// One-letter typo, insertion or deletion.
fn within_one_edit(a: &str, b: &str) -> bool {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    if a.len().abs_diff(b.len()) > 1 {
        return false;
    }
    let (mut i, mut j, mut edits) = (0, 0, 0);
    while i < a.len() && j < b.len() {
        if a[i] == b[j] {
            i += 1;
            j += 1;
            continue;
        }
        edits += 1;
        if edits > 1 {
            return false;
        }
        match a.len().cmp(&b.len()) {
            std::cmp::Ordering::Greater => i += 1,
            std::cmp::Ordering::Less => j += 1,
            std::cmp::Ordering::Equal => {
                i += 1;
                j += 1;
            }
        }
    }
    edits + (a.len() - i) + (b.len() - j) <= 1
}

/// The informative words of a tool call's arguments.
pub(crate) struct Query {
    tokens: Vec<String>,
    set: HashSet<String>,
    /// Words that appear only in a sort clause (`ORDER BY updated DESC`): the
    /// caller orders by them, which does not mean they want them displayed.
    sort: HashSet<String>,
}

/// How much a field named only in a sort clause counts as "mentioned".
const SORT_WEIGHT: f64 = 0.35;

/// Split off the words of a trailing `ORDER BY ...` clause (up to `LIMIT`,
/// `OFFSET`, `GROUP` or `HAVING`).
fn split_sort_clause(text: &str) -> (String, String) {
    let lower = text.to_lowercase();
    let Some(at) = lower.find("order by") else {
        return (text.to_string(), String::new());
    };
    // `to_lowercase` can change byte offsets for exotic characters; stay on
    // char boundaries of the original text.
    if !text.is_char_boundary(at) || !text.is_char_boundary(at + 8) {
        return (text.to_string(), String::new());
    }
    let tail = &text[at + 8..];
    let tail_lower = &lower[at + 8..];
    let end = ["limit", "offset", "group by", "having"]
        .iter()
        .filter_map(|k| tail_lower.find(k))
        .min()
        .unwrap_or(tail.len());
    let end = if tail.is_char_boundary(end) {
        end
    } else {
        tail.len()
    };
    let mut main = text[..at].to_string();
    main.push(' ');
    main.push_str(&tail[end..]);
    (main, tail[..end].to_string())
}

impl Query {
    /// `None` if the arguments contain no informative words.
    pub(crate) fn new(text: &str) -> Option<Query> {
        let (main, sort_text) = split_sort_clause(text);
        let mut tokens = Vec::new();
        let mut set = HashSet::new();
        for w in words(&main) {
            if informative(&w) {
                let n = norm(&w);
                if set.insert(n.clone()) {
                    tokens.push(n);
                }
            }
        }
        let mut sort = HashSet::new();
        for w in words(&sort_text) {
            if informative(&w) {
                let n = norm(&w);
                if !set.contains(&n) {
                    sort.insert(n);
                }
            }
        }
        if tokens.is_empty() && sort.is_empty() {
            None
        } else {
            Some(Query { tokens, set, sort })
        }
    }

    /// By name, in `0..=1`: does the caller mention this field (`SELECT
    /// Amount, CloseDate`, `status.phase`)? Only the field's own name words
    /// (`leaf`) count; words of its ancestors can only confirm a partial
    /// match, so a query about `status.phase` does not credit `status.podIP`.
    pub(crate) fn name_match(&self, leaf: &[String], ancestors: &[String]) -> f64 {
        let leaf: Vec<String> = leaf.iter().map(|w| norm(w)).collect();
        // Words named in the query count in full; words only sorted by, weakly.
        let hits: f64 = leaf
            .iter()
            .map(|w| {
                if self.set.contains(w) {
                    1.0
                } else if self.sort.contains(w) {
                    SORT_WEIGHT
                } else {
                    0.0
                }
            })
            .sum();
        if leaf.is_empty() || hits == 0.0 {
            return 0.0;
        }
        let frac = hits / leaf.len() as f64;
        if frac >= 1.0 {
            1.0
        } else if ancestors.iter().any(|w| self.set.contains(&norm(w))) {
            0.5 + 0.5 * frac
        } else {
            0.5 * frac
        }
    }

    /// By value, in `0..=1`: how much of the query occurs among the field's
    /// (normalised) values.
    pub(crate) fn value_match(&self, values: &HashSet<String>) -> f64 {
        let hits = self.tokens.iter().filter(|t| token_hit(t, values)).count();
        if hits == 0 {
            0.0
        } else {
            0.5 + 0.5 * hits as f64 / self.tokens.len() as f64
        }
    }

    /// Does any query word occur among these values?
    pub(crate) fn mentions(&self, values: &HashSet<String>) -> bool {
        self.tokens.iter().any(|t| token_hit(t, values))
    }

    /// Strongest of the name and value matches (name words given whole).
    #[cfg(test)]
    pub(crate) fn relevance(&self, name_words: &[String], values: &HashSet<String>) -> f64 {
        self.name_match(name_words, &[])
            .max(self.value_match(values))
    }
}

fn token_hit(t: &str, values: &HashSet<String>) -> bool {
    values.contains(t)
        || (t.len() >= 6 && values.iter().any(|v| v.len() >= 5 && within_one_edit(v, t)))
}

/// Normalised informative words of a value, for matching against a query.
pub(crate) fn value_tokens(text: &str, out: &mut HashSet<String>, cap: usize) {
    for w in words(text) {
        if out.len() >= cap {
            return;
        }
        if informative(&w) {
            out.insert(norm(&w));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(ws: &[&str]) -> HashSet<String> {
        ws.iter().map(|w| w.to_string()).collect()
    }

    #[test]
    fn query_keeps_values_drops_keywords() {
        let q = Query::new("SELECT Id, Name, CloseDate FROM Opportunity WHERE Stage = 'Negotiation' ORDER BY x DESC").unwrap();
        assert!(
            q.tokens.contains(&"close".to_string())
                && q.tokens.contains(&"negotiation".to_string())
        );
        assert!(
            !q.tokens.contains(&"select".to_string()) && !q.tokens.contains(&"where".to_string())
        );
        assert!(Query::new("the and of 12").is_none());
    }

    #[test]
    fn matches_by_name_and_by_value() {
        let q = Query::new("amount close date negotiation").unwrap();
        let none = set(&[]);
        assert_eq!(q.relevance(&["amount".into()], &none), 1.0);
        assert_eq!(q.relevance(&["close".into(), "date".into()], &none), 1.0);
        assert!(q.relevance(&["owner".into()], &none) == 0.0);
        let v = set(&["negotiation", "won"]);
        assert!(q.relevance(&["stage".into()], &v) >= 0.5);
        // Plurals and one-letter typos in longer words.
        let q = Query::new("opportunities negociation").unwrap();
        assert!(q.relevance(&["x".into()], &set(&["negotiation"])) >= 0.5);
    }

    #[test]
    fn name_matches_only_on_the_fields_own_name() {
        let q = Query::new("status.phase=Failed namespace").unwrap();
        let w = |x: &[&str]| -> Vec<String> { x.iter().map(|s| s.to_string()).collect() };
        // The field the caller referred to.
        assert_eq!(q.name_match(&w(&["phase"]), &w(&["status"])), 1.0);
        // Siblings that merely share an ancestor word get nothing.
        assert_eq!(q.name_match(&w(&["pod", "ip"]), &w(&["status"])), 0.0);
        // A partly matching name is confirmed by a matching ancestor.
        assert!(q.name_match(&w(&["namespace", "uid"]), &w(&["status"])) < 1.0);
        assert!(
            q.name_match(&w(&["phase", "x"]), &w(&["status"]))
                > q.name_match(&w(&["phase", "x"]), &[])
        );
    }

    #[test]
    fn sorting_by_a_field_is_weak_evidence_that_it_is_wanted() {
        let q =
            Query::new("project = OPS AND status = open ORDER BY updated DESC LIMIT 50").unwrap();
        let w = |x: &[&str]| -> Vec<String> { x.iter().map(|s| s.to_string()).collect() };
        assert_eq!(q.name_match(&w(&["status"]), &[]), 1.0);
        let sorted = q.name_match(&w(&["updated"]), &[]);
        assert!(sorted > 0.0 && sorted < 0.5, "{sorted}");
        // Sort words are not value evidence.
        let vals: HashSet<String> = ["updated".to_string()].into_iter().collect();
        assert_eq!(q.value_match(&vals), 0.0);
        // Only a sort clause: still a usable query.
        assert!(Query::new("ORDER BY created").is_some());
    }

    #[test]
    fn edit_distance_one() {
        assert!(within_one_edit("negotiation", "negociation"));
        assert!(within_one_edit("abcdef", "abcdefg"));
        assert!(!within_one_edit("abcdef", "abxxef"));
    }
}
