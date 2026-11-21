//! Offline toolchain installation and project pins.

use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;
use walkdir::WalkDir;

use crate::error::ToolchainError;
use crate::manifest::{MANIFEST_NAME, MARKER_NAME, ToolchainManifest, validate_checksum};
use crate::verify::{read_marker, tree_checksum, verify_signature, verify_tree, write_marker};

/// The project file that pins a toolchain.
pub const PIN_NAME: &str = "seal-toolchain.toml";

/// One installed toolchain.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct InstalledToolchain {
    /// The install directory.
    pub root: PathBuf,
    /// The manifest recorded at install time.
    pub manifest: ToolchainManifest,
    /// Whether the install already existed and was verified, not copied.
    pub reused: bool,
}

/// A toolchain store rooted inside a project.
#[derive(Clone, Debug)]
pub struct ToolchainStore {
    root: PathBuf,
}

impl ToolchainStore {
    /// Opens the store below `<project>/seal/toolchains`.
    pub fn for_project(project: &Path) -> Self {
        Self {
            root: project.join("seal").join("toolchains"),
        }
    }

    /// Opens a store at an explicit root.
    pub fn at(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Returns the store root.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Returns the directory of one version.
    pub fn version_dir(&self, version: &str) -> PathBuf {
        self.root.join(version)
    }

    /// Lists every installed toolchain, sorted by semantic version.
    pub fn list(&self) -> Result<Vec<InstalledToolchain>, ToolchainError> {
        if !self.root.exists() {
            return Ok(Vec::new());
        }
        let entries = fs::read_dir(&self.root).map_err(|error| {
            ToolchainError::io(format!("cannot read `{}`", self.root.display()), error)
        })?;
        let mut installed = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|error| {
                ToolchainError::io(format!("cannot read `{}`", self.root.display()), error)
            })?;
            let file_type = entry.file_type().map_err(|error| {
                ToolchainError::io(
                    format!("cannot inspect `{}`", entry.path().display()),
                    error,
                )
            })?;
            if !file_type.is_dir() || entry.file_name().to_string_lossy().starts_with('.') {
                continue;
            }
            let root = entry.path();
            let marker = read_marker(&root)?;
            let manifest = marker.manifest;
            verify_tree(&root, &manifest)?;
            installed.push(InstalledToolchain {
                root,
                manifest,
                reused: true,
            });
        }
        installed.sort_by(|left, right| {
            left.manifest
                .semantic_version()
                .ok()
                .cmp(&right.manifest.semantic_version().ok())
                .then_with(|| left.manifest.version.cmp(&right.manifest.version))
        });
        Ok(installed)
    }

    /// Installs a verified local bundle. No network access is attempted.
    pub fn install_from(&self, bundle: &Path) -> Result<InstalledToolchain, ToolchainError> {
        self.install_from_with_key(bundle, None)
    }

    /// Installs a bundle and verifies its optional Ed25519 signature.
    pub fn install_from_with_key(
        &self,
        bundle: &Path,
        public_key: Option<&str>,
    ) -> Result<InstalledToolchain, ToolchainError> {
        let manifest_path = bundle.join(MANIFEST_NAME);
        let manifest = ToolchainManifest::load(&manifest_path)?;
        verify_signature(&manifest, public_key)?;
        let checksum = verify_tree(bundle, &manifest)?;
        let destination = self.version_dir(&manifest.version);
        if destination.exists() {
            let marker = read_marker(&destination)?;
            if marker.verified_checksum != checksum {
                return Err(ToolchainError::ChecksumMismatch {
                    expected: checksum,
                    actual: marker.verified_checksum,
                    subject: format!(
                        "the already installed toolchain `{}`",
                        destination.display()
                    ),
                });
            }
            verify_tree(&destination, &marker.manifest)?;
            return Ok(InstalledToolchain {
                root: destination,
                manifest: marker.manifest,
                reused: true,
            });
        }
        fs::create_dir_all(&self.root).map_err(|error| {
            ToolchainError::io(format!("cannot create `{}`", self.root.display()), error)
        })?;
        let staging = self.root.join(format!(
            ".install-{}-{}-{}",
            manifest.version,
            std::process::id(),
            sequence()
        ));
        let _ = fs::remove_dir_all(&staging);
        let result = (|| {
            copy_tree(bundle, &staging)?;
            let copied = tree_checksum(&staging)?;
            if copied != checksum {
                return Err(ToolchainError::ChecksumMismatch {
                    expected: checksum.clone(),
                    actual: copied,
                    subject: format!("the staged toolchain `{}`", staging.display()),
                });
            }
            write_marker(&staging, &manifest, &checksum)?;
            fs::rename(&staging, &destination).map_err(|error| {
                ToolchainError::io(
                    format!(
                        "cannot install the toolchain at `{}`",
                        destination.display()
                    ),
                    error,
                )
            })?;
            Ok(InstalledToolchain {
                root: destination.clone(),
                manifest: manifest.clone(),
                reused: false,
            })
        })();
        if result.is_err() {
            let _ = fs::remove_dir_all(&staging);
        }
        result
    }

    /// Verifies one installed version.
    pub fn verify(&self, version: &str) -> Result<ToolchainManifest, ToolchainError> {
        let root = self.version_dir(version);
        if !root.is_dir() {
            return Err(ToolchainError::NotInstalled {
                version: version.to_owned(),
            });
        }
        let marker = read_marker(&root)?;
        verify_tree(&root, &marker.manifest)?;
        Ok(marker.manifest)
    }

    /// Returns the latest installed toolchain.
    pub fn latest(&self) -> Result<InstalledToolchain, ToolchainError> {
        self.list()?
            .into_iter()
            .next_back()
            .ok_or_else(|| ToolchainError::NotInstalled {
                version: "any".to_owned(),
            })
    }

    /// Writes `seal-toolchain.toml` pinning an installed version.
    pub fn pin(&self, project: &Path, version: &str) -> Result<ToolchainManifest, ToolchainError> {
        let manifest = self.verify(version)?;
        write_pin(project, &manifest)?;
        Ok(manifest)
    }

    /// Pins the latest installed version.
    pub fn update(&self, project: &Path) -> Result<ToolchainManifest, ToolchainError> {
        let latest = self.latest()?;
        write_pin(project, &latest.manifest)?;
        Ok(latest.manifest)
    }

    /// Returns the installed toolchain pinned by `seal-toolchain.toml`.
    ///
    /// The pin is a copy of the manifest recorded at install time. Comparing
    /// both rejects a hand-edited checksum, target list, or signature before a
    /// build trusts the installation.
    pub fn which(&self, project: &Path) -> Result<Option<InstalledToolchain>, ToolchainError> {
        let Some(manifest) = read_pin(project)? else {
            return Ok(None);
        };
        let root = self.version_dir(&manifest.version);
        if !root.is_dir() {
            return Err(ToolchainError::NotInstalled {
                version: manifest.version,
            });
        }
        let installed = read_marker(&root)?;
        if installed.manifest != manifest {
            return Err(ToolchainError::PinMismatch {
                detail: format!(
                    "`{}` does not match the manifest recorded for toolchain `{}`; re-run `seal toolchain pin {}` after verifying the installation",
                    project.join(PIN_NAME).display(),
                    manifest.version,
                    manifest.version
                ),
            });
        }
        verify_tree(&root, &manifest)?;
        Ok(Some(InstalledToolchain {
            root,
            manifest,
            reused: true,
        }))
    }
}

/// Reads the project pin, or `None` when the project does not pin one.
pub fn read_pin(project: &Path) -> Result<Option<ToolchainManifest>, ToolchainError> {
    let path = project.join(PIN_NAME);
    if !path.exists() {
        return Ok(None);
    }
    ToolchainManifest::load(&path).map(Some)
}

/// Writes a project pin atomically.
pub fn write_pin(project: &Path, manifest: &ToolchainManifest) -> Result<(), ToolchainError> {
    manifest.validate()?;
    let text = manifest.to_toml()?;
    seal_fs::write_atomic(&project.join(PIN_NAME), text.as_bytes()).map_err(|error| {
        ToolchainError::io(
            format!("cannot write `{}`", project.join(PIN_NAME).display()),
            error,
        )
    })
}

/// Checks that the pin can be used for `target` by the embedded compiler.
pub fn ensure_compatible(
    pin: &ToolchainManifest,
    current: &str,
    target: &str,
) -> Result<(), ToolchainError> {
    if pin.version != current {
        return Err(ToolchainError::Incompatible {
            pinned: pin.version.clone(),
            current: current.to_owned(),
        });
    }
    validate_checksum(&pin.checksum)?;
    if !pin.declares_target(target) {
        return Err(ToolchainError::TargetNotDeclared {
            triple: target.to_owned(),
            version: pin.version.clone(),
        });
    }
    Ok(())
}

fn copy_tree(source: &Path, destination: &Path) -> Result<(), ToolchainError> {
    fs::create_dir_all(destination).map_err(|error| {
        ToolchainError::io(format!("cannot create `{}`", destination.display()), error)
    })?;
    for entry in WalkDir::new(source).follow_links(false) {
        let entry = entry.map_err(|error| {
            ToolchainError::manifest(format!(
                "cannot inspect the toolchain bundle `{}`: {error}",
                source.display()
            ))
        })?;
        let relative = entry.path().strip_prefix(source).map_err(|_| {
            ToolchainError::manifest(format!(
                "the bundle entry `{}` leaves `{}`",
                entry.path().display(),
                source.display()
            ))
        })?;
        if relative.as_os_str().is_empty() {
            continue;
        }
        if relative.to_string_lossy() == MARKER_NAME {
            continue;
        }
        let target = destination.join(relative);
        let file_type = entry.file_type();
        if file_type.is_symlink() {
            return Err(ToolchainError::UnsafeEntry {
                path: relative.to_path_buf(),
            });
        }
        if file_type.is_dir() {
            fs::create_dir_all(&target).map_err(|error| {
                ToolchainError::io(format!("cannot create `{}`", target.display()), error)
            })?;
        } else if file_type.is_file() {
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent).map_err(|error| {
                    ToolchainError::io(format!("cannot create `{}`", parent.display()), error)
                })?;
            }
            fs::copy(entry.path(), &target).map_err(|error| {
                ToolchainError::io(
                    format!(
                        "cannot copy `{}` to `{}`",
                        entry.path().display(),
                        target.display()
                    ),
                    error,
                )
            })?;
        } else {
            return Err(ToolchainError::UnsafeEntry {
                path: relative.to_path_buf(),
            });
        }
    }
    Ok(())
}

fn sequence() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    SEQUENCE.fetch_add(1, Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use super::{ToolchainStore, ensure_compatible, read_pin, write_pin};
    use crate::ToolchainError;
    use crate::manifest::ToolchainManifest;
    use crate::verify::tree_checksum;

    fn scratch(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "seal-toolchain-store-{name}-{}-{}",
            std::process::id(),
            super::sequence()
        ));
        fs::create_dir_all(&path).expect("scratch");
        path
    }

    fn bundle(name: &str, version: &str) -> (PathBuf, ToolchainManifest) {
        let root = scratch(name);
        fs::create_dir_all(root.join("bin")).expect("bin");
        fs::write(root.join("bin/sealc"), format!("compiler {version}")).expect("sealc");
        let checksum = tree_checksum(&root).expect("checksum");
        let manifest = ToolchainManifest {
            channel: "stable".to_owned(),
            version: version.to_owned(),
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
    fn installs_verifies_lists_and_reuses_a_bundle() {
        let project = scratch("project");
        let store = ToolchainStore::for_project(&project);
        let (bundle, manifest) = bundle("bundle", "0.1.0");
        let installed = store.install_from(&bundle).expect("installs");
        assert!(!installed.reused);
        assert_eq!(installed.manifest, manifest);
        assert!(installed.root.join("bin/sealc").is_file());
        assert!(installed.root.join(".seal-installed.json").is_file());

        let reused = store.install_from(&bundle).expect("reuses");
        assert!(reused.reused);
        assert_eq!(store.list().expect("lists").len(), 1);
        assert_eq!(store.verify("0.1.0").expect("verifies"), manifest);

        let _ = fs::remove_dir_all(project);
        let _ = fs::remove_dir_all(bundle);
    }

    #[test]
    fn a_tampered_bundle_is_never_installed() {
        let project = scratch("tampered-project");
        let store = ToolchainStore::for_project(&project);
        let (bundle, _manifest) = bundle("tampered-bundle", "0.1.0");
        fs::write(bundle.join("bin/sealc"), b"tampered").expect("tamper");
        assert!(matches!(
            store.install_from(&bundle),
            Err(ToolchainError::ChecksumMismatch { .. })
        ));
        assert!(!store.version_dir("0.1.0").exists());
        let _ = fs::remove_dir_all(project);
        let _ = fs::remove_dir_all(bundle);
    }

    #[test]
    fn pin_update_and_which_follow_the_store() {
        let project = scratch("pin-project");
        let store = ToolchainStore::for_project(&project);
        assert_eq!(store.which(&project).expect("no pin"), None);

        let (first, _) = bundle("pin-first", "0.1.0");
        let (second, second_manifest) = bundle("pin-second", "0.1.1");
        store.install_from(&first).expect("first");
        store.install_from(&second).expect("second");
        store.pin(&project, "0.1.0").expect("pin");
        assert_eq!(
            read_pin(&project).expect("read").expect("pin").version,
            "0.1.0"
        );
        assert_eq!(
            store
                .which(&project)
                .expect("which")
                .expect("installed")
                .manifest
                .version,
            "0.1.0"
        );

        let mut altered = read_pin(&project).expect("read").expect("pin");
        altered.checksum = "0".repeat(64);
        write_pin(&project, &altered).expect("altered pin");
        assert!(matches!(
            store.which(&project),
            Err(ToolchainError::PinMismatch { .. })
        ));
        store.pin(&project, "0.1.0").expect("re-pin");

        store.update(&project).expect("update");
        assert_eq!(
            read_pin(&project).expect("read").expect("pin"),
            second_manifest
        );
        assert_eq!(
            store
                .which(&project)
                .expect("which")
                .expect("installed")
                .manifest
                .version,
            "0.1.1"
        );

        let _ = fs::remove_dir_all(project);
        let _ = fs::remove_dir_all(first);
        let _ = fs::remove_dir_all(second);
    }

    #[test]
    fn pinning_a_missing_version_fails() {
        let project = scratch("missing-project");
        let store = ToolchainStore::for_project(&project);
        assert!(matches!(
            store.pin(&project, "9.9.9"),
            Err(ToolchainError::NotInstalled { .. })
        ));
        let _ = fs::remove_dir_all(project);
    }

    #[test]
    fn compatibility_requires_the_current_version_and_a_declared_target() {
        let (_root, manifest) = bundle("compat", "0.1.0");
        ensure_compatible(&manifest, "0.1.0", "x86_64-unknown-linux-gnu").expect("compatible");
        assert!(matches!(
            ensure_compatible(&manifest, "0.2.0", "x86_64-unknown-linux-gnu"),
            Err(ToolchainError::Incompatible { .. })
        ));
        assert!(matches!(
            ensure_compatible(&manifest, "0.1.0", "aarch64-unknown-linux-gnu"),
            Err(ToolchainError::TargetNotDeclared { .. })
        ));
    }

    #[test]
    fn a_pin_is_written_inside_the_project() {
        let project = scratch("write-pin");
        let (_root, manifest) = bundle("write-pin-bundle", "0.1.0");
        write_pin(&project, &manifest).expect("writes");
        assert!(project.join("seal-toolchain.toml").is_file());
        assert_eq!(read_pin(&project).expect("read"), Some(manifest));
        let _ = fs::remove_dir_all(project);
    }

    #[test]
    fn the_store_root_is_project_local() {
        let store = ToolchainStore::for_project(Path::new("/project"));
        assert_eq!(store.root(), Path::new("/project/seal/toolchains"));
    }
}
