//! `urn:jsonld:prune`, the trust-boundary egress filter (ledger #1180), over the collaboration
//! layer's allowlist context exactly as the design writes it (devtools
//! `claude/research/collab-layer-v1-2026-10-10.md`, section 7).
//!
//! What vanilla `urn:jsonld:compact` does with the same document and context is pinned first
//! ([`compaction_alone_filters_nothing`]): that is the defect prune exists for, so the
//! comparison stays in the suite.

use futures::executor::block_on;
use ikigai_core::{
    ArgRef, Capability, Error, Exact, FnEndpoint, Invocation, Iri, Kernel, ReprType,
    Representation, Request, Verb,
};
use std::sync::Arc;

/// The allowlist context (design section 7). No `@vocab`, prefixes only for writing terms.
const COLLAB: &str = r#"{ "@context": {
    "collab": "https://ikigai-rs.dev/ns/collab#",
    "dcterms": "http://purl.org/dc/terms/",
    "prov": "http://www.w3.org/ns/prov#",
    "xsd": "http://www.w3.org/2001/XMLSchema#",
    "WorkOffer": "collab:WorkOffer", "Message": "collab:Message", "Question": "collab:Question",
    "Handback": "collab:Handback", "Release": "collab:Release", "Progress": "collab:Progress",
    "Revoke": "collab:Revoke", "Took": "collab:Took",
    "title":        "dcterms:title",
    "brief":        "collab:brief",
    "body":         "collab:body",
    "repo":         { "@id": "collab:repo",         "@type": "@id" },
    "offer":        { "@id": "collab:offer",        "@type": "@id" },
    "inReplyTo":    { "@id": "collab:inReplyTo",    "@type": "@id" },
    "corrects":     { "@id": "collab:corrects",     "@type": "@id" },
    "evidence":     { "@id": "collab:evidence",     "@type": "@id" },
    "offeredBy":    { "@id": "collab:offeredBy",    "@type": "@id" },
    "attributedTo": { "@id": "prov:wasAttributedTo", "@type": "@id" },
    "created":      { "@id": "dcterms:created",     "@type": "xsd:dateTime" },
    "seq":          { "@id": "collab:seq",          "@type": "xsd:integer" }
} }"#;

const LEDGER: &str = "https://ikigai-rs.dev/ns/ledger#";

/// An offer as the mirror's CONSTRUCT should build it: every key a term of [`COLLAB`].
const ALLOWED: &str = r#"{
  "@context": {
    "collab": "https://ikigai-rs.dev/ns/collab#",
    "dcterms": "http://purl.org/dc/terms/",
    "xsd": "http://www.w3.org/2001/XMLSchema#"
  },
  "@id": "urn:collab:bobby:offer:01",
  "@type": "collab:WorkOffer",
  "dcterms:title": "Prune the boundary",
  "collab:brief": "Write urn:jsonld:prune.",
  "collab:repo": { "@id": "https://github.com/ikigai-rs/ikigai-jsonld" },
  "dcterms:created": { "@value": "2026-10-10T12:00:00Z", "@type": "xsd:dateTime" },
  "collab:seq": { "@value": "7", "@type": "xsd:integer" }
}"#;

/// The same offer with what must never cross: a ledger predicate, a ledger type, a predicate
/// reachable only through the `collab:` PREFIX, a nested node carrying only ledger content, and
/// a literal typed with a datatype the context never names.
const LEAKY: &str = r#"{
  "@context": {
    "collab": "https://ikigai-rs.dev/ns/collab#",
    "dcterms": "http://purl.org/dc/terms/",
    "xsd": "http://www.w3.org/2001/XMLSchema#",
    "ledger": "https://ikigai-rs.dev/ns/ledger#"
  },
  "@id": "urn:collab:bobby:offer:01",
  "@type": ["collab:WorkOffer", "ledger:Item"],
  "dcterms:title": "Prune the boundary",
  "collab:brief": "Write urn:jsonld:prune.",
  "collab:repo": { "@id": "https://github.com/ikigai-rs/ikigai-jsonld" },
  "dcterms:created": { "@value": "2026-10-10T12:00:00Z", "@type": "xsd:dateTime" },
  "collab:seq": { "@value": "7", "@type": "xsd:integer" },
  "ledger:number": 1180,
  "ledger:state": "private",
  "collab:hidden": "reachable only through the prefix",
  "collab:body": [
    "kept",
    { "@value": "a secret", "@type": "ledger:Secret" }
  ],
  "collab:evidence": { "@id": "urn:iki:ledger:default:item:x", "ledger:comment": "private" }
}"#;

fn kernel() -> Kernel {
    Kernel::new(Arc::new(ikigai_jsonld::space()))
}

fn request(iri: &str, args: &[(&str, &str)]) -> Request {
    let mut request = Request::new(Verb::Source, Iri::parse(iri).unwrap());
    for &(name, value) in args {
        request = request.with_arg(name, ArgRef::Inline(value.as_bytes().to_vec()));
    }
    request
}

fn issue(kernel: &Kernel, request: Request) -> Result<Representation, Error> {
    block_on(kernel.issue(request, &Capability::root()))
}

fn body(repr: Representation) -> String {
    String::from_utf8(repr.bytes).expect("UTF-8")
}

fn prune(content: &str, context: &str) -> String {
    body(
        issue(
            &kernel(),
            request(
                "urn:jsonld:prune",
                &[("content", content), ("context", context)],
            ),
        )
        .unwrap(),
    )
}

fn report(content: &str, context: &str) -> String {
    let repr = issue(
        &kernel(),
        request(
            "urn:jsonld:prune",
            &[
                ("content", content),
                ("context", context),
                ("face", "report"),
            ],
        ),
    )
    .unwrap();
    assert_eq!(repr.repr_type.media_type, "application/json");
    body(repr)
}

/// A pruned document's body, after its `@context` (which names every term, used or not).
fn without_context(pretty: &str) -> String {
    let end = pretty
        .find("\n  },\n")
        .expect("a pretty-printed @context block");
    pretty[end..].to_string()
}

fn compact(content: &str, context: &str) -> String {
    body(
        issue(
            &kernel(),
            request(
                "urn:jsonld:compact",
                &[("content", content), ("context", context)],
            ),
        )
        .unwrap(),
    )
}

/// The claim the item makes, reproduced: compaction keeps every unmapped predicate (as a full
/// IRI, or as a compact IRI under a prefix the context declares) and every unmapped type.
#[test]
fn compaction_alone_filters_nothing() {
    let out = compact(LEAKY, COLLAB);
    assert!(out.contains(&format!("{LEDGER}number")), "{out}");
    assert!(out.contains(&format!("{LEDGER}Item")), "{out}");
    assert!(out.contains("collab:hidden"), "{out}");
    assert!(out.contains(&format!("{LEDGER}comment")), "{out}");
}

#[test]
fn a_ledger_predicate_is_dropped() {
    let out = prune(LEAKY, COLLAB);
    assert!(
        !out.contains("ledger"),
        "nothing of the ledger crosses: {out}"
    );
    assert!(!out.contains("1180"), "{out}");
    assert!(!out.contains("private"), "{out}");
    let report = report(LEAKY, COLLAB);
    assert!(report.contains(&format!("{LEDGER}number")), "{report}");
    assert!(report.contains(&format!("{LEDGER}state")), "{report}");
}

#[test]
fn an_unmapped_type_is_dropped_and_a_term_type_kept() {
    let out = prune(LEAKY, COLLAB);
    assert!(out.contains("\"@type\": \"WorkOffer\""), "{out}");
    assert!(!out.contains("Item"), "{out}");
    let report = report(LEAKY, COLLAB);
    assert!(report.contains(&format!("{LEDGER}Item")), "{report}");
}

/// ⚠ A prefix is not a term: `collab:` lets the CONTEXT write `collab:brief`; it must not let
/// the document say `collab:anything`.
#[test]
fn a_predicate_reachable_only_through_a_prefix_is_dropped() {
    let out = prune(LEAKY, COLLAB);
    assert!(!out.contains("hidden"), "{out}");
    assert!(
        out.contains("\"brief\""),
        "the term under the same prefix crosses: {out}"
    );
    let report = report(LEAKY, COLLAB);
    assert!(
        report.contains("https://ikigai-rs.dev/ns/collab#hidden"),
        "{report}"
    );

    // And the prefix's own IRI is not a property either.
    let bare = r#"{"@id":"urn:x:1","https://ikigai-rs.dev/ns/collab#":"the namespace itself",
                   "http://purl.org/dc/terms/title":"t"}"#;
    let out = prune(bare, COLLAB);
    assert!(!out.contains("namespace itself"), "{out}");
    assert!(out.contains("\"title\": \"t\""), "{out}");
}

#[test]
fn a_node_left_empty_is_dropped_and_a_reference_is_kept() {
    let out = without_context(&prune(LEAKY, COLLAB));
    // The evidence node carried only a ledger comment: it goes, and the property with it.
    assert!(!out.contains("evidence"), "{out}");
    assert!(!out.contains("urn:iki:ledger"), "{out}");
    // `repo` is a reference (only an `@id`), never content: it stays.
    assert!(
        out.contains("\"repo\": \"https://github.com/ikigai-rs/ikigai-jsonld\""),
        "{out}"
    );
    assert!(report(LEAKY, COLLAB).contains("\"emptyNodes\": 1"));
}

#[test]
fn a_value_with_a_datatype_the_context_never_names_is_dropped() {
    let out = prune(LEAKY, COLLAB);
    assert!(!out.contains("a secret"), "{out}");
    assert!(out.contains("\"body\": \"kept\""), "{out}");
    // A datatype the context coerces to is kept, and compacts back under its term.
    assert!(
        out.contains("\"created\": \"2026-10-10T12:00:00Z\""),
        "{out}"
    );
    assert!(out.contains("\"seq\": \"7\""), "{out}");
    let report = report(LEAKY, COLLAB);
    assert!(report.contains(&format!("{LEDGER}Secret")), "{report}");
}

#[test]
fn the_report_counts_and_lists_every_drop() {
    let report = report(LEAKY, COLLAB);
    let expected = format!(
        r#"{{
  "dropped": 7,
  "properties": [
    "https://ikigai-rs.dev/ns/collab#hidden",
    "{LEDGER}comment",
    "{LEDGER}number",
    "{LEDGER}state"
  ],
  "reverseProperties": [],
  "types": [
    "{LEDGER}Item"
  ],
  "datatypes": [
    "{LEDGER}Secret"
  ],
  "emptyNodes": 1
}}"#
    );
    assert_eq!(report, expected);
}

/// An allowed document is a FIXED POINT: prune changes nothing compaction would not, reports
/// nothing, and pruning its own output returns it byte for byte.
#[test]
fn an_allowed_document_is_a_fixed_point() {
    let once = prune(ALLOWED, COLLAB);
    assert_eq!(once, compact(ALLOWED, COLLAB), "nothing removed");
    assert!(report(ALLOWED, COLLAB).contains("\"dropped\": 0"));
    let twice = prune(&once, COLLAB);
    assert_eq!(twice, once, "prune is idempotent");
    assert!(report(&once, COLLAB).contains("\"dropped\": 0"));

    // And what a leaky document comes out as is allowed: pruning it again drops nothing.
    let cleaned = prune(LEAKY, COLLAB);
    assert_eq!(prune(&cleaned, COLLAB), cleaned);
    assert!(report(&cleaned, COLLAB).contains("\"dropped\": 0"));
}

fn refused_context(context: &str) -> String {
    match issue(
        &kernel(),
        request(
            "urn:jsonld:prune",
            &[("content", ALLOWED), ("context", context)],
        ),
    ) {
        Err(Error::InvalidArgument { name, detail }) => {
            assert_eq!(name, "context");
            detail
        }
        other => panic!("expected InvalidArgument on `context`, got {other:?}"),
    }
}

#[test]
fn a_context_with_vocab_is_refused() {
    let detail = refused_context(
        r#"{"@context":{"@vocab":"https://ikigai-rs.dev/ns/ledger#","title":"http://purl.org/dc/terms/title"}}"#,
    );
    assert!(detail.contains("@vocab"), "{detail}");
    // In any entry of an array context too, and as a bare context value.
    refused_context(
        r#"{"@context":[{"title":"http://purl.org/dc/terms/title"},{"@vocab":"urn:x:"}]}"#,
    );
    refused_context(r#"{"@vocab":"urn:x:","title":"http://purl.org/dc/terms/title"}"#);
    // Compaction against the same context is not refused: the refusal is prune's.
    compact(
        ALLOWED,
        r#"{"@context":{"@vocab":"https://ikigai-rs.dev/ns/ledger#"}}"#,
    );
}

#[test]
fn a_scoped_context_is_refused() {
    let detail = refused_context(
        r#"{"@context":{"collab":"https://ikigai-rs.dev/ns/collab#",
            "repo":{"@id":"collab:repo","@context":{"secret":"urn:x:secret"}}}}"#,
    );
    assert!(detail.contains("scoped"), "{detail}");
}

#[test]
fn a_reverse_property_crosses_only_as_a_reverse_term() {
    let context = r#"{"@context":{"title":"http://purl.org/dc/terms/title",
        "offers":{"@reverse":"https://ikigai-rs.dev/ns/collab#offeredBy"}}}"#;
    let doc = r#"{"@context":{"title":"http://purl.org/dc/terms/title"},
        "@id":"urn:x:brian","title":"Brian",
        "@reverse":{
          "https://ikigai-rs.dev/ns/collab#offeredBy":{"@id":"urn:x:offer","title":"An offer"},
          "https://ikigai-rs.dev/ns/ledger#claimedBy":{"@id":"urn:x:item","title":"private"}}}"#;
    let out = prune(doc, context);
    assert!(out.contains("\"offers\""), "{out}");
    assert!(out.contains("An offer"), "{out}");
    assert!(!out.contains("private"), "{out}");
    assert!(!out.contains("claimedBy"), "{out}");
    let report = report(doc, context);
    assert!(
        report.contains(&format!(
            "\"reverseProperties\": [\n    \"{LEDGER}claimedBy\""
        )),
        "{report}"
    );
}

#[test]
fn graphs_and_lists_are_pruned_inside() {
    let context = r#"{"@context":{"title":"http://purl.org/dc/terms/title",
        "steps":{"@id":"https://ikigai-rs.dev/ns/collab#steps","@container":"@list"}}}"#;
    let doc = r#"{"@context":{"title":"http://purl.org/dc/terms/title"},"@id":"urn:x:g","@graph":[
        {"@id":"urn:x:a","title":"A","https://ikigai-rs.dev/ns/ledger#state":"private"},
        {"@id":"urn:x:b","https://ikigai-rs.dev/ns/ledger#state":"private"},
        {"@id":"urn:x:c","https://ikigai-rs.dev/ns/collab#steps":{"@list":[
            {"title":"one"},{"https://ikigai-rs.dev/ns/ledger#state":"private"},"two"]}}
      ]}"#;
    let out = prune(doc, context);
    assert!(!out.contains("private"), "{out}");
    assert!(
        !out.contains("urn:x:b"),
        "a node left empty inside a graph goes: {out}"
    );
    assert!(out.contains("\"title\": \"A\""), "{out}");
    assert!(out.contains("\"title\": \"one\""), "{out}");
    assert!(out.contains("\"two\""), "{out}");
    assert!(report(doc, context).contains("\"emptyNodes\": 2"));
}

/// The context is a resource too, as compact's is: by IRI through the kernel.
#[test]
fn the_context_resolves_by_reference() {
    let space = ikigai_jsonld::space().bind(
        Exact::new("urn:collab:bobby:context"),
        FnEndpoint::new("context", |_inv: &Invocation<'_>| {
            Ok(Representation::new(
                ReprType::new("application/ld+json"),
                COLLAB.as_bytes().to_vec(),
            )
            .cacheable())
        }),
    );
    let kernel = Kernel::new(Arc::new(space));
    let out = body(
        issue(
            &kernel,
            request(
                "urn:jsonld:prune",
                &[("content", LEAKY), ("context", "urn:collab:bobby:context")],
            ),
        )
        .unwrap(),
    );
    assert_eq!(out, prune(LEAKY, COLLAB));
}

#[test]
fn an_unknown_face_is_refused() {
    match issue(
        &kernel(),
        request(
            "urn:jsonld:prune",
            &[
                ("content", ALLOWED),
                ("context", COLLAB),
                ("face", "summary"),
            ],
        ),
    ) {
        Err(Error::InvalidArgument { name, .. }) => assert_eq!(name, "face"),
        other => panic!("expected InvalidArgument on `face`, got {other:?}"),
    }
}

/// Both faces are pure functions of the inline arguments: cached.
#[test]
fn both_faces_are_cacheable() {
    let kernel = kernel();
    for face in ["document", "report"] {
        let req = || {
            request(
                "urn:jsonld:prune",
                &[("content", LEAKY), ("context", COLLAB), ("face", face)],
            )
        };
        issue(&kernel, req()).unwrap();
        assert!(kernel.is_cached(&req(), &Capability::root()), "{face}");
    }
}
