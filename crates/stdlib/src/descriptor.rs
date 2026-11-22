//! The standard-library descriptor.

use std::collections::BTreeMap;
use std::path::Path;

use semver::Version;
use serde::Deserialize;

use crate::error::{StdlibError, StdlibResult};

/// The descriptor format understood by this compiler.
pub const FORMAT_VERSION: u32 = 1;

/// The top-level `stdlib.toml` descriptor.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Descriptor {
    /// Descriptor schema version.
    pub format: u32,
    /// Language version the sources use, such as `0.1`.
    pub language: String,
    /// Standard-library release version.
    pub version: String,
    /// Packages in dependency order.
    pub packages: Vec<String>,
    /// Package name to prelude module path inside that package.
    #[serde(default)]
    pub prelude: BTreeMap<String, String>,
    /// Digest of the descriptor metadata, manifests, and sources.
    pub digest: String,
}

impl Descriptor {
    /// Reads and validates `stdlib.toml` in `root`.
    pub fn load(root: &Path) -> StdlibResult<Self> {
        let path = root.join("stdlib.toml");
        if !path.is_file() {
            return Err(StdlibError::descriptor(format!(
                "the standard-library descriptor `{}` is missing",
                path.display()
            )));
        }
        let text = std::fs::read_to_string(&path).map_err(|error| {
            StdlibError::descriptor(format!(
                "cannot read the standard-library descriptor `{}`: {error}",
                path.display()
            ))
        })?;
        let descriptor: Self = toml::from_str(&text).map_err(|error| {
            StdlibError::descriptor(format!(
                "the standard-library descriptor `{}` is invalid: {error}",
                path.display()
            ))
        })?;
        descriptor.validate()?;
        Ok(descriptor)
    }

    /// Validates every descriptor invariant that does not require package files.
    pub fn validate(&self) -> StdlibResult<()> {
        if self.format != FORMAT_VERSION {
            return Err(StdlibError::descriptor(format!(
                "the standard-library descriptor uses format {}, but this compiler understands format {FORMAT_VERSION}",
                self.format
            )));
        }
        let language = seal_version::LanguageVersion::parse(&self.language).map_err(|error| {
            StdlibError::descriptor(format!(
                "the standard-library language `{}` is not a language version: {error}",
                self.language
            ))
        })?;
        if language != seal_version::LanguageVersion::CURRENT {
            return Err(StdlibError::language(format!(
                "the standard library targets language `{language}`, but this compiler implements `{}`",
                seal_version::LanguageVersion::CURRENT
            )));
        }
        Version::parse(&self.version).map_err(|error| {
            StdlibError::descriptor(format!(
                "the standard-library version `{}` is not semantic: {error}",
                self.version
            ))
        })?;
        if self.packages.is_empty() {
            return Err(StdlibError::descriptor(
                "the standard-library descriptor lists no packages",
            ));
        }
        let mut names = std::collections::BTreeSet::new();
        for name in &self.packages {
            validate_name(name)?;
            if !names.insert(name) {
                return Err(StdlibError::dependencies(format!(
                    "the standard library names the package `{name}` twice"
                )));
            }
        }
        for (name, module) in &self.prelude {
            if !names.contains(name) {
                return Err(StdlibError::descriptor(format!(
                    "the prelude for `{name}` names a package the descriptor does not list"
                )));
            }
            if module.is_empty() || module.split("::").any(str::is_empty) {
                return Err(StdlibError::descriptor(format!(
                    "the prelude for `{name}` has an empty module path"
                )));
            }
        }
        if self.digest.len() != 64 || !self.digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(StdlibError::digest(
                "the standard-library descriptor must carry a 64-digit hexadecimal digest",
            ));
        }
        Ok(())
    }

    /// Returns the prelude module selected for `package`.
    pub fn prelude_for(&self, package: &str) -> Option<&str> {
        self.prelude.get(package).map(String::as_str)
    }
}

fn validate_name(name: &str) -> StdlibResult<()> {
    if name.is_empty()
        || !name.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_' || byte == b'-'
        })
    {
        return Err(StdlibError::descriptor(format!(
            "`{name}` is not a valid standard-library package name"
        )));
    }
    Ok(())
}
