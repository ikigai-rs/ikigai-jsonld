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

Every answer is cacheable, so every answer is a function of its input alone: the
same document gives the same bytes on every computation, blank-node labels and
node order included (`tests/determinism.rs`).

| IRI | args | behavior |
|-----|------|----------|
| `urn:jsonld:expand` | `base=` optional base IRI | the expanded form: every term a full IRI, no context |
| `urn:jsonld:flatten` | `base=` | the flattened form: every node at the top level, sorted by `@id` (the spec's `ordered` flattening) |
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
  datatype), and every node left empty by that, wherever it sits. Originally
  empty allowed properties, graphs and included arrays stay empty; only containers
  emptied by filtering are removed. A node that
  was only ever a reference (`{"@id": …}`) is a value, and stays;
- **refused** (`InvalidArgument` on `context`): a context with `@vocab` anywhere,
  which would make every unmapped key look defined, and a scoped `@context` in a
  term definition, which makes "the terms this context defines" depend on
  position. An egress context names each term it allows, and needs neither;
- **`face=report`** is what was removed, as JSON: `dropped` (the total), the
  distinct `properties`, `reverseProperties`, `types` and `datatypes`, and
  `emptyNodes`. A caller that must never lose content silently refuses to
  publish when `dropped` is not 0.

An allowed document changes nothing compaction would not. Documents that retain
node content after compaction are fixed points: pruning the output returns it
byte for byte. ⚠ A context decides
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

JSON nesting alone does not bound context processing: even a flat context can
chain term definitions. Before processing a context, the module also refuses a
potential term-definition dependency depth past **64**
(`MAX_CONTEXT_DEPENDENCY_DEPTH`), naming `content` or `context` and asking for
direct IRI mappings. References through `@id`, `@type`, `@reverse`, prefixes,
and scoped contexts share this budget. The check is structural and conservative:
it counts dependencies even when definition order would avoid recursion, and
counts a cyclic group by its number of definitions. Independent terms do not
consume one another's budget. The graph check uses iterative traversal and
linear space and time; the child-process regressions are in
`tests/context_depth.rs`.

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

### Correctness audit (ledger #3481)

`cargo test --test w3c_subset` runs a finite, offline subset of the
[W3C JSON-LD API tests](https://github.com/w3c/json-ld-api/tree/55514738d854e56bb8bad4c53cda2f0e23be4a50/tests),
vendored at that revision in `tests/fixtures/w3c` with the upstream license.
`manifest.json` records the original test IDs, names, input/expected/context paths,
and effective base. Each case runs through our public endpoint and an independent
`json-ld` call, and compares both with the W3C expected result. Object key order
and arbitrary blank-node labels are ignored; expansion/flattening array order is
ignored except for `@list` and value payloads. Compaction arrays retain their order.

| Endpoint | Passed | Failed |
|---|---:|---:|
| expand | 14 | 0 |
| compact | 12 | 0 |
| flatten | 9 | 0 |

The subset covers null/unmapped values, empty sets/lists, keyword aliases,
datatype and IRI coercion, graphs, included nodes, nesting, scoped contexts,
and expansion base overrides. No upstream failures were found in this subset.
This is not full JSON-LD 1.1 conformance evidence. The wrapper exposes `base`,
uses the library's default JSON-LD 1.1 processing mode, and forces ordered
flattening; `expandContext`, `processingMode` and caller-selected `ordered`
are not endpoint arguments.

`tests/audit_boundaries.rs` additionally checks keyword aliases, a prefix
redefined after a term was defined, input property/type scopes, refused target
scopes, nested graphs and included nodes, protected contexts, and base-aware
compaction/expansion round trips. Absolute and relative embedded remote contexts
and imports are refused by `NoLoader`; the relative cases check the resolved
IRI in the error. There is no built-in static set of remote contexts. Only the
explicit target `context` resource is resolved through the kernel, as described
above. Existing semantic and determinism tests cover loader call counts, typed
values, JSON literals, and repeated computations on fresh kernels.

The reproduced wrapper defect was over-pruning: an allowed empty property or
empty graph/included container was removed and its node incorrectly counted as
empty. `tests/prune.rs` checks preservation, unchanged compaction results, and
zero drops, as well as continued removal of disallowed empty properties and
containers emptied by filtering. An originally empty reverse map also reports
zero drops and matches plain compaction; plain compaction can turn it into a
free-floating `@id` reference that disappears on a subsequent expansion.

## Status

0.4.0: move to ikigai-core 0.3.0, the scoped content store release (ledger #2965), ikigai-module 0.6.0, and ikigai-conformance 0.14.0. This is a breaking dependency update because core types appear in the public API.

0.3.0: move to ikigai-core 0.2.0, the streams release (ledger #2261), and ikigai-module 0.5.0. This is a breaking dependency update because core types appear in the public API. Conformance 0.11.0 runs DATATYPES and FAILURES by default, with no opt-outs.

0.2.2: `urn:jsonld:prune`, the trust-boundary egress filter: keep only what a context defines, refuse `@vocab`, report what was removed. 0.2.1 bounded caller JSON-LD in nesting (64) and processed it on a sized thread. Depends only on published crates. Dual-licensed MIT / Apache-2.0.

## Accessibility

What this crate emits that people read (JSON documents and error messages), the rules that
output follows, how it is tested and how to report a barrier are in
[ACCESSIBILITY.md](ACCESSIBILITY.md).
