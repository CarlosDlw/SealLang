//! Concurrent query execution tests.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Barrier};
use std::thread;

use seal_hash::{StableHasher, stable_hash_of};
use seal_query::{InputKind, QueryDb, QueryKey, QueryKind, QueryOutput};

fn fingerprint(text: &str) -> seal_hash::Fingerprint {
    let mut hasher = StableHasher::new();
    hasher.write_str(text);
    hasher.finish()
}

#[test]
fn concurrent_readers_share_values_without_deadlock() {
    let db = Arc::new(QueryDb::new());
    let source = db
        .set_input(InputKind::Source, "src/main.sl", b"func main() {}")
        .expect("source");
    let input = db.input_dependency(source).expect("dependency");
    let key = QueryKey::new(QueryKind::Ast, "src/main.sl");
    let computed = Arc::new(AtomicUsize::new(0));
    let barrier = Arc::new(Barrier::new(8));

    let mut threads = Vec::new();
    for _ in 0..8 {
        let db = Arc::clone(&db);
        let input = input.clone();
        let key = key.clone();
        let computed = Arc::clone(&computed);
        let barrier = Arc::clone(&barrier);
        threads.push(thread::spawn(move || {
            barrier.wait();
            let output = db
                .query(key, [input], || {
                    computed.fetch_add(1, Ordering::Relaxed);
                    Ok(QueryOutput::new(
                        Arc::<str>::from("ast"),
                        fingerprint("ast"),
                    ))
                })
                .expect("concurrent query");
            assert_eq!(&***output.value(), "ast");
        }));
    }
    for thread in threads {
        thread.join().expect("worker");
    }
    assert!(computed.load(Ordering::Relaxed) >= 1);
    assert_eq!(db.len(), 1);
    assert_eq!(
        db.stats().total(),
        8,
        "every request is either a hit or an execution"
    );
}

#[test]
fn independent_query_chains_run_in_parallel() {
    let db = Arc::new(QueryDb::new());
    let source = db
        .set_input(InputKind::Source, "src/lib.sl", b"module")
        .expect("source");
    let input = db.input_dependency(source).expect("dependency");
    let barrier = Arc::new(Barrier::new(6));
    let mut threads = Vec::new();
    for index in 0..6 {
        let db = Arc::clone(&db);
        let input = input.clone();
        let barrier = Arc::clone(&barrier);
        threads.push(thread::spawn(move || {
            barrier.wait();
            let signature_key = QueryKey::new(QueryKind::Signature, format!("fn{index}"));
            let signature = db
                .query(signature_key.clone(), [input.clone()], || {
                    Ok(QueryOutput::new(
                        format!("sig{index}"),
                        stable_hash_of(&(index as u64)),
                    ))
                })
                .expect("signature");
            let signature_dep = db.query_dependency(&signature_key).expect("signature dep");
            let body = db
                .query(
                    QueryKey::new(QueryKind::BodyHir, format!("fn{index}")),
                    [input, signature_dep],
                    || {
                        Ok(QueryOutput::new(
                            format!("body{index}"),
                            stable_hash_of(&((index as u64) + 100)),
                        ))
                    },
                )
                .expect("body");
            assert_eq!(signature.value().as_str(), format!("sig{index}"));
            assert_eq!(body.value().as_str(), format!("body{index}"));
        }));
    }
    for thread in threads {
        thread.join().expect("worker");
    }
    assert_eq!(db.len(), 12);
}
