//! Safe extraction of package archives.

use std::io::Read;
use std::path::{Component, Path, PathBuf};

use crate::error::{RegistryError, RegistryResult};

const MAX_ENTRIES: usize = 50_000;
const MAX_FILE: u64 = 64 * 1024 * 1024;
const MAX_TOTAL: u64 = 512 * 1024 * 1024;
const MAX_PATH: usize = 1024;

/// Extracts a `.tar.zst` archive into `destination`.
///
/// The destination must not exist. Absolute paths, `..`, symlinks, hard links,
/// devices, FIFOs, duplicate entries, and decompression bombs are rejected.
/// The directory is removed on every error.
pub fn extract(archive: &Path, destination: &Path) -> RegistryResult<()> {
    if destination.exists() {
        return Err(RegistryError::archive(format!(
            "the extraction destination `{}` already exists",
            destination.display()
        )));
    }
    std::fs::create_dir_all(destination).map_err(|error| {
        RegistryError::archive(format!(
            "cannot create `{}`: {error}",
            destination.display()
        ))
    })?;
    let result = extract_inner(archive, destination);
    if result.is_err() {
        let _ = std::fs::remove_dir_all(destination);
    }
    result
}

fn extract_inner(archive: &Path, destination: &Path) -> RegistryResult<()> {
    let file = std::fs::File::open(archive).map_err(|error| {
        RegistryError::archive(format!("cannot open `{}`: {error}", archive.display()))
    })?;
    let decoder = zstd::Decoder::new(file).map_err(|error| {
        RegistryError::archive(format!(
            "cannot decompress `{}`: {error}",
            archive.display()
        ))
    })?;
    let mut archive = tar::Archive::new(decoder);
    let mut count = 0usize;
    let mut total = 0u64;
    let mut seen = std::collections::BTreeSet::new();
    let entries = archive
        .entries()
        .map_err(|error| RegistryError::archive(format!("cannot read the archive: {error}")))?;
    for entry in entries {
        let entry = entry
            .map_err(|error| RegistryError::archive(format!("invalid archive entry: {error}")))?;
        let mode = entry.header().mode().unwrap_or(0o644) & 0o777;
        count += 1;
        if count > MAX_ENTRIES {
            return Err(RegistryError::archive(format!(
                "the archive has more than {MAX_ENTRIES} entries"
            )));
        }
        let path = entry
            .path()
            .map_err(|error| RegistryError::archive(format!("invalid entry path: {error}")))?;
        let relative = safe_path(&path)?;
        if !seen.insert(relative.clone()) {
            return Err(RegistryError::archive(format!(
                "the archive contains `{}` twice",
                relative.display()
            )));
        }
        let kind = entry.header().entry_type();
        match kind {
            tar::EntryType::Directory => {
                std::fs::create_dir_all(destination.join(&relative)).map_err(|error| {
                    RegistryError::archive(format!(
                        "cannot create `{}`: {error}",
                        destination.join(&relative).display()
                    ))
                })?;
                continue;
            }
            tar::EntryType::Regular | tar::EntryType::Continuous => {}
            tar::EntryType::Symlink
            | tar::EntryType::Link
            | tar::EntryType::Char
            | tar::EntryType::Block
            | tar::EntryType::Fifo => {
                return Err(RegistryError::archive(format!(
                    "the archive entry `{}` is a link or special file",
                    relative.display()
                )));
            }
            other => {
                return Err(RegistryError::archive(format!(
                    "the archive entry `{}` has unsupported type {:?}",
                    relative.display(),
                    other
                )));
            }
        }
        let size = entry.header().size().map_err(|error| {
            RegistryError::archive(format!(
                "the entry `{}` has no valid size: {error}",
                relative.display()
            ))
        })?;
        if size > MAX_FILE {
            return Err(RegistryError::archive(format!(
                "the archive entry `{}` is {size} bytes, above the {} byte limit",
                relative.display(),
                MAX_FILE
            )));
        }
        total = total.saturating_add(size);
        if total > MAX_TOTAL {
            return Err(RegistryError::archive(format!(
                "the archive expands to more than {MAX_TOTAL} bytes"
            )));
        }
        let output = destination.join(&relative);
        if let Some(parent) = output.parent() {
            std::fs::create_dir_all(parent).map_err(|error| {
                RegistryError::archive(format!("cannot create `{}`: {error}", parent.display()))
            })?;
        }
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&output)
            .map_err(|error| {
                RegistryError::archive(format!("cannot create `{}`: {error}", output.display()))
            })?;
        let mut reader = entry.take(size + 1);
        std::io::copy(&mut reader, &mut file).map_err(|error| {
            RegistryError::archive(format!("cannot write `{}`: {error}", output.display()))
        })?;
        if file.metadata().map(|meta| meta.len()).unwrap_or(0) != size {
            return Err(RegistryError::archive(format!(
                "the archive entry `{}` was truncated",
                relative.display()
            )));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = if mode & 0o111 != 0 { 0o755 } else { 0o644 };
            std::fs::set_permissions(&output, std::fs::Permissions::from_mode(mode)).map_err(
                |error| {
                    RegistryError::archive(format!(
                        "cannot set permissions on `{}`: {error}",
                        output.display()
                    ))
                },
            )?;
        }
    }
    if !destination.join("seal.toml").is_file() {
        return Err(RegistryError::archive(
            "the package archive has no `seal.toml` at its root",
        ));
    }
    Ok(())
}

fn safe_path(path: &Path) -> RegistryResult<PathBuf> {
    if path.as_os_str().len() > MAX_PATH {
        return Err(RegistryError::archive("an archive path is too long"));
    }
    let mut relative = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => {
                if part.is_empty() {
                    return Err(RegistryError::archive(
                        "an archive path has an empty component",
                    ));
                }
                relative.push(part);
            }
            _ => {
                return Err(RegistryError::archive(format!(
                    "the archive path `{}` is absolute or escapes the package",
                    path.display()
                )));
            }
        }
    }
    if relative.as_os_str().is_empty() {
        return Err(RegistryError::archive("an archive entry has an empty path"));
    }
    Ok(relative)
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    static NEXT: AtomicU64 = AtomicU64::new(0);

    fn temporary() -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "seal-archive-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    fn write_archive(path: &Path, entries: &[(&str, tar::EntryType, u64, Option<&str>)]) {
        let encoder = zstd::Encoder::new(Vec::new(), 3).unwrap();
        let mut builder = tar::Builder::new(encoder);
        for (name, kind, size, link) in entries {
            let mut header = tar::Header::new_gnu();
            header.set_path(name).unwrap();
            header.set_entry_type(*kind);
            header.set_size(*size);
            header.set_mode(0o644);
            if let Some(link) = link {
                header.set_link_name(link).unwrap();
            }
            header.set_cksum();
            builder.append(&header, std::io::empty()).unwrap();
        }
        let encoder = builder.into_inner().unwrap();
        std::fs::write(path, encoder.finish().unwrap()).unwrap();
    }

    #[test]
    fn parent_and_absolute_paths_are_rejected() {
        assert!(safe_path(Path::new("../escape")).is_err());
        assert!(safe_path(Path::new("/absolute")).is_err());
        assert!(safe_path(Path::new("a/../../escape")).is_err());
        assert!(safe_path(Path::new("a/b")).is_ok());
    }

    #[test]
    fn links_and_special_files_are_rejected() {
        let root = temporary();
        for kind in [
            tar::EntryType::Symlink,
            tar::EntryType::Link,
            tar::EntryType::Char,
            tar::EntryType::Block,
            tar::EntryType::Fifo,
        ] {
            let archive = root.join("package.tar.zst");
            write_archive(&archive, &[("bad", kind, 0, Some("../outside"))]);
            let destination = root.join("out");
            let error = extract(&archive, &destination).unwrap_err();
            assert!(error.message().contains("link or special file"), "{error}");
            assert!(!destination.exists());
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn duplicate_entries_are_rejected() {
        let root = temporary();
        let archive = root.join("package.tar.zst");
        write_archive(
            &archive,
            &[
                ("seal.toml", tar::EntryType::Regular, 0, None),
                ("seal.toml", tar::EntryType::Regular, 0, None),
            ],
        );
        let error = extract(&archive, &root.join("out")).unwrap_err();
        assert!(error.message().contains("twice"), "{error}");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn an_archive_without_a_manifest_is_rejected() {
        let root = temporary();
        let archive = root.join("package.tar.zst");
        write_archive(
            &archive,
            &[("src/lib.sl", tar::EntryType::Regular, 0, None)],
        );
        let error = extract(&archive, &root.join("out")).unwrap_err();
        assert!(error.message().contains("seal.toml"), "{error}");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn an_entry_above_the_file_limit_is_rejected_before_writing() {
        let root = temporary();
        let archive = root.join("package.tar.zst");
        write_archive(
            &archive,
            &[("seal.toml", tar::EntryType::Regular, MAX_FILE + 1, None)],
        );
        let error = extract(&archive, &root.join("out")).unwrap_err();
        assert!(error.message().contains("above the"), "{error}");
        assert!(!root.join("out").exists());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn an_existing_destination_is_never_reused() {
        let root = temporary();
        let archive = root.join("package.tar.zst");
        write_archive(&archive, &[("seal.toml", tar::EntryType::Regular, 0, None)]);
        let destination = root.join("out");
        std::fs::create_dir(&destination).unwrap();
        let error = extract(&archive, &destination).unwrap_err();
        assert!(error.message().contains("already exists"), "{error}");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_written_archive_keeps_the_manifest() {
        let root = temporary();
        let archive = root.join("package.tar.zst");
        let encoder = zstd::Encoder::new(Vec::new(), 3).unwrap();
        let mut builder = tar::Builder::new(encoder);
        let manifest = b"[package]\nname = \"foo\"\nversion = \"1.0.0\"\n";
        let mut header = tar::Header::new_gnu();
        header.set_path("seal.toml").unwrap();
        header.set_size(manifest.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        builder.append(&header, manifest.as_slice()).unwrap();
        let encoder = builder.into_inner().unwrap();
        std::fs::write(&archive, encoder.finish().unwrap()).unwrap();
        let destination = root.join("out");
        extract(&archive, &destination).unwrap();
        assert_eq!(
            std::fs::read(destination.join("seal.toml")).unwrap(),
            manifest
        );
        let _ = std::fs::remove_dir_all(root);
    }
}
