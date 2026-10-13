//! Property-style tests over randomly generated documents.

use shrinkfit::json::{self, Layout};
use shrinkfit::prune::Pruner;
use shrinkfit::size::Model;
use shrinkfit::{shrink, shrink_json, Format, InputFormat, Options, Value};

/// Tiny deterministic PRNG (xorshift64*), so tests need no dependencies.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545F4914F6CDD1D)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
}

const WORDS: &[&str] = &[
    "alpha",
    "beta",
    "gamma",
    "delta",
    "héllo",
    "wörld",
    "😀",
    "quote\"d",
    "back\\slash",
    "tab\there",
    "x",
];

fn text(r: &mut Rng, max_words: u64) -> String {
    let n = r.below(max_words) + 1;
    let mut s = String::new();
    for i in 0..n {
        if i > 0 {
            s.push_str(match r.below(12) {
                0 => "\n",
                1 => "\n\n",
                2 => ". ",
                _ => " ",
            });
        }
        s.push_str(WORDS[r.below(WORDS.len() as u64) as usize]);
    }
    s
}

fn gen(r: &mut Rng, depth: u32) -> Value {
    let leaf = depth >= 5 || r.below(10) < 3;
    if leaf {
        return match r.below(6) {
            0 => Value::Null,
            1 => Value::Bool(r.below(2) == 0),
            2 => Value::Number(format!("{}", r.below(100_000) as i64 - 50_000)),
            3 => Value::Number("1.25e-3".into()),
            _ => {
                let words = if r.below(4) == 0 { 400 } else { 8 };
                Value::String(text(r, words))
            }
        };
    }
    if r.below(2) == 0 {
        // Arrays of similar records exercise sampling.
        let template = r.below(3);
        // Only flat record arrays get long; nested arrays stay small so the
        // generated documents don't explode combinatorially.
        let cap = if template == 0 && r.below(2) == 0 {
            200
        } else {
            6
        };
        let n = r.below(cap);
        let mut items = Vec::new();
        for i in 0..n {
            if template == 0 {
                let name = Value::String(text(r, 6));
                let mut tags = Vec::new();
                for _ in 0..r.below(4) {
                    tags.push(Value::String(text(r, 1)));
                }
                items.push(Value::Object(vec![
                    ("id".into(), Value::int(i as i64)),
                    ("name".into(), name),
                    ("tags".into(), Value::Array(tags)),
                ]));
            } else {
                items.push(gen(r, depth + 1));
            }
        }
        Value::Array(items)
    } else {
        let n = r.below(8);
        let mut entries = Vec::new();
        for i in 0..n {
            let k = format!("{}_{i}", WORDS[r.below(5) as usize]);
            entries.push((k, gen(r, depth + 1)));
        }
        Value::Object(entries)
    }
}

fn root_min(v: &Value, opts: &Options) -> usize {
    let layout = if opts.indent == 0 {
        Layout::Compact
    } else {
        Layout::Pretty(opts.indent)
    };
    let o = opts.clone();
    Pruner::new(Model::new(layout), &o).analyze(v, 0).min
}

#[test]
fn json_output_always_parses_and_fits() {
    let mut r = Rng(0x5eed_1234_abcd_0001);
    let mut checked = 0;
    for doc in 0..300 {
        let v = gen(&mut r, 0);
        for indent in [0usize, 2] {
            let full = shrink(
                &v,
                &Options {
                    indent,
                    ..Options::new(usize::MAX)
                },
            )
            .report
            .input_chars;
            let budgets = [
                1,
                10,
                25,
                50,
                80,
                120,
                200,
                400,
                800,
                1600,
                4000,
                full / 2,
                full.saturating_sub(1),
                full,
            ];
            for &b in &budgets {
                let opts = Options {
                    indent,
                    ..Options::new(b)
                };
                let out = shrink(&v, &opts);
                let len = out.text.chars().count();
                assert_eq!(len, out.report.output_chars);
                let reparsed = json::parse(&out.text)
                    .unwrap_or_else(|e| panic!("doc {doc} budget {b}: {e}\n{}", out.text));
                assert_eq!(
                    reparsed, out.value,
                    "rendered text must match returned value"
                );
                if b >= root_min(&v, &opts) {
                    assert!(
                        out.report.fits,
                        "doc {doc} budget {b} min {}",
                        root_min(&v, &opts)
                    );
                    assert!(
                        len <= b,
                        "doc {doc} indent {indent} budget {b}: got {len}\n{}",
                        out.text
                    );
                }
                if b >= full {
                    assert!(!out.report.pruned);
                    assert_eq!(out.value, v);
                }
                checked += 1;
            }
        }
    }
    assert!(checked > 5000);
}

#[test]
fn yaml_output_fits_when_possible() {
    let mut r = Rng(0xfeed_beef_0000_0042);
    for doc in 0..150 {
        let v = gen(&mut r, 0);
        let full = shrink(&v, &Options::new(usize::MAX).yaml())
            .report
            .input_chars;
        for b in [60, 150, 300, 700, 1500, full / 2, full] {
            let out = shrink(&v, &Options::new(b).yaml());
            if out.report.fits {
                assert!(out.report.output_chars <= b, "doc {doc} budget {b}");
            } else {
                assert!(b < 200, "doc {doc}: should fit at budget {b}\n{}", out.text);
            }
        }
    }
}

/// Compact single-line JSON options, so tests can assert on exact text.
fn compact(max: usize) -> Options {
    Options {
        indent: 0,
        ..Options::new(max)
    }
}

fn records(n: usize) -> String {
    let items: Vec<String> = (0..n)
        .map(|i| format!(r#"{{"id":{i},"name":"user {i}","email":"u{i}@example.com","active":{},"roles":["admin","dev"],"bio":"{}"}}"#,
            i % 2 == 0, "Long biography text. ".repeat(20)))
        .collect();
    format!(
        r#"{{"status":"ok","page":1,"total":{n},"results":[{}],"next":"/api/users?page=2"}}"#,
        items.join(",")
    )
}

#[test]
fn records_keep_schema_and_counts() {
    let src = records(1000);
    for budget in [400, 1000, 3000, 10_000] {
        let out = shrink_json(&src, &compact(budget)).unwrap();
        assert!(out.report.output_chars <= budget, "{budget}");
        let t = &out.text;
        // Top-level scalars survive intact.
        for k in [
            r#""status":"ok""#,
            r#""total":1000"#,
            r#""next":"/api/users?page=2""#,
        ] {
            assert!(t.contains(k), "budget {budget} lost {k}:\n{t}");
        }
        // The first record's full key set survives.
        for k in [
            "\"id\":0",
            "\"name\"",
            "\"email\"",
            "\"active\"",
            "\"roles\"",
            "\"bio\"",
        ] {
            assert!(t.contains(k), "budget {budget} lost {k}:\n{t}");
        }
        assert!(t.contains("items omitted"), "{t}");
    }
}

#[test]
fn generous_budget_shows_whole_records_and_last_item() {
    let out = shrink_json(&records(50), &compact(6000)).unwrap();
    let t = &out.text;
    assert!(t.contains("\"id\":49"), "last item should be kept:\n{t}");
    assert!(t.contains("\"id\":1,"), "{t}");
    assert!(
        out.report.output_chars * 10 >= 6000 * 9,
        "budget underused: {}",
        out.report.output_chars
    );
}

#[test]
fn heterogeneous_arrays_sample_each_shape() {
    let src = r#"[{"type":"a","x":1},{"type":"a","x":2},{"type":"a","x":3},{"type":"b","y":[1,2,3]},{"type":"a","x":4},{"type":"c","z":"zzz"},{"type":"a","x":5},{"type":"a","x":6},{"type":"a","x":7},{"type":"a","x":8}]"#;
    let out = shrink_json(src, &compact(140)).unwrap();
    let t = &out.text;
    assert!(t.contains("\"y\""), "{t}");
    assert!(t.contains("\"z\""), "{t}");
    assert!(out.report.output_chars <= 140);
}

#[test]
fn long_markdown_string_keeps_headings() {
    let sections: Vec<String> = (1..=30)
        .map(|i| {
            format!(
                "## Step {i}\n{}",
                "Do the thing carefully and check the output. ".repeat(8)
            )
        })
        .collect();
    let doc = format!(
        r#"{{"readme":{}}}"#,
        json::to_string(&Value::String(sections.join("\n\n")), Layout::Compact)
    );
    let out = shrink_json(&doc, &compact(3000)).unwrap();
    let readme = match &out.value {
        Value::Object(e) => match &e[0].1 {
            Value::String(s) => s.clone(),
            _ => panic!(),
        },
        _ => panic!(),
    };
    for i in 1..=30 {
        assert!(
            readme.contains(&format!("## Step {i}\n")),
            "missing step {i}:\n{readme}"
        );
    }
    assert!(readme.contains("…[+"));
}

#[test]
fn wide_object_drops_trailing_keys_with_marker() {
    let entries: Vec<String> = (0..300)
        .map(|i| format!(r#""key_{i}":"{}""#, "v".repeat(30)))
        .collect();
    let src = format!("{{{}}}", entries.join(","));
    let out = shrink_json(&src, &compact(500)).unwrap();
    assert!(out.report.output_chars <= 500);
    assert!(out.text.contains(r#""key_0""#));
    assert!(
        out.text.contains(r#""…":""#) && out.text.contains("keys omitted"),
        "{}",
        out.text
    );
}

#[test]
fn deeply_nested_input() {
    let mut s = String::from("\"leaf\"");
    for i in 0..250 {
        s = format!(r#"{{"level{i}":[{s}]}}"#);
    }
    // 500 levels of nesting: within the parser's limit.
    let out = shrink_json(&s, &compact(200)).unwrap();
    assert!(
        out.report.fits && out.report.output_chars <= 200,
        "{}",
        out.text
    );
    // Beyond the limit, input is rejected with an error rather than crashing.
    let too_deep = format!("{}1{}", "[".repeat(600), "]".repeat(600));
    assert!(shrink_json(&too_deep, &compact(200)).is_err());
}

#[test]
fn tiny_budget_returns_minimal_form() {
    let out = shrink_json(&records(10), &compact(5)).unwrap();
    assert!(!out.report.fits);
    assert_eq!(out.text, r#"{"…":"5 keys omitted"}"#);
}

#[test]
fn formats_and_options_from_json() {
    let o = Options::from_value(
        &json::parse(r#"{"max_chars":100,"format":"yaml","string_head_chars":50}"#).unwrap(),
    )
    .unwrap();
    assert_eq!(o.format, Format::Yaml);
    assert_eq!(o.indent, 2);
    assert_eq!(o.text.head, 50);
    assert!(Options::from_value(&json::parse(r#"{"max_chars":1,"bogus":1}"#).unwrap()).is_err());
    // max_chars is optional; defaults: auto formats, indent 2.
    let d = Options::from_value(&json::parse("{}").unwrap()).unwrap();
    assert_eq!(d.max_chars, 10_000);
    assert_eq!(
        (d.format, d.input_format, d.indent),
        (Format::Auto, InputFormat::Auto, 2)
    );
    assert!(Options::from_value(&json::parse(r#"{"format":"yaml","indent":0}"#).unwrap()).is_err());
}

#[test]
fn default_output_is_indented_json() {
    let out = shrink_json(r#"{"a":[1,2],"b":{"c":null}}"#, &Options::default()).unwrap();
    assert_eq!(
        out.text,
        "{
  \"a\": [
    1,
    2
  ],
  \"b\": {
    \"c\": null
  }
}"
    );
    assert_eq!(out.report.format, Format::Json);
}

#[test]
fn shrink_str_autodetects_and_follows_input_format() {
    use shrinkfit::{parse_input, shrink_str};
    let o = Options::default();
    let j = shrink_str(r#"{"a":1}"#, &o).unwrap();
    assert_eq!(j.report.format, Format::Json);
    assert!(j.text.starts_with(
        "{
"
    ));
    for (src, want) in [
        ("[1, 2]", InputFormat::Json),
        ("\"str\"", InputFormat::Json),
        ("42", InputFormat::Json),
        (
            "a: 1
b: [x, y]
",
            InputFormat::Yaml,
        ),
        (
            "- a
- b
",
            InputFormat::Yaml,
        ),
        ("just some words", InputFormat::Yaml),
    ] {
        let got = parse_input(src, InputFormat::Auto);
        if cfg!(feature = "yaml") || want == InputFormat::Json {
            assert_eq!(got.unwrap().1, want, "{src}");
        }
    }
    // Broken JSON-looking input reports the JSON error; empty input is an error.
    assert!(matches!(
        shrink_str("[1", &o),
        Err(shrinkfit::Error::Parse(_))
    ));
    assert!(shrink_str("", &o).is_err());
    #[cfg(feature = "yaml")]
    {
        let y = shrink_str(
            "a: 1
b: 2
",
            &o,
        )
        .unwrap();
        assert_eq!(y.report.format, Format::Yaml);
        assert_eq!(
            y.text,
            "a: 1
b: 2
"
        );
        // An explicit output format wins over the input's.
        let o2 = Options {
            format: Format::Json,
            indent: 0,
            ..Options::default()
        };
        assert_eq!(
            shrink_str(
                "a: 1
b: 2
",
                &o2
            )
            .unwrap()
            .text,
            r#"{"a":1,"b":2}"#
        );
    }
}

#[test]
fn large_input_is_fast() {
    let src = records(20_000); // ~11 MB
    let t = std::time::Instant::now();
    let out = shrink_json(&src, &Options::new(8000)).unwrap();
    let elapsed = t.elapsed();
    assert!(out.report.output_chars <= 8000);
    eprintln!(
        "{} bytes -> {} chars in {:?}",
        src.len(),
        out.report.output_chars,
        elapsed
    );
}

/// Our YAML emitter's output must parse back (via the real YAML parser) to
/// the same value, and shrinking YAML text must equal shrinking its value.
#[cfg(feature = "yaml")]
#[test]
fn yaml_input_roundtrips_and_shrinks() {
    use shrinkfit::{shrink_str, InputFormat};
    let mut r = Rng(0x1234_5678_9abc_def1);
    for doc in 0..200 {
        let v = gen(&mut r, 0);
        let y = shrinkfit::yaml::to_string(&v, 2);
        let back = shrinkfit::yaml_in::parse(&y).unwrap_or_else(|e| panic!("doc {doc}: {e}\n{y}"));
        // Numbers may be re-spelled (1.25e-3 stays as is; ints are exact).
        assert_eq!(back, v, "doc {doc}\n{y}");

        for b in [150usize, 700, 3000] {
            let mut o = Options::new(b);
            o.input_format = InputFormat::Yaml;
            o.format = Format::Yaml;
            let from_text = shrink_str(&y, &o).unwrap();
            let from_value = shrink(&v, &o);
            assert_eq!(from_text.text, from_value.text, "doc {doc} budget {b}");
        }
    }
}

/// The `condense` strategy must keep every invariant of the default one.
#[test]
fn condense_output_always_parses_and_fits() {
    let mut r = Rng(0xc0de_5eed_0000_0007);
    let mut condensed = 0;
    for doc in 0..100 {
        let v = gen(&mut r, 0);
        for indent in [0usize, 2] {
            for scorer in [
                shrinkfit::ScorerKind::Heuristic,
                shrinkfit::ScorerKind::Compact,
                shrinkfit::ScorerKind::KeyOrder,
            ] {
                let base = |b: usize| Options {
                    indent,
                    array_strategy: shrinkfit::ArrayStrategy::Condense,
                    field_scorer: scorer,
                    ..Options::new(b)
                };
                let full = shrink(&v, &base(usize::MAX)).report.input_chars;
                for b in [60, 150, 400, 1000, 2500, full / 2, full.saturating_sub(1)] {
                    let opts = base(b);
                    let out = shrink(&v, &opts);
                    let len = out.text.chars().count();
                    let reparsed = json::parse(&out.text)
                        .unwrap_or_else(|e| panic!("doc {doc} budget {b}: {e}\n{}", out.text));
                    assert_eq!(reparsed, out.value);
                    if b >= root_min(&v, &opts) {
                        assert!(out.report.fits && len <= b, "doc {doc} budget {b}: {len}");
                    }
                    condensed += out.report.stats.arrays_condensed;
                }
            }
        }
    }
    assert!(condensed > 50, "condense never kicked in ({condensed})");
}

fn issue_records(n: usize) -> String {
    let rows: Vec<String> = (0..n)
        .map(|i| {
            format!(
                r#"{{"id":"{}","key":"OPS-{i}","self":"https://x/api/{i}","fields":{{"summary":"Issue {i} autoscaler flaps under load","status":{{"name":"Open","iconUrl":"https://x/i.png","self":"https://x/s"}},"labels":[],"avatarUrls":{{"48x48":"https://a/48"}}}}}}"#,
                10_000 + i
            )
        })
        .collect();
    format!(r#"{{"total":{n},"issues":[{}]}}"#, rows.join(","))
}

#[test]
fn condense_keeps_every_record_with_the_same_fields() {
    let src = issue_records(60);
    let opts = Options {
        indent: 0,
        array_strategy: shrinkfit::ArrayStrategy::Condense,
        ..Options::new(4000)
    };
    let out = shrink_json(&src, &opts).unwrap();
    assert!(out.report.fits && out.report.output_chars <= 4000);
    let Value::Object(top) = &out.value else {
        panic!()
    };
    let Value::Array(issues) = &top.iter().find(|(k, _)| k == "issues").unwrap().1 else {
        panic!()
    };
    // Every record is kept, in order, with no gap markers.
    assert_eq!(issues.len(), 60);
    let keys = |v: &Value| -> Vec<String> {
        let Value::Object(e) = v else { panic!("{v:?}") };
        e.iter().map(|(k, _)| k.clone()).collect()
    };
    let first = keys(&issues[0]);
    assert!(first.contains(&"key".to_string()), "{first:?}");
    for it in issues {
        assert_eq!(keys(it), first, "all records keep the same fields");
    }
    // Boilerplate goes first; what was dropped is reported, not hidden.
    assert!(!out.text.contains("avatarUrls") && !out.text.contains("iconUrl"));
    let st = &out.report.stats;
    assert_eq!(st.arrays_condensed, 1);
    assert!(st.fields_dropped > 0 && !st.dropped_fields.is_empty());
    assert!(st
        .dropped_fields
        .iter()
        .all(|(p, n)| p.starts_with("/issues/*/") && *n == 60));
    assert!(out
        .report
        .summary()
        .contains("fields dropped from all items"));
    assert_eq!(st.items_omitted, 0);
}

#[test]
fn condense_falls_back_to_sampling_for_non_uniform_arrays() {
    // Mixed scalars and objects: not a table of records.
    let src = format!(
        "[{}]",
        (0..200)
            .map(|i| if i % 2 == 0 {
                format!("{i}")
            } else {
                format!(r#"{{"a":{i}}}"#)
            })
            .collect::<Vec<_>>()
            .join(",")
    );
    let opts = Options {
        indent: 0,
        array_strategy: shrinkfit::ArrayStrategy::Condense,
        ..Options::new(120)
    };
    let out = shrink_json(&src, &opts).unwrap();
    assert_eq!(out.report.stats.arrays_condensed, 0);
    assert!(out.text.contains("items omitted"), "{}", out.text);
}

#[test]
fn custom_scorer_can_be_swapped_in() {
    use shrinkfit::{shrink_with, ColumnInfo, FieldScorer};
    // Likes only fields under `status`; everything else is equally unloved.
    struct OnlyStatus;
    impl FieldScorer for OnlyStatus {
        fn name(&self) -> &'static str {
            "only_status"
        }
        fn score(&self, c: &ColumnInfo<'_>) -> f64 {
            if c.path.iter().any(|k| k == "status") {
                1.0
            } else {
                0.1
            }
        }
    }
    let v = json::parse(&issue_records(60)).unwrap();
    let opts = Options {
        indent: 0,
        array_strategy: shrinkfit::ArrayStrategy::Condense,
        ..Options::new(7000)
    };
    let ours = shrink_with(&v, &opts, &OnlyStatus);
    let builtin = shrink(&v, &opts);
    assert!(ours.report.fits && builtin.report.fits);
    assert_ne!(ours.text, builtin.text);
    assert!(ours.text.contains("status"), "{}", ours.text);
    assert!(!builtin.text.contains("status") || builtin.text.contains("summary"));
    assert!(builtin.text.contains("summary"), "{}", builtin.text);
}

fn condense_opts(max: usize) -> Options {
    Options {
        indent: 0,
        array_strategy: shrinkfit::ArrayStrategy::Condense,
        ..Options::new(max)
    }
}

#[test]
fn condense_falls_back_when_it_would_keep_only_categories_or_ids() {
    // Rich-text nodes: `type` is a category and the text is nested deeper.
    let nodes: Vec<String> = (0..40)
        .map(|i| {
            let kind = if i % 3 == 0 { "bulletList" } else { "paragraph" };
            format!(
                r#"{{"type":"{kind}","content":[{{"type":"text","text":"Sentence number {i} about the autoscaler."}}]}}"#
            )
        })
        .collect();
    let src = format!("[{}]", nodes.join(","));
    let out = shrink_json(&src, &condense_opts(900)).unwrap();
    assert_eq!(out.report.stats.arrays_condensed, 0, "{}", out.text);
    assert!(out.text.contains("Sentence number"), "{}", out.text);

    // Records that would shrink to nothing but ids.
    let rows: Vec<String> = (0..60)
        .map(|i| format!(r#"{{"id":{i},"body":"{}"}}"#, "text ".repeat(60)))
        .collect();
    let src = format!("[{}]", rows.join(","));
    let out = shrink_json(&src, &condense_opts(400)).unwrap();
    assert_eq!(out.report.stats.arrays_condensed, 0, "{}", out.text);
}

#[test]
fn condense_chooses_object_keys_by_value_not_position() {
    // One big object that must lose keys: the useful ones are at the end.
    let mut fields: Vec<String> = (0..30)
        .map(|i| format!(r#""zz_internal_{i}":"{}""#, "x".repeat(40)))
        .collect();
    fields.push(r#""title":"Autoscaler oscillates""#.into());
    fields.push(r#""status":"Open""#.into());
    let src = format!("{{{}}}", fields.join(","));
    let sampled = shrink_json(
        &src,
        &Options {
            indent: 0,
            ..Options::new(260)
        },
    )
    .unwrap();
    let condensed = shrink_json(&src, &condense_opts(260)).unwrap();
    assert!(
        !sampled.text.contains("\"title\""),
        "prefix trimming keeps the first keys"
    );
    assert!(
        condensed.text.contains("\"title\":") && condensed.text.contains("\"status\":\"Open\""),
        "{}",
        condensed.text
    );
    assert!(condensed.report.fits);
}

#[test]
fn arguments_boost_the_fields_a_call_mentions() {
    // Records whose `amount` field is unremarkable by name or value.
    let rows: Vec<String> = (0..80)
        .map(|i| {
            format!(
                r#"{{"id":"R-{i}","title":"Item number {i}","amount":{},"region":"north","owner":"person {i}"}}"#,
                1000 + i * 37
            )
        })
        .collect();
    let src = format!("[{}]", rows.join(","));
    let at = |budget: usize, args: Option<&str>| -> String {
        let opts = Options {
            indent: 0,
            array_strategy: shrinkfit::ArrayStrategy::Condense,
            arguments: args.map(String::from),
            ..Options::new(budget)
        };
        shrink_json(&src, &opts).unwrap().text
    };
    // Somewhere in the tight range, the plain heuristic drops `amount` but a
    // call that asked for it keeps it.
    let found = (1500..4000).step_by(100).any(|b| {
        !at(b, None).contains("amount")
            && at(b, Some("SELECT amount FROM deals")).contains("amount")
    });
    assert!(found, "arguments never changed which fields were kept");
    // Argument *names* are ignored: only values count.
    let v = json::parse(r#"{"amount": "nothing here", "q": ["title"]}"#).unwrap();
    assert_eq!(shrinkfit::arguments_text(&v).unwrap(), "nothing here title");
    // And options parse them from a string or an object.
    for a in [r#""amount""#, r#"{"column":"amount"}"#] {
        let o =
            Options::from_value(&json::parse(&format!(r#"{{"arguments":{a}}}"#)).unwrap()).unwrap();
        assert_eq!(o.arguments.as_deref(), Some("amount"));
    }
}

#[test]
fn arguments_naming_many_category_fields_cannot_lose_the_condensed_result() {
    // A query that names status, assignee and priority (as a JQL filter would)
    // boosts three category fields. The boosted plan is rejected as category-only,
    // so condense must retry without the arguments instead of dropping to sampling.
    let people = ["ada", "alan", "grace", "linus"];
    let rows: Vec<String> = (0..60)
        .map(|i| {
            format!(
                r#"{{"key":"OPS-{i}","summary":"Autoscaler oscillates under load, case {i}","status":"{}","assignee":"{}","priority":"{}"}}"#,
                ["Open", "Closed", "Review"][i % 3],
                people[i % 4],
                ["High", "Low"][i % 2]
            )
        })
        .collect();
    let src = format!("[{}]", rows.join(","));
    let args = "status assignee priority";
    let with = |a: Option<&str>| Options {
        arguments: a.map(String::from),
        ..condense_opts(4500)
    };
    let plain = shrink_json(&src, &with(None)).unwrap();
    let boosted = shrink_json(&src, &with(Some(args))).unwrap();
    assert_eq!(plain.report.stats.arrays_condensed, 1, "{}", plain.text);
    assert_eq!(boosted.report.stats.arrays_condensed, 1, "{}", boosted.text);
    assert!(boosted.text.contains("Autoscaler"), "{}", boosted.text);
}

fn find<'a>(v: &'a Value, key: &str) -> Option<&'a Value> {
    match v {
        Value::Object(e) => e.iter().find(|(k, _)| k == key).map(|(_, x)| x),
        _ => None,
    }
}

fn strings_of(v: &Value, key: &str) -> Vec<Option<String>> {
    let Value::Array(items) = v else {
        panic!("not an array")
    };
    items
        .iter()
        .map(|it| match find(it, key) {
            Some(Value::String(s)) => Some(s.clone()),
            _ => None,
        })
        .collect()
}

/// Atlassian-style search results: the page id is nested in `content`, urls are
/// relative, and container metadata competes for space.
fn confluence_like(n: usize) -> String {
    let words = [
        "platform", "runbook", "capacity", "billing", "security", "roadmap",
    ];
    let rows: Vec<String> = (0..n)
        .map(|i| {
            let id = 3_100_000_000u64 + i as u64 * 7_919;
            let title = format!("{} {} {} review {i}", words[i % 6], words[(i + 2) % 6], words[(i + 4) % 6]);
            format!(
                r#"{{"content":{{"id":"{id}","type":"page","status":"current","title":"{title}","space":{{"key":"ENG"}}}},"title":"{title}","excerpt":"{}","url":"/spaces/ENG/pages/{id}/{}","resultGlobalContainer":{{"title":"Engineering","displayUrl":"/spaces/ENG"}},"entityType":"content","lastModified":"2026-03-1{}T10:00:00.000Z","score":0.{i}}}"#,
                "capacity planning notes for the platform team ".repeat(4),
                title.replace(' ', "+"),
                i % 9
            )
        })
        .collect();
    format!(
        r#"{{"results":[{}],"start":0,"size":{n},"_links":{{"next":"/rest/api/search?cursor={}"}}}}"#,
        rows.join(","),
        "c".repeat(40)
    )
}

#[test]
fn nested_page_ids_survive_exactly_in_search_results() {
    let src = confluence_like(100);
    let original = json::parse(&src).unwrap();
    let out = shrink_json(&src, &condense_opts(10_000)).unwrap();
    assert!(out.report.fits);
    let results = find(&out.value, "results").unwrap();
    let Value::Array(items) = results else {
        panic!()
    };
    assert_eq!(items.len(), 100, "every record is kept");
    let ids = |v: &Value| -> Vec<Option<String>> {
        let Value::Array(items) = find(v, "results").unwrap() else {
            panic!()
        };
        items
            .iter()
            .map(|it| match find(it, "content").and_then(|c| find(c, "id")) {
                Some(Value::String(s)) => Some(s.clone()),
                _ => None,
            })
            .collect()
    };
    assert_eq!(
        ids(&out.value),
        ids(&original),
        "all page ids kept, byte for byte"
    );
    // The pagination link is untouched.
    assert_eq!(find(&out.value, "_links"), find(&original, "_links"));
}

#[test]
fn identifier_values_are_never_shortened() {
    // Loom-like videos: long ARIs that are useless if cut.
    let rows: Vec<String> = (0..50)
        .map(|i| {
            format!(
                r#"{{"ari":"ari:cloud:loom::video/{:032x}","name":"Onboarding walkthrough number {i} for new hires","createdAt":"2026-01-1{}T09:30:00Z","duration":{},"embedUrl":"https://www.loom.com/embed/{:032x}"}}"#,
                0xabcdef0123456789u128 * (i as u128 + 1),
                i % 9,
                30 + i * 17,
                0x1234567890abcdefu128 * (i as u128 + 3)
            )
        })
        .collect();
    let src = format!(r#"{{"videos":[{}]}}"#, rows.join(","));
    let original = json::parse(&src).unwrap();
    let out = shrink_json(&src, &condense_opts(5_000)).unwrap();
    assert!(out.report.fits);
    let (a, b) = (
        find(&original, "videos").unwrap(),
        find(&out.value, "videos").unwrap(),
    );
    let (ori, got) = (strings_of(a, "ari"), strings_of(b, "ari"));
    assert_eq!(ori.len(), got.len(), "all records kept");
    assert!(
        got.iter().all(|x| x.is_some()),
        "every record keeps its ARI"
    );
    assert_eq!(ori, got, "ARIs are exact");
    // Unique links, when kept, are exact as well (or dropped, never cut).
    for (o, g) in strings_of(a, "embedUrl")
        .iter()
        .zip(strings_of(b, "embedUrl"))
    {
        if let Some(g) = g {
            assert_eq!(o.as_ref(), Some(&g));
        }
    }
}

#[test]
fn protect_keeps_named_values_exact_in_every_strategy() {
    let rows: Vec<String> = (0..40)
        .map(|i| {
            format!(
                r#"{{"ref":"R-{i}","token":"{}","note":"{}"}}"#,
                format!("tok{i:04}").repeat(8),
                "some descriptive text about the record ".repeat(6)
            )
        })
        .collect();
    let src = format!(r#"[{}]"#, rows.join(","));
    for strategy in [
        shrinkfit::ArrayStrategy::Sample,
        shrinkfit::ArrayStrategy::Condense,
    ] {
        let opts = Options {
            indent: 0,
            array_strategy: strategy,
            protect: vec!["token".into()],
            ..Options::new(3_000)
        };
        let out = shrink_json(&src, &opts).unwrap();
        assert!(out.report.fits, "{strategy:?}");
        let Value::Array(items) = &out.value else {
            panic!()
        };
        let mut checked = 0;
        for it in items {
            if let Some(Value::String(t)) = find(it, "token") {
                assert_eq!(
                    t.len(),
                    56,
                    "token must not be shortened ({strategy:?}): {t}"
                );
                assert!(!t.contains('…'));
                checked += 1;
            }
        }
        assert!(checked >= 2, "{strategy:?}: expected tokens to be present");
    }
}

#[test]
fn protected_keys_survive_trimming_and_overflow_reports_no_fit() {
    // A wide object whose protected key is last: prefix trimming would drop it.
    let mut fields: Vec<String> = (0..60)
        .map(|i| format!(r#""field_{i:02}":"{}""#, "v".repeat(30)))
        .collect();
    fields.push(r#""snapshot_token":"snap-0123456789abcdef0123456789abcdef""#.into());
    let src = format!("{{{}}}", fields.join(","));
    let opts = Options {
        indent: 0,
        protect: vec!["/snapshot_token".into()],
        ..Options::new(300)
    };
    let out = shrink_json(&src, &opts).unwrap();
    assert!(out.report.fits, "{}", out.text);
    assert!(
        out.text.contains("snap-0123456789abcdef0123456789abcdef"),
        "{}",
        out.text
    );
    // A protected value larger than the whole budget is not cut to fit: no-fit.
    let big = format!(r#"{{"a":1,"cursor":"{}"}}"#, "x".repeat(500));
    let opts = Options {
        indent: 0,
        protect: vec!["cursor".into()],
        ..Options::new(100)
    };
    let out = shrink_json(&big, &opts).unwrap();
    assert!(!out.report.fits);
    assert!(
        out.text.contains(&"x".repeat(500)),
        "the protected value is never altered"
    );
}

#[test]
fn sorting_by_a_field_does_not_crowd_out_summaries_but_selecting_it_does_keep_it() {
    let rows: Vec<String> = (0..60)
        .map(|i| {
            format!(
                r#"{{"key":"OPS-{i}","summary":"Autoscaler oscillates under bursty load, case number {i}","updated":"2026-03-1{}T10:{:02}:00.000+0000"}}"#,
                i % 9,
                i % 60
            )
        })
        .collect();
    let src = format!("[{}]", rows.join(","));
    let with = |a: &str| Options {
        arguments: Some(a.into()),
        ..condense_opts(4_800)
    };
    // Only a sort clause names `updated`: summaries come first.
    let sorted = shrink_json(&src, &with("project = OPS ORDER BY updated DESC")).unwrap();
    assert!(
        sorted.text.contains("Autoscaler oscillates"),
        "{}",
        sorted.text
    );
    assert!(!sorted.text.contains("\"updated\""), "{}", sorted.text);
    // Asking for the field in a select list keeps it.
    let selected = shrink_json(&src, &with("SELECT key, updated FROM issues")).unwrap();
    assert!(selected.text.contains("\"updated\""), "{}", selected.text);
}
