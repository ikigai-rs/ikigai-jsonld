# ikigai-jsonld

JSON-LD operators for [ikigai](https://github.com/ikigai-rs) — the three
JSON-LD 1.1 API algorithms as `urn:jsonld:*` resources, on the
[`json-ld`](https://crates.io/crates/json-ld) crate with a static, no-network
context loader.

```text
source urn:file:ada.jsonld | urn:jsonld:expand
source urn:file:ada.jsonld | urn:jsonld:compact context=urn:file:foaf-context.jsonld
```

Like `ikigai-xslt`, this is a standalone **module crate** with a heavy
dependency tree, so a host links it in and mounts [`space`] — or, with
`--features module`, builds it as a lazy-loadable WASM module
(`ikigai_jsonld.wasm`) and resolves `urn:jsonld:*` against it, keeping the
`json-ld` tree out of the host's own wasm bundle.

## Endpoints

Every endpoint is a single-verb `Source`, takes the document as `content`
(usually piped in), and serves `application/ld+json; charset=utf-8`. They are
`application/ld+json → application/ld+json` *transformations* — the caller's
document after the algorithm, not a graph this module authors — so the output is
exactly as named, typed and skolemized as the input: an anonymous input node is
anonymous in the output (or freshly labeled `_:0`, `_:1`, … by `flatten`).

| IRI | args | behavior |
|-----|------|----------|
| `urn:jsonld:expand` | `base=` optional base IRI | the expanded form: every term a full IRI, no context |
| `urn:jsonld:flatten` | `base=` | the flattened form: every node at the top level |
| `urn:jsonld:compact` | `context=` (required); `base=` | the compacted form against `context` |

Framing is not in the underlying crate; the "shape a graph by root" need is
served by SPARQL `CONSTRUCT` (`ikigai-linkeddata`).

### The context is a resource

`compact`'s `context` is either inline JSON (`{…}` / `[…]`, a bare context or a
`{"@context": …}` document) or **the IRI of a resource resolved through the
kernel** — `urn:file:…`, any bound `urn:`, or `http(s)://…`, the last by issuing
`urn:httpGet` (bound by `ikigai-http`). So the context that shapes a compaction
is itself addressable, and the result inherits its expiry and golden threads:

- an inline context makes the compaction a pure function of its inputs —
  cacheable, no thread;
- a context served under a golden thread (a watched file) makes the compaction
  cacheable under that same thread, recomputed when the thread is cut;
- a context served live (uncacheable) makes the compaction uncacheable;
- a remote context is gated where the fetch is: sub-resolutions run under the
  caller's capability, so a caller with no `urn:cap:net:<host>` grant gets a
  typed `Denied`, and a host that binds no `urn:httpGet` gets `Unresolved`.
  `compact` itself declares no capability — with an inline or `urn:` context it
  reaches no network, and the manifold offers it to every caller.

Every result is marked `.cacheable()`; the kernel's effective expiry does the
rest.

## Conformance

The module **passes
[`ikigai-conformance`](https://github.com/ikigai-rs/ikigai-conformance)** with
no opt-outs: `tests/conformance.rs` walks every `urn:jsonld:*` endpoint and runs
every check (ArgSpec completeness, declared = enforced, skolemized RDF faces,
vocabulary terms, cacheability, pipeline citizenship, naming). All three are
declared `pure` and `cacheable` over an inline context; the same file walks a
context-by-reference kernel twice (threaded and live) and pins by hand what the
suite cannot see — the blank-node mirror above, the remote-context gate, and
that each action serves exactly the one face it declares.

## Status

0.1.2. Depends only on published crates. Dual-licensed MIT / Apache-2.0.
