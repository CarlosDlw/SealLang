//! The on-disk content-addressed store.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crate::digest::Digest;
use crate::error::CasError;

/// Directory name used for the project cache.
pub const DEFAULT_CACHE_DIR: &str = "seal/cache";

/// A content-addressed object store.
#[derive(Clone, Debug)]
pub struct Cas {
    root: PathBuf,
}

/// Aggregate cache statistics.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CasStats {
    /// Number of stored objects.
    pub objects: u64,
    /// Total bytes of stored objects.
    pub bytes: u64,
}

/// Result of verifying every object in a CAS.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VerifyReport {
    /// Objects whose content matches their key.
    pub checked: u64,
    /// Objects whose content does not match their key.
    pub corrupt: Vec<CorruptObject>,
}

/// One object whose content does not match its key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CorruptObject {
    /// The path on disk.
    pub path: PathBuf,
    /// The expected digest.
    pub expected: String,
    /// The actual digest.
    pub actual: String,
}

/// Result of an LRU prune.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PruneReport {
    /// Objects removed.
    pub removed: u64,
    /// Bytes reclaimed.
    pub bytes: u64,
    /// Object bytes still present.
    pub remaining_bytes: u64,
}

impl Cas {
    /// Opens or creates a CAS at `root`.
    ///
    /// The store has a private directory layout: `objects/` for content,
    /// `actions/` for action records, `locks/` for advisory locks, and `tmp/`
    /// for atomic staging. Directories are owner-only on Unix.
    pub fn new(root: impl Into<PathBuf>) -> Result<Self, CasError> {
        let root = root.into();
        for directory in ["objects", "actions", "locks", "tmp"] {
            let path = root.join(directory);
            fs::create_dir_all(&path).map_err(|error| {
                CasError::io(
                    path.display().to_string(),
                    format!("cannot create: {error}"),
                )
            })?;
            make_private_directory(&path)?;
        }
        Ok(Self { root })
    }

    /// Returns the cache root.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Returns the object path for a digest.
    pub fn object_path(&self, digest: Digest) -> PathBuf {
        let hex = digest.to_hex();
        self.root.join("objects").join(&hex[..2]).join(&hex[2..])
    }

    /// Returns whether an object exists.
    pub fn contains(&self, digest: Digest) -> bool {
        self.object_path(digest).is_file()
    }

    /// Stores bytes and returns their digest.
    pub fn put_bytes(&self, bytes: &[u8]) -> Result<Digest, CasError> {
        let digest = Digest::of_bytes(bytes);
        self.store_bytes(digest, bytes)?;
        Ok(digest)
    }

    /// Stores a file and returns its digest.
    pub fn put_file(&self, path: &Path) -> Result<Digest, CasError> {
        let bytes = fs::read(path).map_err(|error| {
            CasError::io(path.display().to_string(), format!("cannot read: {error}"))
        })?;
        self.put_bytes(&bytes)
    }

    /// Reads an object after verifying its content against its key.
    pub fn get(&self, digest: Digest) -> Result<Vec<u8>, CasError> {
        let path = self.object_path(digest);
        let bytes = fs::read(&path).map_err(|error| {
            CasError::io(
                path.display().to_string(),
                format!("cannot read object: {error}"),
            )
        })?;
        let actual = Digest::of_bytes(&bytes);
        if actual != digest {
            return Err(CasError::corrupt(
                digest.to_hex(),
                format!("content hashes to `{actual}`"),
            ));
        }
        Ok(bytes)
    }

    /// Writes an object to `destination` atomically.
    pub fn materialize(&self, digest: Digest, destination: &Path) -> Result<(), CasError> {
        let bytes = self.get(digest)?;
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                CasError::io(
                    parent.display().to_string(),
                    format!("cannot create: {error}"),
                )
            })?;
        }
        let temporary = temporary_path(destination);
        let mut file = File::create(&temporary).map_err(|error| {
            CasError::io(
                temporary.display().to_string(),
                format!("cannot create temporary file: {error}"),
            )
        })?;
        file.write_all(&bytes).map_err(|error| {
            CasError::io(
                temporary.display().to_string(),
                format!("cannot write temporary file: {error}"),
            )
        })?;
        file.sync_all().map_err(|error| {
            CasError::io(
                temporary.display().to_string(),
                format!("cannot flush temporary file: {error}"),
            )
        })?;
        drop(file);
        fs::rename(&temporary, destination).map_err(|error| {
            let _ = fs::remove_file(&temporary);
            CasError::io(
                destination.display().to_string(),
                format!("cannot replace destination: {error}"),
            )
        })
    }

    /// Returns statistics for every object.
    pub fn stats(&self) -> Result<CasStats, CasError> {
        let mut stats = CasStats::default();
        for (path, _) in self.objects()? {
            let metadata = fs::metadata(&path).map_err(|error| {
                CasError::io(path.display().to_string(), format!("cannot stat: {error}"))
            })?;
            stats.objects += 1;
            stats.bytes += metadata.len();
        }
        Ok(stats)
    }

    /// Verifies every object and reports corrupt entries without deleting them.
    pub fn verify(&self) -> Result<VerifyReport, CasError> {
        let mut report = VerifyReport::default();
        for (path, expected) in self.objects()? {
            let bytes = fs::read(&path).map_err(|error| {
                CasError::io(path.display().to_string(), format!("cannot read: {error}"))
            })?;
            let actual = Digest::of_bytes(&bytes);
            if actual == expected {
                report.checked += 1;
            } else {
                report.corrupt.push(CorruptObject {
                    path,
                    expected: expected.to_hex(),
                    actual: actual.to_hex(),
                });
            }
        }
        Ok(report)
    }

    /// Removes the least-recently-used objects until the store fits `max_bytes`.
    pub fn prune_lru(&self, max_bytes: u64) -> Result<PruneReport, CasError> {
        let mut objects = Vec::new();
        let mut total = 0_u64;
        for (path, digest) in self.objects()? {
            let metadata = fs::metadata(&path).map_err(|error| {
                CasError::io(path.display().to_string(), format!("cannot stat: {error}"))
            })?;
            total += metadata.len();
            objects.push((accessed(&metadata), metadata.len(), path, digest));
        }
        objects.sort_by_key(|(time, _, _, _)| *time);
        let mut report = PruneReport::default();
        for (_, size, path, _) in objects {
            if total <= max_bytes {
                break;
            }
            match fs::remove_file(&path) {
                Ok(()) => {
                    total = total.saturating_sub(size);
                    report.removed += 1;
                    report.bytes += size;
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(CasError::io(
                        path.display().to_string(),
                        format!("cannot remove: {error}"),
                    ));
                }
            }
        }
        report.remaining_bytes = total;
        Ok(report)
    }

    /// Acquires an advisory lock scoped to `key`.
    pub fn lock(&self, key: &str) -> Result<LockGuard, CasError> {
        let name = Digest::of_bytes(key.as_bytes()).to_hex();
        let path = self.root.join("locks").join(format!("{name}.lock"));
        let deadline = SystemTime::now() + Duration::from_secs(10);
        loop {
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(mut file) => {
                    let _ = writeln!(file, "{}", std::process::id());
                    let _ = file.sync_all();
                    return Ok(LockGuard { path });
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    if is_stale(&path) {
                        let _ = fs::remove_file(&path);
                        continue;
                    }
                    if SystemTime::now() >= deadline {
                        return Err(CasError::LockTimeout {
                            key: key.to_owned(),
                        });
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(error) => {
                    return Err(CasError::io(
                        path.display().to_string(),
                        format!("cannot create lock: {error}"),
                    ));
                }
            }
        }
    }

    fn store_bytes(&self, digest: Digest, bytes: &[u8]) -> Result<(), CasError> {
        let destination = self.object_path(digest);
        if destination.is_file() {
            // A corrupt object must be rebuilt, not preserved just because a
            // file with the right name exists.
            if Digest::of_file(&destination).is_ok_and(|actual| actual == digest) {
                return Ok(());
            }
            let _ = fs::remove_file(&destination);
        }
        let parent = destination.parent().unwrap_or(&self.root);
        fs::create_dir_all(parent).map_err(|error| {
            CasError::io(
                parent.display().to_string(),
                format!("cannot create: {error}"),
            )
        })?;
        make_private_directory(parent)?;
        let temporary = temporary_path(&destination);
        let mut file = File::create(&temporary).map_err(|error| {
            CasError::io(
                temporary.display().to_string(),
                format!("cannot create temporary object: {error}"),
            )
        })?;
        file.write_all(bytes).map_err(|error| {
            CasError::io(
                temporary.display().to_string(),
                format!("cannot write temporary object: {error}"),
            )
        })?;
        file.sync_all().map_err(|error| {
            CasError::io(
                temporary.display().to_string(),
                format!("cannot flush temporary object: {error}"),
            )
        })?;
        drop(file);
        // A concurrent writer may have completed first. Both bytes are
        // content-addressed, so keeping the existing object is correct.
        if destination.is_file() {
            let _ = fs::remove_file(&temporary);
            return Ok(());
        }
        fs::rename(&temporary, &destination).map_err(|error| {
            let _ = fs::remove_file(&temporary);
            CasError::io(
                destination.display().to_string(),
                format!("cannot publish object: {error}"),
            )
        })?;
        make_private_file(&destination)?;
        Ok(())
    }

    fn objects(&self) -> Result<Vec<(PathBuf, Digest)>, CasError> {
        let root = self.root.join("objects");
        let mut objects = Vec::new();
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
                if !entry
                    .file_type()
                    .map(|kind| kind.is_file())
                    .unwrap_or(false)
                {
                    continue;
                }
                let name = format!(
                    "{}{}",
                    shard.file_name().to_string_lossy(),
                    entry.file_name().to_string_lossy()
                );
                let digest = Digest::parse(&name).map_err(|error| {
                    CasError::corrupt(entry.path().display().to_string(), error.to_string())
                })?;
                objects.push((entry.path(), digest));
            }
        }
        Ok(objects)
    }
}

/// An advisory lock removed when dropped.
#[derive(Debug)]
pub struct LockGuard {
    path: PathBuf,
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn temporary_path(destination: &Path) -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let name = destination
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "object".to_owned());
    destination.with_file_name(format!(
        ".{name}.{}.{}.tmp",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}

fn accessed(metadata: &fs::Metadata) -> SystemTime {
    metadata
        .accessed()
        .or_else(|_| metadata.modified())
        .unwrap_or(SystemTime::UNIX_EPOCH)
}

fn is_stale(path: &Path) -> bool {
    fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .map(|modified| {
            SystemTime::now()
                .duration_since(modified)
                .map(|age| age > Duration::from_secs(60))
                .unwrap_or(false)
        })
        .unwrap_or(false)
}

fn make_private_directory(path: &Path) -> Result<(), CasError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(|error| {
            CasError::io(
                path.display().to_string(),
                format!("cannot set mode: {error}"),
            )
        })?;
    }
    Ok(())
}

fn make_private_file(path: &Path) -> Result<(), CasError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600)).map_err(|error| {
            CasError::io(
                path.display().to_string(),
                format!("cannot set mode: {error}"),
            )
        })?;
    }
    Ok(())
}
