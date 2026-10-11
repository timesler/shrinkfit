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
//! `head` and `section` scale together (keeping their ratio) so that the
//! result uses as much of the budget as possible: a generous budget shows
//! longer previews, a tight one shorter previews, and only a very tight one
//! starts dropping sections. Separators (newlines, blank lines) are kept
//! verbatim so the outline of the text survives.

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
#[derive(Debug, Clone, Copy)]
struct Section<'a> {
    content: &'a str,
    sep: &'a str,
    chars: usize,
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
fn mk_sections<'a>(v: Vec<(&'a str, &'a str)>) -> Vec<Section<'a>> {
    v.into_iter()
        .map(|(content, sep)| Section {
            content,
            sep,
            chars: nchars(content) + nchars(sep),
        })
        .collect()
}

fn segment(s: &str) -> Vec<Section<'_>> {
    let mk = mk_sections;
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

/// Preview of one section's content: first `max` chars + omission marker,
/// unless truncating would not actually save anything.
fn preview(content: &str, max: usize, cost: &dyn Fn(&str) -> usize) -> String {
    let total = nchars(content);
    if total <= max {
        return content.to_string();
    }
    let lines = split_lines(content);
    if lines.len() >= 2 {
        let out = preview_lines(&lines, total, max, cost);
        if cost(&out) < cost(content) {
            return out;
        }
        return content.to_string();
    }
    let (head, kept) = cut(content, max);
    let mut out = String::with_capacity(head.len() + 16);
    out.push_str(head);
    out.push_str(&chars_marker(total - kept));
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
) -> String {
    let per_line = (max / 4).max(8);
    let allowance = max.saturating_mul(2);
    let mut out = String::new();
    let mut used = 0;
    let mut shown_chars = 0;
    let mut shown = 0;
    for (i, (content, sep)) in lines.iter().enumerate() {
        let p = preview(content, if i == 0 { max } else { per_line }, cost);
        let cost_here = nchars(&p) + nchars(sep);
        if i > 0 && used + cost_here > allowance {
            break;
        }
        out.push_str(&p);
        out.push_str(sep);
        used += cost_here;
        shown_chars += nchars(content) + nchars(sep);
        shown += 1;
    }
    if shown < lines.len() {
        out.push_str(&lines_marker(lines.len() - shown, total - shown_chars));
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
            preview(secs[0].content, head.max(section), cost),
            secs[0].sep
        ));
        i = 1;
    }
    for s in &secs[i..] {
        pieces.push(format!("{}{}", preview(s.content, section, cost), s.sep));
    }
    pieces
}

/// Size of a string made of these pieces, using additivity of the string
/// cost: cost(a+b) = cost(a) + cost(b) - 2 (one pair of quotes).
fn pieces_cost(pieces: &[String], cost: &dyn Fn(&str) -> usize) -> usize {
    2 + pieces.iter().map(|p| cost(p) - 2).sum::<usize>()
}

/// Head-only truncation for text with no internal structure.
fn fit_flat(s: &str, total: usize, budget: usize, cost: &dyn Fn(&str) -> usize) -> String {
    // Largest prefix length L with cost(prefix + marker) <= budget.
    let render = |l: usize| -> String {
        let (p, kept) = cut(s, l);
        format!("{}{}", p, chars_marker(total - kept))
    };
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
    if cost(s) <= budget {
        return s.to_string();
    }
    let total = nchars(s);
    let secs = segment(s);
    if secs.len() < 2 {
        return fit_flat(s, total, budget, cost);
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
        let pieces = render_pieces(&secs, head_for(section), section, cost);
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
    let pieces = render_pieces(&secs, head_for(min_section), min_section, cost);
    let mut used = 2;
    let mut omitted_chars: usize = total;
    let mut best_m = 0;
    let mut best_marker = sections_marker(secs.len(), total);
    if used + cost(&best_marker) - 2 > budget {
        // Not even the marker fits on its own; fall back to flat truncation,
        // which degrades to the shortest possible marker.
        return fit_flat(s, total, budget, cost);
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
            let joined: String = segment(s)
                .iter()
                .map(|x| format!("{}{}", x.content, x.sep))
                .collect();
            assert_eq!(joined, s, "{s:?}");
        }
    }

    #[test]
    fn unicode_safe() {
        let s = "héllo wörld 😀 ".repeat(200);
        let out = fit_text(&s, 120, &TextParams::default(), &cost);
        assert!(cost(&out) <= 120);
    }
}
