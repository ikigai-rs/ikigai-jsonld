//! Synthetic boundary cases for the correctness audit, through public kernel requests.
use futures::executor::block_on;
use ikigai_core::{ArgRef, Capability, Error, Iri, Kernel, Request, Verb};
use json_ld::syntax::{Parse, UnorderedPartialEq, Value};
use std::sync::Arc;

fn run(op: &str, doc: &str, context: &str, base: &str, face: &str) -> Result<String, Error> {
    let kernel = Kernel::new(Arc::new(ikigai_jsonld::space()));
    let mut request = Request::new(
        Verb::Source,
        Iri::parse(format!("urn:jsonld:{op}")).unwrap(),
    );
    for (key, value) in [
        ("content", doc),
        ("context", context),
        ("base", base),
        ("face", face),
    ] {
        request = request.with_arg(key, ArgRef::Inline(value.as_bytes().to_vec()));
    }
    block_on(kernel.issue(request, &Capability::root()))
        .map(|r| String::from_utf8(r.bytes).unwrap())
}

fn json(text: &str) -> Value {
    Value::parse_str(text).unwrap().0
}

const BASE: &str = "https://example.org/a/document";
const CONTEXT: &str = r#"[{"ex":"urn:ok:","id":"@id","kind":"@type","T":"ex:T","p":"ex:p","nested":"@nest","included":"@included","graph":"@graph"},{"ex":"urn:other:"}]"#;

#[test]
fn keyword_aliases_and_redefined_prefixes_do_not_change_the_allowlist() {
    let doc = r#"{"@context":{"id":"@id","kind":"@type","nested":"@nest","p":"urn:ok:p"},"id":"urn:arbitrary:s","kind":["urn:ok:T","urn:blocked:T"],"nested":{"p":{"@set":[{"id":"urn:arbitrary:o"},"kept"]},"urn:blocked:p":"removed"},"urn:other:p":"removed"}"#;
    let expected = r#"[{"@id":"urn:arbitrary:s","@type":["urn:ok:T"],"urn:ok:p":[{"@id":"urn:arbitrary:o"},{"@value":"kept"}]}]"#;
    let pruned = run("prune", doc, CONTEXT, BASE, "document").unwrap();
    let expanded = run("expand", &pruned, "{}", BASE, "document").unwrap();
    assert!(json(&expanded).unordered_eq(&json(expected)), "{expanded}");
    assert_eq!(
        run("prune", &pruned, CONTEXT, BASE, "document").unwrap(),
        pruned
    );
    let report = run("prune", doc, CONTEXT, BASE, "report").unwrap();
    assert!(report.contains("\"dropped\": 3"), "{report}");
}

#[test]
fn input_scopes_do_not_extend_the_target_allowlist() {
    for scoped in [
        r#"{"@context":{"p":{"@id":"urn:ok:p","@context":{"secret":"urn:blocked:secret","p":"urn:ok:p"}}},"p":{"p":"kept","secret":"removed"}}"#,
        r#"{"@context":{"T":{"@id":"urn:ok:T","@context":{"secret":"urn:blocked:secret","p":"urn:ok:p"}}},"@type":"T","secret":"removed","p":"kept"}"#,
    ] {
        let pruned = run("prune", scoped, CONTEXT, BASE, "document").unwrap();
        assert!(!pruned.contains("removed"), "{pruned}");
        assert!(pruned.contains("kept"), "{pruned}");
        let report = run("prune", scoped, CONTEXT, BASE, "report").unwrap();
        assert!(report.contains("urn:blocked:secret"), "{report}");
    }
    // Both property- and type-scoped target contexts are explicitly unsupported.
    for scoped in [
        r#"{"p":{"@id":"urn:ok:p","@context":{"q":"urn:ok:q"}}}"#,
        r#"{"T":{"@id":"urn:ok:T","@context":{"q":"urn:ok:q"}}}"#,
    ] {
        assert!(
            matches!(run("prune", "{}", scoped, BASE, "document"), Err(Error::InvalidArgument { name, detail }) if name == "context" && detail.contains("scoped"))
        );
    }
}

#[test]
fn included_aliases_and_nested_graphs_are_filtered_recursively() {
    let doc = r#"{"@context":{"g":"@graph","i":"@included","p":"urn:ok:p"},"@id":"urn:g","g":[{"@id":"urn:s","p":"kept","i":[{"p":"also kept"},{"urn:blocked:p":"removed"}]}]}"#;
    let expected = r#"[{"@id":"urn:g","@graph":[{"@id":"urn:s","urn:ok:p":[{"@value":"kept"}],"@included":[{"urn:ok:p":[{"@value":"also kept"}]}]}]}]"#;
    let pruned = run("prune", doc, CONTEXT, BASE, "document").unwrap();
    let expanded = run("expand", &pruned, "{}", BASE, "document").unwrap();
    assert!(json(&expanded).unordered_eq(&json(expected)), "{expanded}");
    let report = run("prune", doc, CONTEXT, BASE, "report").unwrap();
    assert!(report.contains("\"dropped\": 2"), "{report}");
}

#[test]
fn base_and_protected_contexts_preserve_allowed_data_and_value_kinds() {
    let context =
        r#"{"@protected":true,"p":"urn:ok:p","link":{"@id":"urn:ok:link","@type":"@id"}}"#;
    let doc = r#"{"@id":"s","urn:ok:p":"text","urn:ok:link":{"@id":"../o"}}"#;
    let expanded = run("expand", doc, "{}", BASE, "document").unwrap();
    for op in ["compact", "prune"] {
        let result = run(op, doc, context, BASE, "document").unwrap();
        let roundtrip = run("expand", &result, "{}", BASE, "document").unwrap();
        assert!(
            json(&roundtrip).unordered_eq(&json(&expanded)),
            "{op}: {roundtrip}"
        );
    }
    assert!(run("prune", doc, context, BASE, "report")
        .unwrap()
        .contains("\"dropped\": 0"));
}

#[test]
fn relative_remote_contexts_are_refused() {
    for doc in [
        r#"{"@context":"../context.jsonld","p":"value"}"#,
        r#"{"@context":{"@import":"../context.jsonld"},"p":"value"}"#,
    ] {
        for op in ["expand", "flatten", "compact", "prune"] {
            let err = run(op, doc, CONTEXT, BASE, "document").unwrap_err();
            let message = err.to_string();
            assert!(
                message.contains("https://example.org/context.jsonld"),
                "{op}: {message}"
            );
            assert!(message.contains("no loader"), "{op}: {message}");
        }
    }
}
