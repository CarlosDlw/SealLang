//! Path remapping for reproducible builds.
//!
//! `--remap-path-prefix=from=to` rewrites paths in diagnostics and metadata so
//! that builds made in different directories produce identical output. Matching
//! is done on path components, so `/home/user/project` never matches
//! `/home/user/project-other`.

use std::path::{Path, PathBuf};

use crate::error::RemapError;

/// A set of ordered path prefix replacements.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PathRemapper {
    prefixes: Vec<(PathBuf, PathBuf)>,
}

impl PathRemapper {
    /// Creates an empty remapper, which leaves every path unchanged.
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    /// Parses `from=to` specifications.
    ///
    /// Duplicate `from` prefixes are rejected implicitly by keeping the first
    /// one; callers that need to detect duplicates should validate their input
    /// first.
    pub fn from_specs<'a>(specs: impl IntoIterator<Item = &'a str>) -> Result<Self, RemapError> {
        let mut remapper = Self::new();
        for spec in specs {
            let Some((from, to)) = spec.split_once('=') else {
                return Err(RemapError::MissingSeparator {
                    spec: spec.to_owned(),
                });
            };
            if from.is_empty() {
                return Err(RemapError::EmptyPrefix {
                    spec: spec.to_owned(),
                });
            }
            if remapper
                .prefixes
                .iter()
                .all(|(existing, _)| existing != Path::new(from))
            {
                remapper
                    .prefixes
                    .push((PathBuf::from(from), PathBuf::from(to)));
            }
        }
        // Longest prefix first, so a nested remap wins over its parent.
        remapper
            .prefixes
            .sort_by_key(|(from, _)| std::cmp::Reverse(from.components().count()));
        Ok(remapper)
    }

    /// Returns `true` if no prefix is configured.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.prefixes.is_empty()
    }

    /// Returns the number of configured prefixes.
    #[inline]
    pub fn len(&self) -> usize {
        self.prefixes.len()
    }

    /// Rewrites `path` using the first matching prefix, or returns it unchanged.
    pub fn remap(&self, path: &Path) -> PathBuf {
        for (from, to) in &self.prefixes {
            if let Ok(rest) = path.strip_prefix(from) {
                return if rest.as_os_str().is_empty() {
                    to.clone()
                } else {
                    to.join(rest)
                };
            }
        }
        path.to_path_buf()
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::PathRemapper;

    #[test]
    fn empty_remapper_is_identity() {
        let remapper = PathRemapper::new();
        let path = Path::new("/home/user/project/main.sl");
        assert_eq!(remapper.remap(path), path);
        assert!(remapper.is_empty());
    }

    #[test]
    fn prefixes_are_matched_by_components() {
        let remapper = PathRemapper::from_specs(["/home/user/project=/src"]).expect("valid spec");
        assert_eq!(
            remapper.remap(Path::new("/home/user/project/main.sl")),
            PathBuf::from("/src/main.sl")
        );
        assert_eq!(
            remapper.remap(Path::new("/home/user/project-other/main.sl")),
            PathBuf::from("/home/user/project-other/main.sl")
        );
        assert_eq!(
            remapper.remap(Path::new("/home/user/project")),
            PathBuf::from("/src")
        );
    }

    #[test]
    fn longest_prefix_wins() {
        let remapper = PathRemapper::from_specs(["/home/user=/home", "/home/user/project=/src"])
            .expect("valid specs");
        assert_eq!(remapper.len(), 2);
        assert_eq!(
            remapper.remap(Path::new("/home/user/project/main.sl")),
            PathBuf::from("/src/main.sl")
        );
        assert_eq!(
            remapper.remap(Path::new("/home/user/other.sl")),
            PathBuf::from("/home/other.sl")
        );
    }

    #[test]
    fn invalid_specs_are_rejected() {
        assert!(PathRemapper::from_specs(["no-separator"]).is_err());
        assert!(PathRemapper::from_specs(["=/dest"]).is_err());
    }

    #[test]
    fn duplicate_prefixes_keep_the_first() {
        let remapper = PathRemapper::from_specs(["/a=/first", "/a=/second"]).expect("valid specs");
        assert_eq!(remapper.len(), 1);
        assert_eq!(remapper.remap(Path::new("/a/b")), PathBuf::from("/first/b"));
    }
}
