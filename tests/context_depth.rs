//! Context dependency depth is independent of JSON nesting. Run every dangerous probe in a
//! child process, so a regression aborts only that child, as in tests/json_depth.rs.
use futures::executor::block_on;
use ikigai_core::{
    ArgRef, Capability, Exact, FnEndpoint, Invocation, Iri, Kernel, ReprType, Representation,
    Request, Verb,
};
use std::{process::Command, sync::Arc};

const PROBE: &str = "IKIGAI_JSONLD_CONTEXT_DEPTH_PROBE";
const CONTEXT_IRI: &str = "urn:test:context";
const BOUND: usize = 64;

/// Exactly n dependent definitions, with direct, prefix, datatype or reverse dependencies.
fn chain(kind: &str, n: usize) -> String {
    let mut entries = (0..n - 1)
        .map(|i| match kind {
            "id" => format!(r#""t{i}":"t{}""#, i + 1),
            "prefix" => format!(r#""t{i}":{{"@id":"t{}:x","@prefix":true}}"#, i + 1),
            "type" => format!(r#""t{i}":{{"@id":"urn:ex:p{i}","@type":"t{}"}}"#, i + 1),
            "cycle" => format!(r#""t{i}":"t{}""#, i + 1),
            "reverse" => format!(r#""t{i}":{{"@reverse":"t{}"}}"#, i + 1),
            other => panic!("unknown chain: {other}"),
        })
        .collect::<Vec<_>>();
    if kind == "cycle" {
        entries.push(format!(r#""t{}":"t0""#, n - 1));
    } else {
        entries.push(format!(r#""t{}":"urn:ex:p""#, n - 1));
    }
    format!("{{{}}}", entries.join(","))
}

fn issue(door: &str, context: String) -> ikigai_core::Result<Representation> {
    let resource_context = context.clone();
    let space = ikigai_jsonld::space().bind(
        Exact::new(CONTEXT_IRI),
        FnEndpoint::new("context", move |_inv: &Invocation<'_>| {
            Ok(Representation::new(
                ReprType::new("application/ld+json"),
                resource_context.as_bytes().to_vec(),
            ))
        }),
    );
    let kernel = Kernel::new(Arc::new(space));
    let (op, content, target) = match door {
        "expand" | "flatten" | "compact" | "prune" => (
            door,
            format!(r#"{{"@context":{context},"t0":"value"}}"#),
            r#"{"p":"urn:ex:p"}"#.to_string(),
        ),
        "compact-context" => ("compact", "{}".to_string(), context),
        "prune-context" => ("prune", "{}".to_string(), context),
        "compact-ref" => ("compact", "{}".to_string(), CONTEXT_IRI.to_string()),
        "prune-ref" => ("prune", "{}".to_string(), CONTEXT_IRI.to_string()),
        other => panic!("unknown door: {other}"),
    };
    let request = Request::new(
        Verb::Source,
        Iri::parse(format!("urn:jsonld:{op}")).unwrap(),
    )
    .with_arg("content", ArgRef::Inline(content.into_bytes()))
    .with_arg("context", ArgRef::Inline(target.into_bytes()));
    block_on(kernel.issue(request, &Capability::root()))
}

#[test]
fn probe_child() {
    let Ok(spec) = std::env::var(PROBE) else {
        return;
    };
    let mut parts = spec.split(':');
    let door = parts.next().unwrap().to_string();
    let kind = parts.next().unwrap().to_string();
    let n = parts.next().unwrap().parse().unwrap();
    let result = std::thread::Builder::new()
        .stack_size(2 << 20)
        .spawn(move || match issue(&door, chain(&kind, n)) {
            Ok(_) => "ok".to_string(),
            Err(error) => format!("err {error}"),
        })
        .unwrap()
        .join()
        .unwrap();
    println!("\nOUTCOME {result}");
}

fn probe(door: &str, kind: &str, n: usize) -> String {
    let out = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "probe_child", "--nocapture", "--test-threads=1"])
        .env(PROBE, format!("{door}:{kind}:{n}"))
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{door} {kind} {n}: {}: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).unwrap();
    stdout
        .lines()
        .find_map(|line| line.strip_prefix("OUTCOME "))
        .unwrap_or_else(|| panic!("no outcome: {stdout}"))
        .to_string()
}

#[test]
fn long_flat_chains_are_refused_at_every_door_without_aborting() {
    for door in [
        "expand",
        "flatten",
        "compact",
        "prune",
        "compact-context",
        "prune-context",
        "compact-ref",
        "prune-ref",
    ] {
        for kind in ["id", "prefix", "type", "reverse", "cycle"] {
            let outcome = probe(door, kind, 10_000);
            let argument = if door.contains('-') {
                "context"
            } else {
                "content"
            };
            assert!(
                outcome.starts_with(&format!("err invalid argument `{argument}`"))
                    && outcome.contains("deeper than 64 (MAX_CONTEXT_DEPENDENCY_DEPTH)"),
                "{door} {kind}: {outcome}"
            );
        }
    }
}

#[test]
fn chains_at_the_bound_are_processed_and_one_past_is_refused() {
    assert_eq!(ikigai_jsonld::MAX_CONTEXT_DEPENDENCY_DEPTH, BOUND);
    for door in [
        "expand",
        "flatten",
        "compact",
        "prune",
        "compact-context",
        "prune-context",
        "compact-ref",
        "prune-ref",
    ] {
        assert_eq!(probe(door, "id", BOUND), "ok", "{door}");
        assert!(
            probe(door, "id", BOUND + 1).contains("MAX_CONTEXT_DEPENDENCY_DEPTH"),
            "{door}"
        );
    }
}

#[test]
fn wide_independent_contexts_are_still_accepted() {
    let entries = (0..10_000)
        .map(|i| format!(r#""t{i}":"urn:ex:p{i}""#))
        .collect::<Vec<_>>();
    issue("expand", format!("{{{}}}", entries.join(","))).unwrap();
    let mut shared = (0..10_000)
        .map(|i| format!(r#""t{i}":{{"@id":"urn:ex:p{i}","@type":"datatype"}}"#))
        .collect::<Vec<_>>();
    shared.push(r#""datatype":"urn:ex:datatype""#.to_string());
    issue("expand", format!("{{{}}}", shared.join(","))).unwrap();
}

#[test]
fn scoped_definitions_share_the_dependency_budget() {
    let inner = chain("id", 40);
    let outer = chain("id", 40).replace(
        r#""t39":"urn:ex:p""#,
        &format!(r#""t39":{{"@id":"urn:ex:p","@context":{inner}}}"#),
    );
    for door in ["expand", "compact-context", "compact-ref"] {
        let error = issue(door, outer.clone()).unwrap_err().to_string();
        assert!(
            error.contains("MAX_CONTEXT_DEPENDENCY_DEPTH"),
            "{door}: {error}"
        );
    }
}
