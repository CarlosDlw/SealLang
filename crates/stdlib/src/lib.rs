//! Locating, validating, and digesting the Seal standard library.
//!
//! The standard library is a directory of ordinary Seal packages described by
//! one `stdlib.toml`. This crate turns that directory into validated metadata:
//! package roots, source files, internal dependency edges, the selected
//! preludes, and one stable content digest. It never compiles Seal source and
//! never depends on the compiler driver, so the same rules apply to `seal`,
//! `sealc`, the build cache, and `seal stdlib verify`.

#![warn(missing_docs)]

mod descriptor;
mod digest;
mod error;
mod locate;
mod package;

use std::path::{Path, PathBuf};

pub use descriptor::{Descriptor, FORMAT_VERSION};
pub use error::{StdlibError, StdlibResult};
pub use locate::ENV_VAR;
pub use package::{Dependency, Package};

/// A validated standard library rooted at one directory.
#[derive(Clone, Debug)]
pub struct Stdlib {
    root: PathBuf,
    descriptor: Descriptor,
    packages: Vec<Package>,
    digest: String,
}

impl Stdlib {
    /// Finds the standard-library root without reading it.
    pub fn locate() -> StdlibResult<PathBuf> {
        locate::locate()
    }

    /// Locates and loads the standard library installed for this compiler.
    pub fn load() -> StdlibResult<Self> {
        let root = Self::locate()?;
        Self::load_from(&root)
    }

    /// Loads the standard library rooted at `root`.
    ///
    /// The descriptor, every package manifest, every source file, every
    /// dependency edge, and the stored digest are validated before this
    /// function returns.
    pub fn load_from(root: &Path) -> StdlibResult<Self> {
        Self::load_inner(root, true)
    }

    /// Loads the standard library for inspection without comparing its digest.
    ///
    /// `seal stdlib version` and `seal stdlib verify` need the descriptor and
    /// packages even when an edited tree no longer matches its digest. The
    /// computed digest is stored on the result, so [`Self::verify`] can report
    /// the mismatch as `E8140` instead of failing while loading.
    pub fn load_unverified(root: &Path) -> StdlibResult<Self> {
        Self::load_inner(root, false)
    }

    fn load_inner(root: &Path, check_digest: bool) -> StdlibResult<Self> {
        let root = root.canonicalize().map_err(|error| {
            StdlibError::locate(format!(
                "cannot resolve the standard-library root `{}`: {error}",
                root.display()
            ))
        })?;
        let descriptor = Descriptor::load(&root)?;
        let packages = package::load_all(&root, &descriptor)?;
        let digest = digest::compute(&root, &descriptor, &packages)?;
        if check_digest && digest != descriptor.digest {
            return Err(StdlibError::digest(format!(
                "the standard library at `{}` has digest {digest} but its descriptor claims {}",
                root.display(),
                descriptor.digest
            )));
        }
        Ok(Self {
            root,
            descriptor,
            packages,
            digest,
        })
    }

    /// Loads the standard library with an explicit root.
    pub fn load_with_root(root: &Path) -> StdlibResult<Self> {
        Self::load_from(root)
    }

    /// Returns the canonical standard-library root.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Returns the parsed descriptor.
    pub fn descriptor(&self) -> &Descriptor {
        &self.descriptor
    }

    /// Returns the validated packages in dependency order.
    pub fn packages(&self) -> &[Package] {
        &self.packages
    }

    /// Returns one package by name.
    pub fn package(&self, name: &str) -> Option<&Package> {
        self.packages.iter().find(|package| package.name == name)
    }

    /// Returns the language version the standard library targets.
    pub fn language(&self) -> &str {
        &self.descriptor.language
    }

    /// Returns the standard-library release version.
    pub fn version(&self) -> &str {
        &self.descriptor.version
    }

    /// Returns the digest computed from the descriptor, manifests, and sources.
    pub fn digest(&self) -> &str {
        &self.digest
    }

    /// Returns the prelude module selected for `package`.
    pub fn prelude_for(&self, package: &str) -> Option<&str> {
        self.descriptor.prelude_for(package)
    }

    /// Recomputes the digest and compares it with the stored descriptor value.
    ///
    /// `load_from` already performs this check. This method exists for
    /// `seal stdlib verify`, which reports the comparison explicitly.
    pub fn verify(&self) -> StdlibResult<()> {
        let digest = digest::compute(&self.root, &self.descriptor, &self.packages)?;
        if digest != self.descriptor.digest {
            return Err(StdlibError::verify(format!(
                "the standard library at `{}` has digest {digest} but its descriptor claims {}",
                self.root.display(),
                self.descriptor.digest
            )));
        }
        Ok(())
    }
}
