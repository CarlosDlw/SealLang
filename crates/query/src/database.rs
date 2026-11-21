//! The memoizing, revisioned query database.

use std::any::Any;
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard};

use seal_hash::{Fingerprint, StableHasher};

use crate::error::QueryError;
use crate::key::{InputId, InputKind, QueryKey, QueryKind};
use crate::output::{QueryOutcome, QueryOutput};
use crate::revision::{Dependency, InputStamp, QueryStamp, Revision};

thread_local! {
    static ACTIVE: RefCell<Vec<QueryKey>> = const { RefCell::new(Vec::new()) };
}

struct InputRecord {
    stamp: InputStamp,
    bytes: Arc<[u8]>,
}

struct QueryRecord {
    value: Arc<dyn Any + Send + Sync>,
    fingerprint: Fingerprint,
    dependencies: Vec<Dependency>,
    changed_at: Revision,
}

#[derive(Default)]
struct Inner {
    inputs: Vec<InputRecord>,
    input_ids: HashMap<(InputKind, String), InputId>,
    queries: HashMap<QueryKey, QueryRecord>,
    revision: Revision,
    stats: QueryStats,
}

/// Counters for one query kind.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KindStats {
    /// Requests served from the memo table.
    pub hits: u64,
    /// Requests that ran the computation.
    pub misses: u64,
    /// Recomputed queries whose fingerprint did not change.
    pub backdated: u64,
}

/// A snapshot of query activity.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct QueryStats {
    /// Requests served from the memo table.
    pub hits: u64,
    /// Requests that ran the computation.
    pub misses: u64,
    /// Recomputed queries whose fingerprint did not change.
    pub backdated: u64,
    /// Inputs whose bytes changed.
    pub input_changes: u64,
    /// Per-kind counters.
    pub by_kind: BTreeMap<QueryKind, KindStats>,
}

impl QueryStats {
    /// Returns the total number of query requests.
    pub fn total(&self) -> u64 {
        self.hits.saturating_add(self.misses)
    }
}

/// A memoizing query database.
///
/// Cloning a `QueryDb` shares the same database. The database is safe to use
/// from multiple threads; a value must be `Send + Sync` to be stored.
#[derive(Clone, Default)]
pub struct QueryDb {
    inner: Arc<Mutex<Inner>>,
}

impl QueryDb {
    /// Creates an empty database.
    pub fn new() -> Self {
        Self::default()
    }

    /// Installs or updates an input.
    ///
    /// The input identity is `(kind, name)`. Reinstalling identical bytes keeps
    /// the previous revision. Installing different bytes advances the global
    /// revision exactly once and invalidates every query whose recorded
    /// dependency points at the old bytes.
    pub fn set_input(
        &self,
        kind: InputKind,
        name: impl Into<String>,
        bytes: impl AsRef<[u8]>,
    ) -> Result<InputId, QueryError> {
        let name = name.into();
        if name.is_empty() {
            return Err(QueryError::EmptyInputName);
        }
        let bytes = bytes.as_ref();
        let digest = digest_bytes(bytes);
        let mut inner = self.lock();
        let identity = (kind, name.clone());
        if let Some(&id) = inner.input_ids.get(&identity) {
            if inner.inputs[id.index()].stamp.digest() != digest {
                inner.revision = next_revision(inner.revision);
                let revision = inner.revision;
                let record = &mut inner.inputs[id.index()];
                record.stamp = InputStamp::new(id, kind, name, revision, digest);
                record.bytes = Arc::from(bytes);
                inner.stats.input_changes = inner.stats.input_changes.saturating_add(1);
            }
            return Ok(id);
        }
        let raw = u32::try_from(inner.inputs.len()).map_err(|_| QueryError::TooManyInputs)?;
        let id = InputId::from_raw(raw);
        inner.revision = next_revision(inner.revision);
        let stamp = InputStamp::new(id, kind, name.clone(), inner.revision, digest);
        inner.inputs.push(InputRecord {
            stamp,
            bytes: Arc::from(bytes),
        });
        inner.input_ids.insert(identity, id);
        inner.stats.input_changes = inner.stats.input_changes.saturating_add(1);
        Ok(id)
    }

    /// Returns the current snapshot for `input`.
    pub fn input(&self, input: InputId) -> Result<InputStamp, QueryError> {
        self.lock()
            .inputs
            .get(input.index())
            .map(|record| record.stamp.clone())
            .ok_or(QueryError::UnknownInput(input))
    }

    /// Returns the current bytes for `input`.
    pub fn input_bytes(&self, input: InputId) -> Result<Arc<[u8]>, QueryError> {
        self.lock()
            .inputs
            .get(input.index())
            .map(|record| Arc::clone(&record.bytes))
            .ok_or(QueryError::UnknownInput(input))
    }

    /// Returns a dependency stamp for the current state of `input`.
    pub fn input_dependency(&self, input: InputId) -> Result<Dependency, QueryError> {
        Ok(Dependency::Input(self.input(input)?))
    }

    /// Returns a dependency stamp for the current result of `key`.
    pub fn query_dependency(&self, key: &QueryKey) -> Result<Dependency, QueryError> {
        let inner = self.lock();
        inner
            .queries
            .get(key)
            .map(|record| Dependency::Query(QueryStamp::new(key.clone(), record.fingerprint)))
            .ok_or_else(|| QueryError::MissingDependency(key.clone()))
    }

    /// Returns the current global revision.
    pub fn revision(&self) -> Revision {
        self.lock().revision
    }

    /// Returns the number of memoized queries.
    pub fn len(&self) -> usize {
        self.lock().queries.len()
    }

    /// Returns whether no derived query is memoized.
    pub fn is_empty(&self) -> bool {
        self.lock().queries.is_empty()
    }

    /// Removes every memoized query.
    ///
    /// Inputs and their revisions are retained. Removing a query invalidates
    /// dependents that recorded it, because a missing dependency cannot be
    /// validated.
    pub fn clear(&self) {
        self.lock().queries.clear();
    }

    /// Returns a snapshot of the query counters.
    pub fn stats(&self) -> QueryStats {
        self.lock().stats.clone()
    }

    /// Resets the query counters without touching inputs or memoized values.
    pub fn reset_stats(&self) {
        self.lock().stats = QueryStats::default();
    }

    /// Returns a memoized value or computes it.
    ///
    /// `dependencies` must contain every input and derived query the
    /// computation reads. `compute` runs without the database lock. A result
    /// with the same fingerprint as the previous one is backdated: dependents
    /// remain valid. The closure may call [`QueryDb::query`] for other keys;
    /// a recursive path returns [`QueryError::Cycle`].
    pub fn query<V, F>(
        &self,
        key: QueryKey,
        dependencies: impl IntoIterator<Item = Dependency>,
        compute: F,
    ) -> Result<QueryOutcome<V>, QueryError>
    where
        V: Any + Send + Sync,
        F: FnOnce() -> Result<QueryOutput<V>, QueryError>,
    {
        let _active = ActiveGuard::enter(&key)?;
        let dependencies: Vec<Dependency> = dependencies.into_iter().collect();

        let cached = {
            let mut inner = self.lock();
            match inner.queries.get(&key) {
                Some(record) if !record.value.is::<V>() => {
                    return Err(QueryError::TypeMismatch { key });
                }
                Some(record)
                    if record.dependencies == dependencies
                        && valid(&inner, &record.dependencies) =>
                {
                    let value = Arc::clone(&record.value);
                    let fingerprint = record.fingerprint;
                    record_stats(&mut inner.stats, key.kind(), true, false);
                    Some((value, fingerprint))
                }
                _ => None,
            }
        };

        if let Some((value, fingerprint)) = cached {
            let value = value
                .downcast::<V>()
                .map_err(|_| QueryError::TypeMismatch { key })?;
            return Ok(QueryOutcome::new(value, fingerprint, false, false));
        }

        let output = compute()?;
        let (value, fingerprint) = output.into_parts();
        if fingerprint == Fingerprint::ZERO {
            return Err(QueryError::InvalidFingerprint { key });
        }
        let value: Arc<dyn Any + Send + Sync> = Arc::new(value);

        let mut inner = self.lock();
        let (backdated, changed_at) = match inner.queries.get(&key) {
            Some(previous) => {
                if !previous.value.is::<V>() {
                    return Err(QueryError::TypeMismatch { key });
                }
                (
                    previous.fingerprint == fingerprint,
                    if previous.fingerprint == fingerprint {
                        previous.changed_at
                    } else {
                        inner.revision
                    },
                )
            }
            None => (false, inner.revision),
        };
        inner.queries.insert(
            key.clone(),
            QueryRecord {
                value: Arc::clone(&value),
                fingerprint,
                dependencies,
                changed_at,
            },
        );
        record_stats(&mut inner.stats, key.kind(), false, backdated);
        let value = value
            .downcast::<V>()
            .map_err(|_| QueryError::TypeMismatch { key })?;
        Ok(QueryOutcome::new(value, fingerprint, true, backdated))
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl fmt::Debug for QueryDb {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let inner = self.lock();
        f.debug_struct("QueryDb")
            .field("inputs", &inner.inputs.len())
            .field("queries", &inner.queries.len())
            .field("revision", &inner.revision)
            .finish_non_exhaustive()
    }
}

fn valid(inner: &Inner, dependencies: &[Dependency]) -> bool {
    dependencies.iter().all(|dependency| match dependency {
        Dependency::Input(stamp) => inner
            .inputs
            .get(stamp.id().index())
            .is_some_and(|record| record.stamp.digest() == stamp.digest()),
        Dependency::Query(stamp) => inner
            .queries
            .get(stamp.key())
            .is_some_and(|record| record.fingerprint == stamp.fingerprint()),
    })
}

fn record_stats(stats: &mut QueryStats, kind: QueryKind, hit: bool, backdated: bool) {
    let kind = stats.by_kind.entry(kind).or_default();
    if hit {
        stats.hits = stats.hits.saturating_add(1);
        kind.hits = kind.hits.saturating_add(1);
    } else {
        stats.misses = stats.misses.saturating_add(1);
        kind.misses = kind.misses.saturating_add(1);
    }
    if backdated {
        stats.backdated = stats.backdated.saturating_add(1);
        kind.backdated = kind.backdated.saturating_add(1);
    }
}

fn next_revision(revision: Revision) -> Revision {
    Revision::from_raw(revision.as_raw().saturating_add(1))
}

fn digest_bytes(bytes: &[u8]) -> Fingerprint {
    let mut hasher = StableHasher::new();
    hasher.write_bytes(bytes);
    hasher.finish()
}

struct ActiveGuard {
    key: QueryKey,
}

impl ActiveGuard {
    fn enter(key: &QueryKey) -> Result<Self, QueryError> {
        ACTIVE.with(|active| {
            let mut stack = active.borrow_mut();
            if stack.iter().any(|active| active == key) {
                let mut path = stack.clone();
                path.push(key.clone());
                return Err(QueryError::Cycle { path });
            }
            stack.push(key.clone());
            Ok(Self { key: key.clone() })
        })
    }
}

impl Drop for ActiveGuard {
    fn drop(&mut self) {
        ACTIVE.with(|active| {
            let mut stack = active.borrow_mut();
            if let Some(position) = stack.iter().rposition(|key| key == &self.key) {
                stack.remove(position);
            }
        });
    }
}
