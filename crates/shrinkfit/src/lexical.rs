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
}

impl Query {
    /// `None` if the arguments contain no informative words.
    pub(crate) fn new(text: &str) -> Option<Query> {
        let mut tokens = Vec::new();
        let mut set = HashSet::new();
        for w in words(text) {
            if informative(&w) {
                let n = norm(&w);
                if set.insert(n.clone()) {
                    tokens.push(n);
                }
            }
        }
        if tokens.is_empty() {
            None
        } else {
            Some(Query { tokens, set })
        }
    }

    /// Relevance in `0..=1` of a field with these name words and these
    /// (normalised) value words.
    pub(crate) fn relevance(&self, name_words: &[String], values: &HashSet<String>) -> f64 {
        // By name: the field is mentioned (`SELECT Amount, CloseDate`).
        let name: Vec<String> = name_words.iter().map(|w| norm(w)).collect();
        let name_hits = name.iter().filter(|w| self.set.contains(*w)).count();
        let name_rel = if name.is_empty() || name_hits == 0 {
            0.0
        } else if name_hits == name.len() {
            1.0
        } else {
            0.5 * name_hits as f64 / name.len() as f64
        };
        // By value: query words occur among the field's values.
        let hits = self
            .tokens
            .iter()
            .filter(|t| {
                values.contains(*t)
                    || (t.len() >= 6
                        && values.iter().any(|v| v.len() >= 5 && within_one_edit(v, t)))
            })
            .count();
        let value_rel = if hits == 0 {
            0.0
        } else {
            0.5 + 0.5 * hits as f64 / self.tokens.len() as f64
        };
        name_rel.max(value_rel)
    }
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
    fn edit_distance_one() {
        assert!(within_one_edit("negotiation", "negociation"));
        assert!(within_one_edit("abcdef", "abcdefg"));
        assert!(!within_one_edit("abcdef", "abxxef"));
    }
}
