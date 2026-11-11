//! Semantic and loader probes from the unled JSON-LD audit. These are finite fixtures,
//! not a claim of complete JSON-LD 1.1 or host transreptor conformance.
use futures::executor::block_on;
use ikigai_core::{
    ArgRef, Capability, Exact, FnEndpoint, Invocation, Iri, Kernel, ReprType, Representation,
    Request, Verb,
};
use json_ld::syntax::{Parse, UnorderedPartialEq, Value};
use std::{
    collections::BTreeSet,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};

const CONTEXT: &str =
    r#"{"name":"urn:ex:name","link":{"@id":"urn:ex:p","@type":"@id"},"text":"urn:ex:p"}"#;
const EXPANDED: &str = r#"[{"@id":"urn:ex:s","urn:ex:name":[{"@value":"Ada"}],"urn:ex:p":[{"@id":"urn:ex:o"},{"@value":"urn:ex:o"}]}]"#;
const PREFIXED: &str = r#"{"@context":{"other":"urn:ex:","ref":{"@id":"other:p","@type":"@id"}},"@id":"other:s","other:name":"Ada","ref":"other:o","other:p":"urn:ex:o"}"#;

fn run(kernel: &Kernel, op: &str, doc: &str, context: &str) -> ikigai_core::Result<String> {
    let req = Request::new(
        Verb::Source,
        Iri::parse(format!("urn:jsonld:{op}")).unwrap(),
    )
    .with_arg("content", ArgRef::Inline(doc.as_bytes().to_vec()))
    .with_arg("context", ArgRef::Inline(context.as_bytes().to_vec()));
    block_on(kernel.issue(req, &Capability::scoped(Vec::<String>::new())))
        .map(|r| String::from_utf8(r.bytes).unwrap())
}

fn json(text: &str) -> Value {
    Value::parse_str(text).unwrap().0
}

#[test]
fn a_fixed_context_compacts_equivalent_serializations_and_preserves_value_kinds() {
    let kernel = Kernel::new(Arc::new(ikigai_jsonld::space()));
    let compact = format!(
        r#"{{"@context":{CONTEXT},"@id":"urn:ex:s","name":"Ada","link":"urn:ex:o","text":"urn:ex:o"}}"#
    );
    let graph = format!(r#"{{"@graph":{EXPANDED}}}"#);
    let flattened = run(&kernel, "flatten", PREFIXED, CONTEXT).unwrap();
    for doc in [EXPANDED, PREFIXED, &compact, &graph, &flattened] {
        let result = run(&kernel, "compact", doc, CONTEXT).unwrap();
        assert!(
            json(&result).unordered_eq(&json(&compact)),
            "{doc}: {result}"
        );
        let expanded = run(&kernel, "expand", &result, CONTEXT).unwrap();
        assert!(json(&expanded).unordered_eq(&json(EXPANDED)), "{expanded}");
    }
    // Independent Turtle witness for the RDF graph represented by the expanded fixture.
    // This does not execute a host's Turtle-to-JSON-LD transreptor.
    let turtle = br#"<urn:ex:s> <urn:ex:name> "Ada"; <urn:ex:p> <urn:ex:o>, "urn:ex:o" ."#;
    let triples = |media, bytes: &[u8]| {
        ikigai_conformance::rdf::parse(media, bytes)
            .unwrap()
            .into_iter()
            .map(|t| t.to_string())
            .collect::<BTreeSet<_>>()
    };
    assert_eq!(
        triples("text/turtle", turtle),
        triples("application/ld+json", EXPANDED.as_bytes())
    );
}

#[test]
fn embedded_remote_contexts_and_imports_never_call_the_kernel_loader() {
    let calls = Arc::new(AtomicUsize::new(0));
    let observed = calls.clone();
    let space = ikigai_jsonld::space().bind(
        Exact::new("urn:httpGet"),
        FnEndpoint::new("http-get", move |_inv: &Invocation<'_>| {
            observed.fetch_add(1, Ordering::SeqCst);
            Ok(Representation::new(
                ReprType::new("application/ld+json"),
                br#"{"@context":{"p":"urn:ex:p"}}"#.to_vec(),
            ))
        }),
    );
    let kernel = Kernel::new(Arc::new(space));
    for context in [
        r#""https://example.org/context""#,
        r#"{"@import":"https://example.org/context"}"#,
    ] {
        let doc = format!(r#"{{"@context":{context},"p":"value"}}"#);
        for op in ["expand", "flatten", "compact", "prune"] {
            assert!(run(&kernel, op, &doc, CONTEXT).is_err(), "{op}: {doc}");
        }
        let target = format!(r#"{{"@context":{context}}}"#);
        for op in ["compact", "prune"] {
            assert!(
                run(&kernel, op, EXPANDED, &target).is_err(),
                "{op}: {target}"
            );
        }
    }
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[test]
fn protected_terms_and_self_references_are_refused() {
    let kernel = Kernel::new(Arc::new(ikigai_jsonld::space()));
    for context in [
        r#"[{"p":{"@id":"urn:ex:p","@protected":true}},{"p":"urn:ex:q"}]"#,
        r#"{"p":{"@type":"p","@id":"urn:ex:p"}}"#,
        r#"{"p":"q","q":"p"}"#,
    ] {
        let doc = format!(r#"{{"@context":{context},"p":"value"}}"#);
        for op in ["expand", "flatten", "compact", "prune"] {
            assert!(run(&kernel, op, &doc, CONTEXT).is_err(), "{op}: {doc}");
        }
        for op in ["compact", "prune"] {
            assert!(
                run(&kernel, op, EXPANDED, context).is_err(),
                "{op}: {context}"
            );
        }
    }
}

#[test]
fn long_iris_and_json_literal_contents_keep_their_meaning() {
    let kernel = Kernel::new(Arc::new(ikigai_jsonld::space()));
    let iri = format!("https://example.org/{}", "x".repeat(100_000));
    let context = format!(r#"{{"p":{{"@id":"{iri}","@type":"@id"}}}}"#);
    let doc = format!(r#"{{"@id":"urn:ex:s","{iri}":[{{"@id":"{iri}"}}]}}"#);
    let compacted = run(&kernel, "compact", &doc, &context).unwrap();
    let expanded = run(&kernel, "expand", &compacted, &context).unwrap();
    assert!(json(&expanded).unordered_eq(&json(&format!("[{doc}]"))));

    let literal = r#"[{"@id":"urn:ex:s","urn:ex:p":[{"@value":{"@context":{"p":"q","q":"p"}},"@type":"@json"}]}]"#;
    let compacted = run(&kernel, "compact", literal, r#"{"p":"urn:ex:p"}"#).unwrap();
    let expanded = run(&kernel, "expand", &compacted, CONTEXT).unwrap();
    assert!(json(&expanded).unordered_eq(&json(literal)), "{expanded}");
    let coerced = r#"{"@context":{"p":{"@id":"urn:ex:p","@type":"@json"}},"@id":"urn:ex:s","p":{"@context":{"p":"q","q":"p"}}}"#;
    let expanded = run(&kernel, "expand", coerced, CONTEXT).unwrap();
    assert!(json(&expanded).unordered_eq(&json(literal)), "{expanded}");
}

#[test]
fn an_exact_null_term_mapping_takes_precedence_over_its_prefix() {
    let kernel = Kernel::new(Arc::new(ikigai_jsonld::space()));
    let doc = r#"{"@context":{"p":"p:x","p:x":null},"urn:ex:name":"Ada"}"#;
    let output = run(&kernel, "expand", doc, CONTEXT).unwrap();
    assert!(
        json(&output).unordered_eq(&json(r#"[{"urn:ex:name":[{"@value":"Ada"}]}]"#)),
        "{output}"
    );
}
