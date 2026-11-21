//! Verified package downloads and the local package cache.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use semver::Version;
use sha2::{Digest, Sha256};

use crate::archive;
use crate::client::RegistryClient;
use crate::config::RegistryConfig;
use crate::error::{RegistryError, RegistryResult};
use crate::index::{IndexVersion, validate_checksum};
use crate::tuf::TufSession;

static NEXT: AtomicU64 = AtomicU64::new(0);

/// Downloads, verifies, and extracts packages into the project cache.
pub struct Downloader<'a> {
    client: &'a RegistryClient,
    config: &'a RegistryConfig,
    tuf: Option<&'a TufSession>,
}

impl<'a> Downloader<'a> {
    /// Creates a downloader for a verified client.
    pub fn new(
        client: &'a RegistryClient,
        config: &'a RegistryConfig,
        tuf: Option<&'a TufSession>,
    ) -> Self {
        Self {
            client,
            config,
            tuf,
        }
    }

    /// Ensures that a package is present in the cache and returns its root.
    pub fn ensure(
        &self,
        name: &str,
        version: &Version,
        entry: &IndexVersion,
    ) -> RegistryResult<PathBuf> {
        validate_checksum(&entry.checksum)?;
        let destination = self
            .config
            .cache_root
            .join("packages")
            .join(cache_name(name))
            .join(format!("{version}-{}", &entry.checksum[7..19]));
        if destination.join("seal.toml").is_file() {
            let marker = destination.join(".seal-checksum");
            if std::fs::read_to_string(&marker).ok().as_deref() == Some(entry.checksum.as_str()) {
                return Ok(destination);
            }
            let _ = std::fs::remove_dir_all(&destination);
        }
        if self.config.offline {
            return Err(RegistryError::offline(format!(
                "`{name}` {version} is not cached; run without `--offline` or vendor it first"
            )));
        }
        let archive_bytes = match self.tuf {
            Some(tuf) => tuf.target_from(self.client, self.config, &entry.archive)?,
            None => self.client.fetch(self.config, &entry.archive)?,
        };
        verify_digest(name, version, &archive_bytes, &entry.checksum)?;
        let temporary = self.config.cache_root.join("tmp").join(format!(
            "package-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        if let Some(parent) = temporary.parent() {
            std::fs::create_dir_all(parent).map_err(|error| {
                RegistryError::cache(format!("cannot create `{}`: {error}", parent.display()))
            })?;
        }
        let archive_path = temporary.with_extension("tar.zst");
        std::fs::write(&archive_path, &archive_bytes).map_err(|error| {
            RegistryError::cache(format!(
                "cannot write `{}`: {error}",
                archive_path.display()
            ))
        })?;
        let extracted = temporary.with_extension("dir");
        let result = archive::extract(&archive_path, &extracted);
        let _ = std::fs::remove_file(&archive_path);
        result?;
        std::fs::write(extracted.join(".seal-checksum"), &entry.checksum).map_err(|error| {
            RegistryError::cache(format!(
                "cannot write the checksum marker in `{}`: {error}",
                extracted.display()
            ))
        })?;
        if let Some(parent) = destination.parent() {
            std::fs::create_dir_all(parent).map_err(|error| {
                RegistryError::cache(format!("cannot create `{}`: {error}", parent.display()))
            })?;
        }
        match std::fs::rename(&extracted, &destination) {
            Ok(()) => Ok(destination),
            Err(_error) if destination.join("seal.toml").is_file() => {
                let _ = std::fs::remove_dir_all(&extracted);
                Ok(destination)
            }
            Err(error) => {
                let _ = std::fs::remove_dir_all(&extracted);
                Err(RegistryError::cache(format!(
                    "cannot install `{name}` {version} at `{}`: {error}",
                    destination.display()
                )))
            }
        }
    }
}

/// Returns the cache path for a verified package.
pub fn cached_path(cache_root: &Path, name: &str, version: &Version, checksum: &str) -> PathBuf {
    cache_root
        .join("packages")
        .join(cache_name(name))
        .join(format!(
            "{version}-{}",
            checksum.get(7..19).unwrap_or("invalid")
        ))
}

/// Verifies that a package directory carries the expected checksum marker.
pub fn verify_cached(root: &Path, checksum: &str) -> RegistryResult<()> {
    let marker = root.join(".seal-checksum");
    let found = std::fs::read_to_string(&marker).map_err(|error| {
        RegistryError::integrity(format!(
            "the package `{}` has no checksum marker: {error}",
            root.display()
        ))
    })?;
    if found != checksum {
        return Err(RegistryError::integrity(format!(
            "the package `{}` is `{found}`, but the lockfile expects `{checksum}`",
            root.display()
        )));
    }
    Ok(())
}

/// Returns the SHA-256 digest of bytes as `sha256:<hex>`.
pub fn digest(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("sha256:{}", hex::encode(hasher.finalize()))
}

fn verify_digest(
    name: &str,
    version: &Version,
    bytes: &[u8],
    expected: &str,
) -> RegistryResult<()> {
    let actual = digest(bytes);
    if actual != expected {
        return Err(RegistryError::integrity(format!(
            "the archive for `{name}` {version} is `{actual}`, but the index declares `{expected}`"
        )));
    }
    Ok(())
}

fn cache_name(name: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(name.as_bytes());
    let digest = hex::encode(hasher.finalize());
    digest[..16].to_owned()
}
