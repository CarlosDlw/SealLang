//! DSSE/in-toto/SLSA provenance envelopes.
//!
//! A publication signs a statement that binds the package name, version, archive
//! digest, SBOM digest, builder identity, and build parameters. The registry
//! verifies the signature before storing the archive; the client verifies the
//! same envelope during audit.

use std::collections::BTreeMap;

use base64::Engine;
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::error::{RegistryError, RegistryResult};
use crate::tuf::canonical_json;

/// The DSSE payload type used by Seal publications.
pub const PAYLOAD_TYPE: &str = "application/vnd.in-toto+json";
/// The in-toto statement type.
pub const STATEMENT_TYPE: &str = "https://in-toto.io/Statement/v1";
/// The SLSA provenance predicate type.
pub const PREDICATE_TYPE: &str = "https://slsa.dev/provenance/v1";
/// The Seal build type.
pub const BUILD_TYPE: &str = "https://seal.dev/publish/v1";

/// One DSSE signature.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DsseSignature {
    /// The signing key id.
    pub keyid: String,
    /// The Ed25519 signature as lowercase hexadecimal.
    pub sig: String,
}

/// A DSSE envelope carrying a base64-encoded in-toto statement.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Envelope {
    /// The payload media type.
    #[serde(rename = "payloadType")]
    pub payload_type: String,
    /// The base64-encoded payload.
    pub payload: String,
    /// Detached signatures.
    #[serde(default)]
    pub signatures: Vec<DsseSignature>,
}

/// One in-toto subject.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Subject {
    /// The subject file name.
    pub name: String,
    /// Digests by algorithm; `sha256` is required.
    pub digest: BTreeMap<String, String>,
}

/// The SLSA build definition.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildDefinition {
    /// The build type URI.
    pub build_type: String,
    /// Parameters supplied by the publisher.
    pub external_parameters: BuildParameters,
    /// Parameters fixed by the builder.
    #[serde(default)]
    pub internal_parameters: Value,
    /// Resolved input dependencies.
    #[serde(default)]
    pub resolved_dependencies: Vec<Value>,
}

/// Publisher-controlled build parameters.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildParameters {
    /// The package name.
    pub name: String,
    /// The package version.
    pub version: String,
    /// The SBOM digest, `sha256:<hex>`.
    pub sbom: String,
    /// The API snapshot digest, `sha256:<hex>`.
    pub api: String,
    /// The source repository, when declared.
    #[serde(default)]
    pub repository: Option<String>,
}

/// The SLSA run details.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunDetails {
    /// The builder.
    pub builder: Builder,
    /// Invocation metadata.
    pub metadata: InvocationMetadata,
}

/// The builder identity.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Builder {
    /// The builder URI or OIDC subject.
    pub id: String,
}

/// Invocation metadata.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InvocationMetadata {
    /// A unique invocation id.
    pub invocation_id: String,
    /// The RFC 3339 start time.
    pub started_on: String,
    /// The RFC 3339 finish time.
    pub finished_on: String,
}

/// The SLSA predicate.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Predicate {
    /// The build definition.
    pub build_definition: BuildDefinition,
    /// The run details.
    pub run_details: RunDetails,
}

/// An in-toto statement with a SLSA provenance predicate.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Statement {
    /// The statement type.
    #[serde(rename = "_type")]
    pub kind: String,
    /// The signed subjects.
    pub subject: Vec<Subject>,
    /// The predicate type.
    #[serde(rename = "predicateType")]
    pub predicate_type: String,
    /// The SLSA predicate.
    pub predicate: Predicate,
}

/// The publisher identity recorded next to a signature.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Identity {
    /// The key id.
    pub keyid: String,
    /// The public key as lowercase hexadecimal.
    pub public: String,
    /// The OIDC subject, when trusted publishing was used.
    pub subject: Option<String>,
}

/// Options for verifying an envelope.
#[derive(Clone, Debug)]
pub struct VerifyOptions<'a> {
    /// The public key the caller trusts, when known.
    pub public: Option<&'a str>,
    /// The expected archive SHA-256 without a prefix.
    pub archive_sha256: &'a str,
    /// The expected package name.
    pub name: &'a str,
    /// The expected package version.
    pub version: &'a str,
    /// The required builder identity.
    pub identity: Option<&'a str>,
}

/// Inputs for a signed publication statement.
#[derive(Clone, Debug)]
pub struct SignOptions<'a> {
    /// The package name.
    pub name: &'a str,
    /// The package version.
    pub version: &'a str,
    /// The archive SHA-256 without a prefix.
    pub archive_sha256: &'a str,
    /// The SBOM digest.
    pub sbom: &'a str,
    /// The API snapshot digest.
    pub api: &'a str,
    /// The source repository, when declared.
    pub repository: Option<&'a str>,
    /// The builder identity.
    pub identity: Option<&'a str>,
    /// A unique invocation id.
    pub invocation_id: &'a str,
    /// The RFC 3339 start time.
    pub started_on: &'a str,
    /// The RFC 3339 finish time.
    pub finished_on: &'a str,
}

/// Builds and signs a statement for one package.
pub fn sign(options: &SignOptions<'_>, seed: &[u8; 32]) -> RegistryResult<Envelope> {
    let statement = Statement {
        kind: STATEMENT_TYPE.to_owned(),
        subject: vec![Subject {
            name: format!("{}-{}.tar.zst", options.name, options.version),
            digest: BTreeMap::from([("sha256".to_owned(), options.archive_sha256.to_owned())]),
        }],
        predicate_type: PREDICATE_TYPE.to_owned(),
        predicate: Predicate {
            build_definition: BuildDefinition {
                build_type: BUILD_TYPE.to_owned(),
                external_parameters: BuildParameters {
                    name: options.name.to_owned(),
                    version: options.version.to_owned(),
                    sbom: options.sbom.to_owned(),
                    api: options.api.to_owned(),
                    repository: options.repository.map(ToOwned::to_owned),
                },
                internal_parameters: Value::Null,
                resolved_dependencies: Vec::new(),
            },
            run_details: RunDetails {
                builder: Builder {
                    id: options
                        .identity
                        .unwrap_or("https://seal.dev/seal-cli")
                        .to_owned(),
                },
                metadata: InvocationMetadata {
                    invocation_id: options.invocation_id.to_owned(),
                    started_on: options.started_on.to_owned(),
                    finished_on: options.finished_on.to_owned(),
                },
            },
        },
    };
    let payload = serde_json::to_vec(&statement).map_err(|error| {
        RegistryError::provenance_data(format!("cannot encode the statement: {error}"))
    })?;
    sign_payload(&payload, seed)
}

/// Signs an arbitrary in-toto payload.
pub fn sign_payload(payload: &[u8], seed: &[u8; 32]) -> RegistryResult<Envelope> {
    let key = SigningKey::from_bytes(seed);
    let signature = key.sign(&pre_auth_encoding(PAYLOAD_TYPE, payload));
    Ok(Envelope {
        payload_type: PAYLOAD_TYPE.to_owned(),
        payload: base64::engine::general_purpose::STANDARD.encode(payload),
        signatures: vec![DsseSignature {
            keyid: key_id(&key.verifying_key()),
            sig: hex::encode(signature.to_bytes()),
        }],
    })
}

/// Verifies an envelope and returns its statement.
pub fn verify(envelope: &Envelope, options: &VerifyOptions<'_>) -> RegistryResult<Statement> {
    if envelope.payload_type != PAYLOAD_TYPE {
        return Err(RegistryError::provenance_data(format!(
            "the provenance payload type `{}` is not `{PAYLOAD_TYPE}`",
            envelope.payload_type
        )));
    }
    let payload = base64::engine::general_purpose::STANDARD
        .decode(&envelope.payload)
        .map_err(|error| {
            RegistryError::provenance_data(format!("the provenance payload is not base64: {error}"))
        })?;
    if payload.len() > 4 * 1024 * 1024 {
        return Err(RegistryError::provenance_data(
            "the provenance payload is too large",
        ));
    }
    if envelope.signatures.is_empty() {
        return Err(RegistryError::provenance_data(
            "the provenance envelope has no signatures",
        ));
    }
    let public = options.public.ok_or_else(|| {
        RegistryError::provenance_data("the provenance envelope has no trusted public key")
    })?;
    let public_bytes = decode_public_key(public)?;
    let key = VerifyingKey::from_bytes(&public_bytes).map_err(|error| {
        RegistryError::provenance_data(format!("the publisher key is invalid: {error}"))
    })?;
    let keyid = key_id(&key);
    let message = pre_auth_encoding(&envelope.payload_type, &payload);
    let mut valid = false;
    for signature in &envelope.signatures {
        if signature.keyid != keyid {
            continue;
        }
        let bytes = hex::decode(&signature.sig).map_err(|error| {
            RegistryError::provenance_data(format!("the provenance signature is invalid: {error}"))
        })?;
        let bytes: [u8; 64] = bytes.try_into().map_err(|_| {
            RegistryError::provenance_data("the provenance signature is not 64 bytes")
        })?;
        if key
            .verify_strict(&message, &Signature::from_bytes(&bytes))
            .is_ok()
        {
            valid = true;
            break;
        }
    }
    if !valid {
        return Err(RegistryError::provenance_data(
            "no provenance signature matches the trusted publisher key",
        ));
    }
    let statement: Statement = serde_json::from_slice(&payload).map_err(|error| {
        RegistryError::provenance_data(format!("the provenance statement is invalid: {error}"))
    })?;
    validate_statement(&statement, options)?;
    Ok(statement)
}

fn validate_statement(statement: &Statement, options: &VerifyOptions<'_>) -> RegistryResult<()> {
    if statement.kind != STATEMENT_TYPE {
        return Err(RegistryError::provenance_data(format!(
            "the statement type `{}` is not `{STATEMENT_TYPE}`",
            statement.kind
        )));
    }
    if statement.predicate_type != PREDICATE_TYPE {
        return Err(RegistryError::provenance_data(format!(
            "the predicate type `{}` is not `{PREDICATE_TYPE}`",
            statement.predicate_type
        )));
    }
    let subject = statement
        .subject
        .first()
        .ok_or_else(|| RegistryError::provenance_data("the statement has no subject"))?;
    let digest = subject.digest.get("sha256").ok_or_else(|| {
        RegistryError::provenance_data("the statement subject has no SHA-256 digest")
    })?;
    if !digest.eq_ignore_ascii_case(options.archive_sha256) {
        return Err(RegistryError::provenance_data(format!(
            "the provenance subject digest is `{digest}`, but the archive is `{}`",
            options.archive_sha256
        )));
    }
    let parameters = &statement.predicate.build_definition.external_parameters;
    if parameters.name != options.name || parameters.version != options.version {
        return Err(RegistryError::provenance_data(format!(
            "the provenance names `{}` {}, but the request is `{}` {}",
            parameters.name, parameters.version, options.name, options.version
        )));
    }
    if let Some(identity) = options.identity
        && statement.predicate.run_details.builder.id != identity
    {
        return Err(RegistryError::provenance_data(format!(
            "the provenance builder is `{}`, not the authenticated identity `{identity}`",
            statement.predicate.run_details.builder.id
        )));
    }
    Ok(())
}

/// Returns the key id for a public key.
pub fn key_id(public: &VerifyingKey) -> String {
    let mut hasher = Sha256::new();
    hasher.update(public.to_bytes());
    hex::encode(hasher.finalize())
}

/// Returns the public key for a seed.
pub fn public_key(seed: &[u8; 32]) -> String {
    hex::encode(SigningKey::from_bytes(seed).verifying_key().to_bytes())
}

/// Returns the digest of an encoded envelope.
pub fn envelope_digest(envelope: &Envelope) -> RegistryResult<String> {
    let bytes = canonical_json(&serde_json::to_value(envelope).map_err(|error| {
        RegistryError::provenance_data(format!("cannot encode provenance: {error}"))
    })?);
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    Ok(format!("sha256:{}", hex::encode(hasher.finalize())))
}

fn pre_auth_encoding(payload_type: &str, payload: &[u8]) -> Vec<u8> {
    let mut message = Vec::with_capacity(payload.len() + payload_type.len() + 32);
    message.extend_from_slice(b"DSSEv1 ");
    message.extend_from_slice(payload_type.len().to_string().as_bytes());
    message.push(b' ');
    message.extend_from_slice(payload_type.as_bytes());
    message.push(b' ');
    message.extend_from_slice(payload.len().to_string().as_bytes());
    message.push(b' ');
    message.extend_from_slice(payload);
    message
}

fn decode_public_key(public: &str) -> RegistryResult<[u8; 32]> {
    if public.len() != 64 || !public.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(RegistryError::provenance_data(
            "the publisher key is not 64 hexadecimal characters",
        ));
    }
    let bytes = hex::decode(public).map_err(|error| {
        RegistryError::provenance_data(format!("the publisher key is invalid: {error}"))
    })?;
    bytes
        .try_into()
        .map_err(|_| RegistryError::provenance_data("the publisher key is not 32 bytes"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn digest() -> String {
        "a".repeat(64)
    }

    fn signed_envelope(seed: &[u8; 32]) -> Envelope {
        let sbom = format!("sha256:{}", "b".repeat(64));
        let api = format!("sha256:{}", "c".repeat(64));
        sign(
            &SignOptions {
                name: "demo",
                version: "1.0.0",
                archive_sha256: &digest(),
                sbom: &sbom,
                api: &api,
                repository: None,
                identity: None,
                invocation_id: "invocation",
                started_on: "2026-01-01T00:00:00Z",
                finished_on: "2026-01-01T00:00:01Z",
            },
            seed,
        )
        .unwrap()
    }

    #[test]
    fn a_signed_statement_round_trips_and_verifies() {
        let seed = [3u8; 32];
        let public = public_key(&seed);
        let envelope = signed_envelope(&seed);
        let statement = verify(
            &envelope,
            &VerifyOptions {
                public: Some(&public),
                archive_sha256: &digest(),
                name: "demo",
                version: "1.0.0",
                identity: None,
            },
        )
        .unwrap();
        assert_eq!(
            statement
                .predicate
                .build_definition
                .external_parameters
                .name,
            "demo"
        );
    }

    #[test]
    fn tampering_with_the_payload_is_rejected() {
        let seed = [3u8; 32];
        let public = public_key(&seed);
        let mut envelope = signed_envelope(&seed);
        let payload = base64::engine::general_purpose::STANDARD
            .decode(&envelope.payload)
            .unwrap();
        let tampered = String::from_utf8(payload)
            .unwrap()
            .replace("1.0.0", "9.9.9");
        envelope.payload = base64::engine::general_purpose::STANDARD.encode(tampered);
        let error = verify(
            &envelope,
            &VerifyOptions {
                public: Some(&public),
                archive_sha256: &digest(),
                name: "demo",
                version: "1.0.0",
                identity: None,
            },
        )
        .unwrap_err();
        assert_eq!(error.code(), "E8096");
    }

    #[test]
    fn a_subject_digest_mismatch_is_rejected() {
        let seed = [3u8; 32];
        let public = public_key(&seed);
        let envelope = signed_envelope(&seed);
        let error = verify(
            &envelope,
            &VerifyOptions {
                public: Some(&public),
                archive_sha256: &"d".repeat(64),
                name: "demo",
                version: "1.0.0",
                identity: None,
            },
        )
        .unwrap_err();
        assert_eq!(error.code(), "E8096");
    }
}
