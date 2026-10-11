//! Dependency-free JSON parser and emitter.
//!
//! The parser preserves object key order and number literals. The emitter
//! produces either compact output or pretty output with a configurable
//! indent, matching the layout conventions of `JSON.stringify(v, null, n)`.

use crate::value::Value;
use std::fmt;

/// Maximum nesting depth accepted by the parser. Deeper input is rejected
/// rather than risking a stack overflow in recursive processing.
pub const MAX_DEPTH: usize = 512;

/// A parse error with a 1-based line/column position.
#[derive(Debug, Clone, PartialEq)]
pub struct ParseError {
    pub message: String,
    pub line: usize,
    pub column: usize,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} at line {}, column {}",
            self.message, self.line, self.column
        )
    }
}

impl std::error::Error for ParseError {}

/// Parse a JSON document. A leading UTF-8 BOM is ignored.
pub fn parse(text: &str) -> Result<Value, ParseError> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut p = Parser {
        s: text.as_bytes(),
        src: text,
        i: 0,
    };
    p.ws();
    let v = p.value(0)?;
    p.ws();
    if p.i != p.s.len() {
        return Err(p.err("unexpected trailing characters"));
    }
    Ok(v)
}

struct Parser<'a> {
    s: &'a [u8],
    src: &'a str,
    i: usize,
}

impl<'a> Parser<'a> {
    fn err(&self, msg: &str) -> ParseError {
        let upto = &self.src[..self.i.min(self.src.len())];
        let line = upto.matches('\n').count() + 1;
        let column = upto
            .rsplit('\n')
            .next()
            .map(|l| l.chars().count())
            .unwrap_or(0)
            + 1;
        ParseError {
            message: msg.to_string(),
            line,
            column,
        }
    }

    fn ws(&mut self) {
        while self.i < self.s.len() && matches!(self.s[self.i], b' ' | b'\t' | b'\n' | b'\r') {
            self.i += 1;
        }
    }

    fn peek(&self) -> Option<u8> {
        self.s.get(self.i).copied()
    }

    fn expect_lit(&mut self, lit: &str, v: Value) -> Result<Value, ParseError> {
        if self.s[self.i..].starts_with(lit.as_bytes()) {
            self.i += lit.len();
            Ok(v)
        } else {
            Err(self.err("invalid literal"))
        }
    }

    fn value(&mut self, depth: usize) -> Result<Value, ParseError> {
        if depth > MAX_DEPTH {
            return Err(self.err("nesting too deep"));
        }
        match self.peek() {
            None => Err(self.err("unexpected end of input")),
            Some(b'{') => self.object(depth),
            Some(b'[') => self.array(depth),
            Some(b'"') => Ok(Value::String(self.string()?)),
            Some(b't') => self.expect_lit("true", Value::Bool(true)),
            Some(b'f') => self.expect_lit("false", Value::Bool(false)),
            Some(b'n') => self.expect_lit("null", Value::Null),
            Some(b'-') | Some(b'0'..=b'9') => self.number(),
            Some(_) => Err(self.err("unexpected character")),
        }
    }

    fn array(&mut self, depth: usize) -> Result<Value, ParseError> {
        self.i += 1; // [
        let mut items = Vec::new();
        self.ws();
        if self.peek() == Some(b']') {
            self.i += 1;
            return Ok(Value::Array(items));
        }
        loop {
            self.ws();
            items.push(self.value(depth + 1)?);
            self.ws();
            match self.peek() {
                Some(b',') => self.i += 1,
                Some(b']') => {
                    self.i += 1;
                    return Ok(Value::Array(items));
                }
                _ => return Err(self.err("expected ',' or ']'")),
            }
        }
    }

    fn object(&mut self, depth: usize) -> Result<Value, ParseError> {
        self.i += 1; // {
        let mut entries = Vec::new();
        self.ws();
        if self.peek() == Some(b'}') {
            self.i += 1;
            return Ok(Value::Object(entries));
        }
        loop {
            self.ws();
            if self.peek() != Some(b'"') {
                return Err(self.err("expected string key"));
            }
            let k = self.string()?;
            self.ws();
            if self.peek() != Some(b':') {
                return Err(self.err("expected ':'"));
            }
            self.i += 1;
            self.ws();
            let v = self.value(depth + 1)?;
            entries.push((k, v));
            self.ws();
            match self.peek() {
                Some(b',') => self.i += 1,
                Some(b'}') => {
                    self.i += 1;
                    return Ok(Value::Object(entries));
                }
                _ => return Err(self.err("expected ',' or '}'")),
            }
        }
    }

    fn number(&mut self) -> Result<Value, ParseError> {
        let start = self.i;
        if self.peek() == Some(b'-') {
            self.i += 1;
        }
        match self.peek() {
            Some(b'0') => self.i += 1,
            Some(b'1'..=b'9') => {
                while matches!(self.peek(), Some(b'0'..=b'9')) {
                    self.i += 1;
                }
            }
            _ => return Err(self.err("invalid number")),
        }
        if self.peek() == Some(b'.') {
            self.i += 1;
            if !matches!(self.peek(), Some(b'0'..=b'9')) {
                return Err(self.err("invalid number"));
            }
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.i += 1;
            }
        }
        if matches!(self.peek(), Some(b'e') | Some(b'E')) {
            self.i += 1;
            if matches!(self.peek(), Some(b'+') | Some(b'-')) {
                self.i += 1;
            }
            if !matches!(self.peek(), Some(b'0'..=b'9')) {
                return Err(self.err("invalid number"));
            }
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.i += 1;
            }
        }
        Ok(Value::Number(self.src[start..self.i].to_string()))
    }

    fn hex4(&mut self) -> Result<u32, ParseError> {
        if self.i + 4 > self.s.len() {
            return Err(self.err("truncated \\u escape"));
        }
        let h = std::str::from_utf8(&self.s[self.i..self.i + 4]).ok();
        let n = h.and_then(|h| u32::from_str_radix(h, 16).ok());
        match n {
            Some(n) => {
                self.i += 4;
                Ok(n)
            }
            None => Err(self.err("invalid \\u escape")),
        }
    }

    fn string(&mut self) -> Result<String, ParseError> {
        self.i += 1; // opening quote
        let mut out = String::new();
        loop {
            // Copy a run of plain bytes in one go.
            let run_start = self.i;
            while self.i < self.s.len() {
                let b = self.s[self.i];
                if b == b'"' || b == b'\\' || b < 0x20 {
                    break;
                }
                self.i += 1;
            }
            out.push_str(&self.src[run_start..self.i]);
            match self.peek() {
                None => return Err(self.err("unterminated string")),
                Some(b'"') => {
                    self.i += 1;
                    return Ok(out);
                }
                Some(b'\\') => {
                    self.i += 1;
                    let c = self.peek().ok_or_else(|| self.err("unterminated escape"))?;
                    self.i += 1;
                    match c {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let hi = self.hex4()?;
                            let cp = if (0xD800..0xDC00).contains(&hi) {
                                // Try to combine with a following low surrogate.
                                if self.s[self.i..].starts_with(b"\\u") {
                                    let save = self.i;
                                    self.i += 2;
                                    let lo = self.hex4()?;
                                    if (0xDC00..0xE000).contains(&lo) {
                                        0x10000 + ((hi - 0xD800) << 10) + (lo - 0xDC00)
                                    } else {
                                        self.i = save;
                                        0xFFFD
                                    }
                                } else {
                                    0xFFFD
                                }
                            } else if (0xDC00..0xE000).contains(&hi) {
                                0xFFFD
                            } else {
                                hi
                            };
                            out.push(char::from_u32(cp).unwrap_or('\u{FFFD}'));
                        }
                        _ => return Err(self.err("invalid escape")),
                    }
                }
                Some(_) => return Err(self.err("control character in string")),
            }
        }
    }
}

/// How a JSON document is laid out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    /// No whitespace at all.
    Compact,
    /// One item per line, indented by this many spaces per level.
    Pretty(usize),
}

/// Render a value as JSON text.
pub fn to_string(v: &Value, layout: Layout) -> String {
    let mut out = String::new();
    write_value(&mut out, v, layout, 0);
    out
}

fn indent(out: &mut String, layout: Layout, depth: usize) {
    if let Layout::Pretty(w) = layout {
        for _ in 0..w * depth {
            out.push(' ');
        }
    }
}

fn write_value(out: &mut String, v: &Value, layout: Layout, depth: usize) {
    match v {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => out.push_str(n),
        Value::String(s) => write_string(out, s),
        Value::Array(items) => {
            if items.is_empty() {
                out.push_str("[]");
                return;
            }
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                if let Layout::Pretty(_) = layout {
                    out.push('\n');
                    indent(out, layout, depth + 1);
                }
                write_value(out, item, layout, depth + 1);
            }
            if let Layout::Pretty(_) = layout {
                out.push('\n');
                indent(out, layout, depth);
            }
            out.push(']');
        }
        Value::Object(entries) => {
            if entries.is_empty() {
                out.push_str("{}");
                return;
            }
            out.push('{');
            for (i, (k, val)) in entries.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                if let Layout::Pretty(_) = layout {
                    out.push('\n');
                    indent(out, layout, depth + 1);
                }
                write_string(out, k);
                out.push(':');
                if let Layout::Pretty(_) = layout {
                    out.push(' ');
                }
                write_value(out, val, layout, depth + 1);
            }
            if let Layout::Pretty(_) = layout {
                out.push('\n');
                indent(out, layout, depth);
            }
            out.push('}');
        }
    }
}

/// Write a JSON string literal. Non-ASCII text is emitted as-is (not
/// `\u`-escaped) since budgets are measured in characters.
pub fn write_string(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

/// Rendered length, in characters, of one character inside a JSON string.
#[inline]
pub fn escaped_char_len(c: char) -> usize {
    match c {
        '"' | '\\' | '\n' | '\r' | '\t' | '\u{8}' | '\u{c}' => 2,
        c if (c as u32) < 0x20 => 6,
        _ => 1,
    }
}

/// Rendered length, in characters, of a JSON string literal (with quotes).
pub fn string_len(s: &str) -> usize {
    2 + s.chars().map(escaped_char_len).sum::<usize>()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_preserves_order_and_numbers() {
        let src =
            r#"{"z":1,"a":[1.50,-0,1e400,12345678901234567890],"s":"hé\n\"q\"","n":null,"t":true}"#;
        let v = parse(src).unwrap();
        assert_eq!(
            to_string(&v, Layout::Compact),
            r#"{"z":1,"a":[1.50,-0,1e400,12345678901234567890],"s":"hé\n\"q\"","n":null,"t":true}"#
        );
    }

    #[test]
    fn pretty_matches_js_layout() {
        let v = parse(r#"{"a":[1,{"b":[]},{}],"c":"x"}"#).unwrap();
        let expected = "{\n  \"a\": [\n    1,\n    {\n      \"b\": []\n    },\n    {}\n  ],\n  \"c\": \"x\"\n}";
        assert_eq!(to_string(&v, Layout::Pretty(2)), expected);
    }

    #[test]
    fn surrogate_pairs() {
        let v = parse(r#""😀 \ud800""#).unwrap();
        assert_eq!(v, Value::String("😀 \u{FFFD}".into()));
    }

    #[test]
    fn rejects_bad_input() {
        for bad in [
            "",
            "{",
            "[1,]",
            "{\"a\" 1}",
            "01",
            "1.",
            "\"\u{1}\"",
            "nul",
            "[1] x",
            "{'a':1}",
        ] {
            assert!(parse(bad).is_err(), "should reject {bad:?}");
        }
    }

    #[test]
    fn depth_limit() {
        let deep = "[".repeat(MAX_DEPTH + 2) + &"]".repeat(MAX_DEPTH + 2);
        assert!(parse(&deep).is_err());
        let ok = "[".repeat(100) + &"]".repeat(100);
        assert!(parse(&ok).is_ok());
    }

    #[test]
    fn string_len_matches_writer() {
        for s in ["", "abc", "q\"\\", "\u{1}\u{1f}\n\t", "héllo 😀"] {
            let mut out = String::new();
            write_string(&mut out, s);
            assert_eq!(out.chars().count(), string_len(s));
        }
    }
}
