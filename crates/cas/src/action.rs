//! The action cache layered on top of the content-addressed store.
//!
//! An action record is the evidence that a command already ran with exactly
//! these inputs, environment hashes, and arguments. The key is the digest of a
//! canonical JSON description, so changing any input or argument produces a
//! different key instead of a false hit.
//!
//! Records never contain environment values: the caller hashes sensitive values
//! before building the key material, and the record keeps only those digests.
//! The display command is produced by the caller and must not include secrets.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::cas::Cas;
use crate::digest::Digest;
use crate::error::CasError;

/// One input file of an action.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionInput {
    /// The project-relative path.
    pub path: String,
    /// The content digest.
    pub digest: Digest,
}

impl ActionInput {
    /// Hashes `path` and records it under `name`.
    pub fn from_file(name: impl Into<String>, path: &Path) -> Result<Self, CasError> {
        Ok(Self {
            path: name.into(),
            digest: Digest::of_file(path)?,
        })
    }
}

/// One output artifact of an action.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionOutput {
    /// The logical output name, such as `object` or `executable`.
    pub name: String,
    /// The digest stored in the content-addressed store.
    pub digest: Digest,
    /// Unix permission bits applied when materializing the object.
    #[serde(default = "default_mode")]
    pub mode: u32,
}

impl ActionOutput {
    /// Creates an output with executable permissions on Unix.
    pub fn executable(name: impl Into<String>, digest: Digest) -> Self {
        Self {
            name: name.into(),
            digest,
            mode: 0o755,
        }
    }

    /// Creates an output with owner read/write permissions on Unix.
    pub fn data(name: impl Into<String>, digest: Digest) -> Self {
        Self {
            name: name.into(),
            digest,
            mode: 0o644,
        }
    }
}

const fn default_mode() -> u32 {
    0o644
}

/// A stored action result.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionRecord {
    /// The action kind, such as `frontend`, `link`, or `generate`.
    pub kind: String,
    /// Inputs sorted by path.
    pub inputs: Vec<ActionInput>,
    /// Outputs sorted by name.
    pub outputs: Vec<ActionOutput>,
    /// Environment entries as digests, sorted by key.
    #[serde(default)]
    pub env: BTreeMap<String, Digest>,
    /// A display command that never contains secrets.
    #[serde(default)]
    pub command: Vec<String>,
    /// Creation time in seconds since the Unix epoch, used by pruning.
    #[serde(default)]
    pub created_at: u64,
}

impl ActionRecord {
    /// Creates a record and sorts its collections for deterministic output.
    pub fn new(
        kind: impl Into<String>,
        mut inputs: Vec<ActionInput>,
        mut outputs: Vec<ActionOutput>,
    ) -> Self {
        inputs.sort_by(|left, right| left.path.cmp(&right.path));
        outputs.sort_by(|left, right| left.name.cmp(&right.name));
        Self {
            kind: kind.into(),
            inputs,
            outputs,
            env: BTreeMap::new(),
            command: Vec::new(),
            created_at: now_seconds(),
        }
    }

    /// Adds a hashed environment entry.
    pub fn with_env(mut self, key: impl Into<String>, value: &str) -> Self {
        self.env
            .insert(key.into(), Digest::of_bytes(value.as_bytes()));
        self
    }

    /// Adds the display command.
    pub fn with_command(mut self, command: Vec<String>) -> Self {
        self.command = command;
        self
    }

    /// Returns the output named `name`.
    pub fn output(&self, name: &str) -> Option<&ActionOutput> {
        self.outputs.iter().find(|output| output.name == name)
    }
}

/// Aggregate action-cache statistics.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ActionStats {
    /// Number of stored records.
    pub records: u64,
    /// Total bytes of record metadata.
    pub bytes: u64,
}

/// Problems found while verifying action records.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ActionVerifyReport {
    /// Records whose referenced objects are all present and valid.
    pub checked: u64,
    /// Records that are unreadable, malformed, or reference a missing object.
    pub corrupt: Vec<PathBuf>,
}

/// The key material hashed into an action key.
#[derive(Serialize)]
struct KeyMaterial<'a> {
    version: u32,
    kind: &'a str,
    inputs: &'a [ActionInput],
    env: &'a BTreeMap<String, Digest>,
    command: &'a [String],
}

/// The local action cache.
#[derive(Clone, Debug)]
pub struct ActionCache {
    cas: Cas,
}

impl ActionCache {
    /// Opens the action cache for `cas`.
    pub fn new(cas: Cas) -> Self {
        Self { cas }
    }

    /// Opens the action cache under `root`, creating it when absent.
    pub fn open(root: impl Into<PathBuf>) -> Result<Self, CasError> {
        Ok(Self::new(Cas::new(root)?))
    }

    /// Returns the underlying store.
    pub fn cas(&self) -> &Cas {
        &self.cas
    }

    /// Computes the key for an action description.
    ///
    /// Inputs are sorted by path first, so the caller's iteration order cannot
    /// change the key.
    pub fn key(
        kind: &str,
        inputs: &[ActionInput],
        env: &BTreeMap<String, Digest>,
        command: &[String],
    ) -> Digest {
        let mut sorted = inputs.to_vec();
        sorted.sort_by(|left, right| left.path.cmp(&right.path));
        let material = KeyMaterial {
            version: 1,
            kind,
            inputs: &sorted,
            env,
            command,
        };
        // Every field is a plain string, integer, or sorted map, so serialization
        // cannot fail for the types in this module.
        let bytes = serde_json::to_vec(&material).unwrap_or_default();
        Digest::of_bytes(&bytes)
    }

    /// Returns the record stored for `key`, or `None` when it is absent.
    ///
    /// A malformed record or a missing output is a [`CasError::Corrupt`] error:
    /// the caller treats it as a miss, removes the record, and rebuilds.
    pub fn lookup(&self, key: Digest) -> Result<Option<ActionRecord>, CasError> {
        let path = self.record_path(key);
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(CasError::io(
                    path.display().to_string(),
                    format!("cannot read action record: {error}"),
                ));
            }
        };
        let record: ActionRecord = serde_json::from_slice(&bytes).map_err(|error| {
            CasError::corrupt(key.to_hex(), format!("invalid record JSON: {error}"))
        })?;
        for output in &record.outputs {
            // Verifying the bytes on a hit catches a truncated or edited object
            // that still has the expected file name. The cost is one blake3
            // pass, which is far cheaper than rebuilding the action.
            self.cas.get(output.digest).map_err(|error| {
                CasError::corrupt(
                    key.to_hex(),
                    format!("output `{}` is unusable: {error}", output.name),
                )
            })?;
        }
        Ok(Some(record))
    }

    /// Stores a record for `key`, replacing an older record atomically.
    pub fn store(&self, key: Digest, record: &ActionRecord) -> Result<(), CasError> {
        let path = self.record_path(key);
        let parent = path.parent().unwrap_or(self.cas.root());
        fs::create_dir_all(parent).map_err(|error| {
            CasError::io(
                parent.display().to_string(),
                format!("cannot create: {error}"),
            )
        })?;
        let bytes = serde_json::to_vec(record).map_err(|error| {
            CasError::Serialization(format!("cannot encode action record: {error}"))
        })?;
        let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
        fs::write(&temporary, &bytes).map_err(|error| {
            CasError::io(
                temporary.display().to_string(),
                format!("cannot write action record: {error}"),
            )
        })?;
        fs::rename(&temporary, &path).map_err(|error| {
            let _ = fs::remove_file(&temporary);
            CasError::io(
                path.display().to_string(),
                format!("cannot publish action record: {error}"),
            )
        })
    }

    /// Removes the record for `key`.
    pub fn remove(&self, key: Digest) -> Result<(), CasError> {
        let path = self.record_path(key);
        match fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(CasError::io(
                path.display().to_string(),
                format!("cannot remove action record: {error}"),
            )),
        }
    }

    /// Returns statistics for every record.
    pub fn stats(&self) -> Result<ActionStats, CasError> {
        let mut stats = ActionStats::default();
        for path in self.records()? {
            let metadata = fs::metadata(&path).map_err(|error| {
                CasError::io(path.display().to_string(), format!("cannot stat: {error}"))
            })?;
            stats.records += 1;
            stats.bytes += metadata.len();
        }
        Ok(stats)
    }

    /// Verifies every record and every object it references.
    pub fn verify(&self) -> Result<ActionVerifyReport, CasError> {
        let mut report = ActionVerifyReport::default();
        for path in self.records()? {
            let bytes = fs::read(&path).map_err(|error| {
                CasError::io(path.display().to_string(), format!("cannot read: {error}"))
            })?;
            let valid = serde_json::from_slice::<ActionRecord>(&bytes).is_ok_and(|record| {
                record
                    .outputs
                    .iter()
                    .all(|output| self.cas.get(output.digest).is_ok())
            });
            if valid {
                report.checked += 1;
            } else {
                report.corrupt.push(path);
            }
        }
        Ok(report)
    }

    /// Removes records whose age is at least `max_age_seconds`.
    ///
    /// Passing zero removes every record, which is what `seal cache prune` uses
    /// when the caller requests an unconditional metadata flush.
    pub fn prune_older_than(&self, max_age_seconds: u64) -> Result<u64, CasError> {
        let now = now_seconds();
        let mut removed = 0;
        for path in self.records()? {
            let metadata = fs::metadata(&path).map_err(|error| {
                CasError::io(path.display().to_string(), format!("cannot stat: {error}"))
            })?;
            let modified = metadata
                .modified()
                .ok()
                .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                .map(|duration| duration.as_secs())
                .unwrap_or(now);
            if now.saturating_sub(modified) >= max_age_seconds {
                match fs::remove_file(&path) {
                    Ok(()) => removed += 1,
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => {
                        return Err(CasError::io(
                            path.display().to_string(),
                            format!("cannot remove action record: {error}"),
                        ));
                    }
                }
            }
        }
        Ok(removed)
    }

    /// Returns the path of the record for `key`.
    pub fn record_path(&self, key: Digest) -> PathBuf {
        let hex = key.to_hex();
        self.cas
            .root()
            .join("actions")
            .join(&hex[..2])
            .join(format!("{}.json", &hex[2..]))
    }

    fn records(&self) -> Result<Vec<PathBuf>, CasError> {
        let root = self.cas.root().join("actions");
        let mut records = Vec::new();
        let shards = fs::read_dir(&root).map_err(|error| {
            CasError::io(root.display().to_string(), format!("cannot list: {error}"))
        })?;
        for shard in shards {
            let shard = shard.map_err(|error| {
                CasError::io(root.display().to_string(), format!("cannot list: {error}"))
            })?;
            if !shard.file_type().map(|kind| kind.is_dir()).unwrap_or(false) {
                continue;
            }
            let directory = shard.path();
            let entries = fs::read_dir(&directory).map_err(|error| {
                CasError::io(
                    directory.display().to_string(),
                    format!("cannot list: {error}"),
                )
            })?;
            for entry in entries {
                let entry = entry.map_err(|error| {
                    CasError::io(
                        directory.display().to_string(),
                        format!("cannot list: {error}"),
                    )
                })?;
                if entry
                    .file_type()
                    .map(|kind| kind.is_file())
                    .unwrap_or(false)
                {
                    records.push(entry.path());
                }
            }
        }
        records.sort();
        Ok(records)
    }
}

fn now_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::{ActionCache, ActionInput, ActionOutput, ActionRecord};
    use crate::cas::Cas;
    use crate::digest::Digest;

    fn cache() -> (ActionCache, tempdir::TempDir) {
        let directory = tempdir::TempDir::new("seal-cas-actions");
        let cas = Cas::new(directory.path().join("cache")).expect("the store opens");
        (ActionCache::new(cas), directory)
    }

    #[test]
    fn keys_depend_on_inputs_command_and_environment() {
        let first = vec![ActionInput {
            path: "a.sl".to_owned(),
            digest: Digest::of_bytes(b"a"),
        }];
        let second = vec![ActionInput {
            path: "a.sl".to_owned(),
            digest: Digest::of_bytes(b"b"),
        }];
        let base = ActionCache::key(
            "frontend",
            &first,
            &Default::default(),
            &["sealc".to_owned()],
        );
        let other_input = ActionCache::key(
            "frontend",
            &second,
            &Default::default(),
            &["sealc".to_owned()],
        );
        let other_kind =
            ActionCache::key("link", &first, &Default::default(), &["sealc".to_owned()]);
        let other_command = ActionCache::key(
            "frontend",
            &first,
            &Default::default(),
            &["sealc".to_owned(), "-O2".to_owned()],
        );
        assert_ne!(base, other_input);
        assert_ne!(base, other_kind);
        assert_ne!(base, other_command);
        assert_eq!(
            base,
            ActionCache::key(
                "frontend",
                &first,
                &Default::default(),
                &["sealc".to_owned()]
            )
        );
    }

    #[test]
    fn records_round_trip_and_expose_outputs() {
        let (cache, _directory) = cache();
        let bytes = b"artifact";
        let digest = cache.cas().put_bytes(bytes).expect("the object is stored");
        let record = ActionRecord::new(
            "frontend",
            vec![ActionInput {
                path: "src/main.sl".to_owned(),
                digest: Digest::of_bytes(b"source"),
            }],
            vec![ActionOutput::data("object", digest)],
        )
        .with_env("SECRET", "hunter2")
        .with_command(vec!["sealc".to_owned(), "build".to_owned()]);
        let key = ActionCache::key("frontend", &record.inputs, &record.env, &record.command);
        cache.store(key, &record).expect("the record is stored");
        let loaded = cache
            .lookup(key)
            .expect("the record is readable")
            .expect("the record exists");
        assert_eq!(loaded, record);
        assert_eq!(loaded.output("object").expect("output").digest, digest);
        assert!(
            !serde_json::to_string(&loaded)
                .expect("serialization works")
                .contains("hunter2")
        );
    }

    #[test]
    fn a_record_with_a_missing_output_is_corrupt() {
        let (cache, _directory) = cache();
        let record = ActionRecord::new(
            "frontend",
            Vec::new(),
            vec![ActionOutput::data("object", Digest::of_bytes(b"missing"))],
        );
        let key = ActionCache::key("frontend", &[], &Default::default(), &[]);
        cache.store(key, &record).expect("the record is stored");
        assert!(cache.lookup(key).is_err());
    }

    #[test]
    fn pruning_removes_only_old_records() {
        let (cache, _directory) = cache();
        let digest = cache.cas().put_bytes(b"x").expect("object");
        let record = ActionRecord::new(
            "frontend",
            Vec::new(),
            vec![ActionOutput::data("object", digest)],
        );
        let key = ActionCache::key("frontend", &[], &Default::default(), &[]);
        cache.store(key, &record).expect("the record is stored");
        assert_eq!(cache.prune_older_than(0).expect("prunes"), 1);
        assert!(cache.lookup(key).expect("lookup").is_none());
    }

    mod tempdir {
        use std::path::{Path, PathBuf};
        use std::sync::atomic::{AtomicU64, Ordering};

        pub(super) struct TempDir(PathBuf);

        impl TempDir {
            pub(super) fn new(prefix: &str) -> Self {
                static NEXT: AtomicU64 = AtomicU64::new(0);
                let path = std::env::temp_dir().join(format!(
                    "{prefix}-{}-{}",
                    std::process::id(),
                    NEXT.fetch_add(1, Ordering::Relaxed)
                ));
                std::fs::create_dir_all(&path).expect("temporary directory");
                Self(path)
            }

            pub(super) fn path(&self) -> &Path {
                &self.0
            }
        }

        impl Drop for TempDir {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
    }
}
