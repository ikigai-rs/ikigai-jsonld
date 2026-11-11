//! Offline W3C fixtures through the public endpoints and the underlying library.
use contextual::WithContext;
use futures::{executor::block_on, FutureExt};
use ikigai_core::{ArgRef, Capability, Iri, Kernel, Request, Verb};
use json_ld::{
    syntax::{Parse, Print, TryFromJson, UnorderedPartialEq, Value},
    IriBuf, JsonLdProcessor, NoLoader, RemoteContextReference, RemoteDocument,
};
use std::{collections::BTreeMap, path::PathBuf, sync::Arc};

fn field<'a>(value: &'a Value, key: &str) -> &'a str {
    value
        .as_object()
        .unwrap()
        .get(key)
        .next()
        .unwrap()
        .as_str()
        .unwrap()
}

fn fixture(path: &str) -> String {
    std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/w3c")
            .join(path),
    )
    .unwrap()
}

fn json(text: &str) -> Value {
    Value::parse_str(text).unwrap().0
}

// These fixtures use blank node identifiers only as @id values. Labels are arbitrary;
// normalize them by first occurrence, preserving all array ordering (including @list).
fn normalize(value: &mut Value, labels: &mut BTreeMap<String, String>) {
    match value {
        Value::Array(items) => items.iter_mut().for_each(|v| normalize(v, labels)),
        Value::Object(object) => {
            for (key, value) in object.iter_mut() {
                if key.as_str() == "@id" {
                    if let Some(label) = value.as_str().filter(|s| s.starts_with("_:")) {
                        let next = format!("_:{}", labels.len());
                        let canonical = labels.entry(label.to_string()).or_insert(next);
                        *value = Value::String(canonical.as_str().into());
                    }
                } else {
                    normalize(value, labels);
                }
            }
        }
        _ => (),
    }
}

// JSON-LD expansion/flattening arrays are unordered except @list. Value payloads
// (including @json) are compared as JSON data, preserving their array ordering.
fn equivalent(a: &Value, b: &Value, unordered_arrays: bool) -> bool {
    match (a, b) {
        (Value::Array(a), Value::Array(b)) if unordered_arrays => {
            let mut remaining: Vec<_> = b.iter().collect();
            for item in a {
                let Some(index) = remaining
                    .iter()
                    .position(|other| equivalent(item, other, true))
                else {
                    return false;
                };
                remaining.remove(index);
            }
            remaining.is_empty()
        }
        (Value::Object(a), Value::Object(b)) => {
            a.len() == b.len()
                && a.iter().all(|entry| {
                    b.get(entry.key.as_str()).next().is_some_and(|other| {
                        equivalent(
                            &entry.value,
                            other,
                            unordered_arrays && !matches!(entry.key.as_str(), "@list" | "@value"),
                        )
                    })
                })
        }
        _ => a.unordered_eq(b),
    }
}

#[test]
fn offline_w3c_cases_through_endpoints_match_the_library_and_expected_results() {
    let manifest = json(include_str!("fixtures/w3c/manifest.json"));
    let mut failures = Vec::new();
    for case in manifest.as_array().unwrap() {
        let op = field(case, "operation");
        let id = field(case, "@id");
        let input = fixture(field(case, "input"));
        let base = field(case, "base");
        let context = case
            .as_object()
            .unwrap()
            .get("context")
            .next()
            .map(|v| fixture(v.as_str().unwrap()));
        let kernel = Kernel::new(Arc::new(ikigai_jsonld::space()));
        let mut request = Request::new(
            Verb::Source,
            Iri::parse(format!("urn:jsonld:{op}")).unwrap(),
        )
        .with_arg("content", ArgRef::Inline(input.as_bytes().to_vec()))
        .with_arg("base", ArgRef::Inline(base.as_bytes().to_vec()));
        if let Some(context) = &context {
            request = request.with_arg("context", ArgRef::Inline(context.as_bytes().to_vec()));
        }
        let result = block_on(kernel.issue(request, &Capability::root()));
        let result = match result {
            Ok(rep) => String::from_utf8(rep.bytes).unwrap(),
            Err(e) => {
                failures.push(format!("{op}{id}: endpoint error: {e}"));
                continue;
            }
        };
        // An independent call with the documented options distinguishes wrapper failures
        // from upstream results; no endpoint implementation helpers are used.
        let doc = RemoteDocument::new(
            Some(IriBuf::new(base.to_string()).unwrap()),
            None,
            json(&input),
        );
        let direct = match op {
            "expand" => doc
                .expand(&NoLoader)
                .now_or_never()
                .unwrap()
                .unwrap()
                .with(&())
                .pretty_print()
                .to_string(),
            "flatten" => doc
                .flatten_using(
                    &mut json_ld::rdf_types::generator::Blank::new(),
                    &NoLoader,
                    json_ld::Options {
                        ordered: true,
                        ..Default::default()
                    },
                )
                .now_or_never()
                .unwrap()
                .unwrap()
                .pretty_print()
                .to_string(),
            "compact" => {
                let ctx_doc = json(context.as_ref().unwrap());
                let ctx = ctx_doc
                    .as_object()
                    .unwrap()
                    .get("@context")
                    .next()
                    .unwrap()
                    .clone();
                let ctx = json_ld::syntax::Context::try_from_json(ctx).unwrap();
                doc.compact(
                    RemoteContextReference::Loaded(RemoteDocument::new(None, None, ctx)),
                    &NoLoader,
                )
                .now_or_never()
                .unwrap()
                .unwrap()
                .pretty_print()
                .to_string()
            }
            _ => unreachable!(),
        };
        assert_eq!(result, direct, "wrapper changed library result: {op}{id}");
        let mut got = json(&result);
        let mut expected = json(&fixture(field(case, "expect")));
        normalize(&mut got, &mut BTreeMap::new());
        normalize(&mut expected, &mut BTreeMap::new());
        if !equivalent(&got, &expected, op != "compact") {
            failures.push(format!(
                "{op}{id}: library and endpoint differ from W3C\nactual: {result}\nexpected: {}",
                expected.pretty_print()
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
