//! Shared state for one action execution.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use std::time::Instant;

use seal_cas::{ActionCache, ActionRecord, Digest};
use seal_query::DiskCache;
use seal_sandbox::Sandbox;

use crate::action::Action;
use crate::error::BuildResult;
use crate::{ActionTiming, ProgressEvent, ProgressFn};

/// Cache, sandbox, and reporting state shared by every worker.
pub(crate) struct ActionContext {
    pub(crate) cache: ActionCache,
    pub(crate) query_cache: DiskCache,
    pub(crate) sandbox: Sandbox,
    pub(crate) temp_root: PathBuf,
    pub(crate) use_cache: bool,
    progress: Option<Arc<ProgressFn>>,
    timings: Mutex<Vec<ActionTiming>>,
}

impl ActionContext {
    pub(crate) fn new(
        cache: ActionCache,
        sandbox: Sandbox,
        temp_root: PathBuf,
        use_cache: bool,
        progress: Option<Arc<ProgressFn>>,
    ) -> Self {
        let query_cache = DiskCache::new(
            temp_root
                .parent()
                .map_or_else(|| temp_root.clone(), |root| root.join("query-payloads")),
        );
        Self {
            cache,
            query_cache,
            sandbox,
            temp_root,
            use_cache,
            progress,
            timings: Mutex::new(Vec::new()),
        }
    }

    /// Runs one action, consulting and updating the cache.
    pub(crate) fn run(&self, key: Digest, action: &Action) -> BuildResult<ActionRecord> {
        if self.use_cache {
            match self.cache.lookup(key) {
                Ok(Some(record)) => {
                    self.emit(ProgressEvent::Cached {
                        kind: action.kind().to_owned(),
                        label: action.label().to_owned(),
                        key: key.to_hex(),
                        command: action.command(),
                    });
                    self.timings
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .push(ActionTiming {
                            kind: action.kind().to_owned(),
                            label: action.label().to_owned(),
                            key: key.to_hex(),
                            millis: 0,
                            cached: true,
                        });
                    return Ok(record);
                }
                Ok(None) => {}
                Err(error) => {
                    // A corrupt record is a miss, not a build failure: drop it
                    // and let the action rebuild its output.
                    let _ = self.cache.remove(key);
                    self.emit(ProgressEvent::CacheCorrupt {
                        kind: action.kind().to_owned(),
                        label: action.label().to_owned(),
                        key: key.to_hex(),
                        reason: error.to_string(),
                    });
                }
            }
        }
        self.emit(ProgressEvent::Started {
            kind: action.kind().to_owned(),
            label: action.label().to_owned(),
            key: key.to_hex(),
            command: action.command(),
        });
        let start = Instant::now();
        let record = action.execute(self)?;
        let millis = start.elapsed().as_millis().try_into().unwrap_or(u64::MAX);
        if self.use_cache {
            self.cache.store(key, &record)?;
        }
        self.timings
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(ActionTiming {
                kind: action.kind().to_owned(),
                label: action.label().to_owned(),
                key: key.to_hex(),
                millis,
                cached: false,
            });
        self.emit(ProgressEvent::Finished {
            kind: action.kind().to_owned(),
            label: action.label().to_owned(),
            key: key.to_hex(),
            millis,
        });
        Ok(record)
    }

    /// Returns whether the action with `key` was served from the cache.
    pub(crate) fn was_cached(&self, key: Digest) -> bool {
        let hex = key.to_hex();
        self.timings
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .iter()
            .find(|timing| timing.key == hex)
            .is_some_and(|timing| timing.cached)
    }

    /// Returns the timings collected so far.
    pub(crate) fn timings(&self) -> Vec<ActionTiming> {
        let mut timings = self
            .timings
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        timings.sort_by(|left, right| {
            left.kind
                .cmp(&right.kind)
                .then_with(|| left.label.cmp(&right.label))
                .then_with(|| left.key.cmp(&right.key))
        });
        timings
    }

    fn emit(&self, event: ProgressEvent) {
        if let Some(progress) = &self.progress {
            progress(event);
        }
    }
}
