//! Reusable compiler sessions for in-process incremental builds.
//!
//! A watch or IDE request runs many compilations in one process. Creating a new
//! [`Session`] for every edit would discard the query database and force the
//! frontend, declarations, and every unchanged body to recompute. Sessions are
//! keyed by tracked options plus the diagnostic presentation (the emitter is
//! fixed at construction).
//!
//! A session retains one source generation per file version, because cached
//! values hold spans into those generations. To keep a long watch bounded, the
//! entry is replaced with a fresh session once it accumulates too many source
//! generations. Cached values from the old session become unreachable with it;
//! the next request recomputes from a clean database.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use seal_session::{Options, Session};

/// Maximum source generations retained before a fresh session replaces the
/// cached one. Each edit of a changed file adds one generation.
const MAX_SOURCE_GENERATIONS: usize = 128;

struct CachedSession {
    session: Arc<Mutex<Session>>,
}

fn cache() -> &'static Mutex<HashMap<String, CachedSession>> {
    static CACHE: OnceLock<Mutex<HashMap<String, CachedSession>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Returns the reusable session for `options`, creating or replacing it when
/// necessary.
pub(crate) fn acquire(options: &Options) -> Result<Arc<Mutex<Session>>, String> {
    let key = format!(
        "{}:{:?}:{:?}:{}",
        options.tracked_fingerprint(),
        options.error_format,
        options.color,
        options.time_passes
    );
    let mut sessions = cache()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(entry) = sessions.get(&key)
        && entry
            .session
            .lock()
            .is_ok_and(|session| session.source_generations() <= MAX_SOURCE_GENERATIONS)
    {
        return Ok(Arc::clone(&entry.session));
    }
    let session = Arc::new(Mutex::new(
        Session::new(options.clone()).map_err(|error| error.to_string())?,
    ));
    sessions.insert(
        key,
        CachedSession {
            session: Arc::clone(&session),
        },
    );
    Ok(session)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_options_reuse_one_session() {
        let options = Options::default();
        let first = acquire(&options).expect("first session");
        let second = acquire(&options).expect("second session");
        assert!(Arc::ptr_eq(&first, &second));
    }
}
