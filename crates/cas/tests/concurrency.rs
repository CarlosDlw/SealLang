//! Concurrency, locking, and corruption-recovery tests for the local cache.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use seal_cas::{ActionCache, ActionInput, ActionOutput, ActionRecord, Cas, Digest};

struct TempDir(PathBuf);

impl TempDir {
    fn new(prefix: &str) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "{prefix}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).expect("temporary directory");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn concurrent_writers_do_not_corrupt_objects() {
    let directory = TempDir::new("seal-cas-concurrent");
    let cas = Cas::new(directory.path().join("cache")).expect("the store opens");
    let shared = Arc::new(cas);
    let mut threads = Vec::new();
    for writer in 0..8_u32 {
        let cas = Arc::clone(&shared);
        threads.push(std::thread::spawn(move || {
            for value in 0..32_u32 {
                let bytes = format!("object-{}", value % 4).into_bytes();
                let digest = cas.put_bytes(&bytes).expect("the object is stored");
                assert_eq!(cas.get(digest).expect("the object is readable"), bytes);
                let _ = writer;
            }
        }));
    }
    for thread in threads {
        thread.join().expect("a writer succeeds");
    }
    let stats = shared.stats().expect("the statistics are readable");
    assert_eq!(stats.objects, 4);
    let report = shared.verify().expect("the store verifies");
    assert!(report.corrupt.is_empty());
}

#[test]
fn a_corrupt_object_is_replaced_by_the_next_put() {
    let directory = TempDir::new("seal-cas-corrupt");
    let cas = Cas::new(directory.path().join("cache")).expect("the store opens");
    let digest = cas.put_bytes(b"original").expect("the object is stored");
    let path = cas.object_path(digest);
    std::fs::write(&path, b"tampered").expect("the object is corrupted");
    assert!(cas.get(digest).is_err());
    assert_eq!(cas.verify().expect("the store verifies").corrupt.len(), 1);

    cas.put_bytes(b"original").expect("the object is rewritten");
    assert_eq!(
        cas.get(digest).expect("the object is readable"),
        b"original"
    );
    assert!(cas.verify().expect("the store verifies").corrupt.is_empty());
}

#[test]
fn a_cache_hit_rejects_a_corrupt_output() {
    let directory = TempDir::new("seal-cas-action-corrupt");
    let cas = Cas::new(directory.path().join("cache")).expect("the store opens");
    let cache = ActionCache::new(cas);
    let digest = cache.cas().put_bytes(b"artifact").expect("the object");
    let record = ActionRecord::new(
        "frontend",
        vec![ActionInput {
            path: "main.sl".to_owned(),
            digest: Digest::of_bytes(b"source"),
        }],
        vec![ActionOutput::data("object", digest)],
    );
    let key = ActionCache::key("frontend", &record.inputs, &record.env, &record.command);
    cache.store(key, &record).expect("the record is stored");
    assert!(cache.lookup(key).expect("a valid hit").is_some());

    std::fs::write(cache.cas().object_path(digest), b"damaged").expect("the object is damaged");
    let error = cache.lookup(key).expect_err("the hit is rejected");
    assert_eq!(error.code(), "E8040");
}

#[test]
fn the_lock_excludes_a_second_caller_until_release() {
    let directory = TempDir::new("seal-cas-lock");
    let cas = Arc::new(Cas::new(directory.path().join("cache")).expect("the store opens"));
    let guard = cas.lock("shared").expect("the lock is acquired");
    let other = Arc::clone(&cas);
    let handle = std::thread::spawn(move || {
        let _guard = other.lock("shared").expect("the second lock is acquired");
        std::time::Instant::now()
    });
    std::thread::sleep(Duration::from_millis(50));
    let released = std::time::Instant::now();
    drop(guard);
    let acquired = handle.join().expect("the second caller finishes");
    assert!(acquired >= released);
}

#[test]
fn pruning_removes_least_recently_used_objects() {
    let directory = TempDir::new("seal-cas-prune");
    let cas = Cas::new(directory.path().join("cache")).expect("the store opens");
    let first = cas.put_bytes(b"first").expect("the first object");
    std::thread::sleep(Duration::from_millis(20));
    let second = cas.put_bytes(b"second").expect("the second object");
    let report = cas.prune_lru(5).expect("pruning succeeds");
    assert_eq!(report.removed, 2);
    assert!(!cas.contains(first) && !cas.contains(second));
    assert_eq!(report.remaining_bytes, 0);
}
