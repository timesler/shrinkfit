//! Model scoring against a local stand-in for the Decisions API.
#![cfg(feature = "jev")]

use shrinkfit::json;
use shrinkfit::{shrink_str, ArrayStrategy, JevOptions, Options, Value};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::time::Duration;

type Requests = Arc<Mutex<Vec<String>>>;

/// Serve `handler(request body) -> (status, reply body)` on a free local port.
/// Returns the base url (`http://127.0.0.1:PORT/api`) and the raw requests seen.
fn serve(
    handler: impl Fn(&str) -> (u16, String) + Send + 'static,
    delay_ms: u64,
) -> (String, Requests) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let seen: Requests = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut s) = stream else { continue };
            let mut buf = Vec::new();
            let mut chunk = [0u8; 4096];
            let (head_end, len) = loop {
                let n = s.read(&mut chunk).unwrap_or(0);
                if n == 0 {
                    break (0, 0);
                }
                buf.extend_from_slice(&chunk[..n]);
                let text = String::from_utf8_lossy(&buf).to_string();
                if let Some(i) = text.find("\r\n\r\n") {
                    let len = text[..i]
                        .lines()
                        .find_map(|l| {
                            l.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .map(|v| v.trim().parse::<usize>().unwrap())
                        })
                        .unwrap_or(0);
                    if buf.len() >= i + 4 + len {
                        break (i + 4, len);
                    }
                }
            };
            let request = String::from_utf8_lossy(&buf).to_string();
            let body = String::from_utf8_lossy(&buf[head_end..head_end + len]).to_string();
            log.lock().unwrap().push(request);
            std::thread::sleep(Duration::from_millis(delay_ms));
            let (code, reply) = handler(&body);
            let _ = write!(
                s,
                "HTTP/1.1 {code} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}",
                reply.len()
            );
        }
    });
    (format!("http://127.0.0.1:{port}/api"), seen)
}

/// Answer every question with `score(path)`.
fn scoring(score: fn(&str) -> f64) -> impl Fn(&str) -> (u16, String) {
    move |body| {
        let Value::Object(top) = json::parse(body).unwrap() else {
            panic!()
        };
        let Some((_, Value::Object(qs))) = top.iter().find(|(k, _)| k == "questions") else {
            panic!()
        };
        let answers: Vec<String> = qs
            .iter()
            .map(|(name, q)| {
                let Value::Object(q) = q else { panic!() };
                let Some((_, Value::String(ins))) = q.iter().find(|(k, _)| k == "instructions")
                else {
                    panic!()
                };
                let path = ins.split('`').nth(1).unwrap();
                format!(r#""{name}":{{"type":"noul","noul":{}}}"#, score(path))
            })
            .collect();
        (200, format!(r#"{{"answers":{{{}}}}}"#, answers.join(",")))
    }
}

fn records() -> String {
    let rows: Vec<String> = (0..30)
        .map(|i| {
            format!(
                r#"{{"id":"ID-{i}","alpha":"{}","beta":"{}"}}"#,
                format!("alpha text {i} ").repeat(6),
                format!("beta text {i} ").repeat(6)
            )
        })
        .collect();
    format!(r#"{{"results":[{}]}}"#, rows.join(","))
}

fn options(base: &str, key: Option<&str>) -> Options {
    Options {
        indent: 0,
        array_strategy: ArrayStrategy::Condense,
        field_score_weight: 1.0,
        arguments_raw: Some(r#"{"query":"widgets"}"#.into()),
        jev: Some(JevOptions {
            api_key: key.map(String::from),
            base_url: base.into(),
            tool: Some("search".into()),
            ..JevOptions::default()
        }),
        ..Options::new(1500)
    }
}

fn keys(text: &str) -> Vec<String> {
    let Value::Object(top) = json::parse(text).unwrap() else {
        panic!()
    };
    let Some((_, Value::Array(rows))) = top.iter().find(|(k, _)| k == "results") else {
        panic!()
    };
    let Value::Object(first) = &rows[0] else {
        panic!()
    };
    first.iter().map(|(k, _)| k.clone()).collect()
}

#[test]
fn model_scores_steer_which_fields_survive() {
    let (base, seen) = serve(
        scoring(|p| {
            if p.ends_with("beta") {
                1.0
            } else if p.ends_with("alpha") {
                0.0
            } else {
                0.5
            }
        }),
        0,
    );
    let out = shrink_str(&records(), &options(&base, Some("test-key"))).unwrap();
    assert_eq!(keys(&out.text), ["beta"], "{}", out.text);
    let note = out.report.model_scoring.unwrap();
    assert!(note.starts_with("scored 3 fields"), "{note}");

    let requests = seen.lock().unwrap();
    assert_eq!(requests.len(), 1);
    let r = &requests[0];
    assert!(r.starts_with("POST /api/alpha/decisions"), "{r}");
    assert!(r
        .to_ascii_lowercase()
        .contains("authorization: bearer test-key"));
    assert!(r.contains("typesafe/jev-1.13") && r.contains("results/*/beta"));
    assert!(
        r.contains("widgets") && r.contains("search"),
        "tool and arguments are sent"
    );
}

#[test]
fn flipped_scores_flip_the_result() {
    let (base, _) = serve(
        scoring(|p| {
            if p.ends_with("alpha") {
                1.0
            } else if p.ends_with("beta") {
                0.0
            } else {
                0.5
            }
        }),
        0,
    );
    let out = shrink_str(&records(), &options(&base, Some("k"))).unwrap();
    assert_eq!(keys(&out.text), ["alpha"], "{}", out.text);
}

#[test]
fn failures_fall_back_to_the_built_in_scoring_and_say_why() {
    let plain = shrink_str(
        &records(),
        &Options {
            jev: None,
            ..options("http://unused", Some("k"))
        },
    )
    .unwrap();

    // The service answers with an error.
    let (base, _) = serve(|_| (402, r#"{"error":"credits"}"#.into()), 0);
    let out = shrink_str(&records(), &options(&base, Some("k"))).unwrap();
    assert_eq!(out.text, plain.text);
    let note = out.report.model_scoring.unwrap();
    assert!(
        note.starts_with("unavailable") && note.contains("HTTP 402") && note.contains("built-in"),
        "{note}"
    );

    // The reply is not what was asked for.
    let (base, _) = serve(|_| (200, r#"{"answers":{}}"#.into()), 0);
    let out = shrink_str(&records(), &options(&base, Some("k"))).unwrap();
    assert_eq!(out.text, plain.text);
    assert!(out.report.model_scoring.unwrap().contains("no answer"));

    // Too slow.
    let (base, _) = serve(scoring(|_| 0.5), 800);
    let mut slow = options(&base, Some("k"));
    slow.jev.as_mut().unwrap().timeout_ms = 150;
    let started = std::time::Instant::now();
    let out = shrink_str(&records(), &slow).unwrap();
    assert!(
        started.elapsed() < Duration::from_millis(700),
        "gave up at the timeout"
    );
    assert_eq!(out.text, plain.text);
    assert!(out.report.model_scoring.unwrap().starts_with("unavailable"));

    // Nothing is listening.
    let dead = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://127.0.0.1:{}/api", dead.local_addr().unwrap().port());
    drop(dead);
    let out = shrink_str(&records(), &options(&url, Some("k"))).unwrap();
    assert_eq!(out.text, plain.text);
    assert!(out.report.model_scoring.unwrap().starts_with("unavailable"));
}

#[test]
fn without_a_key_nothing_is_sent() {
    std::env::remove_var(shrinkfit::jev::KEY_ENV);
    let (base, seen) = serve(scoring(|_| 0.5), 0);
    let out = shrink_str(&records(), &options(&base, None)).unwrap();
    assert!(out
        .report
        .model_scoring
        .unwrap()
        .contains("no OpenRouter API key"));
    assert!(seen.lock().unwrap().is_empty());
}

#[test]
fn nothing_is_sent_when_the_document_already_fits_or_the_strategy_cannot_use_it() {
    let (base, seen) = serve(scoring(|_| 0.5), 0);
    let mut roomy = options(&base, Some("k"));
    roomy.max_chars = 1_000_000;
    let out = shrink_str(&records(), &roomy).unwrap();
    assert!(out.report.model_scoring.is_none() && !out.report.pruned);

    let mut sampling = options(&base, Some("k"));
    sampling.array_strategy = ArrayStrategy::Sample;
    let out = shrink_str(&records(), &sampling).unwrap();
    assert!(out.report.model_scoring.unwrap().starts_with("skipped"));
    assert!(seen.lock().unwrap().is_empty());
}

#[test]
fn a_host_can_send_the_request_itself() {
    let (base, seen) = serve(scoring(|p| if p.ends_with("beta") { 1.0 } else { 0.5 }), 0);
    let src = records();
    let opts = options(&base, Some("host-key"));

    // Step 1: the core says what to send. Nothing has been sent yet.
    let req = shrinkfit::model_request(&src, &opts).unwrap().unwrap();
    assert_eq!(req.url, format!("{base}/alpha/decisions"));
    assert!(req
        .headers
        .contains(&("Authorization".into(), "Bearer host-key".into())));
    assert!(req.body.contains("results/*/beta") && req.body.contains("widgets"));
    assert_eq!(req.timeout_ms, 3000);
    assert!(seen.lock().unwrap().is_empty());

    // Step 2: the host sends it (here, the stand-in's logic) and hands back the reply.
    let reply = scoring(|p| if p.ends_with("beta") { 1.0 } else { 0.5 })(&req.body).1;
    let hosted = shrink_str(
        &src,
        &Options {
            jev_reply: Some(reply),
            ..opts.clone()
        },
    )
    .unwrap();
    assert_eq!(keys(&hosted.text), ["beta"]);
    let note = hosted.report.model_scoring.unwrap();
    assert!(
        note.starts_with("scored 3 fields") && !note.contains(" ms"),
        "{note}"
    );
    assert!(
        seen.lock().unwrap().is_empty(),
        "with a reply supplied the core makes no request"
    );

    // A failed attempt by the host is reported and the built-in scoring is used.
    let failed = shrink_str(
        &src,
        &Options {
            jev_error: Some("the service answered HTTP 429".into()),
            ..opts.clone()
        },
    )
    .unwrap();
    let plain = shrink_str(
        &src,
        &Options {
            jev: None,
            ..opts.clone()
        },
    )
    .unwrap();
    assert_eq!(failed.text, plain.text);
    assert!(failed.report.model_scoring.unwrap().contains("HTTP 429"));

    // No request is needed when there is nothing to ask or nothing to cut.
    assert_eq!(
        shrinkfit::model_request(
            &src,
            &Options {
                jev: None,
                ..opts.clone()
            }
        ),
        Ok(None)
    );
    let roomy = Options {
        max_chars: 1_000_000,
        ..opts.clone()
    };
    assert_eq!(shrinkfit::model_request(&src, &roomy), Ok(None));
    // No key: the host is told why.
    std::env::remove_var(shrinkfit::jev::KEY_ENV);
    let keyless = with_jev_cache_off(options(&base, None));
    assert!(shrinkfit::model_request(&src, &keyless)
        .unwrap_err()
        .contains("no OpenRouter API key"));
}

#[test]
fn a_large_request_with_awkward_characters_arrives_intact() {
    // Many fields with awkward characters: quotes, backslashes, newlines, unicode.
    let fields: Vec<String> = (0..100)
        .map(|i| {
            format!(
                r#""f{i}":"line one\nsays \"hi\" \\ café 🙂 {}""#,
                "x".repeat(40)
            )
        })
        .collect();
    let src = format!(
        r#"{{"data":[{{{}}},{{{}}}]}}"#,
        fields.join(","),
        fields.join(",")
    );
    let (base, seen) = serve(scoring(|_| 0.5), 0);
    let mut opts = options(&base, Some("k"));
    opts.max_chars = 800;
    let out = shrink_str(&src, &opts).unwrap();
    let note = out.report.model_scoring.unwrap();
    assert!(note.starts_with("scored 100 fields"), "{note}");
    let raw = &seen.lock().unwrap()[0];
    assert!(
        raw.contains("f99") && raw.contains("caf"),
        "all fields arrive intact"
    );
}

fn with_jev(base: &str, f: impl FnOnce(&mut JevOptions)) -> Options {
    let mut opts = options(base, Some("k"));
    f(opts.jev.as_mut().unwrap());
    opts
}

#[test]
fn the_request_carries_labelled_examples_unless_told_otherwise() {
    let (base, seen) = serve(scoring(|_| 0.5), 0);
    let src = records();
    shrink_str(&src, &with_jev(&base, |_| {})).unwrap();
    shrink_str(&src, &with_jev(&base, |j| j.examples = 0)).unwrap();
    shrink_str(
        &src,
        &with_jev(&base, |j| {
            j.bank = Some(
                r#"[{"tool":"my_own_tool","arguments":"{}","fields":[
                    {"path":"/results/*/id","example_values":["a"],"verdict":"essential"},
                    {"path":"/results/*/alpha","example_values":["b"],"verdict":"drop"}]}]"#
                    .into(),
            );
        }),
    )
    .unwrap();
    let requests = seen.lock().unwrap();
    assert_eq!(requests.len(), 3);
    assert!(requests[0].contains("labelled_examples") && requests[0].contains("verdict"));
    assert!(
        !requests[1].contains("labelled_examples"),
        "examples: 0 sends none"
    );
    assert!(
        requests[2].contains("my_own_tool"),
        "a custom bank replaces the bundled one"
    );
    assert!(!requests[2].contains("airtable"), "{}", &requests[2][..200]);
}

#[test]
fn weight_zero_ignores_the_model_and_the_default_trusts_it() {
    let favour = |p: &str| if p.ends_with("beta") { 1.0 } else { 0.0 };
    let (base, _) = serve(scoring(favour), 0);
    let src = records();
    let plain = shrink_str(
        &src,
        &Options {
            jev: None,
            ..options(&base, Some("k"))
        },
    )
    .unwrap();
    let ignored = shrink_str(&src, &with_jev(&base, |j| j.weight = 0.0)).unwrap();
    assert_eq!(
        ignored.text, plain.text,
        "weight 0 gives the built-in result"
    );
    let trusted = shrink_str(&src, &with_jev(&base, |_| {})).unwrap();
    assert_eq!(keys(&trusted.text), ["beta"]);
}

#[test]
fn only_the_order_of_the_probabilities_matters() {
    // All three answers are close to 0.5; beta is merely the highest.
    let (base, _) = serve(
        scoring(|p| {
            if p.ends_with("beta") {
                0.52
            } else if p.ends_with("alpha") {
                0.51
            } else {
                0.50
            }
        }),
        0,
    );
    let out = shrink_str(&records(), &options(&base, Some("k"))).unwrap();
    assert_eq!(keys(&out.text), ["beta"], "{}", out.text);
}

fn with_jev_cache_off(mut opts: Options) -> Options {
    opts.jev.as_mut().unwrap().cache = false;
    opts
}

/// Same field set, different values: what a tool returns call after call.
fn records_again() -> String {
    records()
        .replace("alpha text", "alpha other")
        .replace("beta text", "beta other")
}

#[test]
fn the_second_response_of_the_same_shape_needs_no_request() {
    let (base, seen) = serve(scoring(|p| if p.ends_with("beta") { 1.0 } else { 0.0 }), 0);
    let opts = options(&base, Some("k"));
    let first = shrink_str(&records(), &opts).unwrap();
    assert!(first
        .report
        .model_scoring
        .as_deref()
        .unwrap()
        .starts_with("scored 3 fields"));
    assert!(!first
        .report
        .model_scoring
        .as_deref()
        .unwrap()
        .contains("cached"));
    // A host that sends requests itself is told none is needed.
    assert_eq!(shrinkfit::model_request(&records_again(), &opts), Ok(None));

    let second = shrink_str(&records_again(), &opts).unwrap();
    let note = second.report.model_scoring.unwrap();
    assert!(
        note.starts_with("scored 3 fields") && note.contains("(cached)"),
        "{note}"
    );
    assert_eq!(
        keys(&second.text),
        ["beta"],
        "the remembered scores still steer the result"
    );
    assert_eq!(
        seen.lock().unwrap().len(),
        1,
        "only the first response was sent"
    );
}

#[test]
fn the_cache_keys_on_tool_endpoint_examples_and_fields_and_can_be_switched_off() {
    let (base, seen) = serve(scoring(|_| 0.5), 0);
    let src = records();
    let requests = || seen.lock().unwrap().len();
    shrink_str(&src, &options(&base, Some("k"))).unwrap();
    assert_eq!(requests(), 1);
    shrink_str(&src, &options(&base, Some("k"))).unwrap();
    assert_eq!(requests(), 1, "the same call is remembered");
    shrink_str(
        &src,
        &with_jev(&base, |j| j.tool = Some("another_tool".into())),
    )
    .unwrap();
    assert_eq!(requests(), 2, "another tool is a different key");
    shrink_str(&src, &with_jev(&base, |j| j.examples = 2)).unwrap();
    assert_eq!(
        requests(),
        3,
        "a different number of examples is a different key"
    );
    let fewer = src.replace(r#","beta":"#, r#","gamma":"#);
    shrink_str(&fewer, &options(&base, Some("k"))).unwrap();
    assert_eq!(requests(), 4, "a different field set is a different key");
    shrink_str(&src, &with_jev(&base, |j| j.cache = false)).unwrap();
    shrink_str(&src, &with_jev(&base, |j| j.cache = false)).unwrap();
    assert_eq!(requests(), 6, "cache: false asks every time");
    // The arguments are deliberately not part of the key.
    let mut other_args = options(&base, Some("k"));
    other_args.arguments_raw = Some(r#"{"query":"something else"}"#.into());
    shrink_str(&src, &other_args).unwrap();
    assert_eq!(requests(), 6);
}
