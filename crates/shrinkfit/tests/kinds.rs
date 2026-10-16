//! Long strings that are not prose: logs, HTML, XML, JSON and rich text stored
//! as text, diffs, blobs. The facts a reader needs must survive a tight budget.

use shrinkfit::json::{self, Layout};
use shrinkfit::value::Value;
use shrinkfit::{shrink, shrink_json, ArrayStrategy, Options};

fn opts(n: usize) -> Options {
    Options {
        indent: 0,
        array_strategy: ArrayStrategy::Condense,
        ..Options::new(n)
    }
}

/// Shrink `{"id": 7, "payload": <text>}` and return the payload text and the report.
fn payload(text: &str, n: usize) -> (String, shrinkfit::Report) {
    let doc = Value::Object(vec![
        ("id".into(), Value::Number("7".into())),
        ("payload".into(), Value::String(text.to_string())),
    ]);
    let out = shrink(&doc, &opts(n));
    assert!(
        out.report.output_chars <= n,
        "{} > {n}",
        out.report.output_chars
    );
    let Value::Object(e) = &out.value else {
        panic!("{}", out.text)
    };
    assert_eq!(e[0].1, Value::Number("7".into()), "{}", out.text);
    let Value::String(s) = &e[1].1 else {
        panic!("payload is not a string: {}", out.text)
    };
    (s.clone(), out.report)
}

fn log() -> String {
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
    v[167] =
        "2026-10-16T05:02:47Z FATAL worker-3 connection pool exhausted: 0/32 connections available"
            .into();
    v[168] = "Traceback (most recent call last):\n  File \"app/db.py\", line 88, in acquire\n    raise PoolExhausted(name)\nPoolExhausted: orders-primary".into();
    v.join("\n")
}

#[test]
fn a_log_keeps_its_error_and_exception_at_any_budget() {
    for n in [400, 800, 1500, 3000] {
        let (p, report) = payload(&log(), n);
        assert!(p.contains("FATAL worker-3"), "{n}: {p}");
        assert!(p.contains("PoolExhausted: orders-primary"), "{n}: {p}");
        assert!(p.contains("lines omitted"), "{n}: {p}");
        assert!(report.stats.strings_reshaped >= 1, "{n}");
    }
}

#[test]
fn command_output_loses_ansi_codes_and_progress_overwrites() {
    let mut v = vec!["\u{1b}[1m\u{1b}[32m   Downloading\u{1b}[0m deps".to_string()];
    v.extend((1..100).map(|i| format!("  % Total\r{i}%  {}  {i}M\r", i * 1000)));
    v.push("scp: /srv/data/export.csv: Permission denied".into());
    v.push("Exit code: 1".into());
    let (p, _) = payload(&v.join("\n"), 700);
    assert!(!p.contains('\u{1b}') && !p.contains('\r'), "{p:?}");
    assert!(
        p.contains("Permission denied") && p.contains("Exit code: 1"),
        "{p}"
    );
}

#[test]
fn a_diff_keeps_file_names_and_changes_whole() {
    let mut d = String::new();
    for k in 0..12 {
        d.push_str(&format!("diff --git a/src/m{k}.rs b/src/m{k}.rs\nindex 1..2 100644\n--- a/src/m{k}.rs\n+++ b/src/m{k}.rs\n"));
        for h in 0..3 {
            d.push_str(&format!("@@ -{0},9 +{0},9 @@ impl T{k}\n context a\n context b\n context c\n-    old_{k}_{h}();\n+    new_{k}_{h}();\n context d\n context e\n", h * 30 + 1));
        }
    }
    let (p, _) = payload(&d, 1400);
    for k in 0..12 {
        assert!(
            p.contains(&format!("diff --git a/src/m{k}.rs")),
            "m{k} missing: {p}"
        );
    }
    assert_eq!(
        p.matches("-    old_").count(),
        p.matches("+    new_").count(),
        "{p}"
    );
    assert!(p.matches("-    old_").count() >= 3, "{p}");
}

#[test]
fn html_becomes_readable_text_and_keeps_links() {
    let mut paras: Vec<String> = (0..60)
        .map(|i| format!("<p class=\"c\"><span>Filler line {i} about nothing in particular, queue cache queue.</span></p>"))
        .collect();
    paras[30] = "<p>The <b>Zeta-9 protocol</b> is described on <a href=\"https://example.org/zeta9\">the standards page</a>.</p>".into();
    let html = format!(
        "<html><head><style>.c{{color:red}}</style></head><body>{}</body></html>",
        paras.concat()
    );
    let (p, report) = payload(&html, 700);
    assert!(!p.contains("<span") && !p.contains("class="), "{p}");
    assert!(
        p.contains("Zeta-9 protocol") && p.contains("https://example.org/zeta9"),
        "{p}"
    );
    assert!(report.stats.strings_reshaped >= 1);
}

#[test]
fn xml_is_read_as_data_and_sampled_like_json() {
    let items: String = (0..80)
        .map(|i| format!("<Item id=\"{i}\"><Sku>SKU-{i:03}</Sku><Name>thing {i}</Name></Item>"))
        .collect();
    let xml =
        format!("<?xml version=\"1.0\"?><Stock><Warehouse>Rotterdam-2</Warehouse>{items}</Stock>");
    let (p, _) = payload(&xml, 900);
    let parsed = json::parse(&p).unwrap_or_else(|e| panic!("not JSON: {e}: {p}"));
    let text = json::to_string(&parsed, Layout::Compact);
    assert!(
        text.contains("Rotterdam-2") && text.contains("\"Item\""),
        "{text}"
    );
    assert!(
        text.contains("SKU-000") && text.contains("SKU-079"),
        "{text}"
    );
}

#[test]
fn json_stored_as_text_is_fitted_as_json() {
    let recs: Vec<String> = (0..120)
        .map(|i| {
            format!("{{\"id\":{i},\"name\":\"item-{i}\",\"owner\":{{\"login\":\"user{i}\"}}}}")
        })
        .collect();
    let text = format!(
        "{{\"results\":[{}],\"next\":\"cur_8F3kQ\"}}",
        recs.join(",")
    );
    let (p, report) = payload(&text, 800);
    let parsed = json::parse(&p).unwrap_or_else(|e| panic!("not JSON: {e}: {p}"));
    let again = json::to_string(&parsed, Layout::Compact);
    assert!(again.contains("cur_8F3kQ"), "{again}");
    assert!(again.contains("item-119"), "{again}");
    assert!(report.stats.strings_reshaped >= 1);
}

fn adf() -> Value {
    let mut content: Vec<String> = (0..40)
        .map(|i| format!("{{\"type\":\"paragraph\",\"content\":[{{\"type\":\"text\",\"text\":\"Paragraph {i} says queue cache queue.\"}}]}}"))
        .collect();
    content[20] = "{\"type\":\"paragraph\",\"content\":[{\"type\":\"mention\",\"attrs\":{\"id\":\"u1\",\"text\":\"@Priya Nair\"}},{\"type\":\"text\",\"text\":\" owns the TLS follow-up \"},{\"type\":\"text\",\"text\":\"runbook\",\"marks\":[{\"type\":\"link\",\"attrs\":{\"href\":\"https://wiki.example/tls\"}}]}]}".into();
    json::parse(&format!(
        "{{\"type\":\"doc\",\"version\":1,\"content\":[{}]}}",
        content.join(",")
    ))
    .unwrap()
}

#[test]
fn rich_text_becomes_its_words_whether_object_or_text() {
    for as_text in [false, true] {
        let a = adf();
        let field = if as_text {
            Value::String(json::to_string(&a, Layout::Compact))
        } else {
            a
        };
        let doc = Value::Object(vec![
            ("key".into(), Value::String("OPS-1".into())),
            ("description".into(), field),
        ]);
        let out = shrink(&doc, &opts(700));
        assert!(out.report.output_chars <= 700);
        assert!(out.text.contains("OPS-1"), "{}", out.text);
        assert!(
            out.text.contains("@Priya Nair") && out.text.contains("https://wiki.example/tls"),
            "as_text={as_text}: {}",
            out.text
        );
        assert!(
            !out.text.contains("\"type\":\"paragraph\"") && !out.text.contains("\\\"type\\\""),
            "as_text={as_text}: {}",
            out.text
        );
    }
}

#[test]
fn blobs_do_not_crowd_out_their_neighbours() {
    let blob = "iVBORw0KGgoAAAANSUhEUgAAAMwAAADACAMAAAB/Pny7AAABLFBMVEX///8IT4ztICb5x8".repeat(400);
    let items: Vec<String> = (0..5)
        .map(|i| {
            format!("{{\"name\":\"Image {i}\",\"caption\":\"Caption {i}\",\"data\":\"{blob}\"}}")
        })
        .collect();
    let src = format!("{{\"items\":[{}]}}", items.join(","));
    let out = shrink_json(&src, &opts(900)).unwrap();
    for i in 0..5 {
        assert!(out.text.contains(&format!("Image {i}")), "{}", out.text);
    }
    assert!(out.text.contains("base64 blob"), "{}", out.text);
}

#[test]
fn protected_values_are_never_converted() {
    let html = format!(
        "<div>{}</div>",
        "<p>keep <b>this</b> exactly</p>".repeat(30)
    );
    let doc = Value::Object(vec![
        ("note".into(), Value::String(html.clone())),
        ("pad".into(), Value::String("x".repeat(3000))),
    ]);
    let o = Options {
        protect: vec!["note".into()],
        ..opts(1500)
    };
    let out = shrink(&doc, &o);
    let Value::Object(e) = &out.value else {
        panic!()
    };
    assert_eq!(e[0].1, Value::String(html));
}

#[test]
fn nothing_ever_exceeds_the_budget() {
    let texts = [
        log(),
        format!("<ul>{}</ul>", "<li><a href=\"https://x.test/a\">link</a> text</li>".repeat(100)),
        format!("<r>{}</r>", "<i a=\"1\"><v>2</v></i>".repeat(100)),
        "{\"a\":[1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16,17,18,19,20,21,22,23,24,25,26,27,28,29,30]}".repeat(1),
        "diff --git a/f b/f\n--- a/f\n+++ b/f\n@@ -1,3 +1,3 @@\n-a\n+b\n c\n".repeat(30),
    ];
    for t in &texts {
        for n in [60, 150, 400, 1000, 5000] {
            let doc = Value::Object(vec![("k".into(), Value::String(t.clone()))]);
            let out = shrink(&doc, &opts(n));
            assert!(
                out.report.output_chars <= n || !out.report.fits,
                "n={n}: {} chars",
                out.report.output_chars
            );
        }
    }
}
