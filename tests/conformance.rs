//! The module recipe as one test: `ikigai-conformance` walks the three endpoints
//! [`ikigai_jsonld::space`] binds and reports every violation at once.
//!
//! ## Three transformations, one face
//!
//! `expand`, `flatten` and `compact` take a JSON-LD document as `content` and serve
//! `application/ld+json` — which the suite treats as an RDF face: it parses the
//! output and reads it for blank nodes and undefined terms. What it reads is the
//! caller's document after the algorithm, not a graph this module authors, so the
//! face is exactly as skolemized as its input ([`over_an_anonymous_document_every_face_has_blank_nodes`]).
//! The walk's fixture is a document with an `@id` on its one node and an `@type`
//! under `foaf:` — named, so SKOLEM-RDF has nothing to report, and typed, so
//! VOCABULARY actually reads a class rather than passing on an empty graph.
//!
//! ## Pure over an inline context — and what the kernel folds in otherwise
//!
//! Every result is marked `.cacheable()`. `expand` and `flatten` are pure functions
//! of `content` (+ `base`); `compact` is pure too WHEN its `context` is inline. A
//! `context` given by IRI is a sub-resolution through the kernel, and the kernel
//! folds that resource's expiry and golden threads into the result: a context
//! served under a thread makes the compaction cacheable under that thread
//! ([`a_context_by_reference_inherits_its_thread`]); a context served live makes
//! it uncacheable ([`over_a_live_context_nothing_is_cached`]). So the declarations
//! here certify behavior over the kernel passed: [`conforms`] declares all three
//! `pure` and `cacheable` over the inline fixture; the by-reference walks declare
//! `compact` only `cacheable` (it has a thread there, the context's) or nothing.
//!
//! ## The remote-context path
//!
//! An `http(s)://` context is fetched by issuing `urn:httpGet` — a resource another
//! module (`ikigai-http`) binds and gates with `urn:cap:net:<host>`. `compact`
//! declares no capability of its own, correctly: with an inline or `urn:` context
//! it reaches no network, and the manifold must offer it to a caller with no
//! `net` grant. The gate lives on the fetch, and sub-resolutions run under the
//! CALLER's capability, so under no grants the fetch is a typed `Denied` and
//! `compact` propagates it unchanged; over a kernel that binds no `urn:httpGet`
//! at all it is `Unresolved` ([`a_remote_context_is_gated_by_the_net_capability`]).
//! The suite cannot see either — its ENFORCED probe uses the fixture's inline
//! context — so this file pins both by hand.
//!
//! No opt-outs, no module namespace (the faces carry the caller's terms, and the
//! fixture's are `foaf:`), and NAMES runs: every id is kebab-case.

use ikigai_conformance::{Check, Fixture, Report, Suite};
use ikigai_core::{
    ArgRef, Capability, Description, Error, Exact, FnEndpoint, Invocation, Iri, Kernel, ReprType,
    Representation, Request, Verb,
};
use std::sync::{Arc, RwLock};

/// The three endpoints `space()` binds, by description id.
const EXPAND: &str = "jsonld-expand";
const FLATTEN: &str = "jsonld-flatten";
const COMPACT: &str = "jsonld-compact";
const ENDPOINTS: [&str; 3] = [EXPAND, FLATTEN, COMPACT];

/// The walk's document: one node, named and typed under a well-known vocabulary.
const DOC: &str = r#"{"@context":{"foaf":"http://xmlns.com/foaf/0.1/"},
  "@id":"http://example.org/ada","@type":"foaf:Person","foaf:name":"Ada"}"#;

/// The same document with no `@id` anywhere: two anonymous nodes.
const ANONYMOUS: &str = r#"{"@context":{"foaf":"http://xmlns.com/foaf/0.1/"},
  "@type":"foaf:Person","foaf:name":"Ada",
  "foaf:knows":{"@type":"foaf:Person","foaf:name":"Charles"}}"#;

/// Two contexts that compact `foaf:name` to different terms, so a recomputation
/// after a cut is visible in the bytes.
const CONTEXT_NAME: &str = r#"{"@context":{"name":"http://xmlns.com/foaf/0.1/name"}}"#;
const CONTEXT_LABEL: &str = r#"{"@context":{"label":"http://xmlns.com/foaf/0.1/name"}}"#;

/// Where the by-reference walks bind the context, doubling as the golden thread
/// the threaded variant names for it (the `ikigai-fs` convention: `depends_on`
/// the resource's own IRI).
const CONTEXT_IRI: &str = "urn:conformance:context";

/// The remote context the stand-in `urn:httpGet` serves, and the scope it demands.
const REMOTE_CONTEXT: &str = "https://example.org/context.jsonld";
const NET_SCOPE: &str = "urn:cap:net:example.org";

/// The suite with one fixture per action over `doc`; `context` is what `compact`
/// receives — inline JSON, or an IRI bound in the kernel under test.
fn suite(doc: &str, context: &str) -> Suite {
    Suite::new()
        .fixture(Fixture::new(EXPAND, Verb::Source).arg("content", doc))
        .fixture(Fixture::new(FLATTEN, Verb::Source).arg("content", doc))
        .fixture(
            Fixture::new(COMPACT, Verb::Source)
                .arg("content", doc)
                .arg("context", context),
        )
}

/// A context resource — what `urn:file:<ctx.jsonld>` or a served vocabulary
/// context is to the module: bytes behind an IRI, resolved through the kernel.
/// `thread` is the golden thread a store that can change names (and cuts);
/// `None` is a live store that must be read every time, served uncacheable. The
/// suite walks this endpoint beside the module's, so it describes itself the way
/// a module endpoint must.
fn context_resource(
    context: Arc<RwLock<&'static str>>,
    thread: Option<&'static str>,
) -> FnEndpoint {
    FnEndpoint::new("context", move |_inv: &Invocation<'_>| {
        let bytes = context.read().expect("context lock").as_bytes().to_vec();
        let repr = Representation::new(ReprType::new("application/ld+json"), bytes);
        Ok(match thread {
            Some(thread) => repr.cacheable().depends_on(thread),
            None => repr,
        })
    })
    .with_description(
        Description::new("context")
            .title("Conformance context resource")
            .summary("A JSON-LD context served as a kernel resource for the walk.")
            .verb(Verb::Source)
            .output("application/ld+json"),
    )
}

/// The module's space plus one context bound at [`CONTEXT_IRI`], and a handle to
/// change it in place. `threaded` selects the store's kind (see the file docs).
fn with_context(threaded: bool) -> (Kernel, Arc<RwLock<&'static str>>) {
    let context = Arc::new(RwLock::new(CONTEXT_NAME));
    let space = ikigai_jsonld::space().bind(
        Exact::new(CONTEXT_IRI),
        context_resource(Arc::clone(&context), threaded.then_some(CONTEXT_IRI)),
    );
    (Kernel::new(Arc::new(space)), context)
}

fn request(iri: &str, args: &[(&str, &str)]) -> Request {
    let mut request = Request::new(Verb::Source, Iri::parse(iri).unwrap());
    for &(name, value) in args {
        request = request.with_arg(name, ArgRef::Inline(value.as_bytes().to_vec()));
    }
    request
}

fn compact_request(context: &str) -> Request {
    request(
        "urn:jsonld:compact",
        &[("content", DOC), ("context", context)],
    )
}

fn issue(kernel: &Kernel, request: Request, capability: &Capability) -> Result<String, Error> {
    futures::executor::block_on(kernel.issue(request, capability))
        .map(|repr| String::from_utf8(repr.bytes).expect("JSON-LD is UTF-8"))
}

/// The walk saw the module's three endpoints plus `extra` fixture endpoints, one
/// Source action each, and skipped nothing. A fourth module endpoint bound
/// without a line here would be held to a weaker standard; a declared id that
/// binds nothing is a stale list.
fn assert_shape(report: &Report, extra: usize) {
    assert_eq!(report.endpoints, ENDPOINTS.len() + extra, "{report}");
    assert_eq!(
        report.actions,
        ENDPOINTS.len() + extra,
        "one Source action each: {report}"
    );
    assert_eq!(
        report.checks.skipped().count(),
        0,
        "every check runs: {report}"
    );
}

#[test]
fn conforms() {
    let kernel = Kernel::new(Arc::new(ikigai_jsonld::space()));
    let suite = ENDPOINTS
        .iter()
        .fold(suite(DOC, CONTEXT_NAME), |suite, id| {
            suite.pure(*id).cacheable(*id)
        });
    let report = suite.run_blocking(&kernel);
    // Printed even when clean (`--nocapture`): the report is the record.
    eprintln!("{report}");
    assert!(report.is_clean(), "{report}");
    assert_shape(&report, 0);
}

/// The half of SKOLEM-RDF the clean walk cannot show: these are transformations,
/// and an anonymous input node is anonymous (or, for `flatten`, freshly labeled
/// `_:0`, `_:1`, …) in the output. The suite reports exactly one blank-node finding per
/// endpoint and nothing else. Skolemizing here would be a semantic decision —
/// the output would no longer be what the JSON-LD algorithm defines — so this
/// test pins the current behavior rather than hiding it behind a named fixture.
#[test]
fn over_an_anonymous_document_every_face_has_blank_nodes() {
    let kernel = Kernel::new(Arc::new(ikigai_jsonld::space()));
    let suite = ENDPOINTS
        .iter()
        .fold(suite(ANONYMOUS, CONTEXT_NAME), |suite, id| {
            suite.pure(*id).cacheable(*id)
        });
    let report = suite.run_blocking(&kernel);
    eprintln!("[anonymous document]\n{report}");
    let mut blank: Vec<&str> = report
        .of(Check::SkolemRdf)
        .map(|f| f.endpoint.as_str())
        .collect();
    blank.sort_unstable();
    let mut expected = ENDPOINTS.to_vec();
    expected.sort_unstable();
    assert_eq!(blank, expected, "one blank-node finding per face: {report}");
    for finding in report.of(Check::SkolemRdf) {
        assert!(finding.detail.contains("blank node"), "{finding}");
    }
    assert_eq!(
        report.findings.len(),
        ENDPOINTS.len(),
        "and nothing else: {report}"
    );
}

/// A context by IRI, served under a thread: the compaction is cached, carries the
/// context's thread, and survives a change to the context until that thread is
/// cut — the kernel's job, not this module's (it has no watcher and holds no
/// context). The suite walk over this kernel is clean with `compact` declared
/// `cacheable` but NOT `pure`: its thread set is the context's, non-empty.
#[test]
fn a_context_by_reference_inherits_its_thread() {
    let (kernel, context) = with_context(true);

    let report = suite(DOC, CONTEXT_IRI)
        .pure(EXPAND)
        .cacheable(EXPAND)
        .pure(FLATTEN)
        .cacheable(FLATTEN)
        .cacheable(COMPACT)
        .run_blocking(&kernel);
    eprintln!("[threaded context]\n{report}");
    assert!(report.is_clean(), "{report}");
    assert_shape(&report, 1);

    let first = issue(&kernel, compact_request(CONTEXT_IRI), &Capability::root()).unwrap();
    assert!(first.contains("\"name\""), "{first}");
    assert!(
        kernel.is_cached(&compact_request(CONTEXT_IRI), &Capability::root()),
        "over a threaded context the compaction is cached"
    );
    let repr = futures::executor::block_on(
        kernel.issue(compact_request(CONTEXT_IRI), &Capability::root()),
    )
    .unwrap();
    assert!(
        repr.threads().iter().any(|t| t.to_string() == CONTEXT_IRI),
        "the compaction carries the context's thread: {:?}",
        repr.threads()
    );

    // The context changes in its store. Nothing in this module notices.
    *context.write().expect("context lock") = CONTEXT_LABEL;
    let stale = issue(&kernel, compact_request(CONTEXT_IRI), &Capability::root()).unwrap();
    assert_eq!(
        stale, first,
        "no watcher here: a change with no cut is served from the cache"
    );

    // The store cuts the thread it named, and the compaction goes with it.
    kernel.cut(CONTEXT_IRI);
    let fresh = issue(&kernel, compact_request(CONTEXT_IRI), &Capability::root()).unwrap();
    assert!(
        fresh.contains("\"label\""),
        "recomputed against the changed context after the cut: {fresh}"
    );
    assert_ne!(fresh, first);
}

/// The other store: the context served uncacheable. The module still says
/// `.cacheable()`, and the kernel hands the compaction back uncacheable — the
/// effective expiry is the context's. Undeclared, that is correct and the walk
/// is clean; DECLARED `cacheable`, the suite reports the downgrade on `compact`
/// alone, which is the only way a silent recompute-every-read becomes visible —
/// the types are identical either way.
#[test]
fn over_a_live_context_nothing_is_cached() {
    let (kernel, _) = with_context(false);

    let report = suite(DOC, CONTEXT_IRI)
        .pure(EXPAND)
        .cacheable(EXPAND)
        .pure(FLATTEN)
        .cacheable(FLATTEN)
        .run_blocking(&kernel);
    eprintln!("[live context, undeclared]\n{report}");
    assert!(report.is_clean(), "{report}");
    assert_shape(&report, 1);
    issue(&kernel, compact_request(CONTEXT_IRI), &Capability::root()).unwrap();
    assert!(
        !kernel.is_cached(&compact_request(CONTEXT_IRI), &Capability::root()),
        "a compaction over an uncacheable context is not cached"
    );

    let report = suite(DOC, CONTEXT_IRI)
        .pure(EXPAND)
        .cacheable(EXPAND)
        .pure(FLATTEN)
        .cacheable(FLATTEN)
        .cacheable(COMPACT)
        .run_blocking(&kernel);
    eprintln!("[live context, compact declared cacheable]\n{report}");
    let downgraded: Vec<&str> = report
        .of(Check::Cacheable)
        .map(|f| f.endpoint.as_str())
        .collect();
    assert_eq!(downgraded, [COMPACT], "{report}");
    assert!(
        report.findings[0].detail.contains("declared cacheable"),
        "the finding names the declaration: {}",
        report.findings[0]
    );
    assert_eq!(report.findings.len(), 1, "and nothing else: {report}");
}

/// The remote-context path, over a stand-in for `ikigai-http`'s `urn:httpGet`
/// that declares (and so, by the kernel's floor, enforces) a `urn:cap:net:` scope.
/// Under no grants the fetch is refused and `compact` propagates the typed
/// `Denied`; under the host scope the compaction succeeds. Over the module's own
/// space, with no `urn:httpGet` bound, the same call is `Unresolved`: reachability
/// of a remote context is the HOST's to provide, and the CALLER's to be granted.
#[test]
fn a_remote_context_is_gated_by_the_net_capability() {
    let http_get = FnEndpoint::new("http-get", |inv: &Invocation<'_>| {
        assert_eq!(inv.inline_str("url")?, REMOTE_CONTEXT);
        Ok(Representation::new(
            ReprType::new("application/ld+json"),
            CONTEXT_NAME.as_bytes().to_vec(),
        ))
    })
    .with_description(
        Description::new("http-get")
            .title("Stand-in urn:httpGet")
            .summary("Serves one remote context, gated the way ikigai-http gates a host.")
            .verb(Verb::Source)
            .requires(NET_SCOPE)
            .output("application/ld+json"),
    );
    let space = ikigai_jsonld::space().bind(Exact::new("urn:httpGet"), http_get);
    let kernel = Kernel::new(Arc::new(space));

    let none = Capability::scoped(Vec::<String>::new());
    match issue(&kernel, compact_request(REMOTE_CONTEXT), &none) {
        Err(Error::Denied(_)) => {}
        other => panic!("expected a typed Denied under no grants, got {other:?}"),
    }

    let granted = Capability::scoped([NET_SCOPE]);
    let body = issue(&kernel, compact_request(REMOTE_CONTEXT), &granted).unwrap();
    assert!(body.contains("\"name\""), "{body}");

    let bare = Kernel::new(Arc::new(ikigai_jsonld::space()));
    match issue(&bare, compact_request(REMOTE_CONTEXT), &Capability::root()) {
        Err(Error::Unresolved(iri)) => assert_eq!(iri.as_str(), "urn:httpGet"),
        other => panic!("expected Unresolved with no urn:httpGet bound, got {other:?}"),
    }
}

/// What `ikigai-conformance` 0.1.0 does not check (its PENDING #11): a declared
/// output is compared with what the action serves only when it is an RDF face,
/// and only in one direction. Read by hand, then pinned both ways: each action
/// declares exactly `application/ld+json` and serves exactly that (with a
/// `charset` parameter the comparison ignores).
#[test]
fn declared_outputs_are_the_media_types_served() {
    let kernel = Kernel::new(Arc::new(ikigai_jsonld::space()));
    let served = [
        (
            "urn:jsonld:expand",
            request("urn:jsonld:expand", &[("content", DOC)]),
        ),
        (
            "urn:jsonld:flatten",
            request("urn:jsonld:flatten", &[("content", DOC)]),
        ),
        ("urn:jsonld:compact", compact_request(CONTEXT_NAME)),
    ];
    for (iri, request) in served {
        let repr = futures::executor::block_on(kernel.issue(request, &Capability::root()))
            .unwrap_or_else(|e| panic!("{iri}: {e}"));
        let got = ikigai_conformance::rdf::bare_media_type(&repr.repr_type.media_type);
        let description = kernel
            .describe_pattern(iri)
            .unwrap_or_else(|| panic!("{iri} describes itself"));
        let declared: Vec<String> = description
            .outputs
            .iter()
            .map(|o| ikigai_conformance::rdf::bare_media_type(o))
            .collect();
        assert_eq!(declared, ["application/ld+json"], "{iri} declares one face");
        assert_eq!(got, declared[0], "{iri} serves the face it declares");
    }
}
