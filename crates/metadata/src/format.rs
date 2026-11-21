//! Binary framing for `.sealmono`.
//!
//! Layout: `MAGIC`, little-endian `u32` version, little-endian `u64` header
//! length, UTF-8 JSON header, then exactly one payload per header module in
//! ascending CGU order.

use std::path::Path;

use crate::error::MetadataError;
use crate::schema::MonoManifest;
use crate::validate::validate;

/// The fixed prefix of every artifact.
pub const MAGIC: &[u8; 8] = b"SEALMONO";

/// Maximum accepted header plus payload size.
pub const MAX_ARCHIVE_BYTES: u64 = 512 * 1024 * 1024;

const PREFIX_BYTES: usize = MAGIC.len() + 4 + 8;

/// One validated archive.
#[derive(Clone, Debug)]
pub struct Archive {
    manifest: MonoManifest,
    modules: Vec<Vec<u8>>,
}

impl Archive {
    /// Returns the manifest.
    #[must_use]
    pub fn manifest(&self) -> &MonoManifest {
        &self.manifest
    }

    /// Returns every module payload in ascending CGU order.
    #[must_use]
    pub fn modules(&self) -> &[Vec<u8>] {
        &self.modules
    }

    /// Returns the payload of one unit.
    #[must_use]
    pub fn module(&self, cgu: u32) -> Option<&[u8]> {
        self.modules.get(cgu as usize).map(Vec::as_slice)
    }
}

/// Serializes `manifest` and `modules` into a new artifact.
///
/// # Errors
///
/// Returns [`MetadataError`] when the modules do not match the manifest, a size
/// is invalid, or JSON serialization fails.
pub fn write_archive(
    manifest: &MonoManifest,
    modules: &[Vec<u8>],
) -> Result<Vec<u8>, MetadataError> {
    let mut manifest = manifest.clone();
    if manifest.modules.len() != modules.len() {
        return Err(MetadataError::new(format!(
            "the header lists {} units but the writer received {}",
            manifest.modules.len(),
            modules.len()
        )));
    }
    for (module, bytes) in manifest.modules.iter_mut().zip(modules) {
        module.size = bytes.len() as u64;
        module.digest = digest(bytes);
    }
    validate(&manifest, modules)?;
    manifest.build_id = build_id(&manifest)?;
    let header = serde_json::to_vec(&manifest)
        .map_err(|error| MetadataError::new(format!("cannot encode the header: {error}")))?;
    let header_len = header.len() as u64;
    let total =
        PREFIX_BYTES as u64 + header_len + modules.iter().map(|m| m.len() as u64).sum::<u64>();
    if total > MAX_ARCHIVE_BYTES {
        return Err(MetadataError::new(format!(
            "the artifact is {total} bytes; the maximum is {MAX_ARCHIVE_BYTES}"
        )));
    }
    let mut output = Vec::with_capacity(total as usize);
    output.extend_from_slice(MAGIC);
    output.extend_from_slice(&super::schema::FORMAT_VERSION.to_le_bytes());
    output.extend_from_slice(&header_len.to_le_bytes());
    output.extend_from_slice(&header);
    for bytes in modules {
        output.extend_from_slice(bytes);
    }
    Ok(output)
}

/// Writes a new artifact atomically.
///
/// # Errors
///
/// Returns [`MetadataError`] when encoding fails or the file cannot be written.
pub fn write_archive_file(
    path: &Path,
    manifest: &MonoManifest,
    modules: &[Vec<u8>],
) -> Result<(), MetadataError> {
    let bytes = write_archive(manifest, modules)?;
    seal_fs::write_atomic(path, &bytes)
        .map_err(|error| MetadataError::new(format!("cannot write `{}`: {error}", path.display())))
}

/// Reads and validates an artifact from memory.
///
/// # Errors
///
/// Returns [`MetadataError`] for a bad magic, unsupported version, malformed
/// header, wrong payload count, truncated payload, or digest mismatch.
pub fn read_archive(bytes: &[u8]) -> Result<Archive, MetadataError> {
    if bytes.len() < PREFIX_BYTES {
        return Err(MetadataError::new(
            "the artifact is truncated before its header",
        ));
    }
    if &bytes[..MAGIC.len()] != MAGIC {
        return Err(MetadataError::new(
            "the artifact does not start with `SEALMONO`",
        ));
    }
    let version = u32::from_le_bytes(
        bytes[MAGIC.len()..MAGIC.len() + 4]
            .try_into()
            .map_err(|_| MetadataError::new("the version field is truncated"))?,
    );
    if version != crate::schema::FORMAT_VERSION {
        return Err(MetadataError::new(format!(
            "unsupported metadata version {version}; this compiler writes version {}",
            crate::schema::FORMAT_VERSION
        )));
    }
    let header_start = MAGIC.len() + 4;
    let header_len = u64::from_le_bytes(
        bytes[header_start..header_start + 8]
            .try_into()
            .map_err(|_| MetadataError::new("the header length is truncated"))?,
    );
    let header_end = (PREFIX_BYTES as u64)
        .checked_add(header_len)
        .ok_or_else(|| MetadataError::new("the header length overflows"))?;
    if header_end > bytes.len() as u64 {
        return Err(MetadataError::new(
            "the artifact is truncated inside its header",
        ));
    }
    let header: MonoManifest = serde_json::from_slice(&bytes[PREFIX_BYTES..header_end as usize])
        .map_err(|error| MetadataError::new(format!("the header is invalid JSON: {error}")))?;
    if header.version != crate::schema::FORMAT_VERSION {
        return Err(MetadataError::new(format!(
            "the header names version {} but the container names {version}",
            header.version
        )));
    }
    let mut offset = header_end as usize;
    let mut modules = Vec::with_capacity(header.modules.len());
    for module in &header.modules {
        let size = usize::try_from(module.size)
            .map_err(|_| MetadataError::new("a module size does not fit in memory"))?;
        let end = offset
            .checked_add(size)
            .ok_or_else(|| MetadataError::new("a module size overflows"))?;
        if end > bytes.len() {
            return Err(MetadataError::new(format!(
                "module {} is truncated: expected {size} bytes",
                module.cgu
            )));
        }
        modules.push(bytes[offset..end].to_vec());
        offset = end;
    }
    if offset != bytes.len() {
        return Err(MetadataError::new(format!(
            "the artifact has {} trailing bytes after its modules",
            bytes.len() - offset
        )));
    }
    validate(&header, &modules)?;
    Ok(Archive {
        manifest: header,
        modules,
    })
}

/// Reads and validates an artifact from a file.
///
/// # Errors
///
/// Returns [`MetadataError`] when the file cannot be read, is too large, or is
/// not a valid artifact.
pub fn read_archive_file(path: &Path) -> Result<Archive, MetadataError> {
    let metadata = std::fs::metadata(path).map_err(|error| {
        MetadataError::new(format!("cannot read `{}`: {error}", path.display()))
    })?;
    if metadata.len() > MAX_ARCHIVE_BYTES {
        return Err(MetadataError::new(format!(
            "`{}` is {} bytes; the maximum is {MAX_ARCHIVE_BYTES}",
            path.display(),
            metadata.len()
        )));
    }
    let bytes = std::fs::read(path).map_err(|error| {
        MetadataError::new(format!("cannot read `{}`: {error}", path.display()))
    })?;
    read_archive(&bytes)
}

/// Returns the lowercase blake3 digest of `bytes`.
pub(crate) fn digest(bytes: &[u8]) -> String {
    hex::encode(blake3::hash(bytes).as_bytes())
}

fn build_id(manifest: &MonoManifest) -> Result<String, MetadataError> {
    let mut copy = manifest.clone();
    copy.build_id.clear();
    let bytes = serde_json::to_vec(&copy)
        .map_err(|error| MetadataError::new(format!("cannot encode the build id: {error}")))?;
    Ok(digest(&bytes))
}

#[cfg(test)]
mod tests {
    use super::{MAGIC, read_archive, write_archive};
    use crate::schema::{MonoManifest, MonoModule};

    fn manifest(count: u32) -> MonoManifest {
        let mut manifest = MonoManifest::new("x86_64-unknown-linux-gnu", count);
        for cgu in 0..count {
            manifest.modules.push(MonoModule {
                cgu,
                name: format!("cgu{cgu}"),
                digest: String::new(),
                size: 0,
                functions: 1,
            });
        }
        manifest
    }

    #[test]
    fn a_round_trip_preserves_units_and_digests() {
        let manifest = manifest(2);
        let bytes = write_archive(&manifest, &[b"one".to_vec(), b"two".to_vec()]).expect("archive");
        let archive = read_archive(&bytes).expect("valid archive");
        assert_eq!(archive.modules(), &[b"one".to_vec(), b"two".to_vec()]);
        assert_eq!(archive.manifest().cgu_count(), 2);
        assert_eq!(archive.manifest().modules[1].size, 3);
        assert!(!archive.manifest().build_id.is_empty());
    }

    #[test]
    fn a_bad_magic_version_or_truncation_is_rejected() {
        let bytes = write_archive(&manifest(1), &[b"one".to_vec()]).expect("archive");
        let mut bad = bytes.clone();
        bad[0] = b'X';
        assert!(read_archive(&bad).is_err());

        let mut bad = bytes.clone();
        bad[8..12].copy_from_slice(&99_u32.to_le_bytes());
        assert!(read_archive(&bad).is_err());

        assert!(read_archive(&bytes[..bytes.len() - 1]).is_err());
    }

    #[test]
    fn a_trailing_byte_or_wrong_count_is_rejected() {
        let bytes = write_archive(&manifest(1), &[b"one".to_vec()]).expect("archive");
        let mut trailing = bytes.clone();
        trailing.push(0);
        assert!(read_archive(&trailing).is_err());

        let mut two = manifest(2);
        two.modules.pop();
        assert!(write_archive(&two, &[b"one".to_vec(), b"two".to_vec()]).is_err());
    }

    #[test]
    fn the_magic_is_stable() {
        let bytes = write_archive(&manifest(1), &[b"one".to_vec()]).expect("archive");
        assert_eq!(&bytes[..MAGIC.len()], MAGIC);
    }
}
