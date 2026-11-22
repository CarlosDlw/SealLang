//! Durable object and action-result storage.
//!
//! The layout is private: objects are sharded by their BLAKE3 hash, action
//! results are protobuf bytes under a separate action directory, and every
//! write goes through an atomic temporary-file plus rename. Directories and
//! files are owner-only on Unix. A restart reopens the same immutable objects.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use seal_cas::Digest;

use crate::error::{ServerError, ServerResult};

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// The persistent remote store.
#[derive(Clone, Debug)]
pub(crate) struct Store {
    root: PathBuf,
}

impl Store {
    /// Opens or creates a store under `root`.
    pub(crate) fn open(root: impl Into<PathBuf>) -> ServerResult<Self> {
        let root = root.into();
        for directory in ["objects", "actions", "tmp", "work"] {
            let path = root.join(directory);
            fs::create_dir_all(&path).map_err(|error| {
                ServerError::internal(format!("cannot create `{}`: {error}", path.display()))
            })?;
            private_directory(&path)?;
        }
        Ok(Self { root })
    }

    /// Stores a blob after verifying its digest and size.
    pub(crate) fn put_blob(&self, digest: Digest, size: u64, bytes: &[u8]) -> ServerResult<()> {
        if bytes.len() as u64 != size {
            return Err(ServerError::integrity(format!(
                "`{digest}` has {} bytes, expected {size}",
                bytes.len()
            )));
        }
        if Digest::of_bytes(bytes) != digest {
            return Err(ServerError::integrity(format!(
                "the bytes for `{digest}` hash to `{}`",
                Digest::of_bytes(bytes)
            )));
        }
        if bytes.is_empty() {
            return Ok(());
        }
        write_atomic(&self.object_path(digest), bytes)
    }

    /// Returns a verified blob.
    pub(crate) fn get_blob(&self, digest: Digest) -> ServerResult<Vec<u8>> {
        if digest == Digest::of_bytes(b"") {
            return Ok(Vec::new());
        }
        let bytes = fs::read(self.object_path(digest)).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                ServerError::missing(format!("the remote CAS does not contain `{digest}`"))
            } else {
                ServerError::internal(format!("cannot read `{digest}`: {error}"))
            }
        })?;
        if Digest::of_bytes(&bytes) != digest {
            return Err(ServerError::integrity(format!(
                "the stored bytes for `{digest}` are corrupt"
            )));
        }
        Ok(bytes)
    }

    /// Returns whether a blob is present.
    pub(crate) fn contains(&self, digest: Digest) -> bool {
        digest == Digest::of_bytes(b"") || self.object_path(digest).is_file()
    }

    /// Stores an encoded action result.
    pub(crate) fn put_action(&self, digest: Digest, bytes: &[u8]) -> ServerResult<()> {
        // An ActionResult with only default fields encodes to zero bytes. That
        // is a valid result for a successful action with no outputs or logs, so
        // emptiness is not an error here.
        write_atomic(&self.action_path(digest), bytes)
    }

    /// Reads an encoded action result.
    pub(crate) fn get_action(&self, digest: Digest) -> ServerResult<Option<Vec<u8>>> {
        match fs::read(self.action_path(digest)) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(ServerError::internal(format!(
                "cannot read the action result `{digest}`: {error}"
            ))),
        }
    }

    /// Creates a unique work directory owned by this server process.
    pub(crate) fn work_directory(&self) -> ServerResult<PathBuf> {
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = self
            .root
            .join("work")
            .join(format!("action-{}-{sequence}", std::process::id()));
        fs::create_dir(&path).map_err(|error| {
            ServerError::internal(format!("cannot create `{}`: {error}", path.display()))
        })?;
        private_directory(&path)?;
        Ok(path)
    }

    fn object_path(&self, digest: Digest) -> PathBuf {
        shard(&self.root.join("objects"), digest)
    }

    fn action_path(&self, digest: Digest) -> PathBuf {
        shard(&self.root.join("actions"), digest)
    }
}

fn shard(root: &Path, digest: Digest) -> PathBuf {
    let hex = digest.to_hex();
    root.join(&hex[..2]).join(&hex[2..])
}

fn write_atomic(path: &Path, bytes: &[u8]) -> ServerResult<()> {
    let parent = path
        .parent()
        .ok_or_else(|| ServerError::internal("a cache path has no parent"))?;
    fs::create_dir_all(parent).map_err(|error| {
        ServerError::internal(format!("cannot create `{}`: {error}", parent.display()))
    })?;
    private_directory(parent)?;
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let temporary = path.with_extension(format!("{}-{sequence}.tmp", std::process::id()));
    let mut file = fs::File::create(&temporary).map_err(|error| {
        ServerError::internal(format!("cannot create `{}`: {error}", temporary.display()))
    })?;
    file.write_all(bytes).map_err(|error| {
        ServerError::internal(format!("cannot write `{}`: {error}", temporary.display()))
    })?;
    file.sync_all().map_err(|error| {
        ServerError::internal(format!("cannot flush `{}`: {error}", temporary.display()))
    })?;
    drop(file);
    set_private_file(&temporary)?;
    fs::rename(&temporary, path).map_err(|error| {
        let _ = fs::remove_file(&temporary);
        ServerError::internal(format!("cannot publish `{}`: {error}", path.display()))
    })
}

fn private_directory(path: &Path) -> ServerResult<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(|error| {
            ServerError::internal(format!("cannot protect `{}`: {error}", path.display()))
        })?;
    }
    Ok(())
}

fn set_private_file(path: &Path) -> ServerResult<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600)).map_err(|error| {
            ServerError::internal(format!("cannot protect `{}`: {error}", path.display()))
        })?;
    }
    Ok(())
}
