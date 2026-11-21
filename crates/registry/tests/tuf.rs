//! TUF verification: signatures, expiry, rollback, and mix-and-match.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use seal_registry::client::RegistryClient;
use seal_registry::config::RegistryConfig;
use seal_registry::tuf::{self, TufSession};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use url::Url;

static NEXT: AtomicU64 = AtomicU64::new(0);

const SEED: [u8; 32] = [7u8; 32];
const EXPIRES: &str = "2099-01-01T00:00:00Z";

struct Fixture {
    root: PathBuf,
    metadata: PathBuf,
    config: RegistryConfig,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn fixture() -> Fixture {
    let root = std::env::temp_dir().join(format!(
        "seal-tuf-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&root);
    let metadata = root.join("metadata");
    std::fs::create_dir_all(&metadata).unwrap();
    let trusted = root.join("root.json");
    let root_document = root_doc();
    std::fs::write(&trusted, serde_json::to_vec_pretty(&root_document).unwrap()).unwrap();
    std::fs::write(
        metadata.join("root.json"),
        serde_json::to_vec_pretty(&root_document).unwrap(),
    )
    .unwrap();
    let config = RegistryConfig {
        name: "test".to_owned(),
        url: Url::from_directory_path(&metadata).unwrap(),
        mirror: None,
        token: None,
        tuf_url: Url::from_directory_path(&metadata).unwrap(),
        tuf_root: Some(trusted),
        advisories: Url::from_directory_path(&metadata)
            .unwrap()
            .join("advisories.json")
            .unwrap(),
        offline: false,
        cache_root: root.join("cache"),
        policy: Default::default(),
    };
    Fixture {
        root,
        metadata,
        config,
    }
}

fn root_doc() -> Value {
    let keyid = tuf::key_id(&SEED);
    let mut keys = Map::new();
    keys.insert(
        keyid.clone(),
        json!({
            "keytype": "ed25519",
            "scheme": "ed25519",
            "keyval": {"public": tuf::public_key(&SEED)}
        }),
    );
    let mut roles = Map::new();
    for role in ["root", "timestamp", "snapshot", "targets"] {
        roles.insert(
            role.to_owned(),
            json!({"keyids": [keyid.clone()], "threshold": 1}),
        );
    }
    signed(
        &json!({
            "type": "root",
            "version": 1,
            "expires": EXPIRES,
            "keys": keys,
            "roles": roles
        }),
        &SEED,
    )
}

fn signed(payload: &Value, seed: &[u8; 32]) -> Value {
    json!({
        "signed": payload,
        "signatures": [tuf::sign_document(payload, seed).unwrap()]
    })
}

fn meta(bytes: &[u8], version: Option<u64>) -> Value {
    let mut value = json!({
        "length": bytes.len(),
        "hashes": {"sha256": hex::encode(Sha256::digest(bytes))}
    });
    if let Some(version) = version {
        value["version"] = json!(version);
    }
    value
}

fn write(path: &Path, value: &Value) -> Vec<u8> {
    let bytes = serde_json::to_vec_pretty(value).unwrap();
    std::fs::write(path, &bytes).unwrap();
    bytes
}

fn install(fixture: &Fixture, timestamp_version: u64, snapshot_version: u64, expires: &str) {
    let targets = signed(
        &json!({
            "type": "targets",
            "version": 1,
            "expires": EXPIRES,
            "targets": {}
        }),
        &SEED,
    );
    let targets_bytes = write(&fixture.metadata.join("targets.json"), &targets);
    let mut snapshot_meta = Map::new();
    let mut targets_meta = meta(&targets_bytes, Some(1));
    targets_meta["version"] = json!(1);
    snapshot_meta.insert("targets.json".to_owned(), targets_meta);
    let snapshot = signed(
        &json!({
            "type": "snapshot",
            "version": snapshot_version,
            "expires": EXPIRES,
            "meta": snapshot_meta
        }),
        &SEED,
    );
    let snapshot_bytes = write(&fixture.metadata.join("snapshot.json"), &snapshot);
    let mut timestamp_meta = Map::new();
    let mut snapshot_entry = meta(&snapshot_bytes, Some(snapshot_version));
    snapshot_entry["version"] = json!(snapshot_version);
    timestamp_meta.insert("snapshot.json".to_owned(), snapshot_entry);
    let timestamp = signed(
        &json!({
            "type": "timestamp",
            "version": timestamp_version,
            "expires": expires,
            "meta": timestamp_meta
        }),
        &SEED,
    );
    write(&fixture.metadata.join("timestamp.json"), &timestamp);
}

fn load(fixture: &Fixture) -> Result<TufSession, seal_registry::RegistryError> {
    let client = RegistryClient::new(&fixture.config).unwrap();
    TufSession::load(&client, &fixture.config)
}

fn load_error(fixture: &Fixture) -> seal_registry::RegistryError {
    match load(fixture) {
        Ok(_) => panic!("TUF verification unexpectedly succeeded"),
        Err(error) => error,
    }
}

#[test]
fn a_valid_chain_loads_and_advances_the_state() {
    let fixture = fixture();
    install(&fixture, 2, 3, EXPIRES);
    let session = load(&fixture).unwrap();
    assert_eq!(session.root_version(), 1);
    assert_eq!(session.targets_version(), 1);
    assert!(session.state_path().is_file());
}

#[test]
fn a_timestamp_rollback_is_rejected() {
    let fixture = fixture();
    install(&fixture, 5, 5, EXPIRES);
    load(&fixture).unwrap();
    install(&fixture, 4, 5, EXPIRES);
    let error = load_error(&fixture);
    assert!(error.message().contains("timestamp rollback"), "{error}");
}

#[test]
fn expired_metadata_is_rejected() {
    let fixture = fixture();
    install(&fixture, 1, 1, "2000-01-01T00:00:00Z");
    let error = load_error(&fixture);
    assert!(error.message().contains("expired"), "{error}");
}

#[test]
fn a_snapshot_version_mismatch_is_rejected() {
    let fixture = fixture();
    install(&fixture, 1, 1, EXPIRES);
    // Rebuild timestamp alone, claiming a snapshot version that does not match.
    let snapshot_bytes = std::fs::read(fixture.metadata.join("snapshot.json")).unwrap();
    let mut timestamp_meta = Map::new();
    timestamp_meta.insert("snapshot.json".to_owned(), meta(&snapshot_bytes, Some(9)));
    let timestamp = signed(
        &json!({
            "type": "timestamp",
            "version": 1,
            "expires": EXPIRES,
            "meta": timestamp_meta
        }),
        &SEED,
    );
    write(&fixture.metadata.join("timestamp.json"), &timestamp);
    let error = load_error(&fixture);
    assert!(error.message().contains("mix-and-match"), "{error}");
}

#[test]
fn a_missing_signature_is_rejected() {
    let fixture = fixture();
    install(&fixture, 1, 1, EXPIRES);
    let path = fixture.metadata.join("timestamp.json");
    let mut value: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    value["signatures"] = json!([]);
    std::fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
    let error = load_error(&fixture);
    assert!(error.message().contains("0 valid signature"), "{error}");
}

#[test]
fn a_target_hash_mismatch_is_rejected() {
    let fixture = fixture();
    let target = b"index";
    std::fs::create_dir_all(fixture.metadata.join("index")).unwrap();
    std::fs::write(fixture.metadata.join("index/foo.json"), target).unwrap();
    let mut targets = serde_json::Map::new();
    let mut wrong = meta(target, None);
    wrong["hashes"]["sha256"] = json!("0".repeat(64));
    targets.insert("index/foo.json".to_owned(), wrong);
    let targets = signed(
        &json!({
            "type": "targets",
            "version": 1,
            "expires": EXPIRES,
            "targets": targets
        }),
        &SEED,
    );
    let targets_bytes = write(&fixture.metadata.join("targets.json"), &targets);
    let mut snapshot_meta = Map::new();
    let mut entry = meta(&targets_bytes, Some(1));
    entry["version"] = json!(1);
    snapshot_meta.insert("targets.json".to_owned(), entry);
    let snapshot = signed(
        &json!({
            "type": "snapshot",
            "version": 1,
            "expires": EXPIRES,
            "meta": snapshot_meta
        }),
        &SEED,
    );
    let snapshot_bytes = write(&fixture.metadata.join("snapshot.json"), &snapshot);
    let mut timestamp_meta = Map::new();
    timestamp_meta.insert("snapshot.json".to_owned(), meta(&snapshot_bytes, Some(1)));
    let timestamp = signed(
        &json!({
            "type": "timestamp",
            "version": 1,
            "expires": EXPIRES,
            "meta": timestamp_meta
        }),
        &SEED,
    );
    write(&fixture.metadata.join("timestamp.json"), &timestamp);
    let session = load(&fixture).unwrap();
    let client = RegistryClient::new(&fixture.config).unwrap();
    let error = session
        .target_from(&client, &fixture.config, "index/foo.json")
        .unwrap_err();
    assert!(error.message().contains("SHA-256"), "{error}");
}

#[test]
fn unlisted_targets_are_rejected() {
    let fixture = fixture();
    install(&fixture, 1, 1, EXPIRES);
    let session = load(&fixture).unwrap();
    let client = RegistryClient::new(&fixture.config).unwrap();
    let error = session
        .target_from(&client, &fixture.config, "index/missing.json")
        .unwrap_err();
    assert!(error.message().contains("does not list"), "{error}");
}
