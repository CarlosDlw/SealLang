//! Versioned persistent cache for serializable query payloads.
//!
//! The cache is content-addressed by [`QueryKey::digest`]. It never stores an
//! absolute path or a timestamp inside the key. Each record carries a magic
//! marker, a format version, the key digest, the payload length, and a BLAKE3
//! checksum. A record that fails any check is a miss; the compiler recomputes
//! the value instead of trusting corrupt bytes.

use std::fmt;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::key::QueryKey;

/// Version of the on-disk cache format.
///
/// Bump this whenever the header, digest domain, or payload encoding changes.
pub const CACHE_VERSION: u32 = 1;

/// Maximum accepted payload size: 512 MiB.
///
/// The limit bounds memory and prevents a corrupt length field from requesting
/// an enormous allocation.
pub const MAX_CACHE_PAYLOAD: u64 = 512 * 1024 * 1024;

const MAGIC: &[u8; 8] = b"SEALQC01";
const DIGEST_LEN: usize = 16;
const CHECKSUM_LEN: usize = 32;
const HEADER_LEN: usize = MAGIC.len() + 4 + DIGEST_LEN + 8 + CHECKSUM_LEN;

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// A failure while reading or writing the persistent cache.
#[derive(Debug)]
pub enum CacheError {
    /// The cache path could not be read, created, or replaced.
    Io {
        /// The path involved.
        path: PathBuf,
        /// The operating-system error.
        message: String,
    },
    /// A payload exceeds [`MAX_CACHE_PAYLOAD`].
    TooLarge {
        /// The path that would hold the payload.
        path: PathBuf,
        /// The payload length.
        len: u64,
        /// The accepted maximum.
        max: u64,
    },
}

impl fmt::Display for CacheError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, message } => {
                write!(
                    f,
                    "cannot access query cache `{}`: {message}",
                    path.display()
                )
            }
            Self::TooLarge { path, len, max } => write!(
                f,
                "the query cache payload for `{}` is {len} bytes, exceeding the {max}-byte limit",
                path.display()
            ),
        }
    }
}

impl std::error::Error for CacheError {}

/// A versioned byte cache rooted at one directory.
#[derive(Clone, Debug)]
pub struct DiskCache {
    root: PathBuf,
}

impl DiskCache {
    /// Opens (and creates on first write) a cache rooted at `root`.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Returns the cache root.
    #[inline]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Returns the file that stores `key`.
    ///
    /// The name is the stable key digest, never an absolute project path.
    pub fn path_for(&self, key: &QueryKey) -> PathBuf {
        let digest = key.digest();
        self.root.join(format!(
            "{}-{:032x}.bin",
            key.kind().as_str(),
            digest.as_u128()
        ))
    }

    /// Loads a payload for `key`.
    ///
    /// A missing, truncated, corrupt, mismatched, oversized, or old-version
    /// record returns `Ok(None)` and is removed on a best-effort basis. I/O
    /// failures while reading an existing path are returned as [`CacheError`].
    pub fn load(&self, key: &QueryKey) -> Result<Option<Vec<u8>>, CacheError> {
        let path = self.path_for(key);
        let metadata = match fs::metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(self.io_error(&path, error)),
        };
        if !metadata.is_file() || metadata.len() < HEADER_LEN as u64 {
            self.discard(&path);
            return Ok(None);
        }
        if metadata.len() > MAX_CACHE_PAYLOAD + HEADER_LEN as u64 {
            self.discard(&path);
            return Ok(None);
        }
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(self.io_error(&path, error)),
        };
        match decode(key, &bytes) {
            Some(payload) => Ok(Some(payload.to_vec())),
            None => {
                self.discard(&path);
                Ok(None)
            }
        }
    }

    /// Stores `payload` for `key` with an atomic rename.
    pub fn store(&self, key: &QueryKey, payload: &[u8]) -> Result<(), CacheError> {
        let len = payload.len() as u64;
        if len > MAX_CACHE_PAYLOAD {
            return Err(CacheError::TooLarge {
                path: self.path_for(key),
                len,
                max: MAX_CACHE_PAYLOAD,
            });
        }
        let path = self.path_for(key);
        if let Some(parent) = path.parent()
            && let Err(error) = fs::create_dir_all(parent)
        {
            return Err(self.io_error(parent, error));
        }
        let mut record = Vec::with_capacity(HEADER_LEN + payload.len());
        record.extend_from_slice(MAGIC);
        record.extend_from_slice(&CACHE_VERSION.to_le_bytes());
        record.extend_from_slice(&key.digest().as_u128().to_le_bytes());
        record.extend_from_slice(&len.to_le_bytes());
        record.extend_from_slice(blake3::hash(payload).as_bytes());
        record.extend_from_slice(payload);

        let temporary = self.temporary_path(&path);
        let write_result = (|| -> io::Result<()> {
            let mut file = fs::File::create(&temporary)?;
            file.write_all(&record)?;
            file.sync_all()?;
            Ok(())
        })();
        if let Err(error) = write_result {
            let _ = fs::remove_file(&temporary);
            return Err(self.io_error(&temporary, error));
        }
        if let Err(error) = fs::rename(&temporary, &path) {
            // On Windows an existing destination makes rename fail. Removing
            // and retrying keeps replacement working there; the cache is only a
            // hint, so a racing writer may win and that is still correct.
            if path.exists() {
                let _ = fs::remove_file(&path);
                if let Err(retry) = fs::rename(&temporary, &path) {
                    let _ = fs::remove_file(&temporary);
                    return Err(self.io_error(&path, retry));
                }
            } else {
                let _ = fs::remove_file(&temporary);
                return Err(self.io_error(&path, error));
            }
        }
        Ok(())
    }

    fn temporary_path(&self, path: &Path) -> PathBuf {
        let counter = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "query".to_owned());
        path.with_file_name(format!(".{name}.{}.{}.tmp", std::process::id(), counter))
    }

    fn discard(&self, path: &Path) {
        let _ = fs::remove_file(path);
    }

    fn io_error(&self, path: &Path, error: io::Error) -> CacheError {
        CacheError::Io {
            path: path.to_path_buf(),
            message: error.to_string(),
        }
    }
}

fn decode<'a>(key: &QueryKey, bytes: &'a [u8]) -> Option<&'a [u8]> {
    if bytes.len() < HEADER_LEN {
        return None;
    }
    let (magic, rest) = bytes.split_at(MAGIC.len());
    if magic != MAGIC {
        return None;
    }
    let (version, rest) = rest.split_at(4);
    if u32::from_le_bytes(version.try_into().ok()?) != CACHE_VERSION {
        return None;
    }
    let (digest, rest) = rest.split_at(DIGEST_LEN);
    if u128::from_le_bytes(digest.try_into().ok()?) != key.digest().as_u128() {
        return None;
    }
    let (len, rest) = rest.split_at(8);
    let len = u64::from_le_bytes(len.try_into().ok()?);
    if len > MAX_CACHE_PAYLOAD {
        return None;
    }
    let (checksum, payload) = rest.split_at(CHECKSUM_LEN);
    if payload.len() as u64 != len {
        return None;
    }
    if blake3::hash(payload).as_bytes() != checksum {
        return None;
    }
    Some(payload)
}
