//! The measurement behind `src/depth.rs`'s bound and stack (ledger #1042), kept so it can be
//! re-run when json-ld moves. Three measurements, each `#[ignore]`d and run by hand:
//!
//! 1. `measure_jsonld_overflow_depths`: how deep each JSON-LD shape goes before the process
//!    aborts, on a thread of a given size. `DOORS` picks what runs: `parse` (json-syntax's parser
//!    and the drop of what it built), `raw-expand`, `raw-flatten`, `raw-compact` (the json-ld
//!    calls the endpoints make, BENEATH this crate's bound), and `expand`, `flatten`, `compact`
//!    (the kernel doors, which refuse past the bound once it exists: measure those against the
//!    code that predates it).
//! 2. `measure_stack_at_the_bound`: the smallest thread each raw door needs for each shape at
//!    JSON depth [`MAX_JSON_NESTING`], found by bisecting the stack size. This is what sizes
//!    `JSON_LD_STACK`.
//! 3. `measure_memory_by_depth`: peak resident memory of a child as the depth grows around one
//!    wide innermost node (macOS `/usr/bin/time -l`), to see whether memory grows with the square
//!    of the depth as oxjsonld's does (ledger #1038).
//!    `SHAPE` is `nodes-wide` (the default) or `list-objects-wide`.
//!
//! ```text
//! cargo test --test jsonld_depth_measure -- --ignored --nocapture measure_jsonld_overflow_depths
//! SHAPES="nodes arrays" DOORS="expand flatten compact" STACK_MIB=2 \
//!     cargo test --release --test jsonld_depth_measure -- --ignored --nocapture overflow
//! cargo test --test jsonld_depth_measure -- --ignored --nocapture measure_stack_at_the_bound
//! SHAPE=list-objects-wide DEPTHS="16 32 64 128" cargo test --release \
//!     --test jsonld_depth_measure -- --ignored --nocapture measure_memory_by_depth
//! ```
//!
//! Every probe runs in a child process, this binary re-executed, so an overflow aborts the
//! child and never the measurement. The overflow search doubles `n` until a probe aborts (or
//! `CAP`), then bisects, and prints the largest `n` that survived and that document's JSON depth
//! (`{` and `[` alike; no shape here puts a bracket in a string).
//!
//! Measured 2026-10-10, json-ld 0.21.4, aarch64-apple-darwin: the tables and figures in
//! `src/depth.rs` are this file's output. Through the kernel doors, before the bound, a build
//! aborted at most a level or two sooner than the raw calls (the kernel's frames).

use futures::executor::block_on;
use futures::future::FutureExt;
use ikigai_core::{ArgRef, Capability, Iri, Kernel, Request, Verb};
use ikigai_jsonld::MAX_JSON_NESTING;
use json_ld::syntax::{Parse, Print, TryFromJson};
use json_ld::{IriBuf, JsonLdProcessor, NoLoader, RemoteContextReference, RemoteDocument};
use std::process::Command;
use std::sync::Arc;

const ENV: &str = "IKIGAI_JSONLD_MEASURE";

/// The context `compact` runs against: one term, so compaction has something to do.
const CONTEXT: &str = r#"{"p":"urn:ex:p"}"#;

/// A JSON-LD document of shape `name`, `n` levels deep.
fn shape(name: &str, n: usize) -> String {
    let r = |s: &str, k: usize| s.repeat(k);
    match name {
        // Node objects as property values: `{"@id":…,"p":{"@id":…,"p":{…}}}`. One JSON level a
        // level. The shape the ledger item reports.
        "nodes" => format!(
            "{}{{\"@id\":\"urn:ex:o\"}}{}",
            r("{\"@id\":\"urn:ex:s\",\"urn:ex:p\":", n),
            r("}", n)
        ),
        // `nodes` around an innermost node carrying 20,000 values (~0.5 MB): what each level
        // costs in MEMORY, if the expander copies a node's content once per level.
        "nodes-wide" => format!(
            "{}{{\"@id\":\"urn:ex:o\",\"urn:ex:q\":[{}]}}{}",
            r("{\"@id\":\"urn:ex:s\",\"urn:ex:p\":", n),
            vec!["\"abcdefghijklmnopqrstuvw\""; 20_000].join(","),
            r("}", n)
        ),
        // The same without `@id`: nested blank nodes.
        "bnodes" => format!(
            "{}{{\"urn:ex:q\":\"x\"}}{}",
            r("{\"urn:ex:p\":", n),
            r("}", n)
        ),
        // Node objects each wrapped in an array, as compacted JSON-LD often writes a property
        // value: two JSON levels a level.
        "node-arrays" => format!(
            "{}{{\"@id\":\"urn:ex:o\"}}{}",
            r("{\"@id\":\"urn:ex:s\",\"urn:ex:p\":[", n),
            r("]}", n)
        ),
        // Arrays of arrays as a property value (flattened by expansion).
        "arrays" => format!(
            "{{\"@id\":\"urn:ex:s\",\"urn:ex:p\":{}\"x\"{}}}",
            r("[", n),
            r("]", n)
        ),
        // A top-level array of arrays around one node.
        "top-arrays" => format!(
            "{}{{\"@id\":\"urn:ex:s\",\"urn:ex:p\":\"x\"}}{}",
            r("[", n),
            r("]", n)
        ),
        // Lists of lists (JSON-LD 1.1): arrays nested inside `@list`.
        "lists" => format!(
            "{{\"@id\":\"urn:ex:s\",\"urn:ex:p\":{{\"@list\":{}\"x\"{}}}}}",
            r("[", n),
            r("]", n)
        ),
        // `list-objects` around 20,000 values: the expander clones a `@list` object's value
        // before it expands it (json-ld-expansion 0.21 `element.rs`, `list_entry`), once a level.
        "list-objects-wide" => format!(
            "{{\"@id\":\"urn:ex:s\",\"urn:ex:p\":{}{}{}}}",
            r("{\"@list\":[", n),
            vec!["\"abcdefghijklmnopqrstuvw\""; 20_000].join(","),
            r("]}", n)
        ),
        // `@list` objects nested in `@list` objects.
        "list-objects" => format!(
            "{{\"@id\":\"urn:ex:s\",\"urn:ex:p\":{}\"x\"{}}}",
            r("{\"@list\":[", n),
            r("]}", n)
        ),
        // Keys that are not IRIs and expand to nothing, so they are dropped.
        "dropped" => format!(
            "{{\"@id\":\"urn:ex:s\",{}\"k\":1{}}}",
            r("\"k\":{", n),
            r("}", n)
        ),
        // Scoped contexts nested in term definitions: `"p":{"@id":…,"@context":{"p":{…}}}`.
        // Two JSON levels a level.
        "contexts" => format!(
            "{{\"@context\":{}{{}}{},\"@id\":\"urn:ex:s\",\"p\":\"x\"}}",
            r("{\"p\":{\"@id\":\"urn:ex:p\",\"@context\":", n),
            r("}}", n)
        ),
        // Arrays nested inside `@context`.
        "context-arrays" => format!(
            "{{\"@context\":{}{{\"p\":\"urn:ex:p\"}}{},\"@id\":\"urn:ex:s\",\"p\":\"x\"}}",
            r("[", n),
            r("]", n)
        ),
        // A JSON literal (`@type: @json`) nesting arrays: kept as JSON, not expanded.
        "json-literal" => format!(
            "{{\"@id\":\"urn:ex:s\",\"urn:ex:p\":{{\"@type\":\"@json\",\"@value\":{}1{}}}}}",
            r("[", n),
            r("]", n)
        ),
        // `@reverse` objects nested: `{"@reverse":{"p":{"@reverse":{…}}}}`.
        "reverse" => format!(
            "{}{{\"@id\":\"urn:ex:o\"}}{}",
            r("{\"@id\":\"urn:ex:s\",\"@reverse\":{\"urn:ex:p\":", n),
            r("}}", n)
        ),
        // Named graphs nested: `{"@id":…,"@graph":[{"@id":…,"@graph":[…]}]}`.
        "graphs" => format!(
            "{}{{\"@id\":\"urn:ex:o\",\"urn:ex:p\":\"x\"}}{}",
            r("{\"@id\":\"urn:ex:g\",\"@graph\":[", n),
            r("]}", n)
        ),
        other => panic!("no shape named {other}"),
    }
}

const SHAPES: [&str; 13] = [
    "nodes",
    "bnodes",
    "node-arrays",
    "arrays",
    "top-arrays",
    "lists",
    "list-objects",
    "dropped",
    "contexts",
    "context-arrays",
    "json-literal",
    "reverse",
    "graphs",
];

fn parse_doc(doc: &str) -> std::result::Result<RemoteDocument<IriBuf>, String> {
    let (value, _) = json_ld::syntax::Value::parse_str(doc).map_err(|e| e.to_string())?;
    Ok(RemoteDocument::new(None, None, value))
}

/// What the endpoints do, called on json-ld directly with no bound in front: parse, run the
/// algorithm, print, and drop everything.
fn raw(door: &str, doc: &str) -> String {
    let parsed = match parse_doc(doc) {
        Ok(parsed) => parsed,
        Err(e) => return format!("err parse {e}"),
    };
    let printed = match door {
        "parse" => return "ok parsed".to_string(),
        "raw-expand" => parsed
            .expand(&NoLoader)
            .now_or_never()
            .expect("single poll")
            .map(|expanded| {
                use contextual::WithContext;
                expanded.with(&()).pretty_print().to_string()
            })
            .map_err(|e| e.to_string()),
        "raw-flatten" => {
            let mut generator = json_ld::rdf_types::generator::Blank::new();
            parsed
                .flatten(&mut generator, &NoLoader)
                .now_or_never()
                .expect("single poll")
                .map(|flattened| flattened.pretty_print().to_string())
                .map_err(|e| e.to_string())
        }
        "raw-compact" => {
            let (value, _) = json_ld::syntax::Value::parse_str(CONTEXT).unwrap();
            let context = json_ld::syntax::Context::try_from_json(value).unwrap();
            let context = RemoteContextReference::Loaded(RemoteDocument::new(None, None, context));
            parsed
                .compact(context, &NoLoader)
                .now_or_never()
                .expect("single poll")
                .map(|compacted| compacted.pretty_print().to_string())
                .map_err(|e| e.to_string())
        }
        other => panic!("no raw door named {other}"),
    };
    match printed {
        Ok(text) => format!("ok {} bytes", text.len()),
        Err(e) => format!("err {}", e.chars().take(120).collect::<String>()),
    }
}

/// Through a kernel door, as a caller reaches it.
fn door(name: &str, doc: &str) -> String {
    let kernel = Kernel::new(Arc::new(ikigai_jsonld::space()));
    let iri = format!("urn:jsonld:{name}");
    let mut request = Request::new(Verb::Source, Iri::parse(&iri).unwrap())
        .with_arg("content", ArgRef::Inline(doc.as_bytes().to_vec()));
    if name == "compact" {
        request = request.with_arg("context", ArgRef::Inline(CONTEXT.as_bytes().to_vec()));
    }
    match block_on(kernel.issue(request, &Capability::root())) {
        Ok(rep) => format!("ok {} bytes", rep.bytes.len()),
        Err(e) => format!(
            "err {}",
            e.to_string().chars().take(120).collect::<String>()
        ),
    }
}

#[test]
fn measure_child() {
    let Ok(spec) = std::env::var(ENV) else {
        return;
    };
    let mut parts = spec.split(':');
    let which = parts.next().unwrap().to_string();
    let name = parts.next().unwrap().to_string();
    let n = parts.next().unwrap().parse::<usize>().unwrap();
    let kib = parts.next().unwrap().parse::<usize>().unwrap();
    let doc = shape(&name, n);
    let outcome = std::thread::Builder::new()
        .stack_size(kib << 10)
        .spawn(move || {
            if which == "parse" || which.starts_with("raw-") {
                raw(&which, &doc)
            } else {
                door(&which, &doc)
            }
        })
        .unwrap()
        .join()
        .unwrap();
    println!("\nOUTCOME {}", outcome.replace('\n', " "));
    std::process::exit(0);
}

/// `Some(outcome)` if the child survived, `None` if it aborted. With `rss`, the child runs under
/// macOS `/usr/bin/time -l` and the outcome ends with its peak resident memory.
fn child(which: &str, name: &str, n: usize, kib: usize, rss: bool) -> Option<String> {
    let exe = std::env::current_exe().unwrap();
    let args = [
        "--exact",
        "measure_child",
        "--nocapture",
        "--test-threads=1",
    ];
    let mut command = if rss {
        let mut command = Command::new("/usr/bin/time");
        command.arg("-l").arg(&exe).args(args);
        command
    } else {
        let mut command = Command::new(&exe);
        command.args(args);
        command
    };
    let out = command
        .env(ENV, format!("{which}:{name}:{n}:{kib}"))
        .output()
        .unwrap();
    if !out.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    let at = stdout.find("\nOUTCOME ")?;
    let mut outcome = stdout[at + "\nOUTCOME ".len()..]
        .lines()
        .next()
        .unwrap_or("")
        .to_string();
    if rss {
        let stderr = String::from_utf8_lossy(&out.stderr);
        let peak = stderr
            .lines()
            .find(|line| line.contains("maximum resident set size"))
            .and_then(|line| line.split_whitespace().next())
            .and_then(|bytes| bytes.parse::<u64>().ok())
            .unwrap_or(0);
        outcome = format!("{outcome}; peak {} MiB", peak >> 20);
    }
    Some(outcome)
}

/// How deep `doc` nests `{` and `[`. The shapes carry no bracket inside a string.
fn json_depth(doc: &[u8]) -> usize {
    let (mut depth, mut deepest) = (0usize, 0usize);
    for &b in doc {
        match b {
            b'{' | b'[' => {
                depth += 1;
                deepest = deepest.max(depth);
            }
            b'}' | b']' => depth -= 1,
            _ => {}
        }
    }
    deepest
}

fn env_list(key: &str, default: &[&str]) -> Vec<String> {
    std::env::var(key)
        .map(|v| v.split_whitespace().map(str::to_string).collect())
        .unwrap_or_else(|_| default.iter().map(|s| s.to_string()).collect())
}

fn env_usize(key: &str, default: usize) -> usize {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn build() -> &'static str {
    if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    }
}

#[test]
#[ignore = "a measurement, run by hand: see the header"]
fn measure_jsonld_overflow_depths() {
    let shapes = env_list("SHAPES", &SHAPES);
    let doors = env_list(
        "DOORS",
        &["parse", "raw-expand", "raw-flatten", "raw-compact"],
    );
    let mib = env_usize("STACK_MIB", 2);
    let cap = env_usize("CAP", 20_000);
    println!("\n{}, {mib} MiB thread, cap {cap}", build());
    for which in &doors {
        for name in &shapes {
            // Double until a probe aborts or the cap, then bisect between the last survivor
            // and the first abort.
            let mut good = 0usize;
            let mut last = String::new();
            let mut n = 1usize;
            let mut bad = None;
            while n <= cap {
                match child(which, name, n, mib << 10, false) {
                    Some(outcome) => {
                        good = n;
                        last = outcome;
                        n *= 2;
                    }
                    None => {
                        bad = Some(n);
                        break;
                    }
                }
            }
            if let Some(mut hi) = bad {
                while hi - good > 1 {
                    let mid = (good + hi) / 2;
                    match child(which, name, mid, mib << 10, false) {
                        Some(outcome) => {
                            good = mid;
                            last = outcome;
                        }
                        None => hi = mid,
                    }
                }
            }
            let depth = json_depth(shape(name, good.max(1)).as_bytes());
            let verdict = match bad {
                Some(_) => format!("survives {good}, aborts {}", good + 1),
                None => format!("survives ≥ {good} (no abort to the cap)"),
            };
            println!(
                "{which:11} {name:14} {verdict:34} json depth {depth:6}  {}",
                last.chars().take(80).collect::<String>()
            );
        }
    }
}

/// The largest `n` whose document nests no deeper than `MAX_JSON_NESTING`.
fn n_at_the_bound(name: &str) -> usize {
    let mut n = 1;
    while json_depth(shape(name, n + 1).as_bytes()) <= MAX_JSON_NESTING {
        n += 1;
    }
    n
}

#[test]
#[ignore = "a measurement, run by hand: see the header"]
fn measure_stack_at_the_bound() {
    let shapes = env_list("SHAPES", &SHAPES);
    let doors = env_list(
        "DOORS",
        &["parse", "raw-expand", "raw-flatten", "raw-compact"],
    );
    println!(
        "\n{}, the smallest thread (KiB) at json depth {MAX_JSON_NESTING}",
        build()
    );
    for which in &doors {
        for name in &shapes {
            let n = n_at_the_bound(name);
            // Bisect in KiB between a stack that aborts and one that survives.
            let (mut lo, mut hi) = (16usize, 256usize << 10);
            if child(which, name, n, hi, false).is_none() {
                println!(
                    "{which:11} {name:14} n {n:4}: aborts even at {} MiB",
                    hi >> 10
                );
                continue;
            }
            while hi - lo > 16 {
                let mid = (lo + hi) / 2;
                if child(which, name, n, mid, false).is_some() {
                    hi = mid;
                } else {
                    lo = mid;
                }
            }
            println!("{which:11} {name:14} n {n:4}: needs ≤ {hi:6} KiB");
        }
    }
}

#[test]
#[ignore = "a measurement, run by hand (macOS): see the header"]
fn measure_memory_by_depth() {
    let doors = env_list("DOORS", &["raw-expand", "raw-flatten", "raw-compact"]);
    let depths = env_list("DEPTHS", &["1", "16", "32", "64", "128", "256"]);
    let mib = env_usize("STACK_MIB", 256);
    let name = std::env::var("SHAPE").unwrap_or_else(|_| "nodes-wide".to_string());
    println!("\n{}, {mib} MiB thread, shape {name}", build());
    for which in &doors {
        for depth in &depths {
            let n: usize = depth.parse().unwrap();
            let outcome =
                child(which, &name, n, mib << 10, true).unwrap_or_else(|| "ABORTED".to_string());
            println!("{which:11} n {n:5}: {outcome}");
        }
    }
}
