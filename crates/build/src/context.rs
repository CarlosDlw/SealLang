//! Shared state for one action execution.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use std::time::Instant;

use seal_cas::{ActionCache, ActionRecord, Digest};
use seal_query::DiskCache;
use seal_sandbox::Sandbox;

use crate::action::Action;
use crate::error::BuildResult;
use crate::remote::{self, RemoteSession};
use crate::{ActionTiming, ProgressEvent, ProgressFn};

/// Cache, sandbox, and reporting state shared by every worker.
pub(crate) struct ActionContext {
    pub(crate) cache: ActionCache,
    pub(crate) query_cache: DiskCache,
    pub(crate) sandbox: Sandbox,
    pub(crate) temp_root: PathBuf,
    pub(crate) use_cache: bool,
    remote: Option<Arc<RemoteSession>>,
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
        remote: Option<Arc<RemoteSession>>,
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
            remote,
            progress,
            timings: Mutex::new(Vec::new()),
        }
    }

    /// Runs one action, consulting local and remote caches first.
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
                    self.record(action, key, 0, true, false);
                    return Ok(record);
                }
                Ok(None) => {}
                Err(error) => {
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

        let descriptor = self
            .remote
            .as_ref()
            .filter(|session| session.available())
            .map(|_| remote::cache_action(action, key));
        if self.use_cache
            && let (Some(session), Some(descriptor)) = (&self.remote, &descriptor)
            && session.available()
        {
            match session.lookup(descriptor) {
                Ok(Some(record)) => {
                    self.cache.store(key, &record)?;
                    self.emit(ProgressEvent::RemoteHit {
                        kind: action.kind().to_owned(),
                        label: action.label().to_owned(),
                        key: key.to_hex(),
                    });
                    self.record(action, key, 0, true, true);
                    return Ok(record);
                }
                Ok(None) => {}
                Err(failure) => {
                    self.remote_fallback(session, failure)?;
                }
            }
            if session.available() && session.execution() && action.supports_remote_execution() {
                if let Some(request) = action.remote_spec(self)? {
                    self.emit(ProgressEvent::Started {
                        kind: action.kind().to_owned(),
                        label: action.label().to_owned(),
                        key: key.to_hex(),
                        command: action.command(),
                    });
                    let start = Instant::now();
                    match session.execute(action, &request) {
                        Ok(record) => {
                            let millis = elapsed_millis(start);
                            self.cache.store(key, &record)?;
                            if let Err(failure) = session.store(descriptor, &record) {
                                self.remote_store_fallback(session, failure)?;
                            }
                            self.record(action, key, millis, false, true);
                            self.emit(ProgressEvent::Finished {
                                kind: action.kind().to_owned(),
                                label: action.label().to_owned(),
                                key: key.to_hex(),
                                millis,
                            });
                            return Ok(record);
                        }
                        Err(failure)
                            if failure.is_action_failure()
                                || matches!(
                                    failure.kind(),
                                    seal_remote::RemoteErrorKind::Integrity
                                        | seal_remote::RemoteErrorKind::MissingOutput
                                ) =>
                        {
                            return Err(remote::error(session, failure));
                        }
                        Err(failure) => {
                            self.remote_fallback(session, failure)?;
                        }
                    }
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
        let millis = elapsed_millis(start);
        if self.use_cache {
            self.cache.store(key, &record)?;
        }
        if self.use_cache
            && let (Some(session), Some(descriptor)) = (&self.remote, &descriptor)
            && session.available()
            && let Err(failure) = session.store(descriptor, &record)
        {
            self.remote_store_fallback(session, failure)?;
        }
        self.record(action, key, millis, false, false);
        self.emit(ProgressEvent::Finished {
            kind: action.kind().to_owned(),
            label: action.label().to_owned(),
            key: key.to_hex(),
            millis,
        });
        Ok(record)
    }

    /// Returns whether the action with `key` was served from a cache.
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

    fn remote_fallback(
        &self,
        session: &RemoteSession,
        failure: seal_remote::RemoteError,
    ) -> BuildResult<()> {
        let reason = session.message(&failure);
        remote::fallback(session, failure)?;
        self.emit(ProgressEvent::RemoteUnavailable { reason });
        Ok(())
    }

    fn remote_store_fallback(
        &self,
        session: &RemoteSession,
        failure: seal_remote::RemoteError,
    ) -> BuildResult<()> {
        let reason = format!(
            "the remote cache update failed: {}",
            session.message(&failure)
        );
        remote::fallback(session, failure)?;
        self.emit(ProgressEvent::RemoteUnavailable { reason });
        Ok(())
    }

    fn record(&self, action: &Action, key: Digest, millis: u64, cached: bool, remote: bool) {
        self.timings
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(ActionTiming {
                kind: action.kind().to_owned(),
                label: action.label().to_owned(),
                key: key.to_hex(),
                millis,
                cached,
                remote,
            });
    }

    fn emit(&self, event: ProgressEvent) {
        if let Some(progress) = &self.progress {
            progress(event);
        }
    }
}

fn elapsed_millis(start: Instant) -> u64 {
    start.elapsed().as_millis().try_into().unwrap_or(u64::MAX)
}
