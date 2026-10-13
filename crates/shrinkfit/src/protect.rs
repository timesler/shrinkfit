//! Protected paths: values that must come out exactly as they went in.
//!
//! An identifier, a cursor or a snapshot token is useless if shortened, so
//! such values are never truncated (and in `condense` their field is always
//! kept). A pattern is a `/`-separated list of keys, where `*` matches any one
//! key or array position:
//!
//! * `/pagination/next_cursor` is anchored: it names exactly that path;
//! * `ari` or `content/id` (no leading `/`) matches any path that *ends* with
//!   those keys, at any depth: every field called `ari`, every `id` inside a
//!   `content` object;
//! * `/results/*/id` protects the `id` of every element of `results`.
//!
//! A protected container is protected as a whole: nothing inside it is cut.
//!
//! If the protected values cannot fit the budget the result is reported with
//! `fits: false` (the smallest form that keeps them) instead of corrupting them.

/// A parsed protection pattern.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Pattern {
    anchored: bool,
    segs: Vec<String>,
}

impl Pattern {
    /// `None` for an empty pattern.
    pub(crate) fn parse(text: &str) -> Option<Pattern> {
        let segs: Vec<String> = text
            .split('/')
            .filter(|s| !s.is_empty())
            .map(String::from)
            .collect();
        if segs.is_empty() {
            None
        } else {
            Some(Pattern {
                anchored: text.starts_with('/'),
                segs,
            })
        }
    }

    /// Does this pattern name the node at `path` (keys, with `*` for array positions)?
    pub(crate) fn matches(&self, path: &[String]) -> bool {
        let n = self.segs.len();
        if path.len() < n || (self.anchored && path.len() != n) {
            return false;
        }
        let tail = &path[path.len() - n..];
        self.segs
            .iter()
            .zip(tail)
            .all(|(pat, key)| pat == "*" || pat == key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(p: &[&str]) -> Vec<String> {
        p.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn anchored_and_suffix_patterns() {
        let anchored = Pattern::parse("/pagination/next").unwrap();
        assert!(anchored.matches(&path(&["pagination", "next"])));
        assert!(!anchored.matches(&path(&["x", "pagination", "next"])));
        let suffix = Pattern::parse("ari").unwrap();
        assert!(suffix.matches(&path(&["videos", "*", "ari"])));
        assert!(suffix.matches(&path(&["ari"])));
        assert!(!suffix.matches(&path(&["videos", "*", "ari2"])));
        let two = Pattern::parse("content/id").unwrap();
        assert!(two.matches(&path(&["results", "*", "content", "id"])));
        assert!(!two.matches(&path(&["results", "*", "id"])));
    }

    #[test]
    fn wildcards_match_one_segment() {
        let p = Pattern::parse("/results/*/id").unwrap();
        assert!(p.matches(&path(&["results", "*", "id"])));
        assert!(!p.matches(&path(&["results", "*", "x", "id"])));
        assert!(Pattern::parse("").is_none());
        assert!(Pattern::parse("/").is_none());
    }
}
