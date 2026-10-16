use super::*;
use shrinkfit::options::ArrayStrategy;
use std::collections::HashSet;

fn field<'a>(v: &'a Value, name: &str) -> &'a Value {
    match v {
        Value::Object(entries) => &entries.iter().find(|(k, _)| k == name).unwrap().1,
        _ => panic!("expected object"),
    }
}
fn string(v: &Value) -> &str {
    match v {
        Value::String(s) => s,
        _ => panic!("expected string"),
    }
}
fn number(v: &Value) -> usize {
    json::to_string(v, json::Layout::Compact).parse().unwrap()
}

#[test]
fn catalog_matches_embedded_fixtures_and_sizes() {
    let Value::Array(entries) = json::parse(TWG_MANIFEST).unwrap() else {
        panic!()
    };
    assert_eq!(entries.len(), TWG_EXAMPLES.len());
    let mut ids = HashSet::new();
    for entry in entries {
        let id = string(field(&entry, "id"));
        assert!(ids.insert(id.to_owned()), "duplicate fixture {id}");
        let text = twg_asset(&format!("/twg/{id}.json")).expect(id);
        let value = json::parse(text).expect(id);
        assert_eq!(string(field(&value, "apiVersion")), "v2");
        assert_eq!(number(field(&entry, "bytes")), text.len(), "{id}");
        assert_eq!(number(field(&entry, "chars")), text.chars().count(), "{id}");
        assert!(text.len() > 10_000, "{id} must exercise oversized output");
        assert!(number(field(&entry, "source_bytes")) > 10_000);
        assert!(string(field(&entry, "provenance")).contains("synthetic"));
    }
}

#[test]
fn fixtures_shrink_with_both_strategies_without_network() {
    let Value::Array(entries) = json::parse(TWG_MANIFEST).unwrap() else {
        panic!()
    };
    for entry in entries {
        let id = string(field(&entry, "id"));
        let text = twg_asset(&format!("/twg/{id}.json")).unwrap();
        for strategy in [ArrayStrategy::Sample, ArrayStrategy::Condense] {
            let opts = Options {
                max_chars: number(field(&entry, "max_chars")),
                array_strategy: strategy,
                ..Options::default()
            };
            let result = shrink_str(text, &opts).expect(id);
            assert!(result.report.fits, "{id}: {strategy:?}");
            assert!(result.report.pruned, "{id}: {strategy:?}");
            assert!(result.text.chars().count() <= opts.max_chars, "{id}");
            assert!(result.report.model_scoring.is_none());
            json::parse(&result.text).expect(id);
        }
    }
}

#[test]
fn assets_are_allowlisted() {
    assert_eq!(twg_asset("/twg/manifest.json"), Some(TWG_MANIFEST));
    for path in [
        "/twg/missing.json",
        "/twg/../index.html",
        "/twg/%2e%2e/manifest.json",
        "/twg/boards.json/extra",
    ] {
        assert!(twg_asset(path).is_none(), "{path}");
    }
}
