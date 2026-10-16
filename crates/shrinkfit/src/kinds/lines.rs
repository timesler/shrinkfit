//! Logs, command output, diffs and outlines: keep the lines that matter.
//!
//! Every line gets a score; lines are kept best first until the budget is
//! used, then shown in their original order with `…[N lines omitted]` where a
//! gap was left. Errors, failures, warnings, summaries, stack frames, the first
//! and the last lines score high; a thousand lines that differ only in a
//! number or an id score low after the first of them.

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Mode {
    Log,
    Diff,
    /// An outline (XML) or any line-oriented text without log semantics.
    Outline,
}

/// Longest line kept whole; longer ones are cut. Lines that matter get twice as much.
const LINE_CHARS: usize = 240;

// ---- cleaning ----

/// Remove ANSI escape sequences, keep only what a progress line finally shows
/// (text after the last carriage return), and trim trailing spaces.
pub(super) fn clean(s: &str) -> Vec<String> {
    let s = strip_ansi(s);
    s.split('\n')
        .map(|l| {
            let l = l.trim_end_matches('\r');
            let l = l.rsplit('\r').find(|p| !p.trim().is_empty()).unwrap_or(l);
            l.trim_end().to_string()
        })
        .collect()
}

fn strip_ansi(s: &str) -> String {
    if !s.contains('\u{1b}') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c != '\u{1b}' {
            out.push(c);
            continue;
        }
        match it.peek() {
            Some('[') => {
                it.next();
                for d in it.by_ref() {
                    if ('@'..='~').contains(&d) {
                        break;
                    }
                }
            }
            Some(']') => {
                // OSC: up to BEL or ESC \
                it.next();
                while let Some(d) = it.next() {
                    if d == '\u{7}' || (d == '\u{1b}' && it.peek() == Some(&'\\')) {
                        if d == '\u{1b}' {
                            it.next();
                        }
                        break;
                    }
                }
            }
            Some(_) => {
                it.next();
            }
            None => {}
        }
    }
    out
}

// ---- detection ----

pub(super) fn is_diff(s: &str) -> bool {
    let (mut files, mut hunks, mut minus, mut plus) = (0, 0, 0, 0);
    for l in s.lines().take(4000) {
        if l.starts_with("diff --git ") {
            files += 1;
        } else if l.starts_with("@@ ") && l[3..].contains(" @@") {
            hunks += 1;
        } else if l.starts_with("--- ") {
            minus += 1;
        } else if l.starts_with("+++ ") {
            plus += 1;
        }
    }
    files >= 1 && hunks >= 1 || (hunks >= 1 && minus >= 1 && plus >= 1)
}

fn tokens(l: &str) -> impl Iterator<Item = &str> {
    l.split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
}

fn has_timestamp(l: &str) -> bool {
    let t = l.trim_start_matches(['[', '(', ' ']);
    let b = t.as_bytes();
    let date = b.len() >= 10
        && b[..4].iter().all(u8::is_ascii_digit)
        && b[4] == b'-'
        && b[5..7].iter().all(u8::is_ascii_digit);
    let time = b.len() >= 8
        && b[..2].iter().all(u8::is_ascii_digit)
        && b[2] == b':'
        && b[3..5].iter().all(u8::is_ascii_digit)
        && b[5] == b':';
    date || time
}

fn level_token(l: &str) -> bool {
    tokens(l).take(8).any(|t| {
        matches!(
            t,
            "ERROR"
                | "FATAL"
                | "CRITICAL"
                | "WARN"
                | "WARNING"
                | "INFO"
                | "DEBUG"
                | "TRACE"
                | "NOTICE"
        )
    })
}

/// Digits and long hex runs collapse to `#`, so lines that differ only in
/// numbers and ids share a template.
fn template(l: &str) -> String {
    let mut out = String::new();
    let mut run = String::new();
    let flush = |run: &mut String, out: &mut String| {
        if run.is_empty() {
            return;
        }
        if run.len() >= 8
            && run.bytes().all(|b| b.is_ascii_hexdigit())
            && run.bytes().any(|b| b.is_ascii_digit())
        {
            out.push('#');
        } else {
            // every run of digits inside the word becomes one '#': 12ms -> #ms, worker-3 -> worker-#
            let mut prev_digit = false;
            for c in run.chars() {
                if c.is_ascii_digit() {
                    if !prev_digit {
                        out.push('#');
                    }
                    prev_digit = true;
                } else {
                    out.push(c);
                    prev_digit = false;
                }
            }
        }
        run.clear();
    };
    for c in l.chars().take(160) {
        if c.is_ascii_alphanumeric() {
            run.push(c);
        } else {
            flush(&mut run, &mut out);
            out.push(c);
        }
    }
    flush(&mut run, &mut out);
    out
}

fn is_frame(l: &str) -> bool {
    let t = l.trim_start();
    let indented = t.len() < l.len();
    (indented && (t.starts_with("at ") && (t.contains('(') || t.contains(':'))))
        || t.starts_with("File \"")
        || t.starts_with("Caused by")
        || t.starts_with("... ") && t.contains("more")
        || (indented
            && t.contains(".rs:")
            && t.starts_with(|c: char| c.is_ascii_digit() || c == 'a'))
}

pub(super) fn is_log(s: &str) -> bool {
    let lines = clean(s);
    let non_empty: Vec<&String> = lines.iter().filter(|l| !l.trim().is_empty()).collect();
    let n = non_empty.len();
    if n < 8 {
        return false;
    }
    let marked = non_empty
        .iter()
        .filter(|l| has_timestamp(l) || level_token(l))
        .count();
    let frames = non_empty.iter().filter(|l| is_frame(l)).count();
    let traceback = non_empty
        .iter()
        .any(|l| l.starts_with("Traceback (most recent call last)"));
    let build = non_empty
        .iter()
        .filter(|l| {
            let t = l.trim_start();
            [
                "Compiling ",
                "Running ",
                "PASS ",
                "FAIL ",
                "npm ERR!",
                "error[",
                "error:",
                "warning:",
                "$ ",
                "> ",
                "make[",
            ]
            .iter()
            .any(|p| t.starts_with(p))
        })
        .count();
    let mut counts = std::collections::HashMap::new();
    for l in &non_empty {
        *counts.entry(template(l)).or_insert(0usize) += 1;
    }
    let repeated: usize = counts.values().filter(|&&c| c >= 3).sum();
    // Many lines of one shape are a log only if they are short and not a document:
    // repeated body text under markdown headings is prose.
    let mut lens: Vec<usize> = non_empty.iter().map(|l| l.chars().count()).collect();
    lens.sort_unstable();
    let short = lens[lens.len() / 2] <= 160;
    let headings = non_empty
        .iter()
        .any(|l| l.starts_with("# ") || l.starts_with("## ") || l.starts_with("### "));
    let uniform = n >= 15 && repeated * 2 >= n && short && !headings;
    marked * 10 >= n * 3 || traceback || frames >= 3 || build >= 3 || uniform
}

// ---- scoring ----

const FAILURE: [&str; 17] = [
    "error",
    "err",
    "fatal",
    "critical",
    "panic",
    "panicked",
    "failed",
    "fail",
    "failure",
    "failures",
    "exception",
    "traceback",
    "segfault",
    "abort",
    "aborted",
    "assertionerror",
    "denied",
];
const WARNING: [&str; 4] = ["warn", "warning", "warnings", "deprecated"];
const SUMMARY: [&str; 9] = [
    "passed", "failed", "skipped", "tests", "test", "errors", "warnings", "total", "ok",
];

fn benign(low: &str) -> bool {
    [
        "0 error",
        "no error",
        "errors: 0",
        "error=0",
        "errors=0",
        "error_count",
        "0 failed",
        "0 fail",
        "failed: 0",
        "failures: 0",
        "0 failures",
    ]
    .iter()
    .any(|p| low.contains(p))
}

fn score_line(l: &str) -> f64 {
    let low = l.to_lowercase();
    if benign(&low) {
        return 0.5;
    }
    if tokens(&low).any(|t| FAILURE.contains(&t)) || l.contains("ERROR") || l.contains("FATAL") {
        return 1.0;
    }
    if low.starts_with("traceback")
        || low.contains("exit code")
            && !low.contains("exit code 0")
            && !low.contains("exit code: 0")
    {
        return 1.0;
    }
    if tokens(&low).any(|t| WARNING.contains(&t)) {
        return 0.7;
    }
    let has_digit = l.bytes().any(|b| b.is_ascii_digit());
    if low.starts_with("test result:")
        || low.starts_with("finished")
        || low.starts_with("done")
        || low.starts_with("build ")
        || low.starts_with("successfully")
    {
        return 0.85;
    }
    if has_digit && tokens(&low).any(|t| SUMMARY.contains(&t)) && l.len() < 200 {
        return 0.75;
    }
    0.0
}

/// Scores for every line of a log (higher is kept first).
fn log_scores(lines: &[String]) -> Vec<f64> {
    let n = lines.len();
    let mut base: Vec<f64> = lines
        .iter()
        .map(|l| {
            if l.trim().is_empty() {
                0.02
            } else {
                score_line(l)
            }
        })
        .collect();
    // Stack traces: header, first frames and the exception line that follows them.
    let mut i = 0;
    while i < n {
        if is_frame(&lines[i]) {
            let start = i;
            while i < n
                && (is_frame(&lines[i])
                    || lines[i].trim().is_empty() && i + 1 < n && is_frame(&lines[i + 1])
                    || i > start
                        && lines[i - 1].trim_start().starts_with("File \"")
                        && lines[i].starts_with(char::is_whitespace))
            {
                i += 1;
            }
            for (k, j) in (start..i).enumerate() {
                let keep = if k < 3 || j + 1 == i { 0.55 } else { 0.15 };
                base[j] = base[j].max(keep);
            }
            if i < n && !lines[i].starts_with(char::is_whitespace) && !lines[i].trim().is_empty() {
                base[i] = base[i].max(1.0); // the exception itself
            }
            if start > 0 && !lines[start - 1].trim().is_empty() {
                base[start - 1] = base[start - 1].max(0.9);
            }
        } else {
            i += 1;
        }
    }
    // The first lines say what this is; the last lines say how it ended.
    for (j, b) in base.iter_mut().enumerate() {
        if j < 3 {
            *b = b.max(0.9);
        }
        if j + 5 >= n {
            *b = b.max(0.85 - 0.02 * (n - 1 - j) as f64);
        }
    }
    // Context around what matters.
    let strong: Vec<usize> = (0..n).filter(|&j| base[j] >= 0.95).collect();
    for j in strong {
        for (d, v) in [(1usize, 0.65), (2, 0.45)] {
            if j >= d {
                base[j - d] = base[j - d].max(v);
            }
            if j + d < n {
                base[j + d] = base[j + d].max(v);
            }
        }
    }
    // Repetition: the first line of a template, then the first and last of a run.
    let tmpl: Vec<String> = lines.iter().map(|l| template(l)).collect();
    let mut seen = std::collections::HashMap::new();
    for (j, t) in tmpl.iter().enumerate() {
        let nth = seen.entry(t.clone()).or_insert(0usize);
        *nth += 1;
        let first = *nth == 1;
        let run_start = j == 0 || tmpl[j - 1] != *t;
        let run_end = j + 1 == n || tmpl[j + 1] != *t;
        let v = if first {
            0.5
        } else if run_start || run_end {
            0.3
        } else {
            0.08
        };
        if base[j] < v && !lines[j].trim().is_empty() {
            base[j] = v;
        }
        // The fourth warning of one shape tells no more than the third.
        if *nth > 3 && base[j] < 0.95 {
            base[j] *= 0.55;
        }
    }
    base
}

fn diff_scores(lines: &[String]) -> Vec<f64> {
    let n = lines.len();
    let mut s = vec![0.1; n];
    let mut in_hunk_pos = 0usize;
    for (j, l) in lines.iter().enumerate() {
        s[j] = if l.starts_with("diff ") {
            in_hunk_pos = 0;
            1.0
        } else if l.starts_with("--- ") || l.starts_with("+++ ") {
            in_hunk_pos = 0;
            0.55
        } else if l.starts_with("new file")
            || l.starts_with("deleted file")
            || l.starts_with("rename ")
            || l.starts_with("Binary files")
        {
            in_hunk_pos = 0;
            0.85
        } else if l.starts_with("index ") {
            in_hunk_pos = 0;
            0.3
        } else if l.starts_with("@@") {
            in_hunk_pos = 0;
            0.7
        } else if l.starts_with('+') || l.starts_with('-') {
            // One score for a whole change, lower for the later changes of a hunk.
            if !(j > 0 && (lines[j - 1].starts_with('+') || lines[j - 1].starts_with('-'))) {
                in_hunk_pos += 1;
            }
            (0.82 - 0.03 * in_hunk_pos as f64).max(0.4)
        } else if l.starts_with('\\') {
            0.1
        } else {
            0.12
        };
    }
    // One line of context on either side of a change.
    for j in 0..n {
        if (lines[j].starts_with('+') || lines[j].starts_with('-'))
            && !lines[j].starts_with("+++")
            && !lines[j].starts_with("---")
        {
            for k in [j.wrapping_sub(1), j + 1] {
                if k < n && s[k] < 0.35 && !lines[k].starts_with('@') {
                    s[k] = 0.35;
                }
            }
        }
    }
    s
}

fn outline_scores(lines: &[String]) -> Vec<f64> {
    let n = lines.len();
    let tmpl: Vec<String> = lines.iter().map(|l| template(l.trim_start())).collect();
    let mut seen = std::collections::HashMap::new();
    let mut s = vec![0.0; n];
    // Words found in few lines make a line worth more: the odd row in a table, the flagged record.
    let sets: Vec<std::collections::BTreeSet<String>> = lines.iter().map(|l| words(l)).collect();
    let mut df: std::collections::BTreeMap<&str, usize> = std::collections::BTreeMap::new();
    for set in &sets {
        for w in set {
            *df.entry(w.as_str()).or_insert(0) += 1;
        }
    }
    let top = (n as f64).ln().max(1.0);
    for j in 0..n {
        let depth = lines[j].len() - lines[j].trim_start().len();
        let nth = seen.entry(tmpl[j].clone()).or_insert(0usize);
        *nth += 1;
        let rarity = if sets[j].is_empty() {
            0.0
        } else {
            sets[j]
                .iter()
                .map(|w| (n as f64 / df[w.as_str()] as f64).ln() / top)
                .sum::<f64>()
                / sets[j].len() as f64
        };
        s[j] = (if *nth == 1 {
            0.7
        } else if *nth <= 3 {
            0.45
        } else {
            0.2
        }) + 0.25 * rarity.min(1.0)
            - 0.01 * (depth / 2) as f64;
        if j < 3 {
            s[j] = s[j].max(0.9);
        }
        if j + 3 >= n {
            s[j] = s[j].max(0.8);
        }
    }
    s
}

// ---- selection ----

fn shorten(l: &str, important: bool) -> String {
    let max = if important {
        LINE_CHARS * 2
    } else {
        LINE_CHARS
    };
    let n = l.chars().count();
    if n <= max {
        return l.to_string();
    }
    let keep = max * 2 / 3;
    let head: String = l.chars().take(keep).collect();
    format!("{head}{}", crate::text::chars_marker(n - keep))
}

fn gap(k: usize, unit: &str) -> String {
    format!("…[{k} {unit} omitted]")
}

fn render(lines: &[String], keep: &[bool]) -> String {
    render_with(lines, keep, "\n", "lines")
}

fn render_with(lines: &[String], keep: &[bool], sep: &str, unit: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut omitted = 0usize;
    for (l, &k) in lines.iter().zip(keep) {
        if k {
            if omitted > 0 {
                out.push(gap(omitted, unit));
                omitted = 0;
            }
            out.push(l.clone());
        } else {
            omitted += 1;
        }
    }
    if omitted > 0 {
        out.push(gap(omitted, unit));
    }
    out.join(sep)
}

fn words(p: &str) -> std::collections::BTreeSet<String> {
    tokens(p)
        .filter(|t| t.chars().count() >= 3)
        .map(|t| t.to_lowercase())
        .collect()
}

/// Paragraphs of prose, chosen by how much only they say: a paragraph made of
/// words found nowhere else in the text (names, numbers, flags, terms) ranks
/// above one made of the words every paragraph uses. Needs at least ten
/// paragraphs, and not many markdown headings (a document with an outline keeps it).
pub(super) fn fit_paragraphs(
    s: &str,
    budget: usize,
    cost: &dyn Fn(&str) -> usize,
) -> Option<String> {
    let paras: Vec<String> = s
        .replace("\r\n", "\n")
        .split("\n\n")
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
        .collect();
    let n = paras.len();
    if n < 10 || paras.iter().filter(|p| p.starts_with('#')).count() * 5 > n {
        return None;
    }
    let sets: Vec<std::collections::BTreeSet<String>> = paras.iter().map(|p| words(p)).collect();
    let mut df: std::collections::BTreeMap<&str, usize> = std::collections::BTreeMap::new();
    for set in &sets {
        for w in set {
            *df.entry(w.as_str()).or_insert(0) += 1;
        }
    }
    let top = (n as f64).ln().max(1.0);
    let mut scores: Vec<f64> = sets
        .iter()
        .zip(&paras)
        .map(|(set, p)| {
            if set.is_empty() {
                return 0.1;
            }
            // Mean rarity of the paragraph's words, 0 (all common) to 1 (all unique).
            let rarity: f64 = set
                .iter()
                .map(|w| (n as f64 / df[w.as_str()] as f64).ln() / top)
                .sum::<f64>()
                / set.len() as f64;
            let link = if p.contains("http://") || p.contains("https://") {
                0.1
            } else {
                0.0
            };
            0.2 + 0.6 * rarity.min(1.0) + link
        })
        .collect();
    for (j, sc) in scores.iter_mut().enumerate() {
        if j < 2 {
            *sc = sc.max(0.9);
        }
        if j + 1 == n {
            *sc = sc.max(0.8);
        }
    }
    let shown: Vec<String> = paras
        .iter()
        .map(|p| {
            let c = p.chars().count();
            if c <= 360 {
                p.clone()
            } else {
                format!(
                    "{}{}",
                    p.chars().take(240).collect::<String>(),
                    crate::text::chars_marker(c - 240)
                )
            }
        })
        .collect();
    let all = vec![true; n];
    let whole = render_with(&shown, &all, "\n\n", "paragraphs");
    if cost(&whole) <= budget {
        return Some(whole);
    }
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&a, &b| {
        scores[b]
            .partial_cmp(&scores[a])
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.cmp(&b))
    });
    let build = |m: usize| -> String {
        let mut keep = vec![false; n];
        for &j in &order[..m] {
            keep[j] = true;
        }
        render_with(&shown, &keep, "\n\n", "paragraphs")
    };
    let (mut lo, mut hi) = (0usize, n);
    while lo < hi {
        let mid = (lo + hi + 1) / 2;
        if cost(&build(mid)) <= budget {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    (lo > 0).then(|| build(lo))
}

/// Shorten line-oriented text to at most `budget` as measured by `cost`.
pub(super) fn fit(
    s: &str,
    mode: Mode,
    budget: usize,
    cost: &dyn Fn(&str) -> usize,
) -> Option<String> {
    let mut lines = clean(s);
    while lines.last().is_some_and(|l| l.trim().is_empty()) {
        lines.pop();
    }
    // Collapse runs of blank lines to one.
    let mut compact: Vec<String> = Vec::with_capacity(lines.len());
    for l in lines {
        if l.trim().is_empty() && compact.last().is_some_and(|p| p.trim().is_empty()) {
            continue;
        }
        compact.push(l);
    }
    let lines = compact;
    let n = lines.len();
    if n == 0 {
        return None;
    }
    let scores = match mode {
        Mode::Log => log_scores(&lines),
        Mode::Diff => diff_scores(&lines),
        Mode::Outline => outline_scores(&lines),
    };
    let shown: Vec<String> = lines
        .iter()
        .zip(&scores)
        .map(|(l, &sc)| shorten(l, sc >= 0.95))
        .collect();
    let all = vec![true; n];
    let whole = render(&shown, &all);
    if cost(&whole) <= budget {
        return Some(whole);
    }
    // Best first; among equals spread the choice over the text instead of taking the head.
    let spread = |j: usize| (j as f64 * 0.618_033_988_749_895) % 1.0;
    // A change is kept or dropped as a whole: its lines share one place in the spread.
    let group: Vec<usize> = (0..n)
        .scan(0usize, |start, j| {
            let change = mode == Mode::Diff
                && (lines[j].starts_with('+') || lines[j].starts_with('-'))
                && !lines[j].starts_with("+++")
                && !lines[j].starts_with("---");
            let prev_change = j > 0
                && mode == Mode::Diff
                && (lines[j - 1].starts_with('+') || lines[j - 1].starts_with('-'));
            if !(change && prev_change) {
                *start = j;
            }
            Some(*start)
        })
        .collect();
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&a, &b| {
        let (ga, gb) = (group[a], group[b]);
        scores[b]
            .partial_cmp(&scores[a])
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(
                spread(ga)
                    .partial_cmp(&spread(gb))
                    .unwrap_or(std::cmp::Ordering::Equal),
            )
            .then(a.cmp(&b))
    });
    let build = |m: usize| -> String {
        let mut keep = vec![false; n];
        for &j in &order[..m] {
            keep[j] = true;
        }
        render(&shown, &keep)
    };
    // Largest prefix of the ranking that fits.
    let (mut lo, mut hi) = (0usize, n);
    while lo < hi {
        let mid = (lo + hi + 1) / 2;
        if cost(&build(mid)) <= budget {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    if lo == 0 {
        return None;
    }
    // A change is shown whole or not at all: finish a half-shown group if it still fits, else drop it.
    let mut keep = vec![false; n];
    for &j in &order[..lo] {
        keep[j] = true;
    }
    let mut start = 0;
    while start < n {
        let mut end = start + 1;
        while end < n && group[end] == group[start] {
            end += 1;
        }
        let shown_count = keep[start..end].iter().filter(|&&k| k).count();
        if shown_count > 0 && shown_count < end - start {
            let mut whole = keep.clone();
            whole[start..end].iter_mut().for_each(|k| *k = true);
            if cost(&render(&shown, &whole)) <= budget {
                keep = whole;
            } else {
                keep[start..end].iter_mut().for_each(|k| *k = false);
            }
        }
        start = end;
    }
    let out = render(&shown, &keep);
    (cost(&out) <= budget && keep.iter().any(|&k| k)).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cost(s: &str) -> usize {
        crate::json::string_len(s)
    }

    fn noisy_log() -> String {
        let mut v: Vec<String> = (0..300)
            .map(|i| {
                format!(
                    "2026-10-16T05:{:02}:{:02}Z INFO  worker-{} processed batch {i} in {}ms",
                    i / 60,
                    i % 60,
                    i % 7,
                    10 + i % 13
                )
            })
            .collect();
        v[167] = "2026-10-16T05:02:47Z FATAL worker-3 connection pool exhausted: 0/32 connections available".into();
        v[168] = "Traceback (most recent call last):\n  File \"app/db.py\", line 88, in acquire\n    raise PoolExhausted(name)\nPoolExhausted: orders-primary".into();
        v.join("\n")
    }

    #[test]
    fn log_keeps_the_fatal_line_and_the_exception() {
        let s = noisy_log();
        assert!(is_log(&s));
        for budget in [600, 1000, 2000] {
            let out = fit(&s, Mode::Log, budget, &cost).unwrap();
            assert!(cost(&out) <= budget, "{budget}: {}", cost(&out));
            assert!(out.contains("FATAL worker-3"), "{budget}: {out}");
            assert!(
                out.contains("PoolExhausted: orders-primary"),
                "{budget}: {out}"
            );
            assert!(out.contains("lines omitted"), "{out}");
        }
    }

    #[test]
    fn ansi_codes_and_progress_overwrites_go() {
        let s = "\u{1b}[32mCompiling\u{1b}[0m foo v1\n10%\r50%\r100% done\nok";
        assert_eq!(clean(s), vec!["Compiling foo v1", "100% done", "ok"]);
    }

    #[test]
    fn diff_keeps_headers_hunks_and_changes_before_context() {
        let mut d =
            String::from("diff --git a/x.rs b/x.rs\nindex 1..2 100644\n--- a/x.rs\n+++ b/x.rs\n");
        for h in 0..6 {
            d.push_str(&format!("@@ -{0},8 +{0},8 @@ fn f{h}\n", h * 20 + 1));
            for c in 0..3 {
                d.push_str(&format!(" context {h} {c} some unchanged code\n"));
            }
            d.push_str(&format!(
                "-let old_{h} = compute({h});\n+let new_{h} = compute_better({h});\n"
            ));
            for c in 3..6 {
                d.push_str(&format!(" context {h} {c} some unchanged code\n"));
            }
        }
        assert!(is_diff(&d));
        let out = fit(&d, Mode::Diff, 700, &cost).unwrap();
        assert!(cost(&out) <= 700);
        assert!(out.contains("diff --git a/x.rs b/x.rs"), "{out}");
        // Changes come whole (the - and its +) and hunk headers outlast context.
        assert_eq!(
            out.matches("-let old_").count(),
            out.matches("+let new_").count(),
            "{out}"
        );
        assert!(out.matches("-let old_").count() >= 2, "{out}");
        assert!(out.matches("@@").count() >= 4, "{out}");
    }

    #[test]
    fn prose_and_markdown_are_not_logs() {
        let p = "# Title\n\nSome paragraph of text that explains the thing in detail.\n\n- first point about something\n- second point about another\n- third, different again\n\nAnother paragraph follows here with more words.\n\nAnd a last one to end the document properly.\n";
        assert!(!is_log(p));
        assert!(!is_diff(p));
    }

    #[test]
    fn build_output_summary_survives() {
        let mut v: Vec<String> = (0..120)
            .map(|i| format!("   Compiling crate-{i} v0.{i}.0"))
            .collect();
        v.push("error[E0308]: mismatched types".into());
        v.push("  --> src/lib.rs:10:5".into());
        v.push("test result: FAILED. 3 passed; 1 failed; 0 ignored".into());
        let s = v.join("\n");
        assert!(is_log(&s));
        let out = fit(&s, Mode::Log, 500, &cost).unwrap();
        assert!(
            out.contains("error[E0308]") && out.contains("test result: FAILED"),
            "{out}"
        );
    }

    #[test]
    fn selection_never_exceeds_the_budget() {
        let s = noisy_log();
        for budget in [60, 120, 300, 777, 1500, 5000] {
            if let Some(out) = fit(&s, Mode::Log, budget, &cost) {
                assert!(cost(&out) <= budget, "{budget}");
            }
        }
    }
}
