//! JSON-line evaluation adapter. Does not print authentication headers or read API keys.
//! `cargo build -p shrinkfit --example evaluate`; see experiments/twg/evaluate.py.
use shrinkfit::{
    json::{self, Layout},
    Options, Value,
};
use std::io::{self, BufRead};
fn get<'a>(v: &'a Value, key: &str) -> &'a Value {
    if let Value::Object(entries) = v {
        if let Some((_, value)) = entries.iter().find(|(k, _)| k == key) {
            return value;
        }
    }
    panic!("missing {key}")
}
fn text(v: &Value) -> &str {
    if let Value::String(s) = v {
        s
    } else {
        panic!("expected string")
    }
}
fn main() {
    for line in io::stdin().lock().lines() {
        let request = json::parse(&line.unwrap()).unwrap();
        let input = text(get(&request, "input"));
        let opts = Options::from_value(get(&request, "options")).unwrap();
        let output = if text(get(&request, "action")) == "request" {
            let req = shrinkfit::model_request(input, &opts)
                .unwrap()
                .expect("model request required");
            req.body
        } else {
            let result = shrinkfit::shrink_str(input, &opts).unwrap();
            format!(
                "{{\"output\":{},\"report\":{}}}",
                json::to_string(&Value::String(result.text), Layout::Compact),
                json::to_string(&result.report.to_value(), Layout::Compact)
            )
        };
        println!("{output}");
    }
}
