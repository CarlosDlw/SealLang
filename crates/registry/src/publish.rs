//! Publication requests, deterministic archives, and the registry write API.
//!
//! The client builds a reproducible `.tar.zst`, signs a DSSE/SLSA statement over
//! its digest, and posts both to `seal-registry-server`. Every response carries a
//! structured error code so the CLI can report the exact rejection.

use std::collections::BTreeSet;
use std::path::{Component, Path};

use base64::Engine;
use seal_api::ApiSnapshot;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::RegistrySession;
use crate::client::HttpResponse;
use crate::error::{RegistryError, RegistryResult};
use crate::index::{LogInclusion, Publisher};
use crate::log::SignedTreeHead;
use crate::provenance::{Envelope, key_id};

/// One file included in a package archive.
#[derive(Clone, Debug)]
pub struct PackageFile {
    /// The path relative to the package root, with `/` separators.
    pub path: String,
    /// The exact file contents.
    pub contents: Vec<u8>,
    /// Whether the executable bit is set.
    pub executable: bool,
}

/// A publication request.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PublishRequest {
    /// The package name, including an optional `@scope/`.
    pub name: String,
    /// The semantic version.
    pub version: String,
    /// The base64-encoded `.tar.zst` archive.
    pub archive: String,
    /// The public-API snapshot.
    pub api: ApiSnapshot,
    /// The CycloneDX SBOM.
    pub sbom: Value,
    /// The signed provenance envelope.
    pub provenance: Envelope,
    /// The signing identity.
    pub publisher: Publisher,
}

/// A successful publication.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PublishResponse {
    /// The response schema version.
    pub format: u32,
    /// The package name.
    pub name: String,
    /// The published version.
    pub version: String,
    /// The archive digest.
    pub checksum: String,
    /// The API snapshot digest.
    pub api: String,
    /// The provenance digest.
    pub provenance: String,
    /// The SBOM digest.
    pub sbom: String,
    /// The transparency-log inclusion metadata.
    pub log: LogInclusion,
    /// The signed tree head covering this entry.
    pub tree_head: SignedTreeHead,
    /// Whether the version was already present and identical.
    #[serde(default)]
    pub already_published: bool,
}

/// A yank or undo request.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct YankRequest {
    /// The package name.
    pub name: String,
    /// The version.
    pub version: String,
    /// `true` yanks, `false` undoes a yank.
    pub yanked: bool,
}

/// The result of a yank or undo.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct YankResponse {
    /// The package name.
    pub name: String,
    /// The version.
    pub version: String,
    /// The resulting yanked state.
    pub yanked: bool,
    /// The transparency entry written for the change.
    pub log: LogInclusion,
}

/// An owner or namespace request.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OwnerRequest {
    /// The namespace.
    pub namespace: String,
    /// `add`, `remove`, or `list`.
    pub action: String,
    /// The other party, for `add` and `remove`.
    #[serde(default)]
    pub owner: Option<String>,
    /// The namespace claim proof, for `claim`.
    #[serde(default)]
    pub proof: Option<NamespaceProof>,
}

/// A namespace ownership proof.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NamespaceProof {
    /// `dns`, `oidc`, or `org`.
    pub kind: String,
    /// A DNS TXT record value or organization name.
    #[serde(default)]
    pub value: Option<String>,
}

/// The result of an owner or namespace operation.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OwnerResponse {
    /// The namespace.
    pub namespace: String,
    /// The current owners.
    pub owners: Vec<String>,
    /// The verification kind.
    pub verified: String,
}

/// The authenticated identity.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WhoAmI {
    /// The token or identity name.
    pub name: String,
    /// The granted scopes.
    pub scopes: Vec<String>,
    /// The owned namespaces.
    pub namespaces: Vec<String>,
    /// The OIDC subject, when trusted publishing authenticated the caller.
    #[serde(default)]
    pub subject: Option<String>,
}

/// Builds a deterministic `.tar.zst` archive from package files.
///
/// Paths must be relative, normalized, and unique. Every header uses the same
/// timestamp, owner, and permissions, so two publishes of unchanged sources
/// produce identical bytes.
pub fn build_archive(files: &[PackageFile]) -> RegistryResult<Vec<u8>> {
    let mut sorted: Vec<&PackageFile> = files.iter().collect();
    sorted.sort_by(|left, right| left.path.cmp(&right.path));
    let mut seen = BTreeSet::new();
    let mut total = 0u64;
    for file in &sorted {
        validate_relative(&file.path)?;
        if !seen.insert(file.path.as_str()) {
            return Err(RegistryError::publish(format!(
                "the package contains `{}` twice",
                file.path
            )));
        }
        total = total.saturating_add(file.contents.len() as u64);
    }
    if sorted.is_empty() {
        return Err(RegistryError::publish("the package has no files"));
    }
    if total > 1024 * 1024 * 1024 {
        return Err(RegistryError::publish(
            "the package sources exceed the 1 GiB archive limit",
        ));
    }
    let encoder = zstd::Encoder::new(Vec::new(), 3)
        .map_err(|error| RegistryError::publish(format!("cannot start zstd: {error}")))?;
    let mut builder = tar::Builder::new(encoder);
    for file in sorted {
        let mut header = tar::Header::new_gnu();
        header.set_path(&file.path).map_err(|error| {
            RegistryError::publish(format!("invalid archive path `{}`: {error}", file.path))
        })?;
        header.set_entry_type(tar::EntryType::Regular);
        header.set_size(file.contents.len() as u64);
        header.set_mode(if file.executable { 0o755 } else { 0o644 });
        header.set_mtime(0);
        header.set_uid(0);
        header.set_gid(0);
        header
            .set_username("")
            .and_then(|()| header.set_groupname(""))
            .map_err(|error| {
                RegistryError::publish(format!("cannot build the archive header: {error}"))
            })?;
        header.set_cksum();
        builder
            .append(&header, file.contents.as_slice())
            .map_err(|error| {
                RegistryError::publish(format!(
                    "cannot append `{}` to the archive: {error}",
                    file.path
                ))
            })?;
    }
    let encoder = builder
        .into_inner()
        .map_err(|error| RegistryError::publish(format!("cannot finish the archive: {error}")))?;
    encoder
        .finish()
        .map_err(|error| RegistryError::publish(format!("cannot compress the archive: {error}")))
}

fn validate_relative(path: &str) -> RegistryResult<()> {
    if path.is_empty() || path.ends_with('/') {
        return Err(RegistryError::publish(
            "an archive path is empty or names a directory",
        ));
    }
    if path.len() > 1024 {
        return Err(RegistryError::publish("an archive path is too long"));
    }
    if path.contains('\\')
        || path
            .split('/')
            .any(|segment| segment.is_empty() || segment == "." || segment == "..")
    {
        return Err(RegistryError::publish(format!(
            "the archive path `{path}` is not normalized"
        )));
    }
    let path = Path::new(path);
    if path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(RegistryError::publish(format!(
            "the archive path `{}` is not a normalized relative path",
            path.display()
        )));
    }
    Ok(())
}

/// Encodes an archive for the JSON request.
pub fn encode_archive(archive: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(archive)
}

/// Decodes and bounds an archive from a request.
pub fn decode_archive(encoded: &str, max_bytes: usize) -> RegistryResult<Vec<u8>> {
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|error| RegistryError::publish(format!("the archive is not base64: {error}")))?;
    if decoded.is_empty() {
        return Err(RegistryError::publish("the archive is empty"));
    }
    if decoded.len() > max_bytes {
        return Err(RegistryError::publish(format!(
            "the archive is {} bytes, above the {max_bytes} byte limit",
            decoded.len()
        )));
    }
    Ok(decoded)
}

/// Returns the `sha256:<hex>` digest of a JSON document.
///
/// `serde_json` maps are ordered, so the encoding is deterministic for the same
/// logical document regardless of how the caller built it.
pub fn document_digest(value: &Value) -> RegistryResult<String> {
    let bytes = serde_json::to_vec(value)
        .map_err(|error| RegistryError::publish(format!("cannot encode the document: {error}")))?;
    let mut hasher = sha2::Sha256::new();
    use sha2::Digest;
    hasher.update(bytes);
    Ok(format!("sha256:{}", hex::encode(hasher.finalize())))
}

/// Builds a publisher identity from a seed.
pub fn publisher(seed: &[u8; 32], subject: Option<&str>) -> Publisher {
    let public = crate::provenance::public_key(seed);
    let keyid = key_id(&ed25519_dalek::SigningKey::from_bytes(seed).verifying_key());
    Publisher {
        keyid,
        public,
        identity: subject.map(ToOwned::to_owned),
    }
}

/// Posts a publication request and parses the response.
pub fn publish(
    session: &RegistrySession,
    request: &PublishRequest,
) -> RegistryResult<PublishResponse> {
    let body = serde_json::to_vec(request).map_err(|error| {
        RegistryError::publish(format!("cannot encode the publication: {error}"))
    })?;
    let response = session.post("api/v1/publish", &body)?;
    parse_response(response)
}

/// Posts a yank or undo request.
pub fn yank(session: &RegistrySession, request: &YankRequest) -> RegistryResult<YankResponse> {
    let body = serde_json::to_vec(request)
        .map_err(|error| RegistryError::publish(format!("cannot encode the request: {error}")))?;
    let response = session.post("api/v1/yank", &body)?;
    parse_response(response)
}

/// Posts an owner or namespace operation.
pub fn owners(session: &RegistrySession, request: &OwnerRequest) -> RegistryResult<OwnerResponse> {
    let body = serde_json::to_vec(request)
        .map_err(|error| RegistryError::publish(format!("cannot encode the request: {error}")))?;
    let response = session.post("api/v1/owners", &body)?;
    parse_response(response)
}

/// Claims a namespace.
pub fn claim(session: &RegistrySession, request: &OwnerRequest) -> RegistryResult<OwnerResponse> {
    let body = serde_json::to_vec(request)
        .map_err(|error| RegistryError::publish(format!("cannot encode the request: {error}")))?;
    let response = session.post("api/v1/namespaces", &body)?;
    parse_response(response)
}

/// Returns the authenticated identity.
pub fn whoami(session: &RegistrySession) -> RegistryResult<WhoAmI> {
    let response = session.post("api/v1/whoami", b"{}")?;
    parse_response(response)
}

fn parse_response<T>(response: HttpResponse) -> RegistryResult<T>
where
    T: serde::de::DeserializeOwned,
{
    if response.status != 200 {
        return Err(error_from(response.status, &response.body));
    }
    serde_json::from_slice(&response.body).map_err(|error| {
        RegistryError::server(format!(
            "the registry returned an invalid response: {error}"
        ))
    })
}

fn error_from(status: u16, body: &[u8]) -> RegistryError {
    #[derive(Deserialize)]
    struct Body {
        error: ErrorBody,
    }
    #[derive(Deserialize)]
    struct ErrorBody {
        code: String,
        message: String,
    }
    let (code, message) = serde_json::from_slice::<Body>(body)
        .map(|body| (body.error.code, body.error.message))
        .unwrap_or_else(|_| {
            (
                "E8099".to_owned(),
                format!("the registry returned HTTP {status}"),
            )
        });
    let static_code = match code.as_str() {
        "E8020" => "E8020",
        "E8021" => "E8021",
        "E8025" => "E8025",
        "E8027" => "E8027",
        "E8037" => "E8037",
        "E8080" => "E8080",
        "E8082" => "E8082",
        "E8083" => "E8083",
        "E8084" => "E8084",
        "E8085" => "E8085",
        "E8093" => "E8093",
        "E8096" => "E8096",
        "E8097" => "E8097",
        "E8098" => "E8098",
        _ => "E8099",
    };
    RegistryError::new(static_code, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(path: &str, contents: &[u8]) -> PackageFile {
        PackageFile {
            path: path.to_owned(),
            contents: contents.to_vec(),
            executable: false,
        }
    }

    #[test]
    fn archives_are_deterministic_and_sorted() {
        let first = build_archive(&[file("b.sl", b"b"), file("a.sl", b"a")]).unwrap();
        let second = build_archive(&[file("a.sl", b"a"), file("b.sl", b"b")]).unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn escaping_paths_are_rejected() {
        for path in ["../escape", "/absolute", "a/../b", "a//b", ""] {
            assert!(build_archive(&[file(path, b"x")]).is_err(), "{path}");
        }
    }

    #[test]
    fn duplicate_paths_are_rejected() {
        let error = build_archive(&[file("a.sl", b"a"), file("a.sl", b"b")]).unwrap_err();
        assert_eq!(error.code(), "E8093");
    }

    #[test]
    fn archives_round_trip_through_base64() {
        let archive = build_archive(&[file("a.sl", b"a")]).unwrap();
        assert_eq!(
            decode_archive(&encode_archive(&archive), 1024).unwrap(),
            archive
        );
        assert!(decode_archive(&encode_archive(&archive), 1).is_err());
    }
}
