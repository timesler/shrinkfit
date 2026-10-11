//! Format-neutral document model.
//!
//! Every supported format (JSON, YAML, ...) is parsed into and rendered from
//! this tree. Object key order is preserved and numbers keep their original
//! literal text, so an untouched subtree round-trips exactly.

/// A JSON-compatible value.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    /// The number's literal text, exactly as it appeared in the input
    /// (no float round-tripping, no precision loss for big integers).
    Number(String),
    String(String),
    Array(Vec<Value>),
    /// Entries in document order. Duplicate keys are kept as-is.
    Object(Vec<(String, Value)>),
}

impl Value {
    /// Short type name, used for shape signatures and messages.
    pub fn kind(&self) -> &'static str {
        match self {
            Value::Null => "null",
            Value::Bool(_) => "bool",
            Value::Number(_) => "number",
            Value::String(_) => "string",
            Value::Array(_) => "array",
            Value::Object(_) => "object",
        }
    }

    /// Convenience constructor for a number from an integer.
    pub fn int(n: i64) -> Value {
        Value::Number(n.to_string())
    }

    /// Convenience constructor for a string.
    pub fn str<S: Into<String>>(s: S) -> Value {
        Value::String(s.into())
    }
}
