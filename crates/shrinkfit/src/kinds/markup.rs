//! HTML and XML text: tags become words (HTML) or data (XML).

use crate::value::Value;

/// Is `s` mostly markup? `Some(true)` for XML, `Some(false)` for HTML.
pub(super) fn detect(s: &str) -> Option<bool> {
    let head: String = s
        .trim_start()
        .chars()
        .take(64)
        .collect::<String>()
        .to_ascii_lowercase();
    let declared_xml = head.starts_with("<?xml");
    let declared_html = head.starts_with("<!doctype html") || head.starts_with("<html");
    let (mut tags, mut tag_chars, mut html_tags) = (0usize, 0usize, 0usize);
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'<'
            && i + 1 < b.len()
            && (b[i + 1].is_ascii_alphabetic() || matches!(b[i + 1], b'/' | b'!' | b'?'))
        {
            // A tag ends at the next '>' unless a '<' comes first.
            let mut j = i + 1;
            while j < b.len() && b[j] != b'>' && b[j] != b'<' && j - i < 2000 {
                j += 1;
            }
            if j < b.len() && b[j] == b'>' {
                tags += 1;
                tag_chars += j + 1 - i;
                let name = tag_name(&s[i + 1..j]);
                if is_html_name(&name) {
                    html_tags += 1;
                }
                i = j + 1;
                continue;
            }
        }
        i += 1;
    }
    let share = tag_chars as f64 / s.len().max(1) as f64;
    let markup = declared_xml || declared_html || (tags >= 3 && share >= 0.2);
    if !markup {
        return None;
    }
    Some(declared_xml || (!declared_html && html_tags == 0))
}

fn tag_name(inner: &str) -> String {
    inner
        .trim_start_matches('/')
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | ':' | '.'))
        .collect::<String>()
        .to_ascii_lowercase()
}

fn is_html_name(n: &str) -> bool {
    matches!(
        n,
        "html"
            | "body"
            | "div"
            | "span"
            | "p"
            | "a"
            | "br"
            | "ul"
            | "ol"
            | "li"
            | "b"
            | "i"
            | "em"
            | "strong"
            | "table"
            | "tr"
            | "td"
            | "th"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "img"
            | "pre"
            | "code"
            | "section"
            | "article"
            | "head"
            | "title"
            | "script"
            | "style"
    )
}

#[derive(Debug, PartialEq)]
enum Tok {
    Text(String),
    Open {
        name: String,
        attrs: Vec<(String, String)>,
        empty: bool,
    },
    Close(String),
}

/// Split markup into tokens. Comments, doctype and processing instructions are
/// dropped; CDATA becomes text; `raw` names (script, style) have their content skipped.
fn tokenize(s: &str, html: bool) -> Vec<Tok> {
    let b = s.as_bytes();
    let mut toks = Vec::new();
    let mut text_start = 0;
    let mut i = 0;
    let flush = |toks: &mut Vec<Tok>, from: usize, to: usize| {
        if to > from {
            toks.push(Tok::Text(decode(&s[from..to])));
        }
    };
    while i < b.len() {
        if b[i] != b'<' {
            i += 1;
            continue;
        }
        let rest = &s[i..];
        if rest.starts_with("<!--") {
            flush(&mut toks, text_start, i);
            i = rest.find("-->").map_or(b.len(), |e| i + e + 3);
            text_start = i;
        } else if rest.starts_with("<![CDATA[") {
            flush(&mut toks, text_start, i);
            let end = rest.find("]]>").map_or(b.len(), |e| i + e);
            let from = (i + 9).min(end);
            if end > from {
                toks.push(Tok::Text(s[from..end].to_string()));
            }
            i = (end + 3).min(b.len());
            text_start = i;
        } else if rest.starts_with("<!") || rest.starts_with("<?") {
            flush(&mut toks, text_start, i);
            i = rest.find('>').map_or(b.len(), |e| i + e + 1);
            text_start = i;
        } else if rest.len() > 1
            && (b[i + 1].is_ascii_alphabetic()
                || (b[i + 1] == b'/' && rest.len() > 2 && b[i + 2].is_ascii_alphabetic()))
        {
            let closing = b[i + 1] == b'/';
            match parse_tag(s, i + if closing { 2 } else { 1 }, html) {
                Some((name, attrs, empty, end)) => {
                    flush(&mut toks, text_start, i);
                    i = end;
                    text_start = i;
                    if closing {
                        toks.push(Tok::Close(name));
                    } else {
                        let raw = html && !empty && matches!(name.as_str(), "script" | "style");
                        toks.push(Tok::Open {
                            name: name.clone(),
                            attrs,
                            empty,
                        });
                        if raw {
                            let close = format!("</{name}");
                            let lower = s[i..].to_ascii_lowercase();
                            let skip = lower.find(&close).map_or(b.len(), |e| i + e);
                            i = skip;
                            text_start = i;
                        }
                    }
                }
                None => i += 1,
            }
        } else {
            i += 1;
        }
    }
    flush(&mut toks, text_start, b.len());
    toks
}

/// Name, attributes, self-closing, and the offset after the `>` of a parsed tag.
type ParsedTag = (String, Vec<(String, String)>, bool, usize);

/// Parse a tag whose name starts at byte `at`. Returns the lowercased name,
/// attributes, whether it is self-closing and the byte offset after the '>'.
fn parse_tag(s: &str, at: usize, html: bool) -> Option<ParsedTag> {
    let b = s.as_bytes();
    let mut i = at;
    while i < b.len() && (b[i].is_ascii_alphanumeric() || matches!(b[i], b'-' | b'_' | b':' | b'.'))
    {
        i += 1;
    }
    // HTML names are case-insensitive; XML names are not.
    let name = if html {
        s[at..i].to_ascii_lowercase()
    } else {
        s[at..i].to_string()
    };
    let mut attrs = Vec::new();
    loop {
        while i < b.len() && (b[i].is_ascii_whitespace() || b[i] == b'/') {
            if b[i] == b'/' && i + 1 < b.len() && b[i + 1] == b'>' {
                return Some((name, attrs, true, i + 2));
            }
            i += 1;
        }
        if i >= b.len() {
            return None;
        }
        if b[i] == b'>' {
            return Some((name, attrs, false, i + 1));
        }
        if b[i] == b'<' {
            return None;
        }
        let ks = i;
        while i < b.len()
            && !b[i].is_ascii_whitespace()
            && !matches!(b[i], b'=' | b'>' | b'/' | b'<')
        {
            i += 1;
        }
        if i == ks {
            i += 1;
            continue;
        }
        let key = if html {
            s[ks..i].to_ascii_lowercase()
        } else {
            s[ks..i].to_string()
        };
        while i < b.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
        let mut val = String::new();
        if i < b.len() && b[i] == b'=' {
            i += 1;
            while i < b.len() && b[i].is_ascii_whitespace() {
                i += 1;
            }
            if i < b.len() && (b[i] == b'"' || b[i] == b'\'') {
                let q = b[i];
                let vs = i + 1;
                let mut j = vs;
                while j < b.len() && b[j] != q {
                    j += 1;
                }
                if j >= b.len() {
                    return None;
                }
                val = decode(&s[vs..j]);
                i = j + 1;
            } else {
                let vs = i;
                while i < b.len() && !b[i].is_ascii_whitespace() && b[i] != b'>' {
                    i += 1;
                }
                val = decode(&s[vs..i]);
            }
        }
        attrs.push((key, val));
    }
}

/// Decode the common named and numeric character references.
fn decode(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(p) = rest.find('&') {
        out.push_str(&rest[..p]);
        rest = &rest[p..];
        let end = rest.find(';').filter(|&e| e <= 10);
        let decoded = end.and_then(|e| entity(&rest[1..e]).map(|c| (c, e)));
        match decoded {
            Some((c, e)) => {
                out.push(c);
                rest = &rest[e + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn entity(name: &str) -> Option<char> {
    if let Some(num) = name.strip_prefix('#') {
        let code = match num.strip_prefix(['x', 'X']) {
            Some(h) => u32::from_str_radix(h, 16).ok()?,
            None => num.parse().ok()?,
        };
        return char::from_u32(code);
    }
    Some(match name {
        "amp" => '&',
        "lt" => '<',
        "gt" => '>',
        "quot" => '"',
        "apos" => '\'',
        "nbsp" => ' ',
        "ndash" => '–',
        "mdash" => '—',
        "hellip" => '…',
        "lsquo" => '‘',
        "rsquo" => '’',
        "ldquo" => '“',
        "rdquo" => '”',
        "copy" => '©',
        "reg" => '®',
        "trade" => '™',
        "bull" => '•',
        "middot" => '·',
        "laquo" => '«',
        "raquo" => '»',
        "euro" => '€',
        "pound" => '£',
        "times" => '×',
        _ => return None,
    })
}

// ---- HTML to text ----

struct Text {
    out: String,
}

impl Text {
    fn ends_with_newlines(&self) -> usize {
        self.out.chars().rev().take_while(|&c| c == '\n').count()
    }

    /// Make sure at least `n` line breaks end the text (none at the very start).
    fn breaks(&mut self, n: usize) {
        if self.out.is_empty() {
            return;
        }
        let have = self.ends_with_newlines();
        for _ in have..n {
            self.out.push('\n');
        }
    }

    fn word(&mut self, w: &str) {
        self.out.push_str(w);
    }

    /// Inline text: whitespace runs collapse to one space, none at a line start.
    fn inline(&mut self, t: &str) {
        let mut prev_space = self.out.is_empty() || self.out.ends_with([' ', '\n']);
        for c in t.chars() {
            if c.is_whitespace() {
                if !prev_space {
                    self.out.push(' ');
                    prev_space = true;
                }
            } else {
                self.out.push(c);
                prev_space = false;
            }
        }
    }
}

const SKIP: [&str; 6] = ["script", "style", "noscript", "svg", "template", "head"];

/// Readable text of an HTML fragment: block tags become line breaks, list items
/// bullets, headings `#` lines, links `text <url>`, images `[image: alt]`.
pub(super) fn html_to_text(s: &str) -> String {
    let toks = tokenize(s, true);
    let mut t = Text { out: String::new() };
    let mut skip_depth = 0usize;
    let mut pre = 0usize;
    let mut links: Vec<Option<String>> = Vec::new();
    let mut link_start: Vec<usize> = Vec::new();
    let mut in_title = false;
    for tok in toks {
        match tok {
            Tok::Text(x) => {
                if skip_depth > 0 && !in_title {
                    continue;
                }
                if pre > 0 {
                    t.word(&x);
                } else {
                    t.inline(&x);
                }
            }
            Tok::Open { name, attrs, empty } => {
                if name == "title" {
                    in_title = true;
                    t.breaks(2);
                    continue;
                }
                if SKIP.contains(&name.as_str()) {
                    if !empty {
                        skip_depth += 1;
                    }
                    continue;
                }
                if skip_depth > 0 {
                    continue;
                }
                match name.as_str() {
                    "br" => t.breaks(1),
                    "p" | "blockquote" | "hr" => t.breaks(2),
                    "pre" => {
                        t.breaks(2);
                        pre += 1;
                    }
                    "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                        t.breaks(2);
                        let level = (name.as_bytes()[1] - b'0') as usize;
                        t.word(&format!("{} ", "#".repeat(level)));
                    }
                    "li" => {
                        t.breaks(1);
                        t.word("- ");
                    }
                    "td" | "th" => {
                        if !t.out.is_empty() && !t.out.ends_with('\n') {
                            t.word(" | ");
                        }
                    }
                    "a" => {
                        let href = attrs
                            .iter()
                            .find(|(k, _)| k == "href")
                            .map(|(_, v)| v.clone());
                        links.push(href.filter(|h| {
                            !h.is_empty() && !h.starts_with('#') && !h.starts_with("javascript:")
                        }));
                        link_start.push(t.out.len());
                    }
                    "img" => {
                        let alt = attrs
                            .iter()
                            .find(|(k, _)| k == "alt")
                            .map(|(_, v)| v.trim().to_string())
                            .unwrap_or_default();
                        if !alt.is_empty() {
                            t.inline(&format!("[image: {alt}]"));
                        }
                    }
                    "div" | "section" | "article" | "header" | "footer" | "nav" | "main"
                    | "aside" | "ul" | "ol" | "table" | "tr" | "form" | "dl" | "dt" | "dd"
                    | "figure" | "figcaption" | "details" | "summary" | "address" | "thead"
                    | "tbody" => t.breaks(1),
                    _ => {}
                }
            }
            Tok::Close(name) => {
                if name == "title" {
                    in_title = false;
                    t.breaks(2);
                    continue;
                }
                if SKIP.contains(&name.as_str()) {
                    skip_depth = skip_depth.saturating_sub(1);
                    continue;
                }
                if skip_depth > 0 {
                    continue;
                }
                match name.as_str() {
                    "p" | "blockquote" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => t.breaks(2),
                    "pre" => {
                        pre = pre.saturating_sub(1);
                        t.breaks(2);
                    }
                    "ul" | "ol" | "table" => t.breaks(2),
                    "div" | "section" | "article" | "header" | "footer" | "nav" | "main"
                    | "aside" | "tr" | "li" | "form" | "dl" | "figure" | "details" => t.breaks(1),
                    "a" => {
                        if let (Some(Some(href)), Some(start)) = (links.pop(), link_start.pop()) {
                            let shown = t.out[start..].trim();
                            if shown != href {
                                t.word(&format!(" <{href}>"));
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    tidy(&t.out)
}

/// Trim trailing spaces on every line and collapse runs of blank lines.
fn tidy(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut blank = 0;
    for line in s.lines() {
        let line = line.trim_end();
        if line.trim().is_empty() {
            blank += 1;
            if blank > 1 || out.is_empty() {
                continue;
            }
        } else {
            blank = 0;
        }
        out.push_str(line);
        out.push('\n');
    }
    out.trim_end().to_string()
}

// ---- XML as data ----

struct Node {
    name: String,
    attrs: Vec<(String, String)>,
    text: String,
    kids: Vec<Node>,
}

/// An XML document as the value it describes: an element is an object (its
/// attributes as `@name`, its text as `#text`), repeated child elements become
/// an array, and an element holding only text is that text. The pruner then
/// treats a SOAP reply or a feed like any other list of records.
pub(super) fn xml_to_value(s: &str) -> Option<Value> {
    let toks = tokenize(s, false);
    let mut stack: Vec<Node> = vec![Node {
        name: String::new(),
        attrs: Vec::new(),
        text: String::new(),
        kids: Vec::new(),
    }];
    for tok in toks {
        match tok {
            Tok::Text(x) => {
                let x = x.split_whitespace().collect::<Vec<_>>().join(" ");
                if !x.is_empty() {
                    let top = stack.last_mut().expect("root");
                    if !top.text.is_empty() {
                        top.text.push(' ');
                    }
                    top.text.push_str(&x);
                }
            }
            Tok::Open { name, attrs, empty } => {
                let attrs = attrs
                    .into_iter()
                    .filter(|(k, _)| !k.starts_with("xmlns"))
                    .collect();
                let node = Node {
                    name,
                    attrs,
                    text: String::new(),
                    kids: Vec::new(),
                };
                if empty || stack.len() > 64 {
                    stack.last_mut().expect("root").kids.push(node);
                } else {
                    stack.push(node);
                }
            }
            Tok::Close(name) => {
                if let Some(pos) = stack.iter().rposition(|n| n.name == name) {
                    if pos > 0 {
                        while stack.len() > pos {
                            let done = stack.pop().expect("non-empty");
                            stack.last_mut().expect("root").kids.push(done);
                        }
                    }
                }
            }
        }
    }
    while stack.len() > 1 {
        let done = stack.pop().expect("non-empty");
        stack.last_mut().expect("root").kids.push(done);
    }
    let root = stack.pop().expect("root");
    let first = root.kids.first()?;
    Some(Value::Object(vec![(first.name.clone(), node_value(first))]))
}

fn node_value(n: &Node) -> Value {
    if n.attrs.is_empty() && n.kids.is_empty() {
        return if n.text.is_empty() {
            Value::Null
        } else {
            Value::String(n.text.clone())
        };
    }
    let mut entries: Vec<(String, Value)> = n
        .attrs
        .iter()
        .map(|(k, v)| (format!("@{k}"), Value::String(v.clone())))
        .collect();
    let mut groups: Vec<(String, Vec<Value>)> = Vec::new();
    for k in &n.kids {
        let v = node_value(k);
        match groups.iter_mut().find(|(name, _)| *name == k.name) {
            Some((_, list)) => list.push(v),
            None => groups.push((k.name.clone(), vec![v])),
        }
    }
    for (name, mut list) in groups {
        entries.push((
            name,
            if list.len() == 1 {
                list.remove(0)
            } else {
                Value::Array(list)
            },
        ));
    }
    if !n.text.is_empty() {
        entries.push(("#text".to_string(), Value::String(n.text.clone())));
    }
    Value::Object(entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tells_markup_from_prose_with_a_tag() {
        assert_eq!(detect("<p>Hello <b>world</b></p><p>more</p>"), Some(false));
        assert_eq!(
            detect(r#"<?xml version="1.0"?><a><b>1</b></a>"#),
            Some(true)
        );
        assert_eq!(
            detect("<root><item id=\"1\">a</item><item id=\"2\">b</item></root>"),
            Some(true)
        );
        let md = "## Heading\nSome long text about <em>things</em> that goes on and on and on and on and on and on and on and on and on and on.";
        assert_eq!(detect(md), None);
        assert_eq!(detect("a < b and c > d, <not a tag"), None);
    }

    #[test]
    fn html_becomes_text_with_structure() {
        let h = r#"<html><head><title>Docs</title><style>p{color:red}</style></head><body>
            <h2>Install</h2><p>Run <code>pip install x</code> &amp; see <a href="https://example.com/docs">the docs</a>.</p>
            <ul><li>one</li><li>two &mdash; three</li></ul><script>alert(1)</script>
            <table><tr><th>a</th><th>b</th></tr><tr><td>1</td><td>2</td></tr></table><img src="x.png" alt="logo"></body></html>"#;
        let t = html_to_text(h);
        assert_eq!(
            t,
            "Docs\n\n## Install\n\nRun pip install x & see the docs <https://example.com/docs>.\n\n- one\n- two — three\n\na | b\n1 | 2

[image: logo]"
        );
    }

    #[test]
    fn wikipedia_snippet_loses_its_spans() {
        let s = r#"<span class="searchmatch">Rust</span> is a <span class="searchmatch">programming</span> language &quot;focused&quot; on safety"#;
        assert_eq!(
            html_to_text(s),
            "Rust is a programming language \"focused\" on safety"
        );
    }

    #[test]
    fn xml_becomes_data_with_case_kept() {
        let x = r#"<?xml version="1.0"?><Feed xmlns="urn:x"><Entry id="7"><Title>First</Title><Link href="http://a/b"/></Entry>
            <Entry id="8"><Title>Second &amp; last</Title></Entry><!-- c --></Feed>"#;
        let v = xml_to_value(x).unwrap();
        assert_eq!(
            crate::json::to_string(&v, crate::json::Layout::Compact),
            r#"{"Feed":{"Entry":[{"@id":"7","Title":"First","Link":{"@href":"http://a/b"}},{"@id":"8","Title":"Second & last"}]}}"#
        );
    }

    #[test]
    fn malformed_markup_does_not_panic() {
        for s in [
            "<a",
            "<a href=\"x",
            "</",
            "<<>>",
            "<!--",
            "<![CDATA[x",
            "<a b=>x</a",
            "&#xZZ; &amp",
            "<p>é<b>ü</p>",
        ] {
            let _ = html_to_text(s);
            let _ = xml_to_value(s);
            let _ = detect(s);
        }
    }
}
