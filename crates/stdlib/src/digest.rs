//! Stable standard-library content digests.

use std::path::Path;

use crate::descriptor::Descriptor;
use crate::error::{StdlibError, StdlibResult};
use crate::package::Package;

/// Computes the digest of a loaded standard library.
///
/// Absolute roots are deliberately excluded. Paths are recorded relative to
/// the standard-library root, so the same tree produces the same digest after
/// it is copied or installed elsewhere.
pub(crate) fn compute(
    root: &Path,
    descriptor: &Descriptor,
    packages: &[Package],
) -> StdlibResult<String> {
    let mut hasher = blake3::Hasher::new();
    update_text(&mut hasher, "seal-stdlib");
    update_text(&mut hasher, "1");
    update_text(&mut hasher, &descriptor.format.to_string());
    update_text(&mut hasher, &descriptor.language);
    update_text(&mut hasher, &descriptor.version);
    for name in &descriptor.packages {
        update_text(&mut hasher, name);
    }
    for (name, module) in &descriptor.prelude {
        update_text(&mut hasher, "prelude");
        update_text(&mut hasher, name);
        update_text(&mut hasher, module);
    }

    let mut ordered: Vec<&Package> = packages.iter().collect();
    ordered.sort_by(|left, right| left.name.cmp(&right.name));
    for package in ordered {
        update_text(&mut hasher, "package");
        update_text(&mut hasher, &package.name);
        update_text(&mut hasher, &package.version);
        let manifest = package.root.join("seal.toml");
        hash_file(&mut hasher, root, &manifest)?;
        for file in &package.files {
            hash_file(&mut hasher, root, file)?;
        }
        for dependency in &package.dependencies {
            update_text(&mut hasher, "dependency");
            update_text(&mut hasher, &dependency.alias);
            update_text(&mut hasher, &dependency.package);
        }
    }
    Ok(hasher.finalize().to_hex().to_string())
}

fn hash_file(hasher: &mut blake3::Hasher, root: &Path, path: &Path) -> StdlibResult<()> {
    let relative = path.strip_prefix(root).map_err(|_| {
        StdlibError::digest(format!(
            "the standard-library file `{}` is outside `{}`",
            path.display(),
            root.display()
        ))
    })?;
    update_text(hasher, &slash_path(relative));
    let bytes = std::fs::read(path).map_err(|error| {
        StdlibError::digest(format!(
            "cannot read the standard-library file `{}` for hashing: {error}",
            path.display()
        ))
    })?;
    hasher.update(&(bytes.len() as u64).to_le_bytes());
    hasher.update(&bytes);
    Ok(())
}

fn update_text(hasher: &mut blake3::Hasher, text: &str) {
    hasher.update(&(text.len() as u64).to_le_bytes());
    hasher.update(text.as_bytes());
}

fn slash_path(path: &Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}
