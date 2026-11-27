use contextual::WithContext;
use futures::FutureExt;
use ikigai_jsonld::PinnedContextLoader;
use json_ld::syntax::{Parse, Print, Value};
use json_ld::{IriBuf, JsonLdProcessor, Loader, RemoteContextReference, RemoteDocument};

const VC: &str = "https://www.w3.org/ns/credentials/v2";
const OB: &str = "https://purl.imsglobal.org/spec/ob/v3p0/context-3.0.3.json";
const MULTIKEY: &str = "https://w3id.org/security/multikey/v1";

fn document(json: &str) -> RemoteDocument {
    RemoteDocument::new(None, None, Value::parse_str(json).unwrap().0)
}

#[test]
fn each_pinned_url_expands_its_terms() {
    for (url, term, expected_type) in [
        (
            VC,
            "VerifiableCredential",
            "https://www.w3.org/2018/credentials#VerifiableCredential",
        ),
        (
            OB,
            "OpenBadgeCredential",
            "https://purl.imsglobal.org/spec/vc/ob/vocab.html#OpenBadgeCredential",
        ),
        (MULTIKEY, "Multikey", "https://w3id.org/security#Multikey"),
    ] {
        let doc = document(&format!(r#"{{"@context":"{url}","type":"{term}"}}"#));
        let expanded = doc
            .expand(&PinnedContextLoader)
            .now_or_never()
            .unwrap()
            .unwrap();
        let json: serde_json::Value =
            serde_json::from_str(&expanded.with(&()).pretty_print().to_string()).unwrap();
        assert_eq!(json[0]["@type"][0], expected_type, "{url}");
    }
}

#[test]
fn badge_contexts_and_data_integrity_terms_work_together() {
    let doc = document(&format!(
        r#"{{
            "@context":["{VC}","{OB}"],
            "type":["VerifiableCredential","OpenBadgeCredential"],
            "proof":{{"type":"DataIntegrityProof","cryptosuite":"eddsa-rdfc-2022"}}
        }}"#,
    ));
    let expanded = doc
        .expand(&PinnedContextLoader)
        .now_or_never()
        .unwrap()
        .unwrap();
    let json: serde_json::Value =
        serde_json::from_str(&expanded.with(&()).pretty_print().to_string()).unwrap();
    assert_eq!(
        json[0]["https://w3id.org/security#proof"][0]["@graph"][0]["@type"][0],
        "https://w3id.org/security#DataIntegrityProof"
    );
}

#[test]
fn compaction_loads_a_pinned_target_context() {
    let doc =
        document(r#"[{"@id":"urn:example:key","@type":["https://w3id.org/security#Multikey"]}]"#);
    let context = RemoteContextReference::Iri(IriBuf::new(MULTIKEY.to_owned()).unwrap());
    let compacted = doc
        .compact(context, &PinnedContextLoader)
        .now_or_never()
        .unwrap()
        .unwrap();
    let json: serde_json::Value =
        serde_json::from_str(&compacted.pretty_print().to_string()).unwrap();
    assert_eq!(
        json["@context"]["Multikey"]["@id"],
        "https://w3id.org/security#Multikey"
    );
    assert_eq!(json["id"], "urn:example:key");
    assert_eq!(json["type"], "Multikey");
}

#[test]
fn refuses_unlisted_urls_including_near_matches_and_redirect_targets() {
    for url in [
        "https://example.org/context",
        "http://www.w3.org/ns/credentials/v2",
        "https://www.w3.org/ns/credentials/v2/",
        "https://www.w3.org/ns/credentials/v2?version=2",
        "https://www.w3.org/ns/credentials/v2#context",
        "https://www.w3.org/2025/credentials/vcdi/multikey/context/v1.jsonld",
    ] {
        let iri = IriBuf::new(url.to_owned()).unwrap();
        let error = PinnedContextLoader
            .load(&iri)
            .now_or_never()
            .unwrap()
            .unwrap_err();
        assert_eq!(error.target, iri);
        assert_eq!(
            error.cause.to_string(),
            format!("The context URL {url} is not pinned. Use a URL listed in the pinned context manifest.")
        );

        let doc = document(&format!(r#"{{"@context":"{url}","type":"Multikey"}}"#));
        let error = doc
            .expand(&PinnedContextLoader)
            .now_or_never()
            .unwrap()
            .unwrap_err();
        assert!(error.to_string().contains(url), "{error}");
    }
}
