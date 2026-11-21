//! Deterministic vendor trees for offline builds.

use std::path::{Component, Path, PathBuf};

use semver::Version;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use walkdir::WalkDir;

use crate::download::verify_cached;
use crate::error::{RegistryError, RegistryResult};

/// The vendor manifest path relative to the project root.
pub const MANIFEST: &str = "vendor/seal-vendor.toml";

/// The vendor manifest.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct VendorManifest {
    /// Schema version.
    pub version: u32,
    /// Vendored packages, sorted by name and version.
    #[serde(default)]
    pub package: Vec<VendorPackage>,
}

/// One vendored package.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VendorPackage {
    /// Package name.
    pub name: String,
    /// Package version.
    pub version: String,
    /// Archive SHA-256 digest.
    pub checksum: String,
    /// Lockfile source string.
    pub source: String,
    /// Directory relative to the project root, always below `vendor/`.
    pub directory: String,
    /// Deterministic digest of the copied tree.
    pub tree: String,
}

/// Input for `seal vendor`.
#[derive(Clone, Debug)]
pub struct VendorInput {
    /// Package name.
    pub name: String,
    /// Package version.
    pub version: Version,
    /// Archive checksum.
    pub checksum: String,
    /// Lockfile source.
    pub source: String,
    /// Verified package root.
    pub root: PathBuf,
}

/// Loads the vendor manifest when present.
pub fn load(project_root: &Path) -> RegistryResult<Option<VendorManifest>> {
    let path = project_root.join(MANIFEST);
    if !path.is_file() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&path).map_err(|error| {
        RegistryError::vendor(format!("cannot read `{}`: {error}", path.display()))
    })?;
    let manifest: VendorManifest = toml::from_str(&text)
        .map_err(|error| RegistryError::vendor(format!("invalid `{}`: {error}", path.display())))?;
    if manifest.version != 1 {
        return Err(RegistryError::vendor(format!(
            "`{}` uses vendor format {}, but this client understands 1",
            path.display(),
            manifest.version
        )));
    }
    for package in &manifest.package {
        if package.name.is_empty()
            || Version::parse(&package.version).is_err()
            || package.checksum.is_empty()
            || package.tree.is_empty()
            || !safe_relative(&package.directory)
        {
            return Err(RegistryError::vendor(format!(
                "`{}` contains an invalid package entry",
                path.display()
            )));
        }
    }
    Ok(Some(manifest))
}

/// Returns a vendored package root after verifying its tree.
pub fn materialize(
    project_root: &Path,
    manifest: &VendorManifest,
    name: &str,
    version: &Version,
    checksum: &str,
) -> RegistryResult<Option<PathBuf>> {
    let Some(package) = manifest.package.iter().find(|package| {
        package.name == name
            && package.version == version.to_string()
            && package.checksum == checksum
    }) else {
        return Ok(None);
    };
    let root = project_root.join(&package.directory);
    if !safe_relative(&package.directory) || !root.is_dir() {
        return Err(RegistryError::vendor(format!(
            "the vendor directory `{}` is missing or unsafe",
            package.directory
        )));
    }
    verify_cached(&root, checksum)?;
    if tree_digest(&root)? != package.tree {
        return Err(RegistryError::vendor(format!(
            "the vendor tree `{}` does not match its recorded digest; run `seal vendor`",
            package.directory
        )));
    }
    Ok(Some(root))
}

/// Copies packages into `vendor/` and writes the manifest.
pub fn install(project_root: &Path, inputs: &[VendorInput]) -> RegistryResult<VendorManifest> {
    let vendor_root = project_root.join("vendor");
    std::fs::create_dir_all(&vendor_root).map_err(|error| {
        RegistryError::vendor(format!(
            "cannot create `{}`: {error}",
            vendor_root.display()
        ))
    })?;
    let previous = load(project_root)?;
    if previous.is_none()
        && std::fs::read_dir(&vendor_root)
            .map_err(|error| {
                RegistryError::vendor(format!("cannot read `{}`: {error}", vendor_root.display()))
            })?
            .next()
            .is_some()
    {
        return Err(RegistryError::vendor(format!(
            "`{}` is not empty and has no `seal-vendor.toml`; refusing to replace it",
            vendor_root.display()
        )));
    }
    if let Some(previous) = &previous {
        for package in &previous.package {
            let path = project_root.join(&package.directory);
            if safe_relative(&package.directory) && path.starts_with(&vendor_root) {
                let _ = std::fs::remove_dir_all(path);
            }
        }
    }
    let mut manifest = VendorManifest {
        version: 1,
        package: Vec::new(),
    };
    let mut inputs: Vec<&VendorInput> = inputs.iter().collect();
    inputs.sort_by(|left, right| {
        left.name
            .cmp(&right.name)
            .then_with(|| left.version.cmp(&right.version))
    });
    for input in inputs {
        verify_cached(&input.root, &input.checksum)?;
        let directory = format!("vendor/{}-{}", safe_component(&input.name), input.version);
        let destination = project_root.join(&directory);
        if destination.exists() {
            std::fs::remove_dir_all(&destination).map_err(|error| {
                RegistryError::vendor(format!(
                    "cannot remove `{}`: {error}",
                    destination.display()
                ))
            })?;
        }
        copy_tree(&input.root, &destination)?;
        std::fs::write(destination.join(".seal-checksum"), &input.checksum).map_err(|error| {
            RegistryError::vendor(format!("cannot write the checksum marker: {error}"))
        })?;
        let tree = tree_digest(&destination)?;
        manifest.package.push(VendorPackage {
            name: input.name.clone(),
            version: input.version.to_string(),
            checksum: input.checksum.clone(),
            source: input.source.clone(),
            directory,
            tree,
        });
    }
    let text = toml::to_string_pretty(&manifest).map_err(|error| {
        RegistryError::vendor(format!("cannot encode the vendor manifest: {error}"))
    })?;
    let path = project_root.join(MANIFEST);
    let temporary = path.with_extension("tmp");
    std::fs::write(&temporary, text).map_err(|error| {
        RegistryError::vendor(format!("cannot write `{}`: {error}", temporary.display()))
    })?;
    std::fs::rename(&temporary, &path).map_err(|error| {
        RegistryError::vendor(format!("cannot replace `{}`: {error}", path.display()))
    })?;
    Ok(manifest)
}

fn copy_tree(source: &Path, destination: &Path) -> RegistryResult<()> {
    for entry in WalkDir::new(source).follow_links(false) {
        let entry = entry.map_err(|error| {
            RegistryError::vendor(format!("cannot walk `{}`: {error}", source.display()))
        })?;
        let relative = entry.path().strip_prefix(source).map_err(|_| {
            RegistryError::vendor(format!("`{}` is outside its root", entry.path().display()))
        })?;
        if relative.as_os_str().is_empty() {
            continue;
        }
        if relative == Path::new(".seal-checksum") {
            continue;
        }
        let target = destination.join(relative);
        let kind = entry.file_type();
        if kind.is_symlink() {
            return Err(RegistryError::vendor(format!(
                "the package contains the symlink `{}`",
                relative.display()
            )));
        }
        if kind.is_dir() {
            std::fs::create_dir_all(&target).map_err(|error| {
                RegistryError::vendor(format!("cannot create `{}`: {error}", target.display()))
            })?;
        } else if kind.is_file() {
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent).map_err(|error| {
                    RegistryError::vendor(format!("cannot create `{}`: {error}", parent.display()))
                })?;
            }
            std::fs::copy(entry.path(), &target).map_err(|error| {
                RegistryError::vendor(format!(
                    "cannot copy `{}` to `{}`: {error}",
                    entry.path().display(),
                    target.display()
                ))
            })?;
        } else {
            return Err(RegistryError::vendor(format!(
                "the package contains the special file `{}`",
                relative.display()
            )));
        }
    }
    Ok(())
}

fn tree_digest(root: &Path) -> RegistryResult<String> {
    let mut files = Vec::new();
    for entry in WalkDir::new(root).follow_links(false) {
        let entry = entry.map_err(|error| {
            RegistryError::vendor(format!("cannot walk `{}`: {error}", root.display()))
        })?;
        if !entry.file_type().is_file() {
            continue;
        }
        let relative = entry.path().strip_prefix(root).map_err(|_| {
            RegistryError::vendor(format!("`{}` is outside its root", entry.path().display()))
        })?;
        if relative == Path::new(".seal-checksum") {
            continue;
        }
        files.push((relative.to_path_buf(), entry.path().to_path_buf()));
    }
    files.sort_by(|left, right| left.0.cmp(&right.0));
    let mut hasher = Sha256::new();
    for (relative, path) in files {
        let name = relative.to_string_lossy();
        hasher.update(name.as_bytes());
        hasher.update([0]);
        let bytes = std::fs::read(&path).map_err(|error| {
            RegistryError::vendor(format!("cannot read `{}`: {error}", path.display()))
        })?;
        hasher.update((bytes.len() as u64).to_le_bytes());
        hasher.update(&bytes);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path)
                .map(|meta| meta.permissions().mode() & 0o111)
                .unwrap_or(0);
            hasher.update([if mode != 0 { 1 } else { 0 }]);
        }
    }
    Ok(format!("sha256:{}", hex::encode(hasher.finalize())))
}

fn safe_relative(path: &str) -> bool {
    let path = Path::new(path);
    let components: Vec<Component<'_>> = path.components().collect();
    components.len() > 1
        && components[0] == Component::Normal(std::ffi::OsStr::new("vendor"))
        && components
            .iter()
            .all(|component| matches!(component, Component::Normal(_)))
}

fn safe_component(name: &str) -> String {
    let mut output = String::new();
    for character in name.chars() {
        if character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.') {
            output.push(character);
        } else {
            output.push('_');
        }
    }
    if output.is_empty() || output == "." || output == ".." {
        output = "package".to_owned();
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vendor_directories_stay_below_vendor() {
        assert!(safe_relative("vendor/foo-1.0.0"));
        assert!(!safe_relative("vendor"));
        assert!(!safe_relative("vendor/../escape"));
        assert!(!safe_relative("other/foo"));
    }

    #[test]
    fn package_names_are_reduced_to_one_safe_component() {
        assert_eq!(safe_component("@scope/name"), "_scope_name");
        assert_eq!(safe_component(".."), "package");
    }
}
