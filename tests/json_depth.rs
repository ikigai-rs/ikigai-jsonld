//! Caller JSON-LD nested deeply is refused before json-ld sees it, at every door, and a
//! document at the bound is answered on a small thread (ledger #1042).
//!
//! The claim: json-ld 0.21's algorithms recurse once per nested JSON object and array, so
//! nested node objects aborted `urn:jsonld:expand`, `:flatten` and `:compact` at 13 levels in a
//! debug build and ~123 in a release one, and nested arrays at ~29 and ~306, on a 2 MiB thread
//! (`tests/jsonld_depth_measure.rs`, against the code before the fix). A stack overflow aborts
//! the WHOLE process, on any thread.
//!
//! Every probe runs in a CHILD PROCESS (this test binary re-executed with one probe named in
//! its environment) on a 2 MiB thread, the size of a tokio worker's, so an abort kills the
//! child and reads here as a failure with the signal named.

use futures::executor::block_on;
use ikigai_core::{
    ArgRef, Capability, EndpointSpace, Exact, Fallback, FnEndpoint, Invocation, Iri, Kernel,
    ReprType, Representation, Request, Space, Verb,
};
use std::process::Command;
use std::sync::Arc;

const PROBE: &str = "IKIGAI_JSONLD_DEPTH_PROBE";
/// The bound, restated so these tests read the same against the code that predates it.
const BOUND: usize = 64;
const CONTEXT: &str = r#"{"p":"urn:ex:p"}"#;
/// Where the by-reference probes find their context.
const CONTEXT_IRI: &str = "urn:data:deep-context";

/// A document of shape `name`, `n` levels deep. `nodes` nests `n + 1` JSON levels; the others
/// say how many they nest.
fn shape(name: &str, n: usize) -> String {
    let r = |s: &str, k: usize| s.repeat(k);
    match name {
        // Node objects as property values: one JSON level a level.
        "nodes" => format!(
            "{}{{\"@id\":\"urn:ex:o\"}}{}",
            r("{\"@id\":\"urn:ex:s\",\"urn:ex:p\":", n),
            r("}", n)
        ),
        // Node objects in arrays: two JSON levels a level.
        "node-arrays" => format!(
            "{}{{\"@id\":\"urn:ex:o\"}}{}",
            r("{\"@id\":\"urn:ex:s\",\"urn:ex:p\":[", n),
            r("]}", n)
        ),
        // Arrays of arrays as a value: `n + 1` levels.
        "arrays" => format!(
            "{{\"@id\":\"urn:ex:s\",\"urn:ex:p\":{}\"x\"{}}}",
            r("[", n),
            r("]", n)
        ),
        // Named graphs nested: two levels a level.
        "graphs" => format!(
            "{}{{\"@id\":\"urn:ex:o\",\"urn:ex:p\":\"x\"}}{}",
            r("{\"@id\":\"urn:ex:g\",\"@graph\":[", n),
            r("]}", n)
        ),
        // `@reverse` objects nested: two levels a level.
        "reverse" => format!(
            "{}{{\"@id\":\"urn:ex:o\"}}{}",
            r("{\"@id\":\"urn:ex:s\",\"@reverse\":{\"urn:ex:p\":", n),
            r("}}", n)
        ),
        // `@list` objects nested: two levels a level.
        "list-objects" => format!(
            "{{\"@id\":\"urn:ex:s\",\"urn:ex:p\":{}\"x\"{}}}",
            r("{\"@list\":[", n),
            r("]}", n)
        ),
        // A context of scoped contexts in term definitions, `2n + 1` levels: for `context=`.
        "context" => format!(
            "{}{{}}{}",
            r("{\"p\":{\"@id\":\"urn:ex:p\",\"@context\":", n),
            r("}}", n)
        ),
        other => panic!("no shape named {other}"),
    }
}

const SHAPES: [&str; 6] = [
    "nodes",
    "node-arrays",
    "arrays",
    "graphs",
    "reverse",
    "list-objects",
];

/// The doors: the four endpoints, and `compact` and `prune` with the deep document as their
/// context, inline (`…-context`) and by reference (`…-context-ref`).
fn issue(door: &str, doc: String) -> ikigai_core::Result<String> {
    let deep_context = doc.clone();
    let context_space = EndpointSpace::new().bind(
        Exact::new(CONTEXT_IRI),
        FnEndpoint::new("deep-context", move |_inv: &Invocation<'_>| {
            Ok(Representation::new(
                ReprType::new("application/ld+json"),
                deep_context.as_bytes().to_vec(),
            ))
        }),
    );
    let kernel = Kernel::new(Arc::new(Fallback::new(vec![
        Arc::new(ikigai_jsonld::space()) as Arc<dyn Space>,
        Arc::new(context_space) as Arc<dyn Space>,
    ])));
    let (iri, content, context) = match door {
        "expand" | "flatten" => (door, doc, None),
        "compact" | "prune" => (door, doc, Some(CONTEXT.to_string())),
        "compact-context" => ("compact", shape("nodes", 1), Some(doc)),
        "compact-context-ref" => ("compact", shape("nodes", 1), Some(CONTEXT_IRI.to_string())),
        "prune-context" => ("prune", shape("nodes", 1), Some(doc)),
        "prune-context-ref" => ("prune", shape("nodes", 1), Some(CONTEXT_IRI.to_string())),
        other => panic!("no door named {other}"),
    };
    let mut request = Request::new(
        Verb::Source,
        Iri::parse(format!("urn:jsonld:{iri}")).unwrap(),
    )
    .with_arg("content", ArgRef::Inline(content.into_bytes()));
    if let Some(context) = context {
        request = request.with_arg("context", ArgRef::Inline(context.into_bytes()));
    }
    block_on(kernel.issue(request, &Capability::root()))
        .map(|rep| String::from_utf8_lossy(&rep.bytes).into_owned())
}

/// The child's half: inert unless a parent named a probe. Runs it on a 2 MiB thread, prints
/// the outcome, and exits before the harness can.
#[test]
fn probe_child() {
    let Ok(spec) = std::env::var(PROBE) else {
        return;
    };
    let mut parts = spec.split(':');
    let door = parts.next().unwrap().to_string();
    let name = parts.next().unwrap().to_string();
    let n = parts.next().unwrap().parse::<usize>().unwrap();
    let outcome = std::thread::Builder::new()
        .stack_size(2 << 20)
        .spawn(move || match issue(&door, shape(&name, n)) {
            Ok(text) => format!("ok {}", text.chars().take(200).collect::<String>()),
            Err(e) => format!("err {}", e.to_string().replace('\n', " ")),
        })
        .unwrap()
        .join()
        .unwrap();
    println!("\nOUTCOME {}", outcome.replace('\n', " "));
    std::process::exit(0);
}

/// The parent's half: run one probe in a child and return what it said, or panic naming how it
/// died. An abort is the defect.
fn probe(door: &str, name: &str, n: usize) -> String {
    let out = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "probe_child", "--nocapture", "--test-threads=1"])
        .env(PROBE, format!("{door}:{name}:{n}"))
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "{name} nested {n} deep through `{door}` did not survive a 2 MiB thread: {} — {}",
        out.status,
        stderr
            .lines()
            .find(|l| l.contains("overflow"))
            .unwrap_or(&stderr)
    );
    let at = stdout
        .find("\nOUTCOME ")
        .unwrap_or_else(|| panic!("the `{door}` {name} probe reported nothing: {stdout}"));
    stdout[at + "\nOUTCOME ".len()..]
        .lines()
        .next()
        .unwrap_or("")
        .to_string()
}

fn assert_refused(outcome: &str, arg: &str, what: &str) {
    assert!(
        outcome.starts_with(&format!("err invalid argument `{arg}`"))
            && outcome.contains("deeper than 64 (MAX_JSON_NESTING)"),
        "{what}: {outcome}"
    );
}

#[test]
fn deep_documents_are_refused_at_every_door_and_abort_nothing() {
    for door in ["expand", "flatten", "compact", "prune"] {
        for name in SHAPES {
            assert_refused(
                &probe(door, name, 3000),
                "content",
                &format!("{door} {name}"),
            );
        }
    }
}

#[test]
fn a_deep_context_is_refused_naming_context_inline_and_by_reference() {
    for door in [
        "compact-context",
        "compact-context-ref",
        "prune-context",
        "prune-context-ref",
    ] {
        assert_refused(&probe(door, "context", 3000), "context", door);
    }
}

/// At the bound every door answers, on the 2 MiB thread the probe gives it: a debug build needs
/// ~9 MiB for 64 levels of node objects, so this passes only because the work runs on its own
/// sized thread. `nodes` nests `n + 1` JSON levels.
#[test]
fn a_document_at_the_bound_is_answered_on_a_small_thread_and_one_past_is_refused() {
    for door in ["expand", "flatten", "compact", "prune"] {
        let ok = probe(door, "nodes", BOUND - 1);
        assert!(ok.starts_with("ok "), "{door} at the bound: {ok}");
        assert_refused(&probe(door, "nodes", BOUND), "content", door);
    }
    // Two levels a level: 31 node objects in arrays nest 63, inside the bound.
    let ok = probe("expand", "node-arrays", BOUND / 2 - 1);
    assert!(ok.starts_with("ok "), "node-arrays at the bound: {ok}");
    // A context of 31 scoped levels nests 63.
    for door in ["compact-context", "compact-context-ref"] {
        let ok = probe(door, "context", BOUND / 2 - 1);
        assert!(ok.starts_with("ok "), "{door} at the bound: {ok}");
        assert_refused(&probe(door, "context", BOUND / 2), "context", door);
    }
}

#[test]
fn brackets_in_strings_are_not_nesting() {
    let deep = "[{".repeat(500);
    let text =
        format!("{{\"@id\":\"urn:ex:s\",\"urn:ex:p\":[\"{deep}\",\"a\\\"{deep}\",\"\\\\\"]}}");
    for door in ["expand", "flatten", "compact", "prune"] {
        let answer = issue(door, text.clone()).unwrap_or_else(|e| panic!("{door}: {e}"));
        assert!(answer.contains("urn:ex:s"), "{door}: {answer}");
    }
}
