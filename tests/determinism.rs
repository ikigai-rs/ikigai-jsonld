//! Every `urn:jsonld:*` answer is `.cacheable()`, so the kernel keeps the FIRST computation
//! and serves it until a thread is cut. That is only sound when a recomputation repeats the
//! bytes: a cut must not change what a consumer reads when nothing it depends on changed.
//! ikigai-conformance's RECOMPUTE check (ledger #330) found `flatten` breaking that over a
//! document with blank nodes (ledger #1564): the `json-ld` crate's node map is a `HashMap`,
//! and with `ordered` off its flattening emits nodes in that map's iteration order, which
//! `std`'s random hash seed makes differ from one computation to the next.
//!
//! The test here is the plain form of that witness: each endpoint resolved on many FRESH
//! kernels (nothing cached between them, so every answer is a computation of its own) must
//! give the same bytes every time, over documents built to have many nodes to order: blank
//! nodes at several depths, named nodes beside them, a named graph, and a reverse property.

use ikigai_core::{ArgRef, Capability, Expiry, Iri, Kernel, Request, Verb};
use std::collections::BTreeSet;
use std::sync::Arc;

/// Computations per endpoint per document. One `HashMap` of eight entries has 8! orders,
/// so a nondeterministic face shows a second output well inside this many runs.
const RUNS: usize = 24;

/// Anonymous nodes at three depths, a list of anonymous nodes, and named nodes between them.
const BLANK_NODES: &str = r#"{
  "@context": {"foaf": "http://xmlns.com/foaf/0.1/", "ex": "http://example.org/"},
  "@type": "foaf:Person", "foaf:name": "Ada",
  "foaf:knows": [
    {"@type": "foaf:Person", "foaf:name": "Charles",
     "foaf:knows": {"foaf:name": "Mary", "foaf:knows": {"foaf:name": "William"}}},
    {"@id": "ex:augustus", "foaf:name": "Augustus"},
    {"foaf:name": "Annabella"},
    {"foaf:name": "Byron", "foaf:based_near": {"foaf:name": "London"}},
    {"@id": "ex:michael", "foaf:name": "Michael"}
  ],
  "ex:notes": {"@list": [{"ex:text": "one"}, {"ex:text": "two"}, {"ex:text": "three"}]}
}"#;

/// A named graph of anonymous nodes beside a default-graph node, and a reverse property.
const NAMED_GRAPH: &str = r#"{
  "@context": {"foaf": "http://xmlns.com/foaf/0.1/", "ex": "http://example.org/",
               "knownBy": {"@reverse": "foaf:knows"}},
  "@id": "ex:ada", "foaf:name": "Ada",
  "knownBy": [{"foaf:name": "Charles"}, {"foaf:name": "Mary"}, {"@id": "ex:william"}],
  "ex:archive": {
    "@id": "ex:graph",
    "@graph": [
      {"foaf:name": "One"}, {"foaf:name": "Two"}, {"@id": "ex:three", "foaf:name": "Three"},
      {"foaf:name": "Four", "foaf:knows": {"foaf:name": "Five"}}, {"foaf:name": "Six"}
    ]
  }
}"#;

/// A context that defines every term the documents use, so `prune` keeps them whole and the
/// compacted faces have real terms to order.
const CONTEXT: &str = r#"{"@context": {"foaf": "http://xmlns.com/foaf/0.1/",
  "ex": "http://example.org/", "name": "foaf:name", "knows": "foaf:knows",
  "Person": "foaf:Person", "based_near": "foaf:based_near", "notes": "ex:notes",
  "text": "ex:text", "archive": "ex:archive"}}"#;

/// `iri` over `doc` (and the shared `context` where the endpoint takes one) on a kernel built
/// for this one call.
fn resolve_fresh(iri: &str, doc: &str) -> Vec<u8> {
    let kernel = Kernel::new(Arc::new(ikigai_jsonld::space()));
    let mut request = Request::new(Verb::Source, Iri::parse(iri).unwrap())
        .with_arg("content", ArgRef::Inline(doc.as_bytes().to_vec()));
    if matches!(iri, "urn:jsonld:compact" | "urn:jsonld:prune") {
        request = request.with_arg("context", ArgRef::Inline(CONTEXT.as_bytes().to_vec()));
    }
    let repr = futures::executor::block_on(kernel.issue(request, &Capability::root()))
        .unwrap_or_else(|e| panic!("{iri}: {e}"));
    assert_eq!(
        repr.expiry,
        Expiry::Never,
        "{iri} is cacheable, so it must repeat"
    );
    repr.bytes
}

/// The distinct outputs of `RUNS` fresh computations.
fn outputs(iri: &str, doc: &str) -> BTreeSet<Vec<u8>> {
    (0..RUNS).map(|_| resolve_fresh(iri, doc)).collect()
}

fn assert_deterministic(iri: &str) {
    for (name, doc) in [("blank nodes", BLANK_NODES), ("named graph", NAMED_GRAPH)] {
        let seen = outputs(iri, doc);
        assert_eq!(
            seen.len(),
            1,
            "{iri} over the {name} document gave {} distinct outputs in {RUNS} fresh \
             computations:\n{}",
            seen.len(),
            seen.iter()
                .map(|bytes| String::from_utf8_lossy(bytes).into_owned())
                .collect::<Vec<_>>()
                .join("\n----\n")
        );
    }
}

#[test]
fn flatten_repeats_its_bytes_on_fresh_kernels() {
    assert_deterministic("urn:jsonld:flatten");
}

#[test]
fn expand_repeats_its_bytes_on_fresh_kernels() {
    assert_deterministic("urn:jsonld:expand");
}

#[test]
fn compact_repeats_its_bytes_on_fresh_kernels() {
    assert_deterministic("urn:jsonld:compact");
}

#[test]
fn prune_repeats_its_bytes_on_fresh_kernels() {
    assert_deterministic("urn:jsonld:prune");
}
