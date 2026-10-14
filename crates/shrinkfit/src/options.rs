//! User-facing options, constructible in Rust or parsed from a JSON object
//! (which is how the C ABI and the language bindings pass them).

use crate::fields::ScorerKind;
use crate::text::TextParams;
use crate::value::Value;

/// Output format.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    /// Same as the input format (JSON when shrinking an in-memory value).
    Auto,
    Json,
    Yaml,
}

/// Input format of the text handed to [`crate::shrink_str`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputFormat {
    /// Try JSON first, then YAML (when the `yaml` feature is on).
    Auto,
    Json,
    /// Requires the `yaml` cargo feature.
    Yaml,
}

/// How an array that is too big is reduced.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArrayStrategy {
    /// Keep a sample of whole items and mark the gaps (the default).
    Sample,
    /// Keep every item and drop the same low-value fields from all of them
    /// (falls back to `Sample` for arrays that are not uniform records).
    Condense,
}

/// Options controlling how a document is shrunk.
#[derive(Clone, Debug, PartialEq)]
pub struct Options {
    /// Hard ceiling on the output size, in Unicode characters
    /// (default [`DEFAULT_MAX_CHARS`]).
    pub max_chars: usize,
    /// Output format (default: same as the input).
    pub format: Format,
    /// Format of text input (default: auto-detect; ignored by [`crate::shrink`], which takes a value).
    pub input_format: InputFormat,
    /// Indent width (default 2). For JSON, 0 means compact single-line
    /// output. YAML cannot be compact: widths below 2 become 2.
    pub indent: usize,
    /// String truncation tunables (see [`TextParams`]).
    pub text: TextParams,
    /// When sampling an array, also keep its last item so readers can see
    /// how the sequence ends.
    pub keep_last_item: bool,
    /// When not even one array item fits whole, how many items with
    /// distinct shapes to show in reduced form.
    pub max_samples: usize,
    /// Smallest budget worth spending on one extra, partially pruned array
    /// item after the whole items that fit.
    pub min_partial_chars: usize,
    /// How oversized arrays are reduced (default: sample items).
    pub array_strategy: ArrayStrategy,
    /// Which scorer ranks fields for [`ArrayStrategy::Condense`].
    pub field_scorer: ScorerKind,
    /// What the caller asked for: the tool call's argument *values* as text
    /// (names are ignored). Fields whose names or values mention these words
    /// are boosted by the `condense` strategy's heuristic scorer.
    pub arguments: Option<String>,
    /// Record every field's score and keep/drop decision in the report
    /// (`condense` only). For tuning and debugging.
    pub explain: bool,
    /// Paths whose values must never be altered (see `protect`): `/results/*/id`,
    /// `ari`, `content/id`. Protected values are kept exactly or, if they cannot
    /// fit, the result is reported with `fits: false`.
    pub protect: Vec<String>,
    /// Field scores supplied from outside (for example by a model that has read
    /// the response): `(path pattern, score in 0..=1)`, with the pattern syntax
    /// of `protect`. The first matching pattern wins. A field they name is scored
    /// `(1 - field_score_weight) * built-in + field_score_weight * external`;
    /// fields they do not name keep the built-in score.
    pub field_scores: Vec<(String, f64)>,
    /// How strongly `field_scores` pull against the built-in score (default 0.5).
    pub field_score_weight: f64,
    /// Score fields with a hosted decision model and blend that in as
    /// `field_scores` (see [`crate::jev`]). Opt-in; needs an OpenRouter API key
    /// and the `jev` cargo feature, and falls back to the built-in scoring.
    pub jev: Option<crate::jev::JevOptions>,
    /// The call's arguments as given (JSON text or a plain string), for context
    /// to the model scoring. Unlike `arguments` this keeps the keys.
    pub arguments_raw: Option<String>,
    /// For hosts that send the model request themselves (see [`crate::model_request`]):
    /// the response body they got back. With it the core makes no request.
    pub jev_reply: Option<String>,
    /// A failed attempt by the host, described in the report; the built-in
    /// scoring is used.
    pub jev_error: Option<String>,
}

/// Default output budget, in characters.
pub const DEFAULT_MAX_CHARS: usize = 10_000;

/// Parse a `field_scores` object (`{"/results/*/title": 0.9, ...}`) into the
/// ordered `(pattern, score)` list used by [`Options::field_scores`].
pub fn parse_field_scores(v: &Value) -> Result<Vec<(String, f64)>, String> {
    let Value::Object(entries) = v else {
        return Err("field_scores must be an object of path pattern to score".into());
    };
    entries
        .iter()
        .map(|(k, v)| match v {
            Value::Number(n) => n
                .parse::<f64>()
                .ok()
                .filter(|x| x.is_finite())
                .map(|x| (k.clone(), x.clamp(0.0, 1.0)))
                .ok_or_else(|| "field_scores values must be numbers".to_string()),
            _ => Err("field_scores values must be numbers".to_string()),
        })
        .collect()
}

/// The values inside tool arguments, as text: a string is used as is, objects
/// and arrays contribute their values (keys are dropped), numbers and
/// booleans their literal text.
pub fn arguments_text(v: &Value) -> Option<String> {
    fn go(v: &Value, out: &mut Vec<String>) {
        match v {
            Value::String(s) => out.push(s.clone()),
            Value::Number(n) => out.push(n.clone()),
            Value::Array(a) => a.iter().for_each(|x| go(x, out)),
            Value::Object(o) => o.iter().for_each(|(_, x)| go(x, out)),
            Value::Bool(_) | Value::Null => {}
        }
    }
    let mut parts = Vec::new();
    go(v, &mut parts);
    let text = parts.join(" ");
    (!text.trim().is_empty()).then_some(text)
}

impl Default for Options {
    fn default() -> Self {
        Options::new(DEFAULT_MAX_CHARS)
    }
}

impl Options {
    pub fn new(max_chars: usize) -> Self {
        Options {
            max_chars,
            format: Format::Auto,
            input_format: InputFormat::Auto,
            indent: 2,
            text: TextParams::default(),
            keep_last_item: true,
            max_samples: 3,
            min_partial_chars: 40,
            array_strategy: ArrayStrategy::Sample,
            field_scorer: ScorerKind::Heuristic,
            explain: false,
            arguments: None,
            protect: Vec::new(),
            field_scores: Vec::new(),
            field_score_weight: 0.5,
            jev: None,
            arguments_raw: None,
            jev_reply: None,
            jev_error: None,
        }
    }

    pub fn yaml(mut self) -> Self {
        self.format = Format::Yaml;
        self
    }

    /// Build options from a JSON object such as
    /// `{"max_chars": 4000, "format": "yaml", "string_head_chars": 300}`.
    /// Unknown keys are rejected so typos don't silently do nothing.
    pub fn from_value(v: &Value) -> Result<Options, String> {
        let entries = match v {
            Value::Object(e) => e,
            _ => return Err("options must be a JSON object".into()),
        };
        let mut o = Options::default();
        for (k, val) in entries {
            let uint = || -> Result<usize, String> {
                match val {
                    Value::Number(n) => n
                        .parse::<usize>()
                        .map_err(|_| format!("{k} must be a non-negative integer")),
                    _ => Err(format!("{k} must be a non-negative integer")),
                }
            };
            let boolean = || -> Result<bool, String> {
                match val {
                    Value::Bool(b) => Ok(*b),
                    _ => Err(format!("{k} must be a boolean")),
                }
            };
            match k.as_str() {
                "max_chars" => o.max_chars = uint()?,
                "format" => {
                    o.format = match val {
                        Value::String(s) if s.eq_ignore_ascii_case("auto") => Format::Auto,
                        Value::String(s) if s.eq_ignore_ascii_case("json") => Format::Json,
                        Value::String(s)
                            if s.eq_ignore_ascii_case("yaml") || s.eq_ignore_ascii_case("yml") =>
                        {
                            Format::Yaml
                        }
                        _ => return Err("format must be \"auto\", \"json\" or \"yaml\"".into()),
                    }
                }
                "input_format" => {
                    o.input_format = match val {
                        Value::String(s) if s.eq_ignore_ascii_case("auto") => InputFormat::Auto,
                        Value::String(s) if s.eq_ignore_ascii_case("json") => InputFormat::Json,
                        Value::String(s)
                            if s.eq_ignore_ascii_case("yaml") || s.eq_ignore_ascii_case("yml") =>
                        {
                            InputFormat::Yaml
                        }
                        _ => {
                            return Err("input_format must be \"auto\", \"json\" or \"yaml\"".into())
                        }
                    }
                }
                "indent" => o.indent = uint()?,
                "string_head_chars" => o.text.head = uint()?,
                "string_section_chars" => o.text.section = uint()?,
                "string_min_section_chars" => o.text.min_section = uint()?,
                "keep_last_item" => o.keep_last_item = boolean()?,
                "max_samples" => o.max_samples = uint()?,
                "min_partial_chars" => o.min_partial_chars = uint()?,
                "explain" => o.explain = boolean()?,
                "protect" => {
                    o.protect = match val {
                        Value::Array(items) => items
                            .iter()
                            .map(|x| match x {
                                Value::String(s) => Ok(s.clone()),
                                _ => Err("protect must be a list of strings".to_string()),
                            })
                            .collect::<Result<Vec<_>, _>>()?,
                        Value::String(s) => vec![s.clone()],
                        _ => return Err("protect must be a list of strings".into()),
                    }
                }
                "field_scores" => o.field_scores = parse_field_scores(val)?,
                "field_score_weight" => {
                    let w = match val {
                        Value::Number(n) => n.parse::<f64>().ok().filter(|x| x.is_finite()),
                        _ => None,
                    };
                    o.field_score_weight = w
                        .filter(|x| (0.0..=1.0).contains(x))
                        .ok_or("field_score_weight must be a number from 0 to 1")?;
                }
                "arguments" => {
                    o.arguments = arguments_text(val);
                    o.arguments_raw = match val {
                        Value::String(s) => Some(s.clone()),
                        Value::Null => None,
                        other => Some(crate::json::to_string(other, crate::json::Layout::Compact)),
                    };
                }
                "jev" => o.jev = crate::jev::JevOptions::from_value(val)?,
                "jev_reply" | "jev_error" => {
                    let Value::String(text) = val else {
                        return Err(format!("{k} must be a string"));
                    };
                    if k == "jev_reply" {
                        o.jev_reply = Some(text.clone());
                    } else {
                        o.jev_error = Some(text.clone());
                    }
                }
                "array_strategy" => {
                    o.array_strategy = match val {
                        Value::String(s) if s.eq_ignore_ascii_case("sample") => {
                            ArrayStrategy::Sample
                        }
                        Value::String(s) if s.eq_ignore_ascii_case("condense") => {
                            ArrayStrategy::Condense
                        }
                        _ => return Err("array_strategy must be \"sample\" or \"condense\"".into()),
                    }
                }
                "field_scorer" => {
                    o.field_scorer = match val {
                        Value::String(s) => ScorerKind::from_name(s).ok_or_else(|| {
                            format!("field_scorer must be one of {:?}", ScorerKind::NAMES)
                        })?,
                        _ => return Err("field_scorer must be a string".into()),
                    }
                }
                other => return Err(format!("unknown option: {other}")),
            }
        }
        if o.format == Format::Yaml && o.indent < 2 {
            return Err("YAML indent must be at least 2".into());
        }
        Ok(o)
    }
}
