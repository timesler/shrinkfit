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
}

/// Default output budget, in characters.
pub const DEFAULT_MAX_CHARS: usize = 10_000;

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
