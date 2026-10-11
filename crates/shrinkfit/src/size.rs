//! Compositional size model.
//!
//! The pruner needs to know what a subtree will cost *before* rendering it.
//! For JSON this model is exact: the size of a container equals its
//! overhead plus the sizes of its keys and children. For other formats it
//! is an estimate, and the outer fitting loop in `lib.rs` corrects for any
//! error by measuring the real rendered text.

use crate::json::{self, Layout};
use crate::value::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Model {
    pub layout: Layout,
}

impl Model {
    pub fn new(layout: Layout) -> Self {
        Model { layout }
    }

    /// Exact rendered size of a string value (quotes and escapes included).
    #[inline]
    pub fn string(&self, s: &str) -> usize {
        json::string_len(s)
    }

    /// Exact rendered size of a scalar (non-container) value.
    pub fn scalar(&self, v: &Value) -> usize {
        match v {
            Value::Null => 4,
            Value::Bool(true) => 4,
            Value::Bool(false) => 5,
            Value::Number(n) => n.chars().count(),
            Value::String(s) => self.string(s),
            _ => unreachable!("scalar() called on a container"),
        }
    }

    /// Cost of an object key including its separator (`"k":` or `"k": `).
    #[inline]
    pub fn key(&self, k: &str) -> usize {
        self.string(k)
            + match self.layout {
                Layout::Compact => 1,
                Layout::Pretty(_) => 2,
            }
    }

    /// Size of a container's brackets, separators, newlines and indentation
    /// for `n` children at nesting `depth`, excluding keys and child values.
    #[inline]
    pub fn overhead(&self, n: usize, depth: usize) -> usize {
        if n == 0 {
            return 2;
        }
        match self.layout {
            Layout::Compact => n + 1,
            Layout::Pretty(w) => n * ((depth + 1) * w + 2) + 2 + depth * w,
        }
    }

    /// Full rendered size of an arbitrary value at `depth`.
    pub fn size(&self, v: &Value, depth: usize) -> usize {
        match v {
            Value::Array(items) => {
                self.overhead(items.len(), depth)
                    + items.iter().map(|x| self.size(x, depth + 1)).sum::<usize>()
            }
            Value::Object(entries) => {
                self.overhead(entries.len(), depth)
                    + entries
                        .iter()
                        .map(|(k, x)| self.key(k) + self.size(x, depth + 1))
                        .sum::<usize>()
            }
            other => self.scalar(other),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json::{parse, to_string};

    #[test]
    fn model_is_exact_for_json() {
        let docs = [
            r#"{"a":[1,{"b":[]},{}],"c":"x\ny","d":{"e":{"f":[null,true,false,1.5e3]}}}"#,
            r#"[]"#,
            r#"[[[[]]],{"":""}]"#,
            r#""\u0001 héllo""#,
        ];
        for d in docs {
            let v = parse(d).unwrap();
            for layout in [
                Layout::Compact,
                Layout::Pretty(2),
                Layout::Pretty(4),
                Layout::Pretty(1),
            ] {
                let m = Model::new(layout);
                assert_eq!(
                    m.size(&v, 0),
                    to_string(&v, layout).chars().count(),
                    "{d} {layout:?}"
                );
            }
        }
    }
}
