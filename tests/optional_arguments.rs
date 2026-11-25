use std::sync::Arc;

use futures::executor::block_on;
use ikigai_core::{
    ArgRef, Capability, ContentId, Error, Iri, Kernel, Representation, Request, Verb,
};

fn run(operation: &str, arg: Option<(&str, ArgRef)>) -> Result<Representation, Error> {
    let content = if matches!(operation, "compact" | "prune") {
        r#"{"@id":"https://example.org/item","https://example.org/name":"Ada"}"#
    } else {
        r#"{"@id":"item","https://example.org/name":"Ada"}"#
    };
    let mut request = Request::new(
        Verb::Source,
        Iri::parse(format!("urn:jsonld:{operation}")).unwrap(),
    )
    .with_arg("content", inline(content));
    if matches!(operation, "compact" | "prune") {
        request = request.with_arg("context", inline(r#"{"name":"https://example.org/name"}"#));
    }
    if let Some((name, value)) = arg {
        request = request.with_arg(name, value);
    }
    let kernel = Kernel::new(Arc::new(ikigai_jsonld::space()));
    block_on(kernel.issue(request, &Capability::root()))
}

fn inline(value: &str) -> ArgRef {
    ArgRef::Inline(value.as_bytes().to_vec())
}

fn assert_invalid(operation: &str, argument: &str, value: ArgRef, explanation: &str) {
    match run(operation, Some((argument, value))) {
        Err(Error::InvalidArgument { name, detail }) => {
            assert_eq!(name, argument, "{operation}");
            assert!(detail.contains(explanation), "{operation}: {detail}");
        }
        result => panic!("{operation}: expected a refusal naming {argument}, got {result:?}"),
    }
}

fn check_base(operation: &str) {
    let absent = run(operation, None).unwrap();
    let valid = run(operation, Some(("base", inline("https://example.org/")))).unwrap();
    assert_ne!(absent.bytes, valid.bytes, "{operation} must use the base");
    let reference = ArgRef::Reference(Iri::parse("https://example.org/").unwrap());
    assert_eq!(
        run(operation, Some(("base", reference))).unwrap().bytes,
        valid.bytes
    );
}

macro_rules! base_tests {
    ($valid:ident, $bad_utf8:ident, $body:ident, $bad_iri:ident, $operation:literal) => {
        #[test]
        fn $valid() {
            check_base($operation);
        }

        #[test]
        fn $bad_utf8() {
            assert_invalid($operation, "base", ArgRef::Inline(vec![0xff]), "UTF-8");
        }

        #[test]
        fn $body() {
            assert_invalid(
                $operation,
                "base",
                ArgRef::Content(ContentId::of(b"body")),
                "body",
            );
        }

        #[test]
        fn $bad_iri() {
            for invalid in ["", "relative/path"] {
                assert_invalid($operation, "base", inline(invalid), "absolute IRI");
            }
        }
    };
}

base_tests!(
    expand_base,
    expand_base_utf8,
    expand_base_body,
    expand_base_iri,
    "expand"
);
base_tests!(
    flatten_base,
    flatten_base_utf8,
    flatten_base_body,
    flatten_base_iri,
    "flatten"
);
base_tests!(
    compact_base,
    compact_base_utf8,
    compact_base_body,
    compact_base_iri,
    "compact"
);
base_tests!(
    prune_base,
    prune_base_utf8,
    prune_base_body,
    prune_base_iri,
    "prune"
);

#[test]
fn prune_face() {
    let absent = run("prune", None).unwrap();
    let document = run("prune", Some(("face", inline("document")))).unwrap();
    let report = run("prune", Some(("face", inline("report")))).unwrap();
    assert_eq!(absent.bytes, document.bytes);
    assert_eq!(document.repr_type.media_type, "application/ld+json");
    assert_eq!(report.repr_type.media_type, "application/json");
}

#[test]
fn prune_face_utf8() {
    assert_invalid("prune", "face", ArgRef::Inline(vec![0xff]), "UTF-8");
}

#[test]
fn prune_face_body() {
    assert_invalid(
        "prune",
        "face",
        ArgRef::Content(ContentId::of(b"body")),
        "body",
    );
}

#[test]
fn prune_face_value() {
    for invalid in ["", "unknown"] {
        assert_invalid(
            "prune",
            "face",
            inline(invalid),
            "use `document` or `report`",
        );
    }
}
