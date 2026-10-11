# ikigai-jsonld

JSON-LD operators for [ikigai](https://github.com/ikigai-rs) — the three
JSON-LD 1.1 API algorithms as `urn:jsonld:*` resources, plus `prune`, the
trust-boundary egress filter built on them, on the
[`json-ld`](https://crates.io/crates/json-ld) crate with a static, no-network
context loader.

```text
source urn:file:ada.jsonld | urn:jsonld:expand
source urn:file:ada.jsonld | urn:jsonld:compact context=urn:file:foaf-context.jsonld
source urn:file:offer.jsonld | urn:jsonld:prune context=urn:file:collab-context.jsonld
```

Like `ikigai-xslt`, this is a standalone **module crate** with a heavy
dependency tree, so a host links it in and mounts [`space`] — or, with
`--features module`, builds it as a lazy-loadable WASM module
(`ikigai_jsonld.wasm`) and resolves `urn:jsonld:*` against it, keeping the
`json-ld` tree out of the host's own wasm bundle.

`space()` names itself `urn:iki:space:jsonld` (the const `SPACE_ID`).

## Endpoints

Every endpoint is a single-verb `Source`, takes the document as `content`
(usually piped in), and serves `application/ld+json; charset=utf-8` (prune's
`face=report` serves `application/json`). They are
`application/ld+json → application/ld+json` *transformations* — the caller's
document after the algorithm, not a graph this module authors — so the output is
exactly as named, typed and skolemized as the input: an anonymous input node is
anonymous in the output (or freshly labeled `_:0`, `_:1`, … by `flatten`).

| IRI | args | behavior |
|-----|------|----------|
| `urn:jsonld:expand` | `base=` optional base IRI | the expanded form: every term a full IRI, no context |
| `urn:jsonld:flatten` | `base=` | the flattened form: every node at the top level |
| `urn:jsonld:compact` | `context=` (required); `base=` | the compacted form against `context` |
| `urn:jsonld:prune` | `context=` (required, no `@vocab`); `base=`; `face=document\|report` | the compacted form with everything `context` does not define removed; `face=report` lists what was removed |

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

`prune`'s `context` works the same way. Every result is marked `.cacheable()`;
the kernel's effective expiry does the rest.

### Prune: the trust-boundary egress filter

Compaction filters nothing: a predicate the context does not map comes out as a
full IRI, one under a prefix the context declares as a compact IRI
(`collab:hidden`), and an unmapped `@type` keeps its IRI. `urn:jsonld:prune`
expands `content`, removes everything `context` does not **define**, and
compacts with the same context, so the output speaks only the context's terms:

- **the allowlist** is read from the processed context: the IRI of every term
  that is not a prefix (property and `@type` values), of every reverse-property
  term (`@reverse`), and every datatype a term coerces to (typed values). ⚠ **A
  prefix is not a term**: `"collab": "https://…/collab#"` lets the context write
  `"brief": "collab:brief"`; it does not let the document say `collab:anything`;
- **removed**: every property, reverse property and `@type` value outside the
  allowlist, every value whose datatype is outside it (the value, not just its
  datatype), and every node left empty by that, wherever it sits. A node that
  was only ever a reference (`{"@id": …}`) is a value, and stays;
- **refused** (`InvalidArgument` on `context`): a context with `@vocab` anywhere,
  which would make every unmapped key look defined, and a scoped `@context` in a
  term definition, which makes "the terms this context defines" depend on
  position. An egress context names each term it allows, and needs neither;
- **`face=report`** is what was removed, as JSON: `dropped` (the total), the
  distinct `properties`, `reverseProperties`, `types` and `datatypes`, and
  `emptyNodes`. A caller that must never lose content silently refuses to
  publish when `dropped` is not 0.

An allowed document is a fixed point: prune changes nothing compaction would
not, and pruning its own output returns it byte for byte. ⚠ A context decides
which PROPERTIES cross, never what a crossing value says, and an `@id` is a
value: redacting values is a CONSTRUCT's job, upstream.

### Nesting is bounded

json-ld's algorithms recurse once per nested JSON object and array, and a stack
overflow aborts the whole host: before 0.2.1, node objects nested 13 deep (a
debug build) or ~123 deep (release) aborted all three endpoints on a 2 MiB
thread. So every `content`, and `compact`'s and `prune`'s `context` whether
inline or resolved, is refused past **64** levels of `{` and `[` (`MAX_JSON_NESTING`,
the same bound `ikigai-rdf` holds) as a typed `InvalidArgument` naming the
argument, before json-ld sees a byte. Natively the work runs on a 32 MiB
thread (`JSON_LD_STACK`), so a debug and a release host admit the same
documents; the wasm module has no threads and holds the bound on its default
stack. Real JSON-LD nests about 10 deep at most. The measurements behind both
numbers live in `src/depth.rs` and `tests/jsonld_depth_measure.rs`.

## Conformance

The module **passes
[`ikigai-conformance`](https://github.com/ikigai-rs/ikigai-conformance)** with
no opt-outs: `tests/conformance.rs` walks every `urn:jsonld:*` endpoint and runs
every check (ArgSpec completeness, declared = enforced, skolemized RDF faces,
vocabulary terms, cacheability, pipeline citizenship, naming). All four are
declared `pure` and `cacheable` over an inline context; the same file walks a
context-by-reference kernel twice (threaded and live) and pins by hand what the
suite cannot see — the blank-node mirror above, the remote-context gate, and
that each action serves exactly the faces it declares. `tests/prune.rs` pins
prune's rules over the collaboration layer's allowlist context.

## Status

0.2.2: `urn:jsonld:prune`, the trust-boundary egress filter: keep only what a context defines, refuse `@vocab`, report what was removed. 0.2.1 bounded caller JSON-LD in nesting (64) and processed it on a sized thread. Depends only on published crates. Dual-licensed MIT / Apache-2.0.
