//! Persistent query cache tests.

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use seal_query::{CACHE_VERSION, DiskCache, QueryKey, QueryKind};

static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

fn temp_dir(label: &str) -> PathBuf {
    let counter = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "seal-query-cache-{label}-{}-{counter}",
        std::process::id()
    ));
    fs::create_dir_all(&path).expect("temporary cache directory");
    path
}

#[test]
fn cache_round_trips_payloads_and_keeps_keys_path_independent() {
    let first_root = temp_dir("roundtrip-a");
    let second_root = temp_dir("roundtrip-b");
    let first = DiskCache::new(&first_root);
    let second = DiskCache::new(&second_root);
    let key = QueryKey::new(QueryKind::CodegenCgu, "cgu0");
    first.store(&key, b"object bytes").expect("store");
    assert_eq!(
        first.load(&key).expect("load"),
        Some(b"object bytes".to_vec())
    );
    assert_eq!(
        first.path_for(&key).file_name(),
        second.path_for(&key).file_name()
    );
    fs::remove_dir_all(first_root).expect("cleanup a");
    fs::remove_dir_all(second_root).expect("cleanup b");
}

#[test]
fn corrupted_payload_is_a_miss_and_is_removed() {
    let root = temp_dir("corrupt");
    let cache = DiskCache::new(&root);
    let key = QueryKey::new(QueryKind::MonoInstance, "instance");
    cache.store(&key, b"payload").expect("store");
    let path = cache.path_for(&key);
    let mut bytes = fs::read(&path).expect("read");
    *bytes.last_mut().expect("payload byte") ^= 0xff;
    fs::write(&path, bytes).expect("write corrupt record");
    assert_eq!(cache.load(&key).expect("corrupt load"), None);
    assert!(!path.exists());
    fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn truncated_record_is_a_miss() {
    let root = temp_dir("truncated");
    let cache = DiskCache::new(&root);
    let key = QueryKey::new(QueryKind::Signature, "signature");
    cache.store(&key, b"payload").expect("store");
    let path = cache.path_for(&key);
    fs::write(&path, b"SEALQ").expect("truncate");
    assert_eq!(cache.load(&key).expect("truncated load"), None);
    fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn old_version_is_a_miss() {
    let root = temp_dir("version");
    let cache = DiskCache::new(&root);
    let key = QueryKey::new(QueryKind::Ast, "module");
    cache.store(&key, b"payload").expect("store");
    let path = cache.path_for(&key);
    let mut bytes = fs::read(&path).expect("read");
    let old = CACHE_VERSION.saturating_sub(1).to_le_bytes();
    bytes[8..12].copy_from_slice(&old);
    fs::write(&path, bytes).expect("write old record");
    assert_eq!(cache.load(&key).expect("old load"), None);
    fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn wrong_key_digest_is_a_miss() {
    let root = temp_dir("wrong-key");
    let cache = DiskCache::new(&root);
    let first = QueryKey::new(QueryKind::Tokens, "first");
    let second = QueryKey::new(QueryKind::Tokens, "second");
    cache.store(&first, b"payload").expect("store");
    let from = cache.path_for(&first);
    let to = cache.path_for(&second);
    fs::rename(&from, &to).expect("move record");
    assert_eq!(cache.load(&second).expect("wrong-key load"), None);
    fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn missing_record_is_a_miss() {
    let root = temp_dir("missing");
    let cache = DiskCache::new(&root);
    assert_eq!(
        cache
            .load(&QueryKey::new(QueryKind::Tokens, "absent"))
            .expect("missing load"),
        None
    );
    fs::remove_dir_all(root).expect("cleanup");
}
