//! Measures a serializable code-generation payload through the disk cache.

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use seal_query::{DiskCache, QueryKey, QueryKind};

const ITERATIONS: usize = 5_000;
const PAYLOAD_SIZE: usize = 256 * 1024;

static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

fn temp_dir() -> PathBuf {
    let counter = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "seal-query-build-bench-{}-{counter}",
        std::process::id()
    ));
    fs::create_dir_all(&path).expect("bench cache directory");
    path
}

fn per_iteration(elapsed: Duration) -> f64 {
    elapsed.as_secs_f64() * 1_000_000.0 / ITERATIONS as f64
}

fn payload() -> Vec<u8> {
    (0..PAYLOAD_SIZE).map(|index| (index % 251) as u8).collect()
}

fn main() {
    let root = temp_dir();
    let cache = DiskCache::new(&root);
    let key = QueryKey::new(QueryKind::CodegenCgu, "cgu0");
    let bytes = payload();
    cache.store(&key, &bytes).expect("warm cache");

    let direct_start = Instant::now();
    for _ in 0..ITERATIONS {
        let generated = payload();
        std::hint::black_box(&generated);
    }
    let direct = per_iteration(direct_start.elapsed());

    let cache_start = Instant::now();
    for _ in 0..ITERATIONS {
        let loaded = cache.load(&key).expect("cache load").expect("cache hit");
        std::hint::black_box(&loaded);
    }
    let cached = per_iteration(cache_start.elapsed());

    println!("build cgu_generate_us={direct:.3}");
    println!("build cgu_cache_hit_us={cached:.3}");
    println!(
        "build cgu_speedup={:.2}x",
        if cached > 0.0 { direct / cached } else { 0.0 }
    );
    fs::remove_dir_all(root).expect("cleanup bench cache");
}
