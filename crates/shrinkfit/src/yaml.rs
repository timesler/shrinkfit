//! Dependency-free YAML (block style) emitter.
//!
//! Output is conservative: a string is written plain only when no YAML 1.1
//! or 1.2 parser could read it as anything but that string; otherwise it is
//! double-quoted. Multi-line strings use literal blocks (`|`), which keeps
//! truncated prose and logs readable. Empty containers are written in flow
//! style (`[]`, `{}`).

use crate::value::Value;

/// Render a value as a YAML document (no `---`). The output always ends
/// with a newline so block scalars at the end keep their exact chomping.
/// `indent` is the number of spaces per nesting level and must be >= 2.
pub fn to_string(v: &Value, indent: usize) -> String {
    let w = indent.max(2);
    let mut lines: Vec<String> = Vec::new();
    match v {
        Value::Array(a) if !a.is_empty() => block(v, 0, w, &mut lines),
        Value::Object(o) if !o.is_empty() => block(v, 0, w, &mut lines),
        Value::String(s) if literal_ok(s) => {
            lines.push(literal_header(s).to_string());
            literal_body(s, w, &mut lines);
        }
        _ => lines.push(inline(v)),
    }
    let mut out = lines.join("\n");
    out.push('\n');
    out
}

fn pad(n: usize) -> String {
    " ".repeat(n)
}

/// A value that fits on the same line as its key or dash.
fn inline(v: &Value) -> String {
    match v {
        Value::Null => "null".into(),
        Value::Bool(b) => if *b { "true" } else { "false" }.into(),
        Value::Number(n) => n.clone(),
        Value::String(s) => scalar_string(s),
        Value::Array(_) => "[]".into(),
        Value::Object(_) => "{}".into(),
    }
}

fn is_block(v: &Value) -> bool {
    match v {
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
        _ => false,
    }
}

/// Emit a non-empty container at nesting `depth` (column depth*w).
fn block(v: &Value, depth: usize, w: usize, out: &mut Vec<String>) {
    let ind = pad(depth * w);
    match v {
        Value::Object(entries) => {
            for (k, val) in entries {
                let key = scalar_key(k);
                match val {
                    Value::String(s) if literal_ok(s) => {
                        out.push(format!("{ind}{key}: {}", literal_header(s)));
                        literal_body_at(s, depth + 1, w, out);
                    }
                    x if is_block(x) => {
                        out.push(format!("{ind}{key}:"));
                        block(x, depth + 1, w, out);
                    }
                    x => out.push(format!("{ind}{key}: {}", inline(x))),
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                match item {
                    Value::String(s) if literal_ok(s) => {
                        out.push(format!("{ind}- {}", literal_header(s)));
                        literal_body_at(s, depth + 1, w, out);
                    }
                    x if is_block(x) => {
                        // Render the child one level deeper, then put the
                        // dash into the indentation of its first line.
                        let start = out.len();
                        block(x, depth + 1, w, out);
                        let first = &out[start];
                        let child_ind = (depth + 1) * w;
                        let dash = format!("{ind}-{}", pad(w - 1));
                        out[start] = format!("{dash}{}", &first[child_ind..]);
                    }
                    x => out.push(format!("{ind}- {}", inline(x))),
                }
            }
        }
        _ => unreachable!(),
    }
}

/// Characters YAML considers printable (YAML 1.2 §5.1), minus tab.
fn printable(c: char) -> bool {
    matches!(c as u32,
        0x20..=0x7E | 0x85 | 0xA0..=0xD7FF | 0xE000..=0xFFFD | 0x10000..=0x10FFFF)
        && !matches!(c, '\u{85}' | '\u{2028}' | '\u{2029}' | '\u{FEFF}')
}

/// Can `s` be written as a literal block scalar?
fn literal_ok(s: &str) -> bool {
    if !s.contains('\n') {
        return false;
    }
    // Every line must be printable (tabs allowed inside lines), and a
    // whitespace-only line would confuse indentation detection.
    let mut first_content: Option<&str> = None;
    for line in s.split('\n') {
        if !line.chars().all(|c| c == '\t' || printable(c)) {
            return false;
        }
        if !line.is_empty() && line.trim().is_empty() {
            return false;
        }
        if first_content.is_none() && !line.is_empty() {
            first_content = Some(line);
        }
    }
    match first_content {
        // Leading spaces on the first line would need an indentation indicator.
        Some(l) => !l.starts_with(' ') && !l.starts_with('\t'),
        None => false,
    }
}

fn literal_header(s: &str) -> &'static str {
    if !s.ends_with('\n') {
        "|-"
    } else if s.ends_with("\n\n") {
        "|+"
    } else {
        "|"
    }
}

fn literal_body(s: &str, w: usize, out: &mut Vec<String>) {
    literal_body_at(s, 1, w, out);
}

fn literal_body_at(s: &str, depth: usize, w: usize, out: &mut Vec<String>) {
    let ind = pad(depth * w);
    // With "|" / "|+" the final newline is implied by the chomping mode.
    let body = s.strip_suffix('\n').unwrap_or(s);
    for line in body.split('\n') {
        if line.is_empty() {
            out.push(String::new());
        } else {
            out.push(format!("{ind}{line}"));
        }
    }
}

const RESERVED: &[&str] = &[
    "null", "~", "true", "false", "yes", "no", "on", "off", "y", "n", "<<", "=",
];

/// Is `s` safe to emit as a plain (unquoted) scalar in block context?
fn plain_ok(s: &str) -> bool {
    let first = match s.chars().next() {
        Some(c) => c,
        None => return false,
    };
    if "-?:,[]{}#&*!|>'\"%@`".contains(first) || first.is_whitespace() {
        return false;
    }
    if s.ends_with(|c: char| c.is_whitespace()) || s.ends_with(':') {
        return false;
    }
    if !s.chars().all(printable) {
        return false;
    }
    if s.contains(": ") || s.contains(" #") {
        return false;
    }
    let lower = s.to_ascii_lowercase();
    if RESERVED.contains(&lower.as_str()) {
        return false;
    }
    // Anything that might read as a number, date, .inf/.nan, etc.
    let mut cs = s.chars();
    let c0 = cs.next().unwrap();
    let c1 = cs.next();
    if c0.is_ascii_digit()
        || c0 == '.'
        || ((c0 == '+' || c0 == '-') && c1.is_some_and(|c| c.is_ascii_digit() || c == '.'))
    {
        return false;
    }
    true
}

fn scalar_string(s: &str) -> String {
    if plain_ok(s) {
        s.to_string()
    } else {
        quoted(s)
    }
}

fn scalar_key(k: &str) -> String {
    scalar_string(k)
}

/// YAML double-quoted scalar. JSON's escapes are a subset of YAML's, and
/// any non-printable character is written as `\uXXXX` / `\UXXXXXXXX`.
fn quoted(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if printable(c) => out.push(c),
            c if (c as u32) <= 0xFFFF => out.push_str(&format!("\\u{:04X}", c as u32)),
            c => out.push_str(&format!("\\U{:08X}", c as u32)),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json::parse;

    fn y(src: &str) -> String {
        let out = to_string(&parse(src).unwrap(), 2);
        assert!(out.ends_with('\n'));
        out[..out.len() - 1].to_string()
    }

    #[test]
    fn basic_mapping_and_sequence() {
        assert_eq!(
            y(r#"{"name":"svc","ports":[80,443],"meta":{"a":true,"b":null},"empty":[],"e2":{}}"#),
            "name: svc\nports:\n  - 80\n  - 443\nmeta:\n  a: true\n  b: null\nempty: []\ne2: {}"
        );
    }

    #[test]
    fn sequence_of_mappings_uses_compact_dash() {
        assert_eq!(
            y(r#"[{"a":1,"b":2},[1,[2]]]"#),
            "- a: 1\n  b: 2\n- - 1\n  - - 2"
        );
    }

    #[test]
    fn wide_indent_stays_aligned() {
        let out = to_string(&parse(r#"[{"a":1,"b":2}]"#).unwrap(), 4);
        assert_eq!(out, "-   a: 1\n    b: 2\n");
    }

    #[test]
    fn ambiguous_strings_are_quoted() {
        for s in [
            "true",
            "No",
            "null",
            "~",
            "123",
            "1.5",
            "-2",
            ".inf",
            "a: b",
            "x #y",
            "",
            " lead",
            "trail ",
            "- item",
            "@at",
            "key:",
            "0x1F",
            "2026-10-11",
        ] {
            let out = y(&format!(
                "{{\"k\":{}}}",
                crate::json::to_string(&Value::String(s.into()), crate::json::Layout::Compact)
            ));
            assert!(out.starts_with("k: \""), "{s:?} -> {out}");
        }
        assert_eq!(
            y(r#"{"k":"plain text, ok…[+12 chars]"}"#),
            "k: plain text, ok…[+12 chars]"
        );
    }

    #[test]
    fn multiline_uses_literal_block() {
        assert_eq!(
            y(r#"{"log":"line 1\nline 2"}"#),
            "log: |-\n  line 1\n  line 2"
        );
        assert_eq!(y(r#"{"log":"a\n\nb\n"}"#), "log: |\n  a\n\n  b");
        assert_eq!(y(r#"{"log":"  indented\nx"}"#), "log: \"  indented\\nx\"");
    }

    #[test]
    fn control_chars_escaped() {
        assert_eq!(y(r#"{"k":"a\u0001b "}"#), "k: \"a\\u0001b\\u2028\"");
    }
}
