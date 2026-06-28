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
//! a single poll — so each drives that future with `now_or_never` (poll once, take the result,
//! never park). Wasm-safe by construction (no executor, no blocking); a clean error if a future
//! ever unexpectedly pends. `expand`/`flatten` are therefore plain synchronous endpoints.
//!
//! `compact` is a genuinely `async` endpoint, because its `context` may be **a resource**: a
//! value starting with `{`/`[` is inline JSON, anything else is an IRI resolved through the
//! kernel (`inv.source`, or `urn:httpGet` for http(s)) — so the context that shapes a
//! compaction is itself addressable (the trust-boundary egress-filter framing), and the
//! result inherits the context's golden thread (cacheable, invalidated when it changes). The
//! one `await` is that resolution; the `json-ld` compaction stays single-poll afterward.
//!
//! Heavy dependency (the `json-ld` tree), so this is a standalone crate meant to be
//! lazy-loaded as a WASM module — the ikigai-xslt playbook — keeping it out of the host's
//! core wasm bundle.

#![forbid(unsafe_code)]

use async_trait::async_trait;
use contextual::WithContext;
use futures::future::FutureExt;
use ikigai_core::{
    ArgRef, ArgSpec, Description, Endpoint, EndpointSpace, Error, Exact, FnEndpoint, Invocation,
    Iri, ReprType, Representation, Request, Result, Verb,
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

/// Is `s` an inline JSON-LD context (`{…}` / `[…]`) rather than a resource reference? A bare
/// `urn:`/`http(s)` IRI is the by-reference case; an object or array literal is inline.
fn is_inline_context(s: &str) -> bool {
    matches!(s.trim_start().chars().next(), Some('{') | Some('['))
}

/// Resolve a `context` reference through the kernel — `urn:`/`file:` directly via `inv.source`,
/// http(s) via the `urn:httpGet` module — recording it as a dependency so the compaction is
/// cacheable and invalidates when the context resource changes. (Mirrors the xslt module's
/// stylesheet/src resolution.)
async fn resolve_context(inv: &Invocation<'_>, uri: &str) -> Result<Representation> {
    if uri.starts_with("http://") || uri.starts_with("https://") {
        let get = Iri::parse("urn:httpGet").expect("urn:httpGet is a valid IRI");
        let request = Request::new(Verb::Source, get)
            .with_arg("url", ArgRef::Inline(uri.as_bytes().to_vec()));
        inv.issue(request).await
    } else {
        let iri = Iri::parse(uri)
            .map_err(|e| Error::Endpoint(format!("bad context IRI `{uri}`: {e}")))?;
        inv.source(&iri).await
    }
}

/// Compact `content` against `context_bytes` (already-loaded JSON-LD context bytes). Sync: the
/// `json-ld` compaction is single-poll under `NoLoader`, and no `json-ld` value crosses an await.
fn compact_doc(content: &[u8], context_bytes: &[u8], base: Option<&str>) -> Result<Representation> {
    let doc = parse_doc(content, base)?;
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

/// `urn:jsonld:compact` — an async endpoint, since its `context` may be a resource to resolve
/// (inline `{…}` JSON is used directly). See the module docs.
struct CompactEndpoint;

#[async_trait]
impl Endpoint for CompactEndpoint {
    async fn invoke(&self, inv: &Invocation<'_>) -> Result<Representation> {
        let content = inv
            .inline_arg("content")
            .map_err(|_| {
                Error::Endpoint("urn:jsonld:compact needs a JSON-LD `content` document".to_string())
            })?
            .to_vec();
        let context_arg = inv.inline_str("context").map_err(|_| {
            Error::Endpoint(
                "urn:jsonld:compact needs a `context`: inline JSON ({…}) or a resolvable \
                 resource IRI"
                    .to_string(),
            )
        })?;
        let base = inv.inline_str("base").ok().map(str::to_string);

        // Inline JSON used directly; anything else is a resource reference resolved through the
        // kernel (the one await — no json-ld value is live across it, so the future stays Send).
        let context_bytes = if is_inline_context(context_arg) {
            context_arg.as_bytes().to_vec()
        } else {
            resolve_context(inv, context_arg).await?.bytes
        };
        compact_doc(&content, &context_bytes, base.as_deref())
    }

    fn name(&self) -> &str {
        "jsonld-compact"
    }

    fn describe(&self) -> Description {
        Description::new("jsonld-compact")
            .title("JSON-LD compact")
            .summary(
                "Compact a JSON-LD document against a supplied context — short terms, and the \
                 basis of the trust-boundary egress filter.",
            )
            .verb(Verb::Source)
            .verb(Verb::Meta)
            .input(content_input())
            .input(
                ArgSpec::new("context")
                    .summary("the JSON-LD context: inline JSON ({…}) or a resolvable resource IRI"),
            )
            .input(base_input())
            .output("application/ld+json")
    }
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
        .bind(Exact::new("urn:jsonld:compact"), CompactEndpoint)
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
    fn compact_resolves_a_context_reference() {
        // A `context=<iri>` that isn't inline JSON is sourced through the kernel — the
        // context is itself a resource. Bind one alongside the jsonld space and compact
        // against it by IRI.
        use ikigai_core::{Fallback, FnEndpoint, Space};
        const CTX: &str = r#"{"@context":{"name":"http://xmlns.com/foaf/0.1/name"}}"#;
        let context_space = EndpointSpace::new().bind(
            Exact::new("urn:data:test-context"),
            FnEndpoint::new("test-context", |_inv: &Invocation<'_>| {
                Ok(Representation::new(
                    ReprType::new("application/ld+json"),
                    CTX.as_bytes().to_vec(),
                ))
            }),
        );
        let root = Fallback::new(vec![
            Arc::new(space()) as Arc<dyn Space>,
            Arc::new(context_space) as Arc<dyn Space>,
        ]);
        let kernel = Kernel::new(Arc::new(root));
        let req = Request::new(Verb::Source, Iri::parse("urn:jsonld:compact").unwrap())
            .with_arg("content", ArgRef::Inline(DOC.as_bytes().to_vec()))
            .with_arg(
                "context",
                ArgRef::Inline("urn:data:test-context".as_bytes().to_vec()),
            );
        let body = String::from_utf8(
            block_on(kernel.issue(req, &Capability::root()))
                .unwrap()
                .bytes,
        )
        .unwrap();
        assert!(
            body.contains("\"name\""),
            "compacted via context resource: {body}"
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
