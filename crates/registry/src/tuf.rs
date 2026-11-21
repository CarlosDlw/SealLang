//! TUF metadata verification.
//!
//! The client trusts a local `root.json`, verifies timestamp, snapshot, and
//! targets signatures, enforces expiry and rollback protection, checks the
//! length and SHA-256 digest of every metadata file, and only then returns
//! target bytes from the protected targets document.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::client::RegistryClient;
use crate::config::{RegistryConfig, endpoint};
use crate::error::{RegistryError, RegistryResult};

const MAX_METADATA: usize = 2 * 1024 * 1024;
const MAX_TARGET: usize = 512 * 1024 * 1024;

/// A signed TUF document.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Signed<T> {
    /// The signed payload.
    pub signed: T,
    /// Detached signatures.
    #[serde(default)]
    pub signatures: Vec<TufSignature>,
}

/// One TUF signature.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TufSignature {
    /// The signing key id.
    pub keyid: String,
    /// The Ed25519 signature as lowercase hexadecimal.
    pub sig: String,
}

/// A TUF key.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TufKey {
    /// Always `ed25519`.
    pub keytype: String,
    /// Always `ed25519`.
    pub scheme: String,
    /// The key value.
    pub keyval: TufKeyValue,
}

/// A TUF key value.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TufKeyValue {
    /// The 32-byte public key as lowercase hexadecimal.
    pub public: String,
}

/// A role definition.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TufRole {
    /// Authorized key ids.
    pub keyids: Vec<String>,
    /// Required number of valid signatures.
    pub threshold: u32,
}

/// Root metadata.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Root {
    /// `root`.
    #[serde(rename = "type")]
    pub kind: String,
    /// The monotonically increasing version.
    pub version: u64,
    /// The RFC 3339 expiry.
    pub expires: String,
    /// Public keys by key id.
    pub keys: BTreeMap<String, TufKey>,
    /// Roles.
    pub roles: BTreeMap<String, TufRole>,
}

/// Snapshot metadata.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Snapshot {
    /// `snapshot`.
    #[serde(rename = "type")]
    pub kind: String,
    /// The version.
    pub version: u64,
    /// The expiry.
    pub expires: String,
    /// Metadata files.
    pub meta: BTreeMap<String, MetaFile>,
}

/// Timestamp metadata.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Timestamp {
    /// `timestamp`.
    #[serde(rename = "type")]
    pub kind: String,
    /// The version.
    pub version: u64,
    /// The expiry.
    pub expires: String,
    /// Metadata files.
    pub meta: BTreeMap<String, MetaFile>,
}

/// Targets metadata.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Targets {
    /// `targets`.
    #[serde(rename = "type")]
    pub kind: String,
    /// The version.
    pub version: u64,
    /// The expiry.
    pub expires: String,
    /// Target files.
    #[serde(default)]
    pub targets: BTreeMap<String, MetaFile>,
}

/// Length and hashes for a metadata or target file.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MetaFile {
    /// The file version, for metadata files.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<u64>,
    /// The exact byte length.
    pub length: u64,
    /// Hashes by algorithm; `sha256` is required.
    pub hashes: BTreeMap<String, String>,
}

/// Persistent rollback state.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct State {
    root_version: u64,
    timestamp_version: u64,
    snapshot_version: u64,
    targets_version: u64,
}

/// A verified TUF session.
pub struct TufSession {
    root: Signed<Root>,
    targets: Signed<Targets>,
    state_path: PathBuf,
}

impl TufSession {
    /// Loads and verifies metadata against the configured trusted root.
    pub fn load(client: &RegistryClient, config: &RegistryConfig) -> RegistryResult<Self> {
        let root_path = config.tuf_root.as_deref().ok_or_else(|| {
            RegistryError::tuf(
                "no trusted TUF root is configured; set `[registry] tuf-root` to the pinned `root.json`",
            )
        })?;
        let trusted = read_root(root_path)?;
        let state_path = config.cache_root.join("tuf").join("state.json");
        let previous = read_state(&state_path).unwrap_or_default();
        if trusted.signed.version < previous.root_version {
            return Err(RegistryError::tuf(format!(
                "the trusted root is version {}, older than the previously verified version {}",
                trusted.signed.version, previous.root_version
            )));
        }
        let remote_root_bytes = fetch_raw(client, config, "root.json")?;
        let remote_root: Signed<Root> = serde_json::from_slice(&remote_root_bytes)
            .map_err(|error| RegistryError::tuf(format!("invalid `root.json`: {error}")))?;
        let root = if remote_root.signed.version < trusted.signed.version {
            return Err(RegistryError::tuf(format!(
                "root rollback: the registry offers version {}, but {} is trusted",
                remote_root.signed.version, trusted.signed.version
            )));
        } else if remote_root.signed.version == trusted.signed.version {
            verify_role(&remote_root, &trusted.signed, "root")?;
            trusted.clone()
        } else {
            verify_role(&remote_root, &trusted.signed, "root")?;
            verify_role(&remote_root, &remote_root.signed, "root")?;
            write_root(root_path, &remote_root_bytes)?;
            remote_root
        };

        let timestamp = fetch_metadata::<Timestamp>(client, config, "timestamp.json")?;
        verify_role(&timestamp, &root.signed, "timestamp")?;
        check_expiry(&timestamp.signed.expires, "timestamp")?;
        if timestamp.signed.version < previous.timestamp_version {
            return Err(RegistryError::tuf(format!(
                "timestamp rollback: the registry offers version {}, previous was {}",
                timestamp.signed.version, previous.timestamp_version
            )));
        }
        let snapshot_meta = timestamp.signed.meta.get("snapshot.json").ok_or_else(|| {
            RegistryError::tuf("timestamp metadata does not list `snapshot.json`")
        })?;
        if let Some(version) = snapshot_meta.version
            && version < previous.snapshot_version
        {
            return Err(RegistryError::tuf(format!(
                "snapshot rollback: timestamp names version {version}, previous was {}",
                previous.snapshot_version
            )));
        }
        let snapshot =
            fetch_metadata_checked::<Snapshot>(client, config, "snapshot.json", snapshot_meta)?;
        verify_role(&snapshot, &root.signed, "snapshot")?;
        check_expiry(&snapshot.signed.expires, "snapshot")?;
        if snapshot_meta.version != Some(snapshot.signed.version) {
            return Err(RegistryError::tuf(format!(
                "snapshot mix-and-match: timestamp names version {:?}, snapshot is version {}",
                snapshot_meta.version, snapshot.signed.version
            )));
        }
        let targets_meta =
            snapshot.signed.meta.get("targets.json").ok_or_else(|| {
                RegistryError::tuf("snapshot metadata does not list `targets.json`")
            })?;
        if let Some(version) = targets_meta.version
            && version < previous.targets_version
        {
            return Err(RegistryError::tuf(format!(
                "targets rollback: snapshot names version {version}, previous was {}",
                previous.targets_version
            )));
        }
        let targets =
            fetch_metadata_checked::<Targets>(client, config, "targets.json", targets_meta)?;
        verify_role(&targets, &root.signed, "targets")?;
        check_expiry(&targets.signed.expires, "targets")?;
        if targets_meta.version != Some(targets.signed.version) {
            return Err(RegistryError::tuf(format!(
                "targets mix-and-match: snapshot names version {:?}, targets is version {}",
                targets_meta.version, targets.signed.version
            )));
        }
        let state = State {
            root_version: root.signed.version,
            timestamp_version: timestamp.signed.version,
            snapshot_version: snapshot.signed.version,
            targets_version: targets.signed.version,
        };
        write_state(&state_path, &state)?;
        Ok(Self {
            root,
            targets,
            state_path,
        })
    }

    /// Fetches and verifies one target below the registry base.
    pub fn target_from(
        &self,
        client: &RegistryClient,
        config: &RegistryConfig,
        path: &str,
    ) -> RegistryResult<Vec<u8>> {
        let meta = self.targets.signed.targets.get(path).ok_or_else(|| {
            RegistryError::tuf(format!("the TUF targets metadata does not list `{path}`"))
        })?;
        if meta.length > MAX_TARGET as u64 {
            return Err(RegistryError::tuf(format!(
                "the target `{path}` declares {} bytes, above the {} limit",
                meta.length, MAX_TARGET
            )));
        }
        let bytes = client.fetch(config, path)?;
        verify_meta(path, &bytes, meta)?;
        Ok(bytes)
    }

    /// Returns the verified root metadata version.
    pub fn root_version(&self) -> u64 {
        self.root.signed.version
    }

    /// Returns the verified targets version.
    pub fn targets_version(&self) -> u64 {
        self.targets.signed.version
    }

    /// Returns the rollback-state path.
    pub fn state_path(&self) -> &Path {
        &self.state_path
    }

    /// Returns the transparency-log public key from the `log` role.
    pub fn log_public_key(&self) -> RegistryResult<String> {
        let role = self
            .root
            .signed
            .roles
            .get("log")
            .ok_or_else(|| RegistryError::tuf("the trusted root has no `log` role"))?;
        let keyid = role
            .keyids
            .first()
            .ok_or_else(|| RegistryError::tuf("the `log` role has no key ids"))?;
        let key = self.root.signed.keys.get(keyid).ok_or_else(|| {
            RegistryError::tuf(format!("the `log` role names the unknown key `{keyid}`"))
        })?;
        Ok(key.keyval.public.clone())
    }
}

fn fetch_metadata<T>(
    client: &RegistryClient,
    config: &RegistryConfig,
    name: &str,
) -> RegistryResult<Signed<T>>
where
    T: for<'de> Deserialize<'de>,
{
    let bytes = fetch_raw(client, config, name)?;
    serde_json::from_slice(&bytes)
        .map_err(|error| RegistryError::tuf(format!("invalid `{name}`: {error}")))
}

fn fetch_raw(
    client: &RegistryClient,
    config: &RegistryConfig,
    name: &str,
) -> RegistryResult<Vec<u8>> {
    let url = endpoint(&config.tuf_url, name)?;
    let bytes = client.fetch_url(&url)?;
    if bytes.len() > MAX_METADATA {
        return Err(RegistryError::tuf(format!(
            "the metadata `{name}` is larger than {} bytes",
            MAX_METADATA
        )));
    }
    Ok(bytes)
}

fn fetch_metadata_checked<T>(
    client: &RegistryClient,
    config: &RegistryConfig,
    name: &str,
    meta: &MetaFile,
) -> RegistryResult<Signed<T>>
where
    T: for<'de> Deserialize<'de>,
{
    let url = endpoint(&config.tuf_url, name)?;
    let bytes = client.fetch_url(&url)?;
    verify_meta(name, &bytes, meta)?;
    serde_json::from_slice(&bytes)
        .map_err(|error| RegistryError::tuf(format!("invalid `{name}`: {error}")))
}

fn read_root(path: &Path) -> RegistryResult<Signed<Root>> {
    let bytes = std::fs::read(path).map_err(|error| {
        RegistryError::tuf(format!(
            "cannot read the trusted root `{}`: {error}",
            path.display()
        ))
    })?;
    if bytes.len() > MAX_METADATA {
        return Err(RegistryError::tuf("the trusted root is too large"));
    }
    let root: Signed<Root> = serde_json::from_slice(&bytes)
        .map_err(|error| RegistryError::tuf(format!("invalid trusted root: {error}")))?;
    if root.signed.kind != "root" {
        return Err(RegistryError::tuf(
            "the trusted root is not a root document",
        ));
    }
    check_expiry(&root.signed.expires, "root")?;
    verify_role(&root, &root.signed, "root")?;
    Ok(root)
}

fn verify_role<T: Serialize>(document: &Signed<T>, root: &Root, role: &str) -> RegistryResult<()> {
    let definition = root
        .roles
        .get(role)
        .ok_or_else(|| RegistryError::tuf(format!("root metadata has no `{role}` role")))?;
    let signed = canonical_json(
        &serde_json::to_value(&document.signed)
            .map_err(|error| RegistryError::tuf(format!("cannot encode {role}: {error}")))?,
    );
    let mut valid = std::collections::BTreeSet::new();
    for signature in &document.signatures {
        if !definition.keyids.contains(&signature.keyid) {
            continue;
        }
        if valid.contains(&signature.keyid) {
            continue;
        }
        let Some(key) = root.keys.get(&signature.keyid) else {
            continue;
        };
        if key.keytype != "ed25519" || key.scheme != "ed25519" {
            continue;
        }
        let public = hex::decode(&key.keyval.public).map_err(|error| {
            RegistryError::tuf(format!(
                "the key `{}` is not hexadecimal: {error}",
                signature.keyid
            ))
        })?;
        let public: [u8; 32] = public.try_into().map_err(|_| {
            RegistryError::tuf(format!("the key `{}` is not 32 bytes", signature.keyid))
        })?;
        let key = VerifyingKey::from_bytes(&public).map_err(|error| {
            RegistryError::tuf(format!("the key `{}` is invalid: {error}", signature.keyid))
        })?;
        let signature_bytes = hex::decode(&signature.sig).map_err(|error| {
            RegistryError::tuf(format!(
                "the signature from `{}` is not hexadecimal: {error}",
                signature.keyid
            ))
        })?;
        let signature_bytes: [u8; 64] = signature_bytes.try_into().map_err(|_| {
            RegistryError::tuf(format!(
                "the signature from `{}` is not 64 bytes",
                signature.keyid
            ))
        })?;
        let parsed = Signature::from_bytes(&signature_bytes);
        if key.verify_strict(&signed, &parsed).is_ok() {
            valid.insert(signature.keyid.clone());
        }
    }
    if valid.len() < definition.threshold as usize {
        return Err(RegistryError::tuf(format!(
            "the `{role}` metadata has {} valid signature(s), but the threshold is {}",
            valid.len(),
            definition.threshold
        )));
    }
    Ok(())
}

fn check_expiry(expires: &str, role: &str) -> RegistryResult<()> {
    let expires = OffsetDateTime::parse(expires, &Rfc3339).map_err(|error| {
        RegistryError::tuf(format!(
            "the `{role}` expiry `{expires}` is not RFC 3339: {error}"
        ))
    })?;
    if expires <= OffsetDateTime::now_utc() {
        return Err(RegistryError::tuf(format!(
            "the `{role}` metadata expired at {expires}"
        )));
    }
    Ok(())
}

fn verify_meta(path: &str, bytes: &[u8], meta: &MetaFile) -> RegistryResult<()> {
    if bytes.len() as u64 != meta.length {
        return Err(RegistryError::tuf(format!(
            "the target `{path}` is {} bytes, but metadata declares {}",
            bytes.len(),
            meta.length
        )));
    }
    let expected = meta
        .hashes
        .get("sha256")
        .ok_or_else(|| RegistryError::tuf(format!("the target `{path}` has no SHA-256 digest")))?;
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let actual = hex::encode(hasher.finalize());
    if !actual.eq_ignore_ascii_case(expected) {
        return Err(RegistryError::tuf(format!(
            "the target `{path}` SHA-256 is `{actual}`, but metadata declares `{expected}`"
        )));
    }
    Ok(())
}

fn read_state(path: &Path) -> Option<State> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

fn write_root(path: &Path, bytes: &[u8]) -> RegistryResult<()> {
    let temporary = path.with_extension("tmp");
    std::fs::write(&temporary, bytes)
        .and_then(|()| std::fs::rename(&temporary, path))
        .map_err(|error| RegistryError::tuf(format!("cannot update `{}`: {error}", path.display())))
}

fn write_state(path: &Path, state: &State) -> RegistryResult<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            RegistryError::tuf(format!("cannot create `{}`: {error}", parent.display()))
        })?;
    }
    let text = serde_json::to_vec_pretty(state)
        .map_err(|error| RegistryError::tuf(format!("cannot encode TUF state: {error}")))?;
    let temporary = path.with_extension("tmp");
    std::fs::write(&temporary, text)
        .and_then(|()| std::fs::rename(&temporary, path))
        .map_err(|error| RegistryError::tuf(format!("cannot write `{}`: {error}", path.display())))
}

/// Returns canonical JSON bytes with object keys sorted recursively.
pub fn canonical_json(value: &Value) -> Vec<u8> {
    let mut output = String::new();
    write_canonical(value, &mut output);
    output.into_bytes()
}

fn write_canonical(value: &Value, output: &mut String) {
    match value {
        Value::Null => output.push_str("null"),
        Value::Bool(value) => output.push_str(if *value { "true" } else { "false" }),
        Value::Number(number) => output.push_str(&number.to_string()),
        Value::String(value) => output
            .push_str(&serde_json::to_string(value).expect("a JSON string is always serializable")),
        Value::Array(values) => {
            output.push('[');
            for (index, value) in values.iter().enumerate() {
                if index > 0 {
                    output.push(',');
                }
                write_canonical(value, output);
            }
            output.push(']');
        }
        Value::Object(values) => {
            output.push('{');
            let mut entries: Vec<_> = values.iter().collect();
            entries.sort_by(|left, right| left.0.cmp(right.0));
            for (index, (key, value)) in entries.into_iter().enumerate() {
                if index > 0 {
                    output.push(',');
                }
                output.push_str(
                    &serde_json::to_string(key).expect("a JSON key is always serializable"),
                );
                output.push(':');
                write_canonical(value, output);
            }
            output.push('}');
        }
    }
}

/// Signs a metadata payload with an Ed25519 seed, for registry fixtures and
/// future publishing tools.
pub fn sign_document(signed: &Value, seed: &[u8; 32]) -> RegistryResult<TufSignature> {
    let key = SigningKey::from_bytes(seed);
    let public = key.verifying_key().to_bytes();
    let mut hasher = Sha256::new();
    hasher.update(public);
    let keyid = hex::encode(hasher.finalize());
    let signature = key.sign(&canonical_json(signed));
    Ok(TufSignature {
        keyid,
        sig: hex::encode(signature.to_bytes()),
    })
}

/// Returns the key id for a seed, for building fixture roots.
pub fn key_id(seed: &[u8; 32]) -> String {
    let key = SigningKey::from_bytes(seed);
    let mut hasher = Sha256::new();
    hasher.update(key.verifying_key().to_bytes());
    hex::encode(hasher.finalize())
}

/// Returns the public key for a seed.
pub fn public_key(seed: &[u8; 32]) -> String {
    hex::encode(SigningKey::from_bytes(seed).verifying_key().to_bytes())
}
