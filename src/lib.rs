//! `ikigai-jsonld` — JSON-LD operators as ikigai resources.
//!
//! `urn:jsonld:expand` / `:flatten` / `:compact` run the JSON-LD 1.1 API algorithms over a
//! piped `content` document, via the `json-ld` crate with a **static, no-network context
//! loader** ([`NoLoader`]). They're `application/ld+json → application/ld+json`
//! *transformations* (not media-type conversions), so they're plain operation endpoints, not
//! transreptors: `expand`/`flatten` are auto-invocable (content only); `compact` is
//! parameterized by a `context`. (Framing is not in the underlying crate; the layers
//! "shape a graph by root" need is served by SPARQL `CONSTRUCT` for now.)
//!
//! The ops are async in `json-ld`, but with `NoLoader` they do **no I/O** — they complete in
//! a single poll — so each endpoint drives the future with `now_or_never` (poll once, take the
//! result, never park) and stays a simple synchronous endpoint. Wasm-safe by construction (no
//! executor, no blocking); a clean error if a future ever unexpectedly pends.
//!
//! Heavy dependency (the `json-ld` tree), so this is a standalone crate meant to be
//! lazy-loaded as a WASM module — the ikigai-xslt playbook — keeping it out of the host's
//! core wasm bundle.

#![forbid(unsafe_code)]

use contextual::WithContext;
use futures::future::FutureExt;
use ikigai_core::{
    ArgSpec, Description, EndpointSpace, Error, Exact, FnEndpoint, Invocation, ReprType,
    Representation, Result, Verb,
};
use json_ld::syntax::{Parse, Print, TryFromJson};
use json_ld::{IriBuf, JsonLdProcessor, NoLoader, RemoteContextReference, RemoteDocument};

/// Parse `content` (JSON-LD bytes) into a `RemoteDocument`, with an optional base IRI.
fn parse_doc(content: &[u8], base: Option<&str>) -> Result<RemoteDocument<IriBuf>> {
    let text = std::str::from_utf8(content)
        .map_err(|e| Error::Endpoint(format!("JSON-LD input is not UTF-8: {e}")))?;
    let (value, _) = json_ld::syntax::Value::parse_str(text)
        .map_err(|e| Error::Endpoint(format!("JSON-LD parse error: {e}")))?;
    let base_iri = match base {
        Some(b) => Some(
            IriBuf::new(b.to_string())
                .map_err(|e| Error::Endpoint(format!("bad base IRI `{b}`: {e}")))?,
        ),
        None => None,
    };
    Ok(RemoteDocument::new(base_iri, None, value))
}

/// An `application/ld+json` representation from a printed JSON string.
fn repr(json: String) -> Representation {
    Representation::new(
        ReprType::new("application/ld+json").with_param("charset", "utf-8"),
        json.into_bytes(),
    )
    .cacheable()
}

fn expand(inv: &Invocation<'_>) -> Result<Representation> {
    let content = inv.inline_arg("content").map_err(|_| {
        Error::Endpoint("urn:jsonld:expand needs a JSON-LD `content` document".to_string())
    })?;
    let doc = parse_doc(content, inv.inline_str("base").ok())?;
    let loader = NoLoader;
    let expanded = doc
        .expand(&loader)
        .now_or_never()
        .ok_or_else(|| Error::Endpoint("JSON-LD expand did not complete synchronously".into()))?
        .map_err(|e| Error::Endpoint(format!("JSON-LD expand failed: {e}")))?;
    // ExpandedDocument prints only with a vocabulary; `()` is the no-op vocabulary (IRIs are
    // IriBuf), and `.with(&())` wraps it as a plain `Print`-able value.
    Ok(repr(expanded.with(&()).pretty_print().to_string()))
}

fn flatten(inv: &Invocation<'_>) -> Result<Representation> {
    let content = inv.inline_arg("content").map_err(|_| {
        Error::Endpoint("urn:jsonld:flatten needs a JSON-LD `content` document".to_string())
    })?;
    let doc = parse_doc(content, inv.inline_str("base").ok())?;
    let loader = NoLoader;
    let mut generator = json_ld::rdf_types::generator::Blank::new();
    let flattened = doc
        .flatten(&mut generator, &loader)
        .now_or_never()
        .ok_or_else(|| Error::Endpoint("JSON-LD flatten did not complete synchronously".into()))?
        .map_err(|e| Error::Endpoint(format!("JSON-LD flatten failed: {e}")))?;
    Ok(repr(flattened.pretty_print().to_string()))
}

fn compact(inv: &Invocation<'_>) -> Result<Representation> {
    let content = inv.inline_arg("content").map_err(|_| {
        Error::Endpoint("urn:jsonld:compact needs a JSON-LD `content` document".to_string())
    })?;
    let context_bytes = inv.inline_arg("context").map_err(|_| {
        Error::Endpoint("urn:jsonld:compact needs a `context` (a JSON-LD context)".to_string())
    })?;
    let doc = parse_doc(content, inv.inline_str("base").ok())?;

    let ctx_text = std::str::from_utf8(context_bytes)
        .map_err(|e| Error::Endpoint(format!("context is not UTF-8: {e}")))?;
    let (ctx_value, _) = json_ld::syntax::Value::parse_str(ctx_text)
        .map_err(|e| Error::Endpoint(format!("context parse error: {e}")))?;
    // Accept either a bare context value (`{"name": …}`) or a context *document*
    // (`{"@context": {…}}`); unwrap the latter to its `@context` value.
    let context_value = match ctx_value.as_object().and_then(|o| o.get("@context").next()) {
        Some(inner) => inner.clone(),
        None => ctx_value,
    };
    let context = json_ld::syntax::Context::try_from_json(context_value)
        .map_err(|e| Error::Endpoint(format!("invalid JSON-LD context: {e}")))?;
    let remote_ctx = RemoteContextReference::Loaded(RemoteDocument::new(None, None, context));

    let loader = NoLoader;
    let compacted = doc
        .compact(remote_ctx, &loader)
        .now_or_never()
        .ok_or_else(|| Error::Endpoint("JSON-LD compact did not complete synchronously".into()))?
        .map_err(|e| Error::Endpoint(format!("JSON-LD compact failed: {e}")))?;
    Ok(repr(compacted.pretty_print().to_string()))
}

fn content_input() -> ArgSpec {
    ArgSpec::new("content").summary("the JSON-LD document — usually piped in")
}
fn base_input() -> ArgSpec {
    ArgSpec::new("base")
        .summary("optional base IRI for relative references")
        .optional()
}

/// The space binding `urn:jsonld:expand` / `:flatten` / `:compact`.
pub fn space() -> EndpointSpace {
    EndpointSpace::new()
        .bind(
            Exact::new("urn:jsonld:expand"),
            FnEndpoint::new("jsonld-expand", |inv: &Invocation<'_>| expand(inv)).with_description(
                Description::new("jsonld-expand")
                    .title("JSON-LD expand")
                    .summary("Expand a JSON-LD document to its fully-explicit (no-context) form.")
                    .verb(Verb::Source)
                    .verb(Verb::Meta)
                    .input(content_input())
                    .input(base_input())
                    .output("application/ld+json"),
            ),
        )
        .bind(
            Exact::new("urn:jsonld:flatten"),
            FnEndpoint::new("jsonld-flatten", |inv: &Invocation<'_>| flatten(inv))
                .with_description(
                    Description::new("jsonld-flatten")
                        .title("JSON-LD flatten")
                        .summary("Flatten a JSON-LD document: every node moved to the top level.")
                        .verb(Verb::Source)
                        .verb(Verb::Meta)
                        .input(content_input())
                        .input(base_input())
                        .output("application/ld+json"),
                ),
        )
        .bind(
            Exact::new("urn:jsonld:compact"),
            FnEndpoint::new("jsonld-compact", |inv: &Invocation<'_>| compact(inv))
                .with_description(
                    Description::new("jsonld-compact")
                        .title("JSON-LD compact")
                        .summary(
                            "Compact a JSON-LD document against a supplied context — short terms, \
                         and the basis of the trust-boundary egress filter.",
                        )
                        .verb(Verb::Source)
                        .verb(Verb::Meta)
                        .input(content_input())
                        .input(
                            ArgSpec::new("context")
                                .summary("the JSON-LD context to compact against"),
                        )
                        .input(base_input())
                        .output("application/ld+json"),
                ),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::executor::block_on;
    use ikigai_core::{ArgRef, Capability, Iri, Kernel, Request};
    use std::sync::Arc;

    const DOC: &str = r#"{"@context":{"name":"http://xmlns.com/foaf/0.1/name"},
        "@id":"http://example.org/ada","name":"Ada"}"#;

    fn run(iri: &str, content: &str, extra: &[(&str, &str)]) -> Result<Representation> {
        let kernel = Kernel::new(Arc::new(space()));
        let mut req = Request::new(Verb::Source, Iri::parse(iri).unwrap())
            .with_arg("content", ArgRef::Inline(content.as_bytes().to_vec()));
        for &(k, v) in extra {
            req = req.with_arg(k, ArgRef::Inline(v.as_bytes().to_vec()));
        }
        block_on(kernel.issue(req, &Capability::root()))
    }

    #[test]
    fn expand_makes_iris_explicit() {
        let rep = run("urn:jsonld:expand", DOC, &[]).unwrap();
        let body = String::from_utf8(rep.bytes).unwrap();
        assert!(body.contains("http://xmlns.com/foaf/0.1/name"), "{body}");
        assert!(body.contains("Ada"));
        assert_eq!(rep.repr_type.media_type, "application/ld+json");
    }

    #[test]
    fn flatten_produces_a_node_list() {
        let rep = run("urn:jsonld:flatten", DOC, &[]).unwrap();
        assert!(String::from_utf8(rep.bytes).unwrap().contains("Ada"));
    }

    #[test]
    fn compact_shortens_against_a_context() {
        let ctx = r#"{"@context":{"name":"http://xmlns.com/foaf/0.1/name"}}"#;
        let rep = run("urn:jsonld:compact", DOC, &[("context", ctx)]).unwrap();
        let body = String::from_utf8(rep.bytes).unwrap();
        assert!(
            body.contains("\"name\""),
            "compacted to the short term: {body}"
        );
        assert!(body.contains("Ada"));
    }

    #[test]
    fn missing_content_is_a_clean_error() {
        let kernel = Kernel::new(Arc::new(space()));
        let req = Request::new(Verb::Source, Iri::parse("urn:jsonld:expand").unwrap());
        assert!(block_on(kernel.issue(req, &Capability::root())).is_err());
    }
}

// ---------------------------------------------------------------------------
// This library *as* a dynamically-loadable WASM module (`--features module`):
// `wasm_module!` emits the cdylib glue (a `invoke_session` entry + the `hostCall`
// import + the !Send→Send bridge) over our `space()`, so a host can lazy-fetch
// `ikigai_jsonld.wasm` and resolve `urn:jsonld:*` against it — the heavy json-ld
// tree kept out of the host's wasm. The whole module is one macro line:
//   cargo build --release --lib --features module --target wasm32-unknown-unknown
// ---------------------------------------------------------------------------
#[cfg(feature = "module")]
ikigai_module::wasm_module!(crate::space);

/// Surface a Rust panic in the browser console (module builds only).
#[cfg(feature = "module")]
#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub fn __module_start() {
    console_error_panic_hook::set_once();
}
