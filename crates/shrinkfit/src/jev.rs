//! Optional scoring by a hosted decision model (Jev, through OpenRouter).
//!
//! The built-in scorer only sees the document. A decision model can also tell
//! which fields matter to a reader: given the tool call and every field path
//! with a couple of example values, it answers one yes/no question per field
//! ("is this worth keeping when the response is truncated?") with a probability.
//! Those probabilities become [`Options::field_scores`] and are blended with the
//! built-in score (see `field_score_weight`).
//!
//! It is strictly opt-in and best effort. Nothing is sent unless
//! [`Options::jev`] is set, the document actually has to be cut, and an
//! OpenRouter API key is available (`jev.api_key`, else the `OPENROUTER_API_KEY`
//! environment variable). Any failure (no key, no network, a timeout, an error
//! status, an unreadable reply) leaves the built-in scoring in charge and is
//! described in [`crate::Report::model_scoring`].
//!
//! **What leaves the machine:** the tool name, the call's arguments (first 400
//! characters), every field path, and up to two example values (60 characters
//! each) per path.
//!
//! **Who sends the request.** This module has no HTTP or TLS code and no
//! dependencies, so it works on every target including wasm (which has no
//! sockets). It builds the request ([`crate::model_request`]) and reads the reply
//! (`Options::jev_reply`). With the `jev` cargo feature (on in the CLI and the
//! native FFI library) the core also sends it, with a small HTTP client; without
//! it, and always on wasm, the host sends the request in between (TypeScript
//! uses `fetch`).

use crate::json::{self, Layout};
use crate::options::Options;
use crate::value::Value;
use std::fmt;

/// The model asked by default.
pub const DEFAULT_MODEL: &str = "typesafe/jev-1.13";
/// Where the Decisions API lives (`<base>/alpha/decisions`).
pub const DEFAULT_BASE_URL: &str = "https://openrouter.ai/api";
/// Environment variable holding the OpenRouter API key.
pub const KEY_ENV: &str = "OPENROUTER_API_KEY";
/// Default time allowed for the whole request.
pub const DEFAULT_TIMEOUT_MS: u64 = 3000;

const MAX_FIELDS: usize = 120;
const MAX_ARRAY_ITEMS: usize = 2000;
const SAMPLE_CHARS: usize = 60;
const ARGUMENT_CHARS: usize = 400;

const GOAL: &str = "one of the fields worth keeping when this response is truncated so that an \
agent can still scan what it contains (some descriptive content) and make targeted follow-up \
calls (identifiers, URLs)";

/// Settings for model scoring. `Default` uses the public OpenRouter endpoint
/// and the key from the environment.
#[derive(Clone, PartialEq)]
pub struct JevOptions {
    /// OpenRouter API key. `None` reads `OPENROUTER_API_KEY`.
    pub api_key: Option<String>,
    pub model: String,
    pub base_url: String,
    pub timeout_ms: u64,
    /// Name of the tool that produced the document (context for the model).
    pub tool: Option<String>,
}

impl Default for JevOptions {
    fn default() -> Self {
        JevOptions {
            api_key: None,
            model: DEFAULT_MODEL.into(),
            base_url: DEFAULT_BASE_URL.into(),
            timeout_ms: DEFAULT_TIMEOUT_MS,
            tool: None,
        }
    }
}

impl fmt::Debug for JevOptions {
    // Never print the key.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("JevOptions")
            .field("api_key", &self.api_key.as_ref().map(|_| "<redacted>"))
            .field("model", &self.model)
            .field("base_url", &self.base_url)
            .field("timeout_ms", &self.timeout_ms)
            .field("tool", &self.tool)
            .finish()
    }
}

impl JevOptions {
    /// Parse the `jev` option: `true`, or an object with any of `api_key`,
    /// `model`, `base_url`, `timeout_ms`, `tool`. `false`/`null` mean off.
    pub fn from_value(v: &Value) -> Result<Option<JevOptions>, String> {
        let entries = match v {
            Value::Bool(true) => return Ok(Some(JevOptions::default())),
            Value::Bool(false) | Value::Null => return Ok(None),
            Value::Object(e) => e,
            _ => return Err("jev must be true, false or an object".into()),
        };
        let mut o = JevOptions::default();
        for (k, val) in entries {
            let text = || match val {
                Value::String(s) => Ok(s.clone()),
                _ => Err(format!("jev.{k} must be a string")),
            };
            match k.as_str() {
                "api_key" => o.api_key = Some(text()?),
                "model" => o.model = text()?,
                "base_url" => o.base_url = text()?,
                "tool" => o.tool = Some(text()?),
                "timeout_ms" => {
                    o.timeout_ms = match val {
                        Value::Number(n) => n.parse::<u64>().ok().filter(|t| *t > 0),
                        _ => None,
                    }
                    .ok_or("jev.timeout_ms must be a positive integer")?
                }
                other => return Err(format!("unknown jev option: {other}")),
            }
        }
        Ok(Some(o))
    }

    fn key(&self) -> Option<String> {
        self.api_key
            .clone()
            .or_else(|| std::env::var(KEY_ENV).ok())
            .map(|k| k.trim().to_string())
            .filter(|k| !k.is_empty())
    }
}

/// One field of the document: its path and example values.
#[derive(Debug, PartialEq)]
pub(crate) struct Field {
    pub path: String,
    pub samples: Vec<String>,
}

/// Every distinct leaf path of the document (arrays as `*`, same notation as
/// `protect` and `field_scores`), in order of first appearance, with up to two
/// distinct example values each. Keys containing `/` or `*` cannot be written
/// as a pattern and are skipped.
pub(crate) fn fields(v: &Value) -> Vec<Field> {
    fn touch(path: &str, out: &mut Vec<Field>) -> usize {
        match out.iter().position(|f| f.path == path) {
            Some(i) => i,
            None => {
                out.push(Field {
                    path: path.to_string(),
                    samples: Vec::new(),
                });
                out.len() - 1
            }
        }
    }
    fn walk(v: &Value, path: &mut String, out: &mut Vec<Field>) {
        let sample = |s: &str| s.chars().take(SAMPLE_CHARS).collect::<String>();
        match v {
            Value::Object(entries) => {
                if entries.is_empty() && !path.is_empty() {
                    touch(path, out);
                }
                for (k, x) in entries {
                    if k.is_empty() || k.contains('/') || k == "*" {
                        continue;
                    }
                    let n = path.len();
                    path.push('/');
                    path.push_str(k);
                    walk(x, path, out);
                    path.truncate(n);
                }
            }
            Value::Array(items) => {
                if items.is_empty() && !path.is_empty() {
                    touch(path, out);
                }
                for x in items.iter().take(MAX_ARRAY_ITEMS) {
                    let n = path.len();
                    path.push_str("/*");
                    walk(x, path, out);
                    path.truncate(n);
                }
            }
            scalar => {
                if path.is_empty() {
                    return;
                }
                let i = touch(path, out);
                let text = match scalar {
                    Value::String(s) => Some(sample(s)),
                    Value::Number(n) => Some(sample(n)),
                    Value::Bool(b) => Some(b.to_string()),
                    _ => None,
                };
                if let Some(t) = text {
                    let f = &mut out[i];
                    if f.samples.len() < 2 && !f.samples.contains(&t) {
                        f.samples.push(t);
                    }
                }
            }
        }
    }
    let mut out = Vec::new();
    walk(v, &mut String::new(), &mut out);
    out.truncate(MAX_FIELDS);
    out
}

fn string(s: &str) -> Value {
    Value::String(s.to_string())
}

fn obj(entries: Vec<(&str, Value)>) -> Value {
    Value::Object(
        entries
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect(),
    )
}

/// The request body for the Decisions API: the state (tool call and fields as
/// JSON text) and one yes/no question per field, `p0`, `p1`, ...
pub(crate) fn request_body(cfg: &JevOptions, arguments: Option<&str>, fields: &[Field]) -> String {
    let mut call = Vec::new();
    if let Some(t) = &cfg.tool {
        call.push(("name", string(t)));
    }
    if let Some(a) = arguments {
        call.push((
            "arguments",
            string(&a.chars().take(ARGUMENT_CHARS).collect::<String>()),
        ));
    }
    let listed = fields
        .iter()
        .map(|f| {
            obj(vec![
                ("path", string(&f.path)),
                (
                    "example_values",
                    Value::Array(f.samples.iter().map(|s| string(s)).collect()),
                ),
            ])
        })
        .collect();
    let state = obj(vec![
        ("tool_call", obj(call)),
        ("output_fields", Value::Array(listed)),
    ]);
    let questions = Value::Object(
        fields
            .iter()
            .enumerate()
            .map(|(i, f)| {
                (
                    format!("p{i}"),
                    obj(vec![
                        ("type", string("noul")),
                        (
                            "instructions",
                            string(&format!("Is the output field `{}` {GOAL}?", f.path)),
                        ),
                    ]),
                )
            })
            .collect(),
    );
    let body = obj(vec![
        ("model", string(&cfg.model)),
        (
            "state",
            Value::String(json::to_string(&state, Layout::Compact)),
        ),
        ("questions", questions),
    ]);
    json::to_string(&body, Layout::Compact)
}

fn get<'a>(v: &'a Value, key: &str) -> Option<&'a Value> {
    match v {
        Value::Object(e) => e.iter().find(|(k, _)| k == key).map(|(_, x)| x),
        _ => None,
    }
}

/// Probabilities of "yes", one per field, from the Decisions API reply.
pub(crate) fn parse_reply(text: &str, n: usize) -> Result<Vec<f64>, String> {
    let reply = json::parse(text).map_err(|_| "the reply was not JSON".to_string())?;
    let answers = get(&reply, "answers").ok_or("the reply had no answers")?;
    (0..n)
        .map(|i| {
            match get(answers, &format!("p{i}")).and_then(|a| get(a, "noul")) {
                Some(Value::Number(x)) => x
                    .parse::<f64>()
                    .ok()
                    .filter(|p| p.is_finite())
                    .map(|p| p.clamp(0.0, 1.0)),
                _ => None,
            }
            .ok_or_else(|| format!("the reply had no answer for field {i}"))
        })
        .collect()
}

/// An HTTP request for the host to send, for hosts that make the call themselves
/// (WebAssembly has no sockets). Send it as is, then pass the response body back
/// as [`Options::jev_reply`], or the failure as [`Options::jev_error`].
#[derive(Clone, Debug, PartialEq)]
pub struct Request {
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: String,
    /// The time the host should allow, in milliseconds.
    pub timeout_ms: u64,
}

/// The request for `v`, or why there cannot be one (no API key, too few fields).
pub(crate) fn build_request(
    v: &Value,
    opts: &Options,
    cfg: &JevOptions,
) -> Result<(Request, usize), String> {
    let key = cfg
        .key()
        .ok_or_else(|| format!("no OpenRouter API key (set {KEY_ENV} or jev.api_key)"))?;
    let listed = fields(v);
    if listed.len() < 2 {
        return Err("too few fields to score".into());
    }
    let body = request_body(cfg, opts.arguments_raw.as_deref(), &listed);
    Ok((
        Request {
            url: format!("{}/alpha/decisions", cfg.base_url.trim_end_matches('/')),
            headers: vec![
                ("Authorization".into(), format!("Bearer {key}")),
                ("Content-Type".into(), "application/json".into()),
            ],
            body,
            timeout_ms: cfg.timeout_ms,
        },
        listed.len(),
    ))
}

/// Ask the model about `v`'s fields. On success: the `(pattern, score)` list for
/// [`Options::field_scores`] and a description for the report.
///
/// The reply comes from the host when it supplied one (`jev_reply`, or
/// `jev_error` for a failed attempt); otherwise a build with the `jev` feature sends the
/// request itself.
pub(crate) fn score_fields(
    v: &Value,
    opts: &Options,
    cfg: &JevOptions,
) -> Result<(Vec<(String, f64)>, String), String> {
    if let Some(why) = &opts.jev_error {
        return Err(why.clone());
    }
    let listed = fields(v);
    if listed.len() < 2 {
        return Err("too few fields to score".into());
    }
    let started = now();
    let reply = match &opts.jev_reply {
        Some(r) => r.clone(),
        None => {
            let (req, _) = build_request(v, opts, cfg)?;
            send(&req)?
        }
    };
    let probs = parse_reply(&reply, listed.len())?;
    let how = match started {
        Some(t) if opts.jev_reply.is_none() => format!(" in {} ms", t.elapsed().as_millis()),
        _ => String::new(),
    };
    let note = format!("scored {} fields with {}{how}", listed.len(), cfg.model);
    Ok((
        listed.into_iter().map(|f| f.path).zip(probs).collect(),
        note,
    ))
}

/// A clock where there is one (wasm has none).
#[cfg(not(target_family = "wasm"))]
fn now() -> Option<std::time::Instant> {
    Some(std::time::Instant::now())
}

#[cfg(target_family = "wasm")]
fn now() -> Option<std::time::Instant> {
    None
}

/// Send the request: a plain HTTPS POST with a hard timeout. Where the build has
/// no HTTP client (the `jev` feature is off, or wasm, which has no sockets) the
/// host must send it and pass the reply back instead (`Options::jev_reply`).
#[cfg(all(feature = "jev", not(target_family = "wasm")))]
fn send(req: &Request) -> Result<String, String> {
    let agent = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_millis(req.timeout_ms))
        .build();
    let mut call = agent.post(&req.url);
    for (k, v) in &req.headers {
        call = call.set(k, v);
    }
    match call.send_string(&req.body) {
        Ok(resp) => resp
            .into_string()
            .map_err(|e| format!("reading the reply failed ({})", e.kind())),
        Err(ureq::Error::Status(code, _)) => Err(format!("the service answered HTTP {code}")),
        Err(ureq::Error::Transport(t)) => {
            if t.to_string().to_ascii_lowercase().contains("timed out") {
                Err("the request timed out".to_string())
            } else {
                Err(format!("the request failed ({})", t.kind()))
            }
        }
    }
}

#[cfg(not(all(feature = "jev", not(target_family = "wasm"))))]
fn send(_req: &Request) -> Result<String, String> {
    Err("this build cannot send the request itself (wasm, or built without the `jev` feature); the host must send it and pass the reply".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(t: &str) -> Value {
        json::parse(t).unwrap()
    }

    #[test]
    fn fields_are_every_leaf_path_with_two_distinct_samples() {
        let v = parse(
            r#"{"total":3,"results":[{"id":"a","tags":["x","y"],"meta":{}},{"id":"b","tags":["x"],"meta":{}},{"id":"a"}],"a/b":1}"#,
        );
        let f = fields(&v);
        let paths: Vec<&str> = f.iter().map(|x| x.path.as_str()).collect();
        assert_eq!(
            paths,
            [
                "/total",
                "/results/*/id",
                "/results/*/tags/*",
                "/results/*/meta"
            ]
        );
        assert_eq!(f[0].samples, ["3"]);
        assert_eq!(f[1].samples, ["a", "b"]);
        assert_eq!(f[2].samples, ["x", "y"]);
        assert!(f[3].samples.is_empty());
    }

    #[test]
    fn request_has_the_state_and_one_question_per_field() {
        let v = parse(r#"{"items":[{"id":"1","name":"alpha"},{"id":"2","name":"beta"}]}"#);
        let f = fields(&v);
        let cfg = JevOptions {
            tool: Some("search".into()),
            ..JevOptions::default()
        };
        let body = parse(&request_body(&cfg, Some(r#"{"q":"alpha"}"#), &f));
        assert_eq!(get(&body, "model"), Some(&string(DEFAULT_MODEL)));
        let Some(Value::String(state)) = get(&body, "state") else {
            panic!("state is JSON text")
        };
        let state = parse(state);
        let call = get(&state, "tool_call").unwrap();
        assert_eq!(get(call, "name"), Some(&string("search")));
        assert_eq!(get(call, "arguments"), Some(&string(r#"{"q":"alpha"}"#)));
        let Some(Value::Array(listed)) = get(&state, "output_fields") else {
            panic!()
        };
        assert_eq!(listed.len(), 2);
        let Some(Value::Object(qs)) = get(&body, "questions") else {
            panic!()
        };
        assert_eq!(qs.len(), 2);
        let Some(Value::String(ins)) = get(&qs[1].1, "instructions") else {
            panic!()
        };
        assert!(ins.contains("`/items/*/name`"));
    }

    #[test]
    fn reply_is_read_and_checked() {
        let ok = r#"{"answers":{"p0":{"type":"noul","noul":0.9},"p1":{"type":"noul","noul":2}}}"#;
        assert_eq!(parse_reply(ok, 2).unwrap(), vec![0.9, 1.0]);
        assert!(parse_reply(ok, 3).is_err());
        assert!(parse_reply("nope", 1).is_err());
        assert!(parse_reply(r#"{"error":"x"}"#, 1).is_err());
    }

    #[test]
    fn options_parse_and_never_print_the_key() {
        assert_eq!(JevOptions::from_value(&parse("false")).unwrap(), None);
        let on = JevOptions::from_value(&parse("true")).unwrap().unwrap();
        assert_eq!(on.model, DEFAULT_MODEL);
        let o = JevOptions::from_value(&parse(
            r#"{"api_key":"sk-or-secret","timeout_ms":900,"tool":"t"}"#,
        ))
        .unwrap()
        .unwrap();
        assert_eq!((o.timeout_ms, o.tool.as_deref()), (900, Some("t")));
        assert!(!format!("{o:?}").contains("sk-or-secret"));
        assert!(JevOptions::from_value(&parse(r#"{"bogus":1}"#)).is_err());
        assert!(JevOptions::from_value(&parse(r#"{"timeout_ms":0}"#)).is_err());
    }
}
