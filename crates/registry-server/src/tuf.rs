//! TUF metadata generation and root rotation.
//!
//! The server produces the same documents the client verifies: a root with
//! Ed25519 keys for root, targets, snapshot, timestamp, and the transparency log;
//! signed targets over every public artifact; a snapshot binding the targets
//! version; and a timestamp binding the snapshot. Rotation adds a new root signed
//! by both the old and the new root key.

use std::collections::BTreeMap;

use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::error::{ServerError, ServerResult};
use crate::state::Keys;

/// The generated TUF metadata files.
pub struct TufFiles {
    /// `root.json`.
    pub root: Vec<u8>,
    /// `targets.json`.
    pub targets: Vec<u8>,
    /// `snapshot.json`.
    pub snapshot: Vec<u8>,
    /// `timestamp.json`.
    pub timestamp: Vec<u8>,
}

impl TufFiles {
    /// Returns every file with its name.
    pub fn entries(&self) -> [(&'static str, &[u8]); 4] {
        [
            ("root.json", &self.root),
            ("targets.json", &self.targets),
            ("snapshot.json", &self.snapshot),
            ("timestamp.json", &self.timestamp),
        ]
    }
}

/// Generates metadata for the current state.
pub fn generate(
    keys: &Keys,
    targets: &BTreeMap<String, Vec<u8>>,
    root_version: u64,
    metadata_version: u64,
    old_root: Option<&[u8; 32]>,
) -> ServerResult<TufFiles> {
    let expires = (OffsetDateTime::now_utc() + time::Duration::days(365))
        .format(&Rfc3339)
        .map_err(|error| ServerError::internal(format!("cannot format the expiry: {error}")))?;
    let root_seed = keys.seed("root")?;
    let targets_seed = keys.seed("targets")?;
    let snapshot_seed = keys.seed("snapshot")?;
    let timestamp_seed = keys.seed("timestamp")?;
    let log_seed = keys.seed("log")?;

    let mut keys_map = Map::new();
    let mut role_ids = BTreeMap::new();
    for (role, seed) in [
        ("root", &root_seed),
        ("targets", &targets_seed),
        ("snapshot", &snapshot_seed),
        ("timestamp", &timestamp_seed),
        ("log", &log_seed),
    ] {
        let keyid = seal_registry::tuf::key_id(seed);
        let public = seal_registry::tuf::public_key(seed);
        keys_map.insert(
            keyid.clone(),
            json!({
                "keytype": "ed25519",
                "scheme": "ed25519",
                "keyval": {"public": public}
            }),
        );
        role_ids.insert(role.to_owned(), keyid);
    }
    let mut roles = Map::new();
    for (role, keyid) in &role_ids {
        roles.insert(role.clone(), json!({"keyids": [keyid], "threshold": 1}));
    }
    let root_signed = json!({
        "type": "root",
        "version": root_version,
        "expires": expires,
        "keys": keys_map,
        "roles": roles
    });
    let mut signatures = vec![seal_registry::tuf::sign_document(&root_signed, &root_seed)?];
    if let Some(old) = old_root {
        if old != &root_seed {
            signatures.push(seal_registry::tuf::sign_document(&root_signed, old)?);
        }
    }
    let root = signed_value(&root_signed, signatures)?;

    let mut target_map = Map::new();
    for (path, bytes) in targets {
        let mut hasher = Sha256::new();
        hasher.update(bytes);
        target_map.insert(
            path.clone(),
            json!({
                "length": bytes.len(),
                "hashes": {"sha256": hex::encode(hasher.finalize())}
            }),
        );
    }
    let targets_signed = json!({
        "type": "targets",
        "version": metadata_version,
        "expires": expires,
        "targets": target_map
    });
    let targets = signed(&targets_signed, &targets_seed)?;

    let targets_meta = meta(&targets, Some(metadata_version));
    let snapshot_signed = json!({
        "type": "snapshot",
        "version": metadata_version,
        "expires": expires,
        "meta": {"targets.json": targets_meta}
    });
    let snapshot = signed(&snapshot_signed, &snapshot_seed)?;

    let snapshot_meta = meta(&snapshot, Some(metadata_version));
    let timestamp_signed = json!({
        "type": "timestamp",
        "version": metadata_version,
        "expires": expires,
        "meta": {"snapshot.json": snapshot_meta}
    });
    let timestamp = signed(&timestamp_signed, &timestamp_seed)?;

    Ok(TufFiles {
        root,
        targets,
        snapshot,
        timestamp,
    })
}

fn signed(signed: &Value, seed: &[u8; 32]) -> ServerResult<Vec<u8>> {
    let signature = seal_registry::tuf::sign_document(signed, seed)?;
    signed_value(signed, vec![signature])
}

fn signed_value(
    signed: &Value,
    signatures: Vec<seal_registry::tuf::TufSignature>,
) -> ServerResult<Vec<u8>> {
    serde_json::to_vec_pretty(&json!({
        "signed": signed,
        "signatures": signatures
    }))
    .map_err(|error| ServerError::internal(format!("cannot encode TUF metadata: {error}")))
}

fn meta(bytes: &[u8], version: Option<u64>) -> Value {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let mut value = json!({
        "length": bytes.len(),
        "hashes": {"sha256": hex::encode(hasher.finalize())}
    });
    if let Some(version) = version {
        value["version"] = Value::from(version);
    }
    value
}

/// Returns the trusted root public key, for diagnostics.
pub fn log_public_key(keys: &Keys) -> ServerResult<String> {
    Ok(seal_registry::tuf::public_key(&keys.seed("log")?))
}
