//! `urn:jsonld:prune`: the trust-boundary egress filter (ledger #1180).
//!
//! Compaction alone filters nothing: a predicate the context does not map comes out as a full
//! IRI, one under a prefix the context declares comes out as a compact IRI (`collab:hidden`), and
//! an unmapped `@type` keeps its IRI. Prune is expand, remove everything the context does not
//! DEFINE, compact:
//!
//! 1. **The context is refused** if it has `@vocab` anywhere (a vocabulary mapping turns every
//!    unmapped key into a defined-looking term, so the allowlist would be the whole world), or a
//!    scoped `@context` inside a term definition (a scoped context changes what a term means by
//!    position, so "the terms this context defines" stops being one set; see below).
//! 2. **The allowlist** is read from the PROCESSED context, so compact IRIs in term definitions
//!    are resolved the way JSON-LD resolves them:
//!    - property and `@type` IRIs: the IRI mapping of every term that is not a prefix and not a
//!      reverse property. ⚠ A prefix is not a term: `"collab": "https://…/collab#"` lets the
//!      context's own definitions say `collab:brief`, and it must not let the document say
//!      `collab:anything`. "Prefix" is JSON-LD 1.1's prefix flag: a simple definition whose IRI
//!      ends in a gen-delim (`/`, `#`, `:` …), or an expanded one with `"@prefix": true`.
//!    - `@reverse` IRIs: the IRI mapping of every reverse-property term.
//!    - value datatypes: the property/type set, plus every datatype a term definition coerces
//!      to (`"@type": "xsd:dateTime"`), so a document the context describes keeps its typed
//!      literals. `@json` literals are kept.
//! 3. **The prune**, over the expanded document: a property, a reverse property or a `@type` value
//!    outside the allowlist is removed; a value whose datatype is outside it is removed (the value,
//!    not just its datatype, which would silently change what the value means); a node object
//!    that HAD content (types, properties, reverse properties, a graph, included nodes) and has
//!    none left is removed, wherever it sits. A node that was only ever a reference
//!    (`{"@id": …}`) is a value and stays.
//! 4. **Compact** the remainder with the same context, so the output speaks only the context's
//!    terms.
//!
//! The `report` face is what was removed, as JSON (`dropped` is the total; the lists are distinct
//! IRIs, sorted), so a caller can refuse to publish when anything was. Both faces are a pure
//! function of `content`, `context` and `base`: cacheable.
//!
//! ⚠ **What a context cannot do**: it decides which PROPERTIES cross, never what a crossing
//! value says, and an `@id` is a value. Value redaction is a CONSTRUCT's job upstream.
//!
//! ⚠ **Scoped contexts are refused, not supported.** Honoring them means tracking the active
//! context through the walk exactly as compaction does (property-scoped contexts propagate,
//! type-scoped ones revert), and a filter that approximates that either leaks (a term allowed
//! where its scope does not reach compacts to a full IRI) or over-drops. An egress context is
//! written for the purpose, and needs none.

use crate::{parse_context, parse_doc};
use futures::future::FutureExt;
use ikigai_core::{Error, ReprType, Representation, Result};
use json_ld::context_processing::{Options, Process};
use json_ld::syntax::{Object as JsonObject, Print, Value as Json};
use json_ld::{
    Id, IriBuf, JsonLdProcessor, NoLoader, RemoteContextReference, RemoteDocument, Term, Type,
    ValidId,
};
use std::collections::BTreeSet;

/// Which face of the prune a caller asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Face {
    /// The pruned, compacted document (`application/ld+json`). The default.
    Document,
    /// What was removed (`application/json`).
    Report,
}

impl Face {
    pub(crate) const NAMES: [&'static str; 2] = ["document", "report"];

    pub(crate) fn parse(value: Option<&str>) -> Result<Self> {
        match value.map(str::trim) {
            None | Some("document") => Ok(Face::Document),
            Some("report") => Ok(Face::Report),
            Some(other) => Err(Error::InvalidArgument {
                name: "face".into(),
                detail: format!(
                    "`{other}` is not a face of urn:jsonld:prune: use `document` or `report`"
                ),
            }),
        }
    }
}

/// Refuse a context the allowlist cannot be read from safely: any `@vocab`, or any scoped
/// `@context` (see the module notes). `context` is the context VALUE (a context document's
/// `@context` already unwrapped), so any `@context` key inside it is a scoped one.
pub(crate) fn refuse_unsafe_context(context: &Json) -> Result<()> {
    fn walk(value: &Json) -> Result<()> {
        match value {
            Json::Array(items) => items.iter().try_for_each(walk),
            Json::Object(object) => {
                for entry in object.iter() {
                    match entry.key.as_str() {
                        "@vocab" => {
                            return Err(refusal(
                                "has `@vocab`, which makes every unmapped key look like a term the \
                                 context defines; an egress context must name each term it allows",
                            ))
                        }
                        "@context" => {
                            return Err(refusal(
                                "has a scoped `@context` in a term definition; urn:jsonld:prune \
                                 reads one set of defined terms and does not support scoped contexts",
                            ))
                        }
                        _ => walk(&entry.value)?,
                    }
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }
    walk(context)
}

fn refusal(why: &str) -> Error {
    Error::InvalidArgument {
        name: "context".into(),
        detail: format!("the context {why}"),
    }
}

/// What a context defines, as expanded IRIs.
#[derive(Debug, Default)]
pub(crate) struct Allowlist {
    /// Property and `@type` IRIs: every term that is neither a prefix nor a reverse property.
    terms: BTreeSet<String>,
    /// `@reverse` IRIs: every reverse-property term.
    reverse: BTreeSet<String>,
    /// Value datatypes: `terms`, plus every datatype a term definition coerces to.
    datatypes: BTreeSet<String>,
}

impl Allowlist {
    /// Process `context` (no loader: a remote context reference is an error) and read its terms.
    fn of(context: &json_ld::syntax::context::Context) -> Result<Self> {
        let active = json_ld::Context::<IriBuf>::default();
        let processed = context
            .process_full(&mut (), &active, &NoLoader, None, Options::default(), ())
            .now_or_never()
            .ok_or_else(|| {
                Error::Endpoint("JSON-LD context processing did not complete synchronously".into())
            })?
            .map_err(|e| Error::InvalidArgument {
                name: "context".into(),
                detail: format!("invalid JSON-LD context: {e}"),
            })?
            .into_processed();
        let mut allow = Allowlist::default();
        for binding in processed.definitions().iter() {
            let definition = binding.definition();
            if let Some(Type::Iri(datatype)) = definition.typ() {
                allow.datatypes.insert(datatype.as_str().to_string());
            }
            let Some(Term::Id(Id::Valid(ValidId::Iri(iri)))) = definition.value() else {
                continue; // a keyword alias, a null (`"x": null`) or a blank node: nothing to allow
            };
            if definition.prefix() {
                continue;
            }
            let iri = iri.as_str().to_string();
            if definition.reverse_property() {
                allow.reverse.insert(iri);
            } else {
                allow.datatypes.insert(iri.clone());
                allow.terms.insert(iri);
            }
        }
        Ok(allow)
    }
}

/// What the prune removed: the `report` face.
#[derive(Debug, Default)]
pub(crate) struct Dropped {
    count: usize,
    properties: BTreeSet<String>,
    reverse: BTreeSet<String>,
    types: BTreeSet<String>,
    datatypes: BTreeSet<String>,
    empty_nodes: usize,
}

impl Dropped {
    fn report(&self) -> String {
        let list = |set: &BTreeSet<String>| {
            Json::Array(
                set.iter()
                    .map(|s| Json::String(s.as_str().into()))
                    .collect(),
            )
        };
        let count = |n: usize| Json::Number(n.into());
        let mut object = JsonObject::new();
        object.push("dropped".into(), count(self.count));
        object.push("properties".into(), list(&self.properties));
        object.push("reverseProperties".into(), list(&self.reverse));
        object.push("types".into(), list(&self.types));
        object.push("datatypes".into(), list(&self.datatypes));
        object.push("emptyNodes".into(), count(self.empty_nodes));
        Json::Object(object).pretty_print().to_string()
    }
}

/// The keys of an expanded node object that carry content, as opposed to naming it (`@id`,
/// `@index`). A node that had any of these and loses them all is "left empty".
fn carries_content(key: &str) -> bool {
    !matches!(key, "@id" | "@index")
}

/// Prune one expanded node object; `None` when it was left empty.
fn prune_node(node: JsonObject, allow: &Allowlist, dropped: &mut Dropped) -> Option<JsonObject> {
    let had_content = node.iter().any(|e| carries_content(e.key.as_str()));
    let mut kept = JsonObject::new();
    for entry in node {
        let key = entry.key.as_str().to_string();
        match key.as_str() {
            "@id" | "@index" => {
                kept.push(entry.key, entry.value);
            }
            "@type" => {
                let types: Vec<Json> = as_array(entry.value)
                    .into_iter()
                    .filter(|t| match t.as_str() {
                        Some(iri) if allow.terms.contains(iri) => true,
                        other => {
                            dropped.count += 1;
                            dropped.types.insert(other.unwrap_or_default().to_string());
                            false
                        }
                    })
                    .collect();
                if !types.is_empty() {
                    kept.push(entry.key, Json::Array(types));
                }
            }
            "@reverse" => {
                let Json::Object(reverse) = entry.value else {
                    continue;
                };
                let mut kept_reverse = JsonObject::new();
                for property in reverse {
                    if !allow.reverse.contains(property.key.as_str()) {
                        dropped.count += 1;
                        dropped.reverse.insert(property.key.as_str().to_string());
                        continue;
                    }
                    let values = prune_values(property.value, allow, dropped);
                    if !values.is_empty() {
                        kept_reverse.push(property.key, Json::Array(values));
                    }
                }
                if !kept_reverse.is_empty() {
                    kept.push(entry.key, Json::Object(kept_reverse));
                }
            }
            "@graph" | "@included" => {
                let nodes = prune_values(entry.value, allow, dropped);
                if !nodes.is_empty() {
                    kept.push(entry.key, Json::Array(nodes));
                }
            }
            property if !property.starts_with('@') && allow.terms.contains(property) => {
                let values = prune_values(entry.value, allow, dropped);
                if !values.is_empty() {
                    kept.push(entry.key, Json::Array(values));
                }
            }
            // Outside the allowlist, and any keyword an expanded node object should not carry:
            // fail closed.
            other => {
                dropped.count += 1;
                dropped.properties.insert(other.to_string());
            }
        }
    }
    let has_content = kept.iter().any(|e| carries_content(e.key.as_str()));
    if had_content && !has_content {
        dropped.count += 1;
        dropped.empty_nodes += 1;
        return None;
    }
    Some(kept)
}

/// Prune an expanded array of values: value objects, list objects and node objects.
fn prune_values(values: Json, allow: &Allowlist, dropped: &mut Dropped) -> Vec<Json> {
    as_array(values)
        .into_iter()
        .filter_map(|value| prune_value(value, allow, dropped))
        .collect()
}

fn prune_value(value: Json, allow: &Allowlist, dropped: &mut Dropped) -> Option<Json> {
    let Json::Object(object) = value else {
        // Expanded form holds only objects here; anything else is passed through unchanged.
        return Some(value);
    };
    if object.contains_key("@value") {
        let datatype = object
            .get("@type")
            .next()
            .and_then(Json::as_str)
            .map(str::to_string);
        return match datatype {
            Some(datatype) if datatype != "@json" && !allow.datatypes.contains(&datatype) => {
                dropped.count += 1;
                dropped.datatypes.insert(datatype);
                None
            }
            _ => Some(Json::Object(object)),
        };
    }
    if object.contains_key("@list") {
        let mut kept = JsonObject::new();
        for entry in object {
            if entry.key.as_str() == "@list" {
                kept.push(
                    entry.key,
                    Json::Array(prune_values(entry.value, allow, dropped)),
                );
            } else {
                kept.push(entry.key, entry.value);
            }
        }
        return Some(Json::Object(kept));
    }
    prune_node(object, allow, dropped).map(Json::Object)
}

fn as_array(value: Json) -> Vec<Json> {
    match value {
        Json::Array(items) => items,
        other => vec![other],
    }
}

/// Prune `content` against `context_bytes`, both bounded in nesting already; runs on the sized
/// thread. Returns the face asked for.
pub(crate) fn prune_doc(
    content: &[u8],
    context_bytes: &[u8],
    base: Option<&str>,
    face: Face,
) -> Result<Representation> {
    let (context_value, context) = parse_context(context_bytes)?;
    refuse_unsafe_context(&context_value)?;
    let allow = Allowlist::of(&context)?;

    let doc = parse_doc(content, base)?;
    let expanded = doc
        .expand(&NoLoader)
        .now_or_never()
        .ok_or_else(|| Error::Endpoint("JSON-LD expand did not complete synchronously".into()))?
        .map_err(|e| Error::Endpoint(format!("JSON-LD expand failed: {e}")))?;
    // Expanded form, as JSON: printed and parsed back, since the prune is a walk over keys.
    let printed = contextual::WithContext::with(&expanded, &())
        .compact_print()
        .to_string();
    let (expanded, _) = <Json as json_ld::syntax::Parse>::parse_str(&printed)
        .map_err(|e| Error::Endpoint(format!("reparsing the expanded document failed: {e}")))?;

    let mut dropped = Dropped::default();
    let pruned = Json::Array(prune_values(expanded, &allow, &mut dropped));

    if face == Face::Report {
        return Ok(Representation::new(
            ReprType::new("application/json").with_param("charset", "utf-8"),
            dropped.report().into_bytes(),
        )
        .cacheable());
    }

    let base_iri = base.and_then(|b| IriBuf::new(b.to_string()).ok());
    let remote = RemoteDocument::new(base_iri, None, pruned);
    let compacted = remote
        .compact(
            RemoteContextReference::Loaded(RemoteDocument::new(None, None, context)),
            &NoLoader,
        )
        .now_or_never()
        .ok_or_else(|| Error::Endpoint("JSON-LD compact did not complete synchronously".into()))?
        .map_err(|e| Error::Endpoint(format!("JSON-LD compact failed: {e}")))?;
    Ok(crate::repr(compacted.pretty_print().to_string()))
}
