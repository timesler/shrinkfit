//! # shrinkfit
//!
//! Shrink a JSON (or YAML) document to fit a character budget while keeping
//! as much of its schema and structure as possible.
//!
//! Built for AI-agent tools that return large structured payloads: instead
//! of failing, or spilling the result to a file, the tool can always answer
//! inline with a faithful, clearly marked sample of the data.
//!
//! ```
//! use shrinkfit::{shrink_json, Options};
//!
//! let big = format!(r#"{{"items":[{}],"total":500}}"#,
//!     (0..500).map(|i| format!(r#"{{"id":{i},"name":"item {i}"}}"#)).collect::<Vec<_>>().join(","));
//! // Output is indented by default; indent 0 gives compact one-line JSON.
//! let opts = Options { indent: 0, ..Options::new(300) };
//! let out = shrink_json(&big, &opts).unwrap();
//! assert!(out.report.output_chars <= 300);
//! assert!(out.text.contains("items omitted"));
//! assert!(out.text.contains(r#""total":500"#));
//! ```
//!
//! Every output is valid in the requested format. Omissions are marked
//! in-band with `…`-prefixed markers (`"…[37 items omitted]"`,
//! `"…": "12 keys omitted"`, `…[+1234 chars]`) so a reader never mistakes a
//! sample for the whole.

mod condense;
pub mod fields;
pub mod jev;
pub mod json;
mod lexical;
pub mod options;
mod protect;
pub mod prune;
pub mod size;
pub mod text;
pub mod value;
pub mod yaml;
#[cfg(feature = "yaml")]
pub mod yaml_in;

pub use fields::{ColumnInfo, FieldScorer, ScorerKind};
pub use jev::JevOptions;
pub use json::{Layout, ParseError};
pub use options::{arguments_text, ArrayStrategy, Format, InputFormat, Options, DEFAULT_MAX_CHARS};
pub use prune::Stats;
pub use text::TextParams;
pub use value::Value;

use prune::Pruner;
use size::Model;

/// What happened during shrinking.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Report {
    /// Size of the input rendered in the output format, in characters.
    pub input_chars: usize,
    /// Size of the returned text, in characters.
    pub output_chars: usize,
    /// The requested budget.
    pub max_chars: usize,
    /// The output format that was used (never `Auto`).
    pub format: Format,
    /// The input format that was read (never `Auto`; JSON for in-memory values).
    pub input_format: InputFormat,
    /// Whether anything was removed.
    pub pruned: bool,
    /// Whether the output is within `max_chars`. Only false when the budget
    /// is smaller than the most compact possible representation, in which
    /// case that representation is returned anyway.
    pub fits: bool,
    /// What happened when model scoring was requested (`Options::jev`): how
    /// many fields were scored, or why it was skipped or failed and the built-in
    /// scoring was used instead. `None` when it was not requested or not needed.
    pub model_scoring: Option<String>,
    pub stats: Stats,
}

fn group(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

impl Report {
    /// One-line, human- and agent-readable description of what was cut, e.g.
    /// `9,981 of 197,236 chars kept (180 array items omitted, 12 strings
    /// truncated)`. Meant for notices next to the shrunk output.
    pub fn summary(&self) -> String {
        let st = &self.stats;
        let mut parts = Vec::new();
        let mut add = |n: usize, what: &str| {
            if n > 0 {
                parts.push(format!("{} {what}", group(n)));
            }
        };
        add(st.items_omitted, "array items omitted");
        add(st.keys_omitted, "object keys omitted");
        add(st.strings_truncated, "strings truncated");
        add(st.containers_collapsed, "containers collapsed");
        if st.protected_omitted > 0 {
            parts.push(format!(
                "{} protected values omitted with their records",
                group(st.protected_omitted)
            ));
        }
        if st.fields_dropped > 0 {
            let shown: Vec<&str> = st
                .dropped_fields
                .iter()
                .take(4)
                .map(|(p, _)| p.as_str())
                .collect();
            let more = if st.fields_dropped > shown.len() {
                ", ..."
            } else {
                ""
            };
            parts.push(format!(
                "{} fields dropped from all items: {}{more}",
                group(st.fields_dropped),
                shown.join(", ")
            ));
        }
        let mut out = format!(
            "{} of {} chars kept",
            group(self.output_chars),
            group(self.input_chars)
        );
        if !parts.is_empty() {
            out.push_str(&format!(" ({})", parts.join(", ")));
        }
        out
    }

    /// The report as a JSON-compatible value (used by the C ABI).
    pub fn to_value(&self) -> Value {
        let n = |x: usize| Value::Number(x.to_string());
        Value::Object(vec![
            ("input_chars".into(), n(self.input_chars)),
            ("output_chars".into(), n(self.output_chars)),
            ("max_chars".into(), n(self.max_chars)),
            (
                "format".into(),
                Value::String(format_name(self.format).into()),
            ),
            (
                "input_format".into(),
                Value::String(
                    format_name(match self.input_format {
                        InputFormat::Yaml => Format::Yaml,
                        _ => Format::Json,
                    })
                    .into(),
                ),
            ),
            ("pruned".into(), Value::Bool(self.pruned)),
            ("fits".into(), Value::Bool(self.fits)),
            (
                "model_scoring".into(),
                self.model_scoring
                    .clone()
                    .map_or(Value::Null, Value::String),
            ),
            ("strings_truncated".into(), n(self.stats.strings_truncated)),
            ("arrays_sampled".into(), n(self.stats.arrays_sampled)),
            ("items_omitted".into(), n(self.stats.items_omitted)),
            ("objects_trimmed".into(), n(self.stats.objects_trimmed)),
            ("keys_omitted".into(), n(self.stats.keys_omitted)),
            (
                "containers_collapsed".into(),
                n(self.stats.containers_collapsed),
            ),
            ("arrays_condensed".into(), n(self.stats.arrays_condensed)),
            ("protected_omitted".into(), n(self.stats.protected_omitted)),
            (
                "field_decisions".into(),
                Value::Array(
                    self.stats
                        .field_decisions
                        .iter()
                        .map(|(path, score, kept, rel)| {
                            Value::Object(vec![
                                ("path".into(), Value::String(path.clone())),
                                (
                                    "score".into(),
                                    Value::Number(format!("{:.3}", *score as f64 / 1000.0)),
                                ),
                                ("kept".into(), Value::Bool(*kept)),
                                (
                                    "relevance".into(),
                                    Value::Number(format!("{:.3}", *rel as f64 / 1000.0)),
                                ),
                            ])
                        })
                        .collect(),
                ),
            ),
            ("fields_dropped".into(), n(self.stats.fields_dropped)),
            (
                "dropped_fields".into(),
                Value::Array(
                    self.stats
                        .dropped_fields
                        .iter()
                        .map(|(path, items)| {
                            Value::Object(vec![
                                ("path".into(), Value::String(path.clone())),
                                ("items".into(), n(*items)),
                            ])
                        })
                        .collect(),
                ),
            ),
        ])
    }
}

/// A shrunk document.
#[derive(Clone, Debug)]
pub struct Shrunk {
    /// The rendered output.
    pub text: String,
    /// The pruned value tree that `text` renders.
    pub value: Value,
    pub report: Report,
}

/// Errors from the string-input entry points.
#[derive(Debug, Clone, PartialEq)]
pub enum Error {
    Parse(ParseError),
    /// Malformed YAML, or YAML input without the `yaml` feature compiled in.
    Yaml(String),
    Options(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Parse(e) => write!(f, "invalid JSON input: {e}"),
            Error::Yaml(e) => write!(f, "invalid YAML input: {e}"),
            Error::Options(e) => write!(f, "invalid options: {e}"),
        }
    }
}

impl std::error::Error for Error {}

fn format_name(f: Format) -> &'static str {
    match f {
        Format::Yaml => "yaml",
        _ => "json",
    }
}

/// Options with `Auto` resolved to `fallback`, and YAML's minimum indent applied.
fn effective(opts: &Options, fallback: Format) -> Options {
    let mut o = opts.clone();
    if o.format == Format::Auto {
        o.format = fallback;
    }
    if o.format == Format::Yaml && o.indent < 2 {
        o.indent = 2;
    }
    o
}

/// Render a value in the format selected by `opts`, without pruning.
/// (`Auto` renders as JSON.)
pub fn render(v: &Value, opts: &Options) -> String {
    match opts.format {
        Format::Json | Format::Auto => json::to_string(v, layout(opts)),
        Format::Yaml => yaml::to_string(v, opts.indent),
    }
}

fn layout(opts: &Options) -> Layout {
    if opts.indent == 0 {
        Layout::Compact
    } else {
        Layout::Pretty(opts.indent)
    }
}

fn chars(s: &str) -> usize {
    s.chars().count()
}

/// Parse JSON text and shrink it.
pub fn shrink_json(input: &str, opts: &Options) -> Result<Shrunk, Error> {
    let v = json::parse(input).map_err(Error::Parse)?;
    Ok(shrink(&v, opts))
}

/// Parse `input` in the format selected by `opts.input_format` and shrink it.
pub fn shrink_str(input: &str, opts: &Options) -> Result<Shrunk, Error> {
    let (v, detected) = parse_input(input, opts.input_format)?;
    let fallback = match detected {
        InputFormat::Yaml => Format::Yaml,
        _ => Format::Json,
    };
    let mut out = shrink(&v, &effective(opts, fallback));
    out.report.input_format = detected;
    Ok(out)
}

#[cfg(feature = "yaml")]
fn parse_yaml(input: &str) -> Result<Value, Error> {
    yaml_in::parse(input).map_err(Error::Yaml)
}

#[cfg(not(feature = "yaml"))]
fn parse_yaml(_: &str) -> Result<Value, Error> {
    Err(Error::Yaml(
        "shrinkfit was built without the `yaml` feature".into(),
    ))
}

/// Parse text as `format`, returning the value and the concrete format
/// (`Json` or `Yaml`) it was read as. `Auto` tries JSON first, then YAML;
/// when both fail, JSON's error is reported for input that looks like JSON
/// (starts with `{`, `[` or `"`) and YAML's otherwise.
pub fn parse_input(input: &str, format: InputFormat) -> Result<(Value, InputFormat), Error> {
    match format {
        InputFormat::Json => Ok((json::parse(input).map_err(Error::Parse)?, InputFormat::Json)),
        InputFormat::Yaml => Ok((parse_yaml(input)?, InputFormat::Yaml)),
        InputFormat::Auto => {
            let jerr = match json::parse(input) {
                Ok(v) => return Ok((v, InputFormat::Json)),
                Err(e) => e,
            };
            let head = input.trim_start_matches('\u{feff}').trim_start();
            if head.is_empty() || head.starts_with(['{', '[', '"']) {
                return Err(Error::Parse(jerr));
            }
            match parse_yaml(input) {
                Ok(v) => Ok((v, InputFormat::Yaml)),
                Err(Error::Yaml(y)) if cfg!(feature = "yaml") => Err(Error::Yaml(y)),
                Err(_) => Err(Error::Parse(jerr)),
            }
        }
    }
}

/// Shrink a value so its rendering fits within `opts.max_chars`.
pub fn shrink(v: &Value, opts: &Options) -> Shrunk {
    shrink_with(v, opts, opts.field_scorer.get())
}

/// Like [`shrink`] with a custom [`FieldScorer`] for the `condense` array
/// strategy (instead of the built-in one named by `opts.field_scorer`).
pub fn shrink_with(v: &Value, opts: &Options, scorer: &dyn FieldScorer) -> Shrunk {
    let opts = &effective(opts, Format::Json);
    let full_text = render(v, opts);
    let input_chars = chars(&full_text);
    if input_chars <= opts.max_chars {
        return Shrunk {
            text: full_text,
            value: v.clone(),
            report: Report {
                input_chars,
                output_chars: input_chars,
                max_chars: opts.max_chars,
                format: opts.format,
                input_format: InputFormat::Json,
                pruned: false,
                fits: true,
                model_scoring: None,
                stats: Stats::default(),
            },
        };
    }

    // Optional model scoring: only worth a request when something has to be cut.
    let mut model_scoring = None;
    let enriched;
    let opts = match &opts.jev {
        Some(cfg) if opts.array_strategy == ArrayStrategy::Condense => {
            match jev::score_fields(v, opts, cfg) {
                Ok((scores, note)) => {
                    let mut o = opts.clone();
                    o.field_scores.extend(scores);
                    enriched = o;
                    model_scoring = Some(note);
                    &enriched
                }
                Err(why) => {
                    model_scoring = Some(format!("unavailable: {why}; used the built-in scoring"));
                    opts
                }
            }
        }
        Some(_) => {
            model_scoring = Some("skipped: model scoring needs array_strategy=condense".into());
            opts
        }
        None => opts,
    };

    // JSON sizes are predicted exactly; YAML sizes are estimated with the
    // pretty-JSON model and corrected below by measuring real output.
    let (model, exact) = match opts.format {
        Format::Json | Format::Auto => (Model::new(layout(opts)), true),
        Format::Yaml => (Model::new(Layout::Pretty(opts.indent.max(2))), false),
    };
    let meta = Pruner::with_scorer(model, opts, scorer).analyze(v, 0);
    let max = opts.max_chars;

    let attempt = |budget: usize| -> (Value, String, usize, Stats) {
        let mut p = Pruner::with_scorer(model, opts, scorer);
        let (out, _) = p.fit(v, &meta, budget, 0);
        let text = render(&out, opts);
        let n = chars(&text);
        (out, text, n, p.stats)
    };

    // Search the internal budget: the largest one whose real output fits.
    let mut best: Option<(Value, String, usize, Stats)> = None;
    let mut smallest: Option<(Value, String, usize, Stats)> = None;
    let (mut lo, mut hi): (Option<usize>, Option<usize>) = (None, None);
    let mut b = max;
    for _ in 0..24 {
        let r = attempt(b);
        let len = r.2;
        if len <= max {
            lo = Some(b);
            let better = best.as_ref().map_or(true, |x| len > x.2);
            let done = exact || max - len <= max / 200;
            if better {
                best = Some(r);
            }
            if done {
                break;
            }
        } else {
            hi = Some(b);
            if smallest.as_ref().map_or(true, |x| len < x.2) {
                smallest = Some(r);
            }
        }
        let next = match (lo, hi) {
            (Some(l), Some(h)) => {
                if h <= l + 1 {
                    break;
                }
                l + (h - l) / 2
            }
            (None, Some(h)) => {
                if h == 0 {
                    break;
                }
                let scaled = (h as f64 * max as f64 / len.max(1) as f64 * 0.97) as usize;
                scaled.min(h - 1)
            }
            (Some(l), None) => {
                let scaled = (l as f64 * max as f64 / len.max(1) as f64) as usize;
                scaled.max(l + 1)
            }
            (None, None) => unreachable!(),
        };
        b = next;
    }

    let (fits, (value, text, output_chars, stats)) = match best {
        Some(r) => (true, r),
        None => (false, smallest.expect("at least one attempt was made")),
    };
    Shrunk {
        text,
        value,
        report: Report {
            input_chars,
            output_chars,
            max_chars: max,
            format: opts.format,
            input_format: InputFormat::Json,
            pruned: true,
            fits,
            model_scoring,
            stats,
        },
    }
}
