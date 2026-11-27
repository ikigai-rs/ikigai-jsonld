use json_ld::syntax::{Parse, Value};
use json_ld::{Iri, IriBuf, LoadError, Loader, RemoteDocument};
use std::io;

fn bundled_context(url: &str) -> Option<&'static str> {
    match url {
        "https://www.w3.org/ns/credentials/v2" => {
            Some(include_str!("../contexts/credentials-v2.jsonld"))
        }
        "https://purl.imsglobal.org/spec/ob/v3p0/context-3.0.3.json" => {
            Some(include_str!("../contexts/open-badges-3.0.3.jsonld"))
        }
        "https://w3id.org/security/multikey/v1" => {
            Some(include_str!("../contexts/multikey-v1.jsonld"))
        }
        _ => None,
    }
}

/// An offline JSON-LD loader for the exact URLs in `contexts/manifest.json`.
///
/// Pass it to the `json_ld::JsonLdProcessor` methods for expansion or compaction.
/// It performs no I/O and never falls back to another loader. It checks the
/// bundled contexts against this crate's depth limits before returning them.
/// Callers using the processor directly are responsible for bounding their own
/// input documents and providing adequate stack space (see [`crate::JSON_LD_STACK`]).
/// The `urn:jsonld:*` endpoints do not use this loader.
#[derive(Clone, Copy, Debug, Default)]
pub struct PinnedContextLoader;

impl Loader for PinnedContextLoader {
    async fn load(&self, url: &Iri) -> Result<RemoteDocument<IriBuf>, LoadError> {
        let Some(text) = bundled_context(url.as_str()) else {
            return Err(LoadError::new(
                url.to_owned(),
                io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("The context URL {url} is not pinned. Use a URL listed in the pinned context manifest."),
                ),
            ));
        };
        let invalid = |detail: String| {
            LoadError::new(
                url.to_owned(),
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("The pinned context at {url} could not be read. Use a build with a valid pin. {detail}"),
                ),
            )
        };
        crate::check_json_nesting(text.as_bytes(), "context")
            .map_err(|e| invalid(e.to_string()))?;
        let (value, _) = Value::parse_str(text).map_err(|e| invalid(e.to_string()))?;
        crate::context_depth::check(&value, "context", false)
            .map_err(|e| invalid(e.to_string()))?;
        Ok(RemoteDocument::new(Some(url.to_owned()), None, value))
    }
}

#[cfg(test)]
mod tests {
    use super::bundled_context;
    use serde_json::Value;
    use sha2::{Digest, Sha256};
    use std::collections::BTreeSet;
    use std::{fs, path::Path};

    #[test]
    fn bundled_bytes_match_the_manifest() {
        let manifest: Vec<Value> =
            serde_json::from_str(include_str!("../contexts/manifest.json")).unwrap();
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("contexts");
        let mut files = BTreeSet::new();
        let mut urls = BTreeSet::new();
        for entry in manifest {
            let file = entry["file"].as_str().unwrap();
            let url = entry["url"].as_str().unwrap();
            assert!(files.insert(file.to_owned()), "Duplicate file {file}");
            assert!(urls.insert(url.to_owned()), "Duplicate URL {url}");
            let bytes = fs::read(dir.join(file)).unwrap();
            assert_eq!(bundled_context(url).unwrap().as_bytes(), bytes, "{url}");
            assert_eq!(
                format!("{:x}", Sha256::digest(&bytes)),
                entry["sha256"].as_str().unwrap(),
                "{file}"
            );
        }
        let context_files = fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .filter(|name| name.ends_with(".jsonld"))
            .collect::<BTreeSet<_>>();
        assert_eq!(files, context_files);
    }
}
