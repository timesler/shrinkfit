//! Content-aware handling of long strings.
//!
//! A string in a tool response is rarely just prose. It may be an HTML or XML
//! fragment, a build log, a unified diff, a base64 blob, or a whole JSON
//! document (Atlassian's rich text, say) that was stored as text. Cutting
//! those as if they were prose (a head plus the start of every line) keeps the
//! least useful part: markup instead of words, the first 40 characters of a
//! thousand identical log lines instead of the one line that says `FATAL`.
//!
//! [`detect`] recognises the kind from the text alone, and [`fit`] shortens the
//! kinds that need no parsing of their own:
//!
//! * **markup** (HTML, XML): tags become text (HTML) or an outline (XML);
//! * **logs and command output**: ANSI codes and progress overwrites go, then
//!   lines are chosen by importance (errors, warnings, summaries, stack
//!   frames, the start and the end), with gaps marked;
//! * **diffs**: headers, hunk headers and changed lines are kept before context;
//! * **blobs**: a short prefix and the size.
//!
//! JSON text is handled by the pruner itself (it needs the pruner to fit the
//! parsed value). Every function here is pure and returns `None` when the
//! kind does not apply or nothing better than the generic cut can be made.
//! [`fit`] never returns text over `budget`.

pub(crate) mod adf;
mod blob;
mod lines;
mod markup;

use crate::text::TextParams;

/// Strings shorter than this are never inspected.
pub(crate) const MIN_LEN: usize = 120;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    /// Text that is a JSON object or array.
    Json,
    Html,
    Xml,
    Blob,
    Log,
    Diff,
    Plain,
}

/// The kind of `s`, judged from its text alone.
pub(crate) fn detect(s: &str) -> Kind {
    if s.len() < MIN_LEN {
        return Kind::Plain;
    }
    if blob::is_blob(s) {
        return Kind::Blob;
    }
    let t = s.trim();
    if (t.starts_with('{') && t.ends_with('}')) || (t.starts_with('[') && t.ends_with(']')) {
        return Kind::Json;
    }
    if let Some(xml) = markup::detect(s) {
        return if xml { Kind::Xml } else { Kind::Html };
    }
    if lines::is_diff(s) {
        return Kind::Diff;
    }
    if lines::is_log(s) {
        return Kind::Log;
    }
    Kind::Plain
}

/// Plain text for the kinds that are really text under markup (HTML) or a blob,
/// or `None`.
pub(crate) fn to_text(kind: Kind, s: &str) -> Option<String> {
    match kind {
        Kind::Html => Some(markup::html_to_text(s)),
        Kind::Blob => Some(blob::summary(s)),
        _ => None,
    }
}

/// Shorten `s` of the given kind to at most `budget` (as measured by `cost`),
/// or `None` if the generic cut should be used.
pub(crate) fn fit(
    kind: Kind,
    s: &str,
    budget: usize,
    params: &TextParams,
    cost: &dyn Fn(&str) -> usize,
) -> Option<String> {
    let out = match kind {
        Kind::Blob => blob::summary(s),
        Kind::Html => fit_plain(to_text(kind, s)?, budget, params, cost, 30),
        Kind::Log => lines::fit(s, lines::Mode::Log, budget, cost)?,
        Kind::Diff => lines::fit(s, lines::Mode::Diff, budget, cost)?,
        Kind::Json | Kind::Xml | Kind::Plain => return None,
    };
    (cost(&out) <= budget).then_some(out)
}

/// Fit plain `text` (already converted from some richer form, or prose).
pub(crate) fn fit_plain(
    text: String,
    budget: usize,
    params: &TextParams,
    cost: &dyn Fn(&str) -> usize,
    outline_from: usize,
) -> String {
    if cost(&text) <= budget {
        return text;
    }
    if let Some(out) = fit_prose(&text, budget, cost) {
        return out;
    }
    if text.lines().count() >= outline_from {
        if let Some(out) = lines::fit(&text, lines::Mode::Outline, budget, cost) {
            return out;
        }
    }
    crate::text::fit_text(&text, budget, params, cost)
}

/// Many paragraphs and little room: show the distinctive paragraphs instead of
/// a few characters of each. `None` when the text is not like that.
pub(crate) fn fit_prose(text: &str, budget: usize, cost: &dyn Fn(&str) -> usize) -> Option<String> {
    let out = lines::fit_paragraphs(text, budget, cost)?;
    (cost(&out) <= budget).then_some(out)
}

/// An XML document as a value, to be fitted like JSON.
pub(crate) fn xml_value(s: &str) -> Option<crate::value::Value> {
    markup::xml_to_value(s)
}
