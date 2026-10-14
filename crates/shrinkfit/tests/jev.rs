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
