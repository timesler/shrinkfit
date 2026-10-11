//! YAML input (behind the `yaml` feature): parse a YAML document into the
//! same [`Value`] tree JSON uses, so everything downstream is shared.
//!
//! Mapping: scalars follow the YAML 1.2 core schema as implemented by
//! `yaml-rust2`; non-string mapping keys are stringified; non-finite floats
//! (`.inf`, `.nan`) have no JSON form and become strings; several documents
//! in one stream become an array; an empty stream is `null`. Aliases are
//! expanded by the parser, so a hostile "billion laughs" document can use a
//! lot of memory: don't feed untrusted YAML of unbounded size.

use crate::json::MAX_DEPTH;
use crate::value::Value;
use yaml_rust2::{Yaml, YamlLoader};

/// Parse YAML text into a [`Value`].
pub fn parse(text: &str) -> Result<Value, String> {
    let docs = YamlLoader::load_from_str(text).map_err(|e| e.to_string())?;
    let mut vals = Vec::with_capacity(docs.len());
    for d in &docs {
        vals.push(convert(d, 0)?);
    }
    Ok(match vals.len() {
        0 => Value::Null,
        1 => vals.pop().unwrap(),
        _ => Value::Array(vals),
    })
}

fn convert(y: &Yaml, depth: usize) -> Result<Value, String> {
    if depth > MAX_DEPTH {
        return Err(format!("nesting deeper than {MAX_DEPTH} levels"));
    }
    Ok(match y {
        Yaml::Null => Value::Null,
        Yaml::Boolean(b) => Value::Bool(*b),
        Yaml::Integer(i) => Value::Number(i.to_string()),
        Yaml::Real(s) => match s.parse::<f64>() {
            Ok(f) if f.is_finite() => Value::Number(json_number(s, f)),
            _ => Value::String(s.clone()),
        },
        Yaml::String(s) => Value::String(s.clone()),
        Yaml::Array(a) => Value::Array(
            a.iter()
                .map(|x| convert(x, depth + 1))
                .collect::<Result<_, _>>()?,
        ),
        Yaml::Hash(h) => {
            let mut out: Vec<(String, Value)> = Vec::with_capacity(h.len());
            for (k, v) in h {
                let key = key_string(k)?;
                let val = convert(v, depth + 1)?;
                // YAML forbids duplicate keys but parsers differ; keep the last.
                match out.iter_mut().find(|(ek, _)| *ek == key) {
                    Some(slot) => slot.1 = val,
                    None => out.push((key, val)),
                }
            }
            Value::Object(out)
        }
        Yaml::Alias(_) => return Err("unresolved alias".into()),
        Yaml::BadValue => return Err("invalid value".into()),
    })
}

/// Keep the literal when it is already a valid JSON number (e.g. `1.50`),
/// otherwise fall back to Rust's shortest round-trip form (`.5`, `1.`, `+1`).
fn json_number(lit: &str, f: f64) -> String {
    match crate::json::parse(lit) {
        Ok(Value::Number(_)) => lit.to_string(),
        _ => format!("{f}"),
    }
}

fn key_string(k: &Yaml) -> Result<String, String> {
    Ok(match k {
        Yaml::String(s) => s.clone(),
        Yaml::Integer(i) => i.to_string(),
        Yaml::Real(s) => s.clone(),
        Yaml::Boolean(b) => b.to_string(),
        Yaml::Null => "null".into(),
        _ => return Err("complex mapping keys are not supported".into()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json::{to_string, Layout};

    #[test]
    fn scalars_and_structure() {
        let v = parse("a: 1\nb: [true, null, 1.50, .5, hi]\nc:\n  d: 'x'\n").unwrap();
        assert_eq!(
            to_string(&v, Layout::Compact),
            r#"{"a":1,"b":[true,null,1.50,0.5,"hi"],"c":{"d":"x"}}"#
        );
    }

    #[test]
    fn keys_docs_and_special_floats() {
        let v = parse("1: a\ntrue: b\n.inf: c\nx: .nan\n").unwrap();
        assert_eq!(
            to_string(&v, Layout::Compact),
            r#"{"1":"a","true":"b",".inf":"c","x":".nan"}"#
        );
        let v = parse("---\na: 1\n---\nb: 2\n").unwrap();
        assert_eq!(to_string(&v, Layout::Compact), r#"[{"a":1},{"b":2}]"#);
        assert_eq!(parse("").unwrap(), Value::Null);
    }

    #[test]
    fn anchors_expand_and_errors_reported() {
        let v = parse("a: &x [1, 2]\nb: *x\n").unwrap();
        assert_eq!(to_string(&v, Layout::Compact), r#"{"a":[1,2],"b":[1,2]}"#);
        assert!(parse("a: [1, 2").is_err());
    }
}
