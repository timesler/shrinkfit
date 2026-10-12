//! Structure-preserving truncation of long strings.
//!
//! A long string is split into *sections* at the coarsest natural boundary
//! it has: paragraphs (blank lines), then lines, then sentences. It is then
//! rendered as:
//!
//! 1. a **head** of whole sections, up to `head` characters, shown verbatim;
//! 2. a **preview** of every following section: its first `section`
//!    characters (cut at a word boundary) followed by `…[+N chars]`;
//! 3. if even minimal previews don't fit, the trailing sections are elided
//!    behind a single `…[+K sections, N chars]` marker.
//!
//! A section that is itself multi-line (say, a paragraph holding a log
//! excerpt) is previewed hierarchically instead of as one opaque prefix: its
//! first line, then a shortened preview of each following line while a small
//! allowance lasts, then a `…[+K lines, N chars]` marker. The outline of its
//! lines survives, not just its opening.
//!
//! **Query focus.** When the caller's arguments are known, a cut keeps what
//! they asked about: if the query's words occur beyond the opening of a string
//! (or of a section), the preview is the start, a gap marker, and a window
//! around the densest cluster of matches, instead of only the start.
//!
//! `head` and `section` scale together (keeping their ratio) so that the
//! result uses as much of the budget as possible: a generous budget shows
//! longer previews, a tight one shorter previews, and only a very tight one
//! starts dropping sections. Separators (newlines, blank lines) are kept
//! verbatim so the outline of the text survives.

use crate::lexical::{Hit, Query};

/// Tunables for string truncation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextParams {
    /// Characters of leading whole sections to show verbatim (at scale 1).
    pub head: usize,
    /// Characters shown from each later section (at scale 1).
    pub section: usize,
    /// Smallest per-section preview before sections start being elided.
    pub min_section: usize,
}

impl Default for TextParams {
    fn default() -> Self {
        TextParams {
            head: 500,
            section: 100,
            min_section: 16,
        }
    }
}

/// Marker for omitted characters at the end of a section or string.
pub fn chars_marker(n: usize) -> String {
    format!("…[+{n} chars]")
}

/// Marker for trailing sections that were dropped entirely.
pub fn sections_marker(k: usize, n: usize) -> String {
    format!("…[+{k} sections, {n} chars]")
}

/// Marker for trailing lines of a multi-line section that were dropped.
pub fn lines_marker(k: usize, n: usize) -> String {
    format!("…[+{k} lines, {n} chars]")
}

/// One section: the content and the separator that followed it.
#[derive(Debug, Clone)]
struct Section<'a> {
    content: &'a str,
    sep: &'a str,
    chars: usize,
    /// Query words in `content` (empty without a query).
    hits: Vec<Hit>,
}

fn nchars(s: &str) -> usize {
    s.chars().count()
}

/// Split into paragraphs: runs of non-blank lines separated by blank lines.
fn split_paragraphs(s: &str) -> Vec<(&str, &str)> {
    let mut out = Vec::new();
    let bytes = s.as_bytes();
    let mut start = 0; // start of current paragraph content
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\n' {
            // Look ahead: is the following line blank?
            let mut j = i + 1;
            let mut blank_lines = 0;
            loop {
                let mut k = j;
                while k < bytes.len() && matches!(bytes[k], b' ' | b'\t' | b'\r') {
                    k += 1;
                }
                if k < bytes.len() && bytes[k] == b'\n' {
                    blank_lines += 1;
                    j = k + 1;
                } else {
                    break;
                }
            }
            if blank_lines > 0 && j < bytes.len() {
                out.push((&s[start..i], &s[i..j]));
                start = j;
                i = j;
                continue;
            }
        }
        i += 1;
    }
    out.push((&s[start..], ""));
    out
}

/// Split into lines, keeping `\n` (and a preceding `\r`) as the separator.
fn split_lines(s: &str) -> Vec<(&str, &str)> {
    let mut out = Vec::new();
    let mut rest = s;
    while let Some(pos) = rest.find('\n') {
        let (line, after) = rest.split_at(pos);
        let (content, sep_start) = match line.strip_suffix('\r') {
            Some(c) => (c, c.len()),
            None => (line, line.len()),
        };
        let sep = &rest[sep_start..pos + 1];
        out.push((content, sep));
        rest = &after[1..];
    }
    if !rest.is_empty() || out.is_empty() {
        out.push((rest, ""));
    }
    out
}

/// Split into sentences at `.`, `!` or `?` followed by whitespace.
fn split_sentences(s: &str) -> Vec<(&str, &str)> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut iter = s.char_indices().peekable();
    while let Some((i, c)) = iter.next() {
        if matches!(c, '.' | '!' | '?') {
            let end = i + c.len_utf8();
            let mut ws_end = end;
            while let Some(&(j, d)) = iter.peek() {
                if d == ' ' || d == '\t' {
                    ws_end = j + d.len_utf8();
                    iter.next();
                } else {
                    break;
                }
            }
            if ws_end > end && ws_end < s.len() {
                out.push((&s[start..end], &s[end..ws_end]));
                start = ws_end;
            }
        }
    }
    out.push((&s[start..], ""));
    out
}

/// Pick the coarsest segmentation that yields at least two sections.
fn mk_sections<'a>(v: Vec<(&'a str, &'a str)>, focus: Option<&Query>) -> Vec<Section<'a>> {
    v.into_iter()
        .map(|(content, sep)| Section {
            content,
            sep,
            chars: nchars(content) + nchars(sep),
            hits: focus.map_or_else(Vec::new, |q| q.positions(content)),
        })
        .collect()
}

fn segment<'a>(s: &'a str, focus: Option<&Query>) -> Vec<Section<'a>> {
    let mk = |v| mk_sections(v, focus);
    let p = split_paragraphs(s);
    if p.len() >= 2 {
        return mk(p);
    }
    let l = split_lines(s);
    if l.len() >= 2 {
        return mk(l);
    }
    mk(split_sentences(s))
}

/// Cut `s` to at most `max` characters, preferring a word boundary in the
/// last ~25% of the allowed span. Returns (prefix, chars kept).
fn cut(s: &str, max: usize) -> (&str, usize) {
    let mut end = s.len();
    let mut count = 0;
    let mut last_ws: Option<(usize, usize)> = None; // (byte idx, chars before it)
    for (i, c) in s.char_indices() {
        if count == max {
            end = i;
            break;
        }
        if c.is_whitespace() {
            last_ws = Some((i, count));
        }
        count += 1;
    }
    if count < max || end == s.len() {
        return (s, count);
    }
    if let Some((i, before)) = last_ws {
        if before * 4 >= max * 3 && before > 0 {
            return (s[..i].trim_end(), nchars(s[..i].trim_end()));
        }
    }
    (&s[..end], count)
}

/// Cut `s` to about `limit` characters and mark what was removed. Plain
/// start-of-string cut, unless query words occur well beyond the start: then the
/// start (a third of the room), a gap marker, and a window around the densest
/// cluster of matches (the rest), with a trailing marker if text follows.
fn shorten(s: &str, limit: usize, hits: &[Hit]) -> String {
    let total = nchars(s);
    if total <= limit {
        return s.to_string();
    }
    if let Some(&(first, _, _)) = hits.first() {
        if first * 5 > limit * 4 && limit >= 24 {
            let head_n = limit / 3;
            let win_n = limit - head_n;
            // Window start: the match that has the most distinct query words
            // within `win_n` after it (earliest on ties), a little earlier for context.
            let best = hits
                .iter()
                .map(|&(p, _, _)| {
                    let mut seen = std::collections::HashSet::new();
                    for &(q, _, t) in hits.iter().filter(|h| h.0 >= p && h.0 < p + win_n) {
                        let _ = q;
                        seen.insert(t);
                    }
                    (seen.len(), std::cmp::Reverse(p))
                })
                .max()
                .map_or(first, |(_, std::cmp::Reverse(p))| p);
            let chars: Vec<char> = s.chars().collect();
            let mut ws = best.saturating_sub(win_n / 4).max(head_n + 1);
            // Start the window at a word boundary.
            while ws > head_n + 1 && !chars[ws - 1].is_whitespace() && best - ws < 24 {
                ws -= 1;
            }
            let mut we = (ws + win_n).min(total);
            if we < total {
                let floor = ws + win_n * 3 / 4;
                while we > floor && !chars[we].is_whitespace() {
                    we -= 1;
                }
            }
            if ws < we && ws > head_n {
                let (head, hk) = cut(s, head_n);
                let window: String = chars[ws..we].iter().collect();
                let mut out = String::with_capacity(head.len() + window.len() + 32);
                out.push_str(head);
                out.push_str(&chars_marker(ws - hk));
                out.push_str(window.trim_end());
                let kept_end = we - (window.len() - window.trim_end().len()).min(we - ws);
                if kept_end < total {
                    out.push_str(&chars_marker(total - kept_end));
                }
                return out;
            }
        }
    }
    let (head, kept) = cut(s, limit);
    let mut out = String::with_capacity(head.len() + 16);
    out.push_str(head);
    out.push_str(&chars_marker(total - kept));
    out
}

/// Preview of one section's content: first `max` chars + omission marker,
/// unless truncating would not actually save anything.
fn preview(
    content: &str,
    max: usize,
    cost: &dyn Fn(&str) -> usize,
    focus: Option<&Query>,
    hits: &[Hit],
) -> String {
    let total = nchars(content);
    if total <= max {
        return content.to_string();
    }
    let lines = split_lines(content);
    if lines.len() >= 2 {
        let out = preview_lines(&lines, total, max, cost, focus);
        if cost(&out) < cost(content) {
            return out;
        }
        return content.to_string();
    }
    let out = shorten(content, max, hits);
    if cost(&out) >= cost(content) {
        return content.to_string();
    }
    out
}

/// Hierarchical preview of a multi-line section: the first line (up to
/// `max` chars), then later lines shortened to `max / 4` each, as long as the
/// whole preview stays within `2 * max` chars, then a lines marker.
fn preview_lines(
    lines: &[(&str, &str)],
    total: usize,
    max: usize,
    cost: &dyn Fn(&str) -> usize,
    focus: Option<&Query>,
) -> String {
    let per_line = (max / 4).max(8);
    let allowance = max.saturating_mul(2);
    let n = lines.len();
    // Show the first line, then (with a query) lines that mention it, then the
    // rest in order, while the allowance lasts. Output stays in text order.
    let mut order: Vec<usize> = (0..n).collect();
    if let Some(q) = focus {
        let hit: Vec<bool> = lines.iter().map(|l| !q.positions(l.0).is_empty()).collect();
        order.sort_by_key(|&i| (i != 0 && !hit[i], i));
    }
    let mut previews: Vec<Option<String>> = vec![None; n];
    let mut used = 0;
    for &i in &order {
        let (content, sep) = lines[i];
        let line_hits = focus.map_or_else(Vec::new, |q| q.positions(content));
        let p = preview(
            content,
            if i == 0 { max } else { per_line },
            cost,
            focus,
            &line_hits,
        );
        let cost_here = nchars(&p) + nchars(sep);
        if i > 0 && used + cost_here > allowance {
            break;
        }
        used += cost_here;
        previews[i] = Some(p);
    }
    let _ = total;
    let mut out = String::new();
    let mut i = 0;
    while i < n {
        if let Some(p) = &previews[i] {
            out.push_str(p);
            out.push_str(lines[i].1);
            i += 1;
        } else {
            let start = i;
            let mut chars = 0;
            while i < n && previews[i].is_none() {
                chars += nchars(lines[i].0) + nchars(lines[i].1);
                i += 1;
            }
            out.push_str(&lines_marker(i - start, chars));
            if i < n {
                out.push('\n');
            }
        }
    }
    out
}

/// Render the head + previews layout for given parameters. Returns pieces,
/// one per section, so callers can elide a suffix without re-rendering.
fn render_pieces(
    secs: &[Section<'_>],
    head: usize,
    section: usize,
    cost: &dyn Fn(&str) -> usize,
    focus: Option<&Query>,
) -> Vec<String> {
    let mut pieces = Vec::with_capacity(secs.len());
    let mut consumed = 0;
    let mut i = 0;
    while i < secs.len() && consumed + secs[i].chars <= head {
        pieces.push(format!("{}{}", secs[i].content, secs[i].sep));
        consumed += secs[i].chars;
        i += 1;
    }
    if i == 0 && !secs.is_empty() {
        // The very first section is longer than the head: preview it with
        // the larger of the two limits so the opening is never starved.
        pieces.push(format!(
            "{}{}",
            preview(
                secs[0].content,
                head.max(section),
                cost,
                focus,
                &secs[0].hits
            ),
            secs[0].sep
        ));
        i = 1;
    }
    for s in &secs[i..] {
        pieces.push(format!(
            "{}{}",
            preview(s.content, section, cost, focus, &s.hits),
            s.sep
        ));
    }
    pieces
}

/// Size of a string made of these pieces, using additivity of the string
/// cost: cost(a+b) = cost(a) + cost(b) - 2 (one pair of quotes).
fn pieces_cost(pieces: &[String], cost: &dyn Fn(&str) -> usize) -> usize {
    2 + pieces.iter().map(|p| cost(p) - 2).sum::<usize>()
}

/// Head-only truncation for text with no internal structure.
fn fit_flat(
    s: &str,
    total: usize,
    budget: usize,
    cost: &dyn Fn(&str) -> usize,
    focus: Option<&Query>,
) -> String {
    // Largest prefix length L with cost(prefix + marker) <= budget (or, with
    // a query, the same for the start-plus-match-window excerpt).
    let hits = focus.map_or_else(Vec::new, |q| q.positions(s));
    let render = |l: usize| -> String { shorten(s, l, &hits) };
    let (mut lo, mut hi) = (0usize, total);
    if cost(&render(0)) > budget {
        return render(0);
    }
    while lo < hi {
        let mid = lo + (hi - lo + 1) / 2;
        if cost(&render(mid)) <= budget {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    render(lo)
}

/// Truncate `s` so that `cost(result) <= budget` where possible.
///
/// `cost` returns the rendered size of a string value in the output format.
/// If `s` already fits it is returned unchanged. If even the bare marker
/// `…[+N chars]` does not fit, that marker is returned (callers treat it as
/// the string's minimum representation).
pub fn fit_text(s: &str, budget: usize, p: &TextParams, cost: &dyn Fn(&str) -> usize) -> String {
    fit_text_focus(s, budget, p, cost, None)
}

/// Like [`fit_text`], keeping what the query asked about when it is known.
pub(crate) fn fit_text_focus(
    s: &str,
    budget: usize,
    p: &TextParams,
    cost: &dyn Fn(&str) -> usize,
    focus: Option<&Query>,
) -> String {
    if cost(s) <= budget {
        return s.to_string();
    }
    let total = nchars(s);
    let secs = segment(s, focus);
    if secs.len() < 2 {
        return fit_flat(s, total, budget, cost, focus);
    }

    let section_unit = p.section.max(1);
    let head_for = |section: usize| -> usize { section.saturating_mul(p.head) / section_unit };
    let min_section = p.min_section.max(1);
    let max_section = secs
        .iter()
        .map(|s| s.chars)
        .max()
        .unwrap_or(0)
        .max(min_section);

    let fits = |section: usize| -> Option<Vec<String>> {
        let pieces = render_pieces(&secs, head_for(section), section, cost, focus);
        if pieces_cost(&pieces, cost) <= budget {
            Some(pieces)
        } else {
            None
        }
    };

    if let Some(mut best) = fits(min_section) {
        // Binary search the largest preview size that still fits.
        let (mut lo, mut hi) = (min_section, max_section);
        while lo < hi {
            let mid = lo + (hi - lo + 1) / 2;
            match fits(mid) {
                Some(p) => {
                    best = p;
                    lo = mid;
                }
                None => hi = mid - 1,
            }
        }
        return best.concat();
    }

    // Even minimal previews are too long: keep a prefix of the minimal
    // rendering and elide the remaining sections behind one marker.
    let pieces = render_pieces(&secs, head_for(min_section), min_section, cost, focus);
    let mut used = 2;
    let mut omitted_chars: usize = total;
    let mut best_m = 0;
    let mut best_marker = sections_marker(secs.len(), total);
    if used + cost(&best_marker) - 2 > budget {
        // Not even the marker fits on its own; fall back to flat truncation,
        // which degrades to the shortest possible marker.
        return fit_flat(s, total, budget, cost, focus);
    }
    for (m, piece) in pieces.iter().enumerate() {
        used += cost(piece) - 2;
        omitted_chars -= secs[m].chars;
        let rest = secs.len() - (m + 1);
        if rest == 0 {
            break;
        }
        let marker = sections_marker(rest, omitted_chars);
        if used + cost(&marker) - 2 <= budget {
            best_m = m + 1;
            best_marker = marker;
        } else {
            break;
        }
    }
    let mut out = pieces[..best_m].concat();
    out.push_str(&best_marker);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json::string_len;

    fn cost(s: &str) -> usize {
        string_len(s)
    }

    #[test]
    fn short_string_untouched() {
        assert_eq!(
            fit_text("hello", 100, &TextParams::default(), &cost),
            "hello"
        );
    }

    #[test]
    fn flat_string_head_truncated_within_budget() {
        let s = "x".repeat(10_000);
        // Below the bare marker's size the marker itself is returned.
        assert_eq!(
            fit_text(&s, 10, &TextParams::default(), &cost),
            "…[+10000 chars]"
        );
        for budget in [20, 50, 100, 1000, 9_000] {
            let out = fit_text(&s, budget, &TextParams::default(), &cost);
            assert!(cost(&out) <= budget, "budget {budget}: {}", cost(&out));
            assert!(out.contains("…[+"), "{out}");
        }
    }

    #[test]
    fn sections_keep_structure() {
        let para = |i: usize| format!("## Section {i}\n{}", "word ".repeat(60).trim_end());
        let doc: Vec<String> = (0..20).map(para).collect();
        let s = doc.join("\n\n");
        let out = fit_text(&s, 2000, &TextParams::default(), &cost);
        assert!(cost(&out) <= 2000);
        // Every section heading should still be visible.
        for i in 0..20 {
            assert!(
                out.contains(&format!("## Section {i}")),
                "missing section {i}:\n{out}"
            );
        }
        assert!(out.contains("\n\n"));
        assert!(out.contains("…[+"));
    }

    #[test]
    fn multiline_section_previews_its_lines() {
        let log: Vec<String> = (0..200)
            .map(|i| {
                format!(
                    "2024-01-01 12:00:{:02} INFO request {i} handled in 12ms",
                    i % 60
                )
            })
            .collect();
        let doc = format!(
            "# Title\n\nIntro paragraph.\n\n{}\n\n## After\n\nEnd text here.",
            log.join("\n")
        );
        let out = fit_text(&doc, 1500, &TextParams::default(), &cost);
        assert!(cost(&out) <= 1500, "{}", cost(&out));
        assert!(out.contains("request 0 handled"), "{out}");
        assert!(out.contains("request 1 handled"), "{out}");
        assert!(out.contains("lines,"), "{out}");
        assert!(out.contains("## After"), "{out}");
    }

    #[test]
    fn tight_budget_elides_sections() {
        let lines: Vec<String> = (0..500)
            .map(|i| format!("line {i}: some log output here"))
            .collect();
        let s = lines.join("\n");
        let out = fit_text(&s, 300, &TextParams::default(), &cost);
        assert!(cost(&out) <= 300, "{}", cost(&out));
        assert!(out.contains("sections,"), "{out}");
        assert!(out.starts_with("line 0"));
    }

    #[test]
    fn uses_budget_generously() {
        let lines: Vec<String> = (0..50)
            .map(|i| format!("{i} {}", "abc ".repeat(50)))
            .collect();
        let s = lines.join("\n");
        let budget = cost(&s) / 2;
        let out = fit_text(&s, budget, &TextParams::default(), &cost);
        assert!(cost(&out) <= budget);
        assert!(
            cost(&out) * 10 >= budget * 8,
            "only used {} of {budget}",
            cost(&out)
        );
    }

    #[test]
    fn segmentation_round_trips() {
        for s in [
            "a\n\nb\n \n\nc",
            "a\nb\r\nc\n",
            "One. Two! Three? Four",
            "solo",
            "",
            "\n\n\n",
        ] {
            let joined: String = segment(s, None)
                .iter()
                .map(|x| format!("{}{}", x.content, x.sep))
                .collect();
            assert_eq!(joined, s, "{s:?}");
        }
    }

    #[test]
    fn query_focus_keeps_the_window_around_the_match() {
        let filler = "lorem ipsum dolor sit amet consectetur ";
        let text = format!(
            "{}The Great Recession officially ended in June 2009, per the NBER. {}",
            filler.repeat(40),
            filler.repeat(40)
        );
        let q = Query::new("great recession official end").unwrap();
        let budget = 400;
        let plain = fit_text(&text, budget, &TextParams::default(), &cost);
        let focused = fit_text_focus(&text, budget, &TextParams::default(), &cost, Some(&q));
        assert!(cost(&focused) <= budget, "{}", cost(&focused));
        assert!(!plain.contains("Recession"), "head cut misses the match");
        assert!(
            focused.contains("Great Recession officially ended in June 2009"),
            "{focused}"
        );
        // Opening kept, with markers on both sides of the window.
        assert!(focused.starts_with("lorem ipsum"), "{focused}");
        assert!(focused.matches("chars]").count() >= 2, "{focused}");
    }

    #[test]
    fn query_focus_is_a_no_op_when_the_match_is_already_visible() {
        let text = format!(
            "great recession begins here. {}",
            "filler words go here ".repeat(80)
        );
        let q = Query::new("great recession").unwrap();
        let a = fit_text(&text, 300, &TextParams::default(), &cost);
        let b = fit_text_focus(&text, 300, &TextParams::default(), &cost, Some(&q));
        assert_eq!(a, b);
    }

    #[test]
    fn unicode_safe() {
        let s = "héllo wörld 😀 ".repeat(200);
        let out = fit_text(&s, 120, &TextParams::default(), &cost);
        assert!(cost(&out) <= 120);
    }
}
