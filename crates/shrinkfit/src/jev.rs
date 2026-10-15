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
use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::{Mutex, OnceLock};

/// The model asked by default.
pub const DEFAULT_MODEL: &str = "typesafe/jev-1.13";
/// Where the Decisions API lives (`<base>/alpha/decisions`).
pub const DEFAULT_BASE_URL: &str = "https://openrouter.ai/api";
/// Environment variable holding the OpenRouter API key.
pub const KEY_ENV: &str = "OPENROUTER_API_KEY";
/// Default time allowed for the whole request.
pub const DEFAULT_TIMEOUT_MS: u64 = 3000;

/// How many labelled examples go into each request by default.
pub const DEFAULT_EXAMPLES: usize = 4;
const MAX_EXAMPLES: usize = 12;
/// How many score sets the process-wide cache keeps.
const CACHE_ENTRIES: usize = 256;
/// The labelled examples shipped with shrinkfit (see experiments/path-selection).
const BANK: &str = include_str!("jev_bank.json");

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
    /// How many labelled examples to show the model with the request (0 for none).
    /// The most similar ones are picked from the bank.
    pub examples: usize,
    /// How strongly the model's scores replace the built-in ones, 0 to 1. At the
    /// default of 1 a field the model scored is ranked by the model alone.
    pub weight: f64,
    /// Your own bank of labelled examples, as JSON text (an array like `jev_bank.json`),
    /// used instead of the one shipped with shrinkfit.
    pub bank: Option<String>,
    /// Remember the scores for a tool's field set, so the next response of the same
    /// shape needs no request (default on). The arguments are not part of the key:
    /// measured on pairs of responses, reusing the scores cost about 0.01 of labelled
    /// value kept, against a gain of about 0.07 from the model.
    pub cache: bool,
}

impl Default for JevOptions {
    fn default() -> Self {
        JevOptions {
            api_key: None,
            model: DEFAULT_MODEL.into(),
            base_url: DEFAULT_BASE_URL.into(),
            timeout_ms: DEFAULT_TIMEOUT_MS,
            tool: None,
            examples: DEFAULT_EXAMPLES,
            weight: 1.0,
            bank: None,
            cache: true,
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
            .field("examples", &self.examples)
            .field("weight", &self.weight)
            .field(
                "bank",
                &self.bank.as_ref().map(|b| format!("{} bytes", b.len())),
            )
            .finish()
    }
}

impl JevOptions {
    /// Parse the `jev` option: `true`, or an object with any of `api_key`,
    /// `model`, `base_url`, `timeout_ms`, `tool`, `examples`, `weight`, `bank`, `cache`.
    /// `false`/`null` mean off.
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
                "examples" => {
                    o.examples = match val {
                        Value::Number(n) => n.parse::<usize>().ok().filter(|k| *k <= MAX_EXAMPLES),
                        _ => None,
                    }
                    .ok_or_else(|| {
                        format!("jev.examples must be an integer from 0 to {MAX_EXAMPLES}")
                    })?
                }
                "weight" => {
                    o.weight = match val {
                        Value::Number(n) => {
                            n.parse::<f64>().ok().filter(|w| (0.0..=1.0).contains(w))
                        }
                        _ => None,
                    }
                    .ok_or("jev.weight must be a number from 0 to 1")?
                }
                "cache" => {
                    o.cache = match val {
                        Value::Bool(b) => *b,
                        _ => return Err("jev.cache must be true or false".into()),
                    }
                }
                "bank" => {
                    let text = match val {
                        Value::String(s) => s.clone(),
                        Value::Array(_) => json::to_string(val, Layout::Compact),
                        _ => return Err("jev.bank must be an array of labelled examples".into()),
                    };
                    parse_bank(&text).map_err(|e| format!("jev.bank: {e}"))?;
                    o.bank = Some(text);
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

/// A labelled example: a tool call, its fields, and a verdict for each.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Example {
    tool: String,
    arguments: String,
    /// `(path, example values, verdict)`.
    fields: Vec<(String, Vec<String>, String)>,
    /// Words in the tool name and the last segment of every path, for finding similar ones.
    words: HashSet<String>,
}

fn add_words(text: &str, out: &mut HashSet<String>) {
    let lower = text.to_lowercase();
    for w in lower.split(|c: char| !c.is_ascii_alphanumeric()) {
        if !w.is_empty() {
            out.insert(w.to_string());
        }
    }
}

fn field_words(tool: Option<&str>, paths: impl Iterator<Item = String>) -> HashSet<String> {
    let mut words = HashSet::new();
    for p in paths {
        add_words(p.rsplit('/').next().unwrap_or(""), &mut words);
    }
    if let Some(t) = tool {
        add_words(t, &mut words);
    }
    words
}

/// Read a bank: a JSON array of `{tool, arguments, fields: [{path, example_values, verdict}]}`.
pub(crate) fn parse_bank(text: &str) -> Result<Vec<Example>, String> {
    let root = json::parse(text).map_err(|_| "not valid JSON".to_string())?;
    let Value::Array(items) = &root else {
        return Err("expected an array of examples".into());
    };
    let str_of = |v: &Value, key: &str| match get(v, key) {
        Some(Value::String(s)) => Some(s.clone()),
        _ => None,
    };
    items
        .iter()
        .enumerate()
        .map(|(n, item)| {
            let tool = str_of(item, "tool").ok_or_else(|| format!("example {n} has no tool"))?;
            let Some(Value::Array(fields)) = get(item, "fields") else {
                return Err(format!("example {n} has no fields"));
            };
            let fields = fields
                .iter()
                .map(|f| {
                    let path = str_of(f, "path")
                        .ok_or_else(|| format!("example {n} has a field without a path"))?;
                    let verdict = str_of(f, "verdict")
                        .filter(|v| matches!(v.as_str(), "essential" | "useful" | "drop"));
                    let verdict = verdict.ok_or_else(|| {
                        format!(
                            "example {n}, field {path}: verdict must be essential, useful or drop"
                        )
                    })?;
                    let samples = match get(f, "example_values") {
                        Some(Value::Array(a)) => a
                            .iter()
                            .filter_map(|x| {
                                if let Value::String(s) = x {
                                    Some(s.clone())
                                } else {
                                    None
                                }
                            })
                            .collect(),
                        _ => Vec::new(),
                    };
                    Ok((path, samples, verdict))
                })
                .collect::<Result<Vec<_>, String>>()?;
            let words = field_words(Some(&tool), fields.iter().map(|f| f.0.clone()));
            Ok(Example {
                arguments: str_of(item, "arguments").unwrap_or_default(),
                tool,
                fields,
                words,
            })
        })
        .collect()
}

fn builtin_bank() -> &'static [Example] {
    static PARSED: OnceLock<Vec<Example>> = OnceLock::new();
    PARSED.get_or_init(|| parse_bank(BANK).expect("the bundled example bank is valid"))
}

/// The `k` labelled examples most like this document (overlap of field names and
/// tool name), at most one per tool so they are not near-duplicates.
fn pick_examples(cfg: &JevOptions, listed: &[Field]) -> Result<Vec<Example>, String> {
    if cfg.examples == 0 {
        return Ok(Vec::new());
    }
    let custom;
    let bank: &[Example] = match &cfg.bank {
        Some(text) => {
            custom = parse_bank(text).map_err(|e| format!("jev.bank: {e}"))?;
            &custom
        }
        None => builtin_bank(),
    };
    let mine = field_words(cfg.tool.as_deref(), listed.iter().map(|f| f.path.clone()));
    let mut scored: Vec<(f64, usize)> = bank
        .iter()
        .enumerate()
        .map(|(i, e)| {
            let both = mine.intersection(&e.words).count() as f64;
            let either = mine.union(&e.words).count().max(1) as f64;
            (both / either, i)
        })
        .collect();
    // Most similar first; ties keep the order of the bank.
    scored.sort_by(|a, b| {
        b.0.partial_cmp(&a.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.1.cmp(&b.1))
    });
    let mut tools = HashSet::new();
    let mut picked = Vec::new();
    for (_, i) in scored {
        if tools.insert(bank[i].tool.clone()) {
            picked.push(bank[i].clone());
        }
        if picked.len() == cfg.examples {
            break;
        }
    }
    Ok(picked)
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

/// The request body for the Decisions API: the state (labelled examples, then the
/// tool call and fields, as JSON text) and one yes/no question per field, `p0`,
/// `p1`, ...
pub(crate) fn request_body(
    cfg: &JevOptions,
    arguments: Option<&str>,
    fields: &[Field],
    examples: &[Example],
) -> String {
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
    let mut state = Vec::new();
    if !examples.is_empty() {
        state.push((
            "task",
            string(
                "Judge each output field of the LAST tool call the way the labelled examples do. \
                 A verdict of essential or useful means worth keeping; drop means not worth keeping.",
            ),
        ));
        state.push((
            "labelled_examples",
            Value::Array(
                examples
                    .iter()
                    .map(|e| {
                        let fields = e
                            .fields
                            .iter()
                            .map(|(path, samples, verdict)| {
                                obj(vec![
                                    ("path", string(path)),
                                    (
                                        "example_values",
                                        Value::Array(samples.iter().map(|s| string(s)).collect()),
                                    ),
                                    ("verdict", string(verdict)),
                                ])
                            })
                            .collect();
                        obj(vec![
                            (
                                "tool_call",
                                obj(vec![
                                    ("name", string(&e.tool)),
                                    ("arguments", string(&e.arguments)),
                                ]),
                            ),
                            ("fields", Value::Array(fields)),
                        ])
                    })
                    .collect(),
            ),
        ));
    }
    state.push(("tool_call", obj(call)));
    state.push(("output_fields", Value::Array(listed)));
    let state = obj(state);
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

/// Scores remembered for a field set, oldest dropped first.
struct Cache {
    scores: HashMap<u64, Vec<f64>>,
    order: VecDeque<u64>,
}

fn cache() -> &'static Mutex<Cache> {
    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
    CACHE.get_or_init(|| {
        Mutex::new(Cache {
            scores: HashMap::new(),
            order: VecDeque::new(),
        })
    })
}

/// What the scores depend on: the model and endpoint, the tool, how the request is
/// built (examples, bank) and the exact field list. Not the arguments or the values.
fn cache_key(cfg: &JevOptions, listed: &[Field]) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (
        &cfg.model,
        &cfg.base_url,
        &cfg.tool,
        cfg.examples,
        &cfg.bank,
    )
        .hash(&mut h);
    for f in listed {
        f.path.hash(&mut h);
    }
    h.finish()
}

fn cache_get(key: u64, n: usize) -> Option<Vec<f64>> {
    let c = cache().lock().ok()?;
    c.scores.get(&key).filter(|s| s.len() == n).cloned()
}

fn cache_put(key: u64, scores: &[f64], cap: usize) {
    let Ok(mut c) = cache().lock() else { return };
    if c.scores.insert(key, scores.to_vec()).is_none() {
        c.order.push_back(key);
    }
    while c.order.len() > cap {
        if let Some(old) = c.order.pop_front() {
            c.scores.remove(&old);
        }
    }
}

/// Are this document's scores already remembered (so no request is needed)?
pub(crate) fn is_cached(v: &Value, cfg: &JevOptions) -> bool {
    if !cfg.cache {
        return false;
    }
    let listed = fields(v);
    listed.len() >= 2 && cache_get(cache_key(cfg, &listed), listed.len()).is_some()
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
    let examples = pick_examples(cfg, &listed)?;
    let body = request_body(cfg, opts.arguments_raw.as_deref(), &listed, &examples);
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

/// Each score replaced by its rank within the response, from 0 (lowest) to 1 (highest);
/// ties share their average rank. The model's raw probabilities rank fields well but are
/// bunched together, so blending them with other scores works better on ranks.
pub(crate) fn rank_scores(p: &[f64]) -> Vec<f64> {
    let n = p.len();
    if n < 2 {
        return vec![0.5; n];
    }
    let mut idx: Vec<usize> = (0..n).collect();
    idx.sort_by(|&a, &b| p[a].partial_cmp(&p[b]).unwrap_or(std::cmp::Ordering::Equal));
    let mut out = vec![0.0; n];
    let mut i = 0;
    while i < n {
        let mut j = i;
        while j + 1 < n && p[idx[j + 1]] == p[idx[i]] {
            j += 1;
        }
        let rank = (i + j) as f64 / 2.0 / (n - 1) as f64;
        for &k in &idx[i..=j] {
            out[k] = rank;
        }
        i = j + 1;
    }
    out
}

/// Ask the model about `v`'s fields. On success: the `(pattern, score)` list for
/// [`Options::model_scores`] (rank scores, 0 to 1) and a description for the report.
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
    let key = cfg.cache.then(|| cache_key(cfg, &listed));
    if opts.jev_reply.is_none() {
        if let Some(ranks) = key.and_then(|k| cache_get(k, listed.len())) {
            let note = format!("scored {} fields with {} (cached)", listed.len(), cfg.model);
            return Ok((
                listed.into_iter().map(|f| f.path).zip(ranks).collect(),
                note,
            ));
        }
        if !CAN_SEND {
            // Say so before anything else: such a build cannot read the key either
            // (wasm has no environment), so "no API key" would be misleading.
            return Err(CANNOT_SEND.into());
        }
    }
    let started = now();
    let reply = match &opts.jev_reply {
        Some(r) => r.clone(),
        None => {
            let (req, _) = build_request(v, opts, cfg)?;
            send(&req)?
        }
    };
    let probs = rank_scores(&parse_reply(&reply, listed.len())?);
    if let Some(k) = key {
        cache_put(k, &probs, CACHE_ENTRIES);
    }
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

/// Whether this build can send the request itself.
const CAN_SEND: bool = cfg!(all(feature = "jev", not(target_family = "wasm")));

const CANNOT_SEND: &str = "this build cannot send the request itself (in TypeScript use shrinkAsync; in Rust enable the `jev` feature); the host must send it and pass the reply";

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
    Err(CANNOT_SEND.into())
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
        let body = parse(&request_body(&cfg, Some(r#"{"q":"alpha"}"#), &f, &[]));
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

    #[cfg(not(feature = "jev"))]
    #[test]
    fn a_build_without_a_client_says_so_before_asking_for_a_key() {
        std::env::remove_var(KEY_ENV);
        let v = parse(r#"{"items":[{"id":"1","name":"a"},{"id":"2","name":"b"}]}"#);
        let opts = Options::default();
        let why = score_fields(&v, &opts, &JevOptions::default()).unwrap_err();
        assert!(why.contains("cannot send the request itself"), "{why}");
        // With a reply supplied by the host there is nothing to send.
        let replied = Options {
            jev_reply: Some(r#"{"answers":{"p0":{"noul":1},"p1":{"noul":0}}}"#.into()),
            ..Options::default()
        };
        assert!(score_fields(&v, &replied, &JevOptions::default()).is_ok());
    }

    fn bank_of(tools: &[(&str, &[&str])]) -> String {
        let entries: Vec<String> = tools
            .iter()
            .map(|(tool, paths)| {
                let fields: Vec<String> = paths
                    .iter()
                    .enumerate()
                    .map(|(i, p)| {
                        let verdict = ["essential", "useful", "drop"][i % 3];
                        format!(r#"{{"path":"{p}","example_values":["x"],"verdict":"{verdict}"}}"#)
                    })
                    .collect();
                format!(
                    r#"{{"tool":"{tool}","arguments":"{{}}","fields":[{}]}}"#,
                    fields.join(",")
                )
            })
            .collect();
        format!("[{}]", entries.join(","))
    }

    #[test]
    fn ranks_replace_probabilities_and_ties_share() {
        assert_eq!(rank_scores(&[0.2, 0.9, 0.5]), vec![0.0, 1.0, 0.5]);
        assert_eq!(rank_scores(&[0.5, 0.5, 0.9]), vec![0.25, 0.25, 1.0]);
        assert_eq!(rank_scores(&[0.3]), vec![0.5]);
        assert!(rank_scores(&[]).is_empty());
    }

    #[test]
    fn the_bundled_bank_is_valid_and_covers_many_tools() {
        let bank = builtin_bank();
        assert!(bank.len() >= 30, "{} examples", bank.len());
        let tools: HashSet<&str> = bank.iter().map(|e| e.tool.as_str()).collect();
        assert!(tools.len() >= 15, "{} tools", tools.len());
        for e in bank {
            assert!(e.fields.len() >= 8 && e.fields.len() <= 40);
            assert!(
                e.fields.iter().any(|f| f.2 == "essential")
                    && e.fields.iter().any(|f| f.2 != "essential")
            );
        }
    }

    #[test]
    fn bank_errors_are_specific() {
        assert!(parse_bank("nope").unwrap_err().contains("JSON"));
        assert!(parse_bank("{}").unwrap_err().contains("array"));
        assert!(parse_bank(r#"[{"fields":[]}]"#)
            .unwrap_err()
            .contains("no tool"));
        let bad = r#"[{"tool":"t","fields":[{"path":"/a","verdict":"maybe"}]}]"#;
        assert!(parse_bank(bad).unwrap_err().contains("verdict"));
        assert_eq!(parse_bank("[]").unwrap().len(), 0);
    }

    #[test]
    fn the_most_similar_examples_are_picked_one_per_tool() {
        let bank = bank_of(&[
            (
                "calendar_list_events",
                &["/items/*/summary", "/items/*/start", "/items/*/end"],
            ),
            (
                "calendar_list_events_v2",
                &["/items/*/summary", "/items/*/start", "/items/*/end"],
            ),
            (
                "shop_search",
                &["/hits/*/sku", "/hits/*/price", "/hits/*/title"],
            ),
            ("weather", &["/days/*/temp", "/days/*/wind", "/days/*/rain"]),
        ]);
        let cfg = JevOptions {
            examples: 3,
            bank: Some(bank),
            tool: Some("calendar_list_events".into()),
            ..JevOptions::default()
        };
        let v = parse(
            r#"{"items":[{"summary":"a","start":"1","end":"2"},{"summary":"b","start":"3","end":"4"}]}"#,
        );
        let picked = pick_examples(&cfg, &fields(&v)).unwrap();
        let tools: Vec<&str> = picked.iter().map(|e| e.tool.as_str()).collect();
        assert_eq!(tools[0], "calendar_list_events", "{tools:?}");
        assert_eq!(picked.len(), 3);
        let none = JevOptions {
            examples: 0,
            ..cfg.clone()
        };
        assert!(pick_examples(&none, &fields(&v)).unwrap().is_empty());
        let one = JevOptions { examples: 1, ..cfg };
        assert_eq!(pick_examples(&one, &fields(&v)).unwrap().len(), 1);
    }

    #[test]
    fn examples_go_into_the_state_before_the_call() {
        let v = parse(r#"{"items":[{"id":"1","name":"alpha"},{"id":"2","name":"beta"}]}"#);
        let f = fields(&v);
        let cfg = JevOptions {
            tool: Some("search".into()),
            ..JevOptions::default()
        };
        let examples = pick_examples(&cfg, &f).unwrap();
        assert_eq!(examples.len(), DEFAULT_EXAMPLES);
        let body = parse(&request_body(&cfg, None, &f, &examples));
        let Some(Value::String(state)) = get(&body, "state") else {
            panic!()
        };
        let Value::Object(state) = parse(state) else {
            panic!()
        };
        let keys: Vec<&str> = state.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(
            keys,
            ["task", "labelled_examples", "tool_call", "output_fields"]
        );
        let Some((_, Value::Array(ex))) = state.iter().find(|(k, _)| k == "labelled_examples")
        else {
            panic!()
        };
        assert_eq!(ex.len(), DEFAULT_EXAMPLES);
        let Some(Value::Array(fs)) = get(&ex[0], "fields") else {
            panic!()
        };
        assert!(fs.iter().all(|f| get(f, "verdict").is_some()));
        // Zero examples leaves the state as before.
        let plain = parse(&request_body(&cfg, None, &f, &[]));
        let Some(Value::String(state)) = get(&plain, "state") else {
            panic!()
        };
        assert!(!state.contains("labelled_examples") && !state.contains("task"));
    }

    #[test]
    fn example_weight_and_bank_options_are_validated() {
        let ok = JevOptions::from_value(&parse(r#"{"examples":2,"weight":0.5}"#))
            .unwrap()
            .unwrap();
        assert_eq!((ok.examples, ok.weight), (2, 0.5));
        let defaults = JevOptions::from_value(&parse("true")).unwrap().unwrap();
        assert_eq!(
            (defaults.examples, defaults.weight),
            (DEFAULT_EXAMPLES, 1.0)
        );
        assert!(JevOptions::from_value(&parse(r#"{"examples":99}"#)).is_err());
        assert!(JevOptions::from_value(&parse(r#"{"examples":-1}"#)).is_err());
        assert!(JevOptions::from_value(&parse(r#"{"weight":1.5}"#)).is_err());
        let bank = JevOptions::from_value(&parse(&format!(
            r#"{{"bank":{}}}"#,
            bank_of(&[("t", &["/a", "/b"])])
        )))
        .unwrap();
        assert!(bank.unwrap().bank.is_some());
        assert!(JevOptions::from_value(&parse(r#"{"bank":[{"tool":"t"}]}"#))
            .unwrap_err()
            .contains("jev.bank"));
    }

    #[test]
    fn the_cache_forgets_the_oldest_entries_first() {
        let base = 0xABCD_0000_0000_0000u64;
        for i in 0..5 {
            cache_put(base + i, &[i as f64, 0.0], 3);
        }
        // Only the three newest of this test's keys can still be there.
        let present: Vec<u64> = (0..5)
            .filter(|i| cache_get(base + i, 2).is_some())
            .collect();
        assert!(present.len() <= 3, "{present:?}");
        assert!(cache_get(base + 4, 2).is_some(), "the newest is kept");
        assert!(cache_get(base, 2).is_none(), "the oldest is gone");
        assert!(
            cache_get(base + 4, 3).is_none(),
            "a different length is a miss"
        );
    }

    #[test]
    fn the_cache_key_ignores_values_but_not_fields_or_settings() {
        let a = fields(&parse(r#"{"items":[{"id":"1","name":"x"}]}"#));
        let b = fields(&parse(r#"{"items":[{"id":"2","name":"y"}]}"#));
        let c = fields(&parse(r#"{"items":[{"id":"2","title":"y"}]}"#));
        let cfg = JevOptions::default();
        assert_eq!(cache_key(&cfg, &a), cache_key(&cfg, &b));
        assert_ne!(cache_key(&cfg, &a), cache_key(&cfg, &c));
        let other = JevOptions {
            model: "other/model".into(),
            ..JevOptions::default()
        };
        assert_ne!(cache_key(&cfg, &a), cache_key(&other, &a));
        let off = JevOptions::from_value(&parse(r#"{"cache":false}"#))
            .unwrap()
            .unwrap();
        assert!(!off.cache);
        assert!(JevOptions::from_value(&parse(r#"{"cache":"no"}"#)).is_err());
    }
}
