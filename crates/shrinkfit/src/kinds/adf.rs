//! Atlassian Document Format (the rich text of Jira and Confluence) as text.
//!
//! An ADF document is a deep tree of `{"type":"paragraph","content":[{"type":"text",...}]}`
//! nodes. Almost all of its size is that scaffolding. This reads it as the
//! text it holds: paragraphs, headings (`#`), lists, code blocks, tables,
//! links (`text <url>`), mentions, status lozenges and attachments.

use crate::value::Value;

fn get<'a>(entries: &'a [(String, Value)], key: &str) -> Option<&'a Value> {
    entries.iter().find(|(k, _)| k == key).map(|(_, v)| v)
}

fn str_of<'a>(entries: &'a [(String, Value)], key: &str) -> Option<&'a str> {
    match get(entries, key) {
        Some(Value::String(s)) => Some(s),
        _ => None,
    }
}

/// Is this object the root of an ADF document?
pub(crate) fn is_adf(entries: &[(String, Value)]) -> bool {
    str_of(entries, "type") == Some("doc")
        && matches!(get(entries, "content"), Some(Value::Array(_)))
}

/// The text of an ADF document.
pub(crate) fn to_text(entries: &[(String, Value)]) -> String {
    let mut out = String::new();
    if let Some(Value::Array(nodes)) = get(entries, "content") {
        blocks(nodes, 0, &mut out);
    }
    tidy(&out)
}

fn attrs(entries: &[(String, Value)]) -> &[(String, Value)] {
    match get(entries, "attrs") {
        Some(Value::Object(a)) => a,
        _ => &[],
    }
}

fn children(entries: &[(String, Value)]) -> &[Value] {
    match get(entries, "content") {
        Some(Value::Array(c)) => c,
        _ => &[],
    }
}

fn blocks(nodes: &[Value], indent: usize, out: &mut String) {
    for n in nodes {
        if let Value::Object(e) = n {
            block(e, indent, out);
        }
    }
}

fn pad(indent: usize) -> String {
    "  ".repeat(indent)
}

fn block(e: &[(String, Value)], indent: usize, out: &mut String) {
    let ty = str_of(e, "type").unwrap_or("");
    match ty {
        "paragraph" => {
            out.push_str(&pad(indent));
            inline(children(e), out);
            out.push_str("\n\n");
        }
        "heading" => {
            let level = match get(attrs(e), "level") {
                Some(Value::Number(n)) => n.parse::<usize>().unwrap_or(1).clamp(1, 6),
                _ => 1,
            };
            out.push_str(&format!("{} ", "#".repeat(level)));
            inline(children(e), out);
            out.push_str("\n\n");
        }
        "bulletList" | "orderedList" | "taskList" | "decisionList" => {
            for (i, item) in children(e).iter().enumerate() {
                if let Value::Object(it) = item {
                    let marker = match (ty, str_of(attrs(it), "state")) {
                        ("orderedList", _) => format!("{}. ", i + 1),
                        ("taskList", Some("DONE")) => "[x] ".to_string(),
                        ("taskList", _) => "[ ] ".to_string(),
                        _ => "- ".to_string(),
                    };
                    out.push_str(&pad(indent));
                    out.push_str(&marker);
                    list_item(children(it), indent, out);
                }
            }
            if indent == 0 {
                out.push('\n');
            }
        }
        "codeBlock" => {
            let lang = str_of(attrs(e), "language").unwrap_or("");
            out.push_str(&format!("```{lang}\n"));
            inline(children(e), out);
            out.push_str("\n```\n\n");
        }
        "blockquote" => {
            let mut inner = String::new();
            blocks(children(e), 0, &mut inner);
            for l in tidy(&inner).lines() {
                out.push_str(&format!("> {l}\n"));
            }
            out.push('\n');
        }
        "rule" => out.push_str("---\n\n"),
        "panel" => {
            let kind = str_of(attrs(e), "panelType").unwrap_or("info");
            out.push_str(&format!("[{kind}] "));
            let mut inner = String::new();
            blocks(children(e), 0, &mut inner);
            out.push_str(tidy(&inner).trim());
            out.push_str("\n\n");
        }
        "expand" | "nestedExpand" => {
            if let Some(t) = str_of(attrs(e), "title") {
                out.push_str(&format!("{t}\n"));
            }
            blocks(children(e), indent, out);
        }
        "table" => {
            for row in children(e) {
                if let Value::Object(r) = row {
                    let cells: Vec<String> = children(r)
                        .iter()
                        .filter_map(|c| {
                            if let Value::Object(c) = c {
                                Some(c)
                            } else {
                                None
                            }
                        })
                        .map(|c| {
                            let mut inner = String::new();
                            blocks(children(c), 0, &mut inner);
                            tidy(&inner).replace('\n', " ")
                        })
                        .collect();
                    out.push_str(&cells.join(" | "));
                    out.push('\n');
                }
            }
            out.push('\n');
        }
        "mediaSingle" | "mediaGroup" => blocks(children(e), indent, out),
        "media" => {
            let a = attrs(e);
            let name = str_of(a, "alt")
                .or_else(|| str_of(a, "id"))
                .unwrap_or("file");
            out.push_str(&format!("[attachment: {name}]\n\n"));
        }
        "blockCard" | "embedCard" => {
            out.push_str(str_of(attrs(e), "url").unwrap_or("[card]"));
            out.push_str("\n\n");
        }
        _ => {
            // Unknown block, or an inline node found at block level.
            if children(e).is_empty() {
                inline_node(e, out);
                out.push('\n');
            } else {
                blocks(children(e), indent, out);
            }
        }
    }
}

fn list_item(nodes: &[Value], indent: usize, out: &mut String) {
    let mut first = true;
    for n in nodes {
        let Value::Object(e) = n else { continue };
        match str_of(e, "type") {
            Some("paragraph") => {
                if !first {
                    out.push_str(&pad(indent + 1));
                }
                inline(children(e), out);
                out.push('\n');
            }
            Some("bulletList" | "orderedList" | "taskList") => {
                if first {
                    out.push('\n');
                }
                block(e, indent + 1, out);
            }
            _ => {
                let mut inner = String::new();
                block(e, indent + 1, &mut inner);
                out.push_str(inner.trim_end());
                out.push('\n');
            }
        }
        first = false;
    }
    if first {
        out.push('\n');
    }
}

fn inline(nodes: &[Value], out: &mut String) {
    for n in nodes {
        if let Value::Object(e) = n {
            inline_node(e, out);
        }
    }
}

fn inline_node(e: &[(String, Value)], out: &mut String) {
    match str_of(e, "type").unwrap_or("") {
        "text" => {
            let text = str_of(e, "text").unwrap_or("");
            let link = match get(e, "marks") {
                Some(Value::Array(marks)) => marks.iter().find_map(|m| match m {
                    Value::Object(m) if str_of(m, "type") == Some("link") => {
                        str_of(attrs(m), "href")
                    }
                    _ => None,
                }),
                _ => None,
            };
            out.push_str(text);
            if let Some(h) = link {
                if h != text {
                    out.push_str(&format!(" <{h}>"));
                }
            }
        }
        "hardBreak" => out.push('\n'),
        "mention" => {
            let a = attrs(e);
            let who = str_of(a, "text")
                .or_else(|| str_of(a, "id"))
                .unwrap_or("someone");
            if who.starts_with('@') {
                out.push_str(who);
            } else {
                out.push_str(&format!("@{who}"));
            }
        }
        "emoji" => {
            let a = attrs(e);
            out.push_str(
                str_of(a, "text")
                    .or_else(|| str_of(a, "shortName"))
                    .unwrap_or(""),
            );
        }
        "inlineCard" => out.push_str(str_of(attrs(e), "url").unwrap_or("[card]")),
        "status" => out.push_str(&format!(
            "[{}]",
            str_of(attrs(e), "text").unwrap_or("status")
        )),
        "date" => match get(attrs(e), "timestamp") {
            Some(Value::String(t)) | Some(Value::Number(t)) => out.push_str(&date(t)),
            _ => out.push_str("[date]"),
        },
        _ => inline(children(e), out),
    }
}

/// `1700000000000` (milliseconds since the epoch) as `2023-11-14`.
fn date(ms: &str) -> String {
    let Ok(ms) = ms.trim().parse::<i64>() else {
        return ms.to_string();
    };
    let z = ms.div_euclid(86_400_000) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

fn tidy(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut blank = 0;
    for line in s.lines() {
        let line = line.trim_end();
        if line.is_empty() {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json::parse;

    fn text(j: &str) -> String {
        let Value::Object(e) = parse(j).unwrap() else {
            panic!()
        };
        assert!(is_adf(&e));
        to_text(&e)
    }

    #[test]
    fn paragraphs_headings_lists_links_and_mentions() {
        let j = r##"{"type":"doc","version":1,"content":[
            {"type":"heading","attrs":{"level":2},"content":[{"type":"text","text":"Plan"}]},
            {"type":"paragraph","content":[{"type":"text","text":"Ask "},{"type":"mention","attrs":{"id":"u1","text":"@Sam"}},
              {"type":"text","text":" about "},{"type":"text","text":"the spec","marks":[{"type":"link","attrs":{"href":"https://x.test/spec"}}]},{"type":"text","text":"."}]},
            {"type":"bulletList","content":[
              {"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"one"}]}]},
              {"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"two"}]},
                {"type":"bulletList","content":[{"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"nested"}]}]}]}]}]},
            {"type":"orderedList","content":[{"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"first"}]}]}]},
            {"type":"codeBlock","attrs":{"language":"rust"},"content":[{"type":"text","text":"let x = 1;"}]},
            {"type":"paragraph","content":[{"type":"status","attrs":{"text":"DONE"}},{"type":"text","text":" on "},{"type":"date","attrs":{"timestamp":"1700000000000"}}]}
        ]}"##;
        assert_eq!(
            text(j),
            "## Plan\n\nAsk @Sam about the spec <https://x.test/spec>.\n\n- one\n- two\n  - nested\n\n1. first\n\n```rust\nlet x = 1;\n```\n\n[DONE] on 2023-11-14"
        );
    }

    #[test]
    fn tables_and_attachments() {
        let j = r#"{"type":"doc","version":1,"content":[
            {"type":"table","content":[
              {"type":"tableRow","content":[{"type":"tableHeader","content":[{"type":"paragraph","content":[{"type":"text","text":"a"}]}]},{"type":"tableHeader","content":[{"type":"paragraph","content":[{"type":"text","text":"b"}]}]}]},
              {"type":"tableRow","content":[{"type":"tableCell","content":[{"type":"paragraph","content":[{"type":"text","text":"1"}]}]},{"type":"tableCell","content":[{"type":"paragraph","content":[{"type":"text","text":"2"}]}]}]}]},
            {"type":"mediaSingle","content":[{"type":"media","attrs":{"id":"x","alt":"screenshot.png"}}]}]}"#;
        assert_eq!(text(j), "a | b\n1 | 2\n\n[attachment: screenshot.png]");
    }

    #[test]
    fn only_documents_qualify() {
        let Value::Object(e) = parse(r#"{"type":"paragraph","content":[]}"#).unwrap() else {
            panic!()
        };
        assert!(!is_adf(&e));
    }
}
