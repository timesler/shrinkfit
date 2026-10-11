//! Property-style tests over randomly generated documents.

use shrinkfit::json::{self, Layout};
use shrinkfit::prune::Pruner;
use shrinkfit::size::Model;
use shrinkfit::{shrink, shrink_json, Format, Options, Value};

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
        let out = shrink_json(&src, &Options::new(budget)).unwrap();
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
    let out = shrink_json(&records(50), &Options::new(6000)).unwrap();
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
    let out = shrink_json(src, &Options::new(140)).unwrap();
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
    let out = shrink_json(&doc, &Options::new(3000)).unwrap();
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
    let out = shrink_json(&src, &Options::new(500)).unwrap();
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
    let out = shrink_json(&s, &Options::new(200)).unwrap();
    assert!(
        out.report.fits && out.report.output_chars <= 200,
        "{}",
        out.text
    );
    // Beyond the limit, input is rejected with an error rather than crashing.
    let too_deep = format!("{}1{}", "[".repeat(600), "]".repeat(600));
    assert!(shrink_json(&too_deep, &Options::new(200)).is_err());
}

#[test]
fn tiny_budget_returns_minimal_form() {
    let out = shrink_json(&records(10), &Options::new(5)).unwrap();
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
    assert!(Options::from_value(&json::parse(r#"{"format":"json"}"#).unwrap()).is_err());
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
