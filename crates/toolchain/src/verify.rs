//! Deterministic SHA-256 verification of a toolchain tree.

use std::fs;
use std::io::Read;
use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use walkdir::WalkDir;

use crate::error::ToolchainError;
use crate::manifest::{MANIFEST_NAME, MARKER_NAME, ToolchainManifest, validate_checksum};

/// The marker stored with an installed toolchain.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub(crate) struct InstallMarker {
    /// The installed manifest, copied verbatim.
    pub(crate) manifest: ToolchainManifest,
    /// The checksum that was verified during installation.
    pub(crate) verified_checksum: String,
}

/// Computes the digest of every regular file below `root`.
///
/// The manifest and install marker are excluded: the manifest contains the
/// digest, and the marker records it. Paths are sorted and rendered with
/// forward slashes, so the digest does not depend on directory iteration order
/// or on the host separator.
pub fn tree_checksum(root: &Path) -> Result<String, ToolchainError> {
    if !root.is_dir() {
        return Err(ToolchainError::manifest(format!(
            "the toolchain root `{}` is not a directory",
            root.display()
        )));
    }
    let mut entries = Vec::new();
    for entry in WalkDir::new(root).follow_links(false) {
        let entry = entry.map_err(|error| {
            ToolchainError::manifest(format!(
                "cannot inspect the toolchain tree `{}`: {error}",
                root.display()
            ))
        })?;
        let relative = entry.path().strip_prefix(root).map_err(|_| {
            ToolchainError::manifest(format!(
                "the entry `{}` leaves the toolchain root",
                entry.path().display()
            ))
        })?;
        if relative.as_os_str().is_empty() {
            continue;
        }
        let relative_text = relative
            .to_str()
            .ok_or_else(|| ToolchainError::UnsafeEntry {
                path: relative.to_path_buf(),
            })?;
        // A backslash is a path separator only on Windows. On Unix it is a
        // valid file-name byte and must not collapse onto a directory path.
        let text = if cfg!(windows) {
            relative_text.replace('\\', "/")
        } else {
            relative_text.to_owned()
        };
        let file_type = entry.file_type();
        if file_type.is_symlink() {
            return Err(ToolchainError::UnsafeEntry {
                path: relative.to_path_buf(),
            });
        }
        if text == MANIFEST_NAME || text == MARKER_NAME {
            continue;
        }
        if file_type.is_dir() {
            continue;
        }
        if !file_type.is_file() {
            return Err(ToolchainError::UnsafeEntry {
                path: relative.to_path_buf(),
            });
        }
        entries.push((text, entry.path().to_path_buf()));
    }
    entries.sort_by(|left, right| left.0.cmp(&right.0));
    let mut hasher = Sha256::new();
    hasher.update(b"seal-toolchain-tree-v1\0");
    for (relative, path) in entries {
        let metadata = fs::metadata(&path).map_err(|error| {
            ToolchainError::io(format!("cannot inspect `{}`", path.display()), error)
        })?;
        hasher.update((relative.len() as u64).to_le_bytes());
        hasher.update(relative.as_bytes());
        hasher.update(metadata.len().to_le_bytes());
        hasher.update([is_executable(&metadata) as u8]);
        hash_file(&mut hasher, &path)?;
    }
    Ok(hex_lower(hasher.finalize().as_slice()))
}

fn hash_file(hasher: &mut Sha256, path: &Path) -> Result<(), ToolchainError> {
    let mut file = fs::File::open(path)
        .map_err(|error| ToolchainError::io(format!("cannot read `{}`", path.display()), error))?;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(|error| {
            ToolchainError::io(format!("cannot read `{}`", path.display()), error)
        })?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(())
}

/// Verifies that `root` matches `manifest`.
pub fn verify_tree(root: &Path, manifest: &ToolchainManifest) -> Result<String, ToolchainError> {
    manifest.validate()?;
    manifest.check_components(root)?;
    let actual = tree_checksum(root)?;
    let expected = validate_checksum(&manifest.checksum)?;
    if actual != expected {
        return Err(ToolchainError::ChecksumMismatch {
            expected,
            actual,
            subject: format!("`{}`", root.display()),
        });
    }
    Ok(actual)
}

/// Verifies an optional Ed25519 signature against a trusted public key.
///
/// A signed manifest requires a key; a supplied key requires a signature. The
/// public key and signature are hexadecimal, with 32 and 64 bytes respectively.
pub(crate) fn verify_signature(
    manifest: &ToolchainManifest,
    public_key: Option<&str>,
) -> Result<(), ToolchainError> {
    match (&manifest.signature, public_key) {
        (None, None) => Ok(()),
        (None, Some(_)) => Err(ToolchainError::Signature {
            detail: "a public key was supplied, but the toolchain manifest is not signed"
                .to_owned(),
        }),
        (Some(_), None) => Err(ToolchainError::Signature {
            detail: "the toolchain manifest is signed; pass `--public-key <HEX>` to install it"
                .to_owned(),
        }),
        (Some(signature), Some(key)) => {
            let key = decode_hex(key, 32, "public key")?;
            let signature = decode_hex(signature, 64, "signature")?;
            let key: [u8; 32] = key.try_into().map_err(|_| ToolchainError::Signature {
                detail: "the public key is not 32 bytes".to_owned(),
            })?;
            let signature: [u8; 64] =
                signature
                    .try_into()
                    .map_err(|_| ToolchainError::Signature {
                        detail: "the signature is not 64 bytes".to_owned(),
                    })?;
            let key = ed25519_dalek::VerifyingKey::from_bytes(&key).map_err(|error| {
                ToolchainError::Signature {
                    detail: format!("the public key is invalid: {error}"),
                }
            })?;
            let signature = ed25519_dalek::Signature::from_bytes(&signature);
            key.verify_strict(&manifest.signature_message(), &signature)
                .map_err(|_| ToolchainError::Signature {
                    detail: "the toolchain signature is invalid".to_owned(),
                })
        }
    }
}

fn decode_hex(value: &str, expected: usize, label: &str) -> Result<Vec<u8>, ToolchainError> {
    if value.len() != expected * 2 {
        return Err(ToolchainError::Signature {
            detail: format!(
                "the {label} must be {} hexadecimal characters",
                expected * 2
            ),
        });
    }
    let mut bytes = Vec::with_capacity(expected);
    for pair in value.as_bytes().chunks_exact(2) {
        let text = std::str::from_utf8(pair).map_err(|_| ToolchainError::Signature {
            detail: format!("the {label} is not ASCII"),
        })?;
        let byte = u8::from_str_radix(text, 16).map_err(|_| ToolchainError::Signature {
            detail: format!("the {label} is not hexadecimal"),
        })?;
        bytes.push(byte);
    }
    Ok(bytes)
}

/// Writes the install marker after a successful verification.
pub(crate) fn write_marker(
    root: &Path,
    manifest: &ToolchainManifest,
    checksum: &str,
) -> Result<(), ToolchainError> {
    let marker = InstallMarker {
        manifest: manifest.clone(),
        verified_checksum: checksum.to_owned(),
    };
    let text = serde_json::to_vec_pretty(&marker).map_err(|error| {
        ToolchainError::manifest(format!("cannot encode the install marker: {error}"))
    })?;
    let path = root.join(MARKER_NAME);
    let mut temporary = path.clone();
    temporary.set_extension(format!("json.{}", std::process::id()));
    fs::write(&temporary, text).map_err(|error| {
        ToolchainError::io(format!("cannot write `{}`", temporary.display()), error)
    })?;
    fs::rename(&temporary, &path).map_err(|error| {
        let _ = fs::remove_file(&temporary);
        ToolchainError::io(format!("cannot replace `{}`", path.display()), error)
    })
}

/// Reads the marker of an installed toolchain.
pub(crate) fn read_marker(root: &Path) -> Result<InstallMarker, ToolchainError> {
    let path = root.join(MARKER_NAME);
    let bytes = fs::read(&path).map_err(|error| {
        ToolchainError::io(
            format!("cannot read the install marker `{}`", path.display()),
            error,
        )
    })?;
    let marker: InstallMarker = serde_json::from_slice(&bytes).map_err(|error| {
        ToolchainError::manifest(format!(
            "cannot parse the install marker `{}`: {error}",
            path.display()
        ))
    })?;
    marker.manifest.validate()?;
    validate_checksum(&marker.verified_checksum)?;
    Ok(marker)
}

fn is_executable(metadata: &fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        let _ = metadata;
        false
    }
}

fn hex_lower(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        text.push(DIGITS[(byte >> 4) as usize] as char);
        text.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    text
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use super::{hex_lower, read_marker, tree_checksum, verify_tree, write_marker};
    use crate::manifest::ToolchainManifest;

    fn scratch(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "seal-toolchain-verify-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time")
                .as_nanos()
        ));
        fs::create_dir_all(&path).expect("scratch");
        path
    }

    fn bundle() -> (PathBuf, ToolchainManifest) {
        let root = scratch("bundle");
        fs::create_dir_all(root.join("bin")).expect("bin");
        fs::write(root.join("bin/sealc"), b"compiler").expect("sealc");
        let checksum = tree_checksum(&root).expect("checksum");
        let manifest = ToolchainManifest {
            channel: "stable".to_owned(),
            version: "0.1.0".to_owned(),
            checksum,
            signature: None,
            components: [("sealc".to_owned(), "bin/sealc".into())]
                .into_iter()
                .collect(),
            targets: vec!["x86_64-unknown-linux-gnu".to_owned()],
        };
        fs::write(
            root.join(crate::manifest::MANIFEST_NAME),
            manifest.to_toml().expect("manifest"),
        )
        .expect("manifest file");
        (root, manifest)
    }

    #[test]
    fn a_tree_is_verified_and_tampering_is_rejected() {
        let (root, manifest) = bundle();
        let checksum = verify_tree(&root, &manifest).expect("verified");
        assert_eq!(checksum, manifest.checksum);

        fs::write(root.join("bin/sealc"), b"tampered").expect("tamper");
        assert!(matches!(
            verify_tree(&root, &manifest),
            Err(crate::ToolchainError::ChecksumMismatch { .. })
        ));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn checksum_ignores_the_manifest_and_marker() {
        let (root, _manifest) = bundle();
        let first = tree_checksum(&root).expect("checksum");
        fs::write(root.join(crate::manifest::MANIFEST_NAME), b"changed").expect("manifest");
        fs::write(root.join(crate::manifest::MARKER_NAME), b"marker").expect("marker");
        assert_eq!(first, tree_checksum(&root).expect("checksum"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn executable_bits_change_the_checksum() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let (root, _manifest) = bundle();
            let before = tree_checksum(&root).expect("checksum");
            let path = root.join("bin/sealc");
            let mut permissions = fs::metadata(&path).expect("metadata").permissions();
            permissions.set_mode(0o755);
            fs::set_permissions(&path, permissions).expect("chmod");
            assert_ne!(before, tree_checksum(&root).expect("checksum"));
            let _ = fs::remove_dir_all(root);
        }
    }

    #[test]
    fn the_marker_round_trips() {
        let (root, manifest) = bundle();
        write_marker(&root, &manifest, &manifest.checksum).expect("marker");
        let marker = read_marker(&root).expect("marker");
        assert_eq!(marker.manifest, manifest);
        assert_eq!(marker.verified_checksum, manifest.checksum);
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_are_rejected() {
        use std::os::unix::fs::symlink;
        let (root, _manifest) = bundle();
        symlink("/etc/passwd", root.join("link")).expect("symlink");
        assert!(matches!(
            tree_checksum(&root),
            Err(crate::ToolchainError::UnsafeEntry { .. })
        ));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn a_signed_manifest_requires_and_verifies_the_trusted_key() {
        use ed25519_dalek::{Signer, SigningKey};

        let (root, mut manifest) = bundle();
        let signing = SigningKey::from_bytes(&[7_u8; 32]);
        let signature = signing.sign(&manifest.signature_message());
        manifest.signature = Some(hex_lower(&signature.to_bytes()));
        let key = hex_lower(signing.verifying_key().as_bytes());

        super::verify_signature(&manifest, Some(&key)).expect("valid signature");
        assert!(super::verify_signature(&manifest, None).is_err());
        let mut tampered = manifest.clone();
        tampered.version = "0.1.1".to_owned();
        assert!(super::verify_signature(&tampered, Some(&key)).is_err());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn a_supplied_key_without_a_signature_is_rejected() {
        let (root, manifest) = bundle();
        assert!(super::verify_signature(&manifest, Some(&"0".repeat(64))).is_err());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn a_missing_component_is_rejected() {
        let (root, mut manifest) = bundle();
        manifest
            .components
            .insert("sealdoc".to_owned(), "bin/sealdoc".into());
        assert!(matches!(
            verify_tree(&root, &manifest),
            Err(crate::ToolchainError::Component { .. })
        ));
        let _ = fs::remove_dir_all(root);
    }
}
