//! The `seal-toolchain.toml` manifest shared by bundles, installs, and pins.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

use semver::Version;
use serde::{Deserialize, Serialize};

use crate::error::ToolchainError;

/// The file name of a bundle, install, and pin manifest.
pub const MANIFEST_NAME: &str = "seal-toolchain.toml";

/// The marker written inside an installed tree.
pub(crate) const MARKER_NAME: &str = ".seal-installed.json";

/// One toolchain manifest.
///
/// `checksum` covers every regular file in the tree except the manifest and the
/// install marker. The path, executable bit, length, and bytes of each entry are
/// hashed in sorted order, so a changed component is always detected.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct ToolchainManifest {
    /// The release channel (`stable`, `beta`, `nightly`, or a project channel).
    pub channel: String,
    /// The semantic version.
    pub version: String,
    /// The lowercase SHA-256 tree digest.
    pub checksum: String,
    /// An optional Ed25519 signature over the canonical manifest message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
    /// Named component paths, relative to the toolchain root.
    #[serde(default)]
    pub components: BTreeMap<String, PathBuf>,
    /// Target triples this toolchain can compile for.
    #[serde(default)]
    pub targets: Vec<String>,
}

impl ToolchainManifest {
    /// Parses and validates one manifest.
    pub fn parse(text: &str) -> Result<Self, ToolchainError> {
        let manifest: Self = toml::from_str(text).map_err(|error| {
            ToolchainError::manifest(format!("cannot parse `{MANIFEST_NAME}`: {error}"))
        })?;
        manifest.validate()?;
        Ok(manifest)
    }

    /// Reads and validates a manifest from `path`.
    pub fn load(path: &Path) -> Result<Self, ToolchainError> {
        let text = std::fs::read_to_string(path).map_err(|error| {
            ToolchainError::io(format!("cannot read `{}`", path.display()), error)
        })?;
        Self::parse(&text)
    }

    /// Validates every manifest field.
    pub fn validate(&self) -> Result<(), ToolchainError> {
        if self.channel.trim().is_empty() {
            return Err(ToolchainError::manifest(
                "the toolchain channel cannot be empty",
            ));
        }
        Version::parse(&self.version).map_err(|error| ToolchainError::Version {
            value: self.version.clone(),
            detail: error.to_string(),
        })?;
        validate_checksum(&self.checksum)?;
        if let Some(signature) = &self.signature
            && (signature.len() != 128 || !signature.bytes().all(|byte| byte.is_ascii_hexdigit()))
        {
            return Err(ToolchainError::manifest(
                "the toolchain signature must be 128 hexadecimal characters",
            ));
        }
        if self.components.is_empty() {
            return Err(ToolchainError::manifest(
                "the toolchain declares no components",
            ));
        }
        if self.targets.is_empty() {
            return Err(ToolchainError::manifest(
                "the toolchain declares no targets",
            ));
        }
        for (name, path) in &self.components {
            if name.trim().is_empty() {
                return Err(ToolchainError::manifest(
                    "a toolchain component has an empty name",
                ));
            }
            validate_relative(path)?;
        }
        for target in &self.targets {
            seal_target::TargetTriple::parse(target)
                .map_err(|error| ToolchainError::manifest(error.to_string()))?;
        }
        Ok(())
    }

    /// Returns the normalized version.
    pub fn semantic_version(&self) -> Result<Version, ToolchainError> {
        Version::parse(&self.version).map_err(|error| ToolchainError::Version {
            value: self.version.clone(),
            detail: error.to_string(),
        })
    }

    /// Returns whether this toolchain declares `target`.
    pub fn declares_target(&self, target: &str) -> bool {
        self.targets.iter().any(|declared| declared == target)
    }

    /// Returns the canonical bytes covered by a signature.
    ///
    /// The message binds the channel, version, checksum, every sorted
    /// component path, and every target, so a valid signature cannot be moved
    /// to a manifest that names a different release or component set.
    ///
    /// Every field is length-prefixed. A newline or `=` in a name therefore
    /// cannot make two different manifests share a canonical message.
    pub fn signature_message(&self) -> Vec<u8> {
        let mut message = Vec::new();
        push_field(&mut message, "seal-toolchain-v1");
        push_field(&mut message, &self.channel);
        push_field(&mut message, &self.version);
        push_field(&mut message, &self.checksum);
        for (name, path) in &self.components {
            push_field(&mut message, name);
            push_bytes(&mut message, path.as_os_str().as_encoded_bytes());
        }
        for target in &self.targets {
            push_field(&mut message, target);
        }
        message
    }

    /// Renders the manifest as TOML.
    pub fn to_toml(&self) -> Result<String, ToolchainError> {
        toml::to_string_pretty(self).map_err(|error| {
            ToolchainError::manifest(format!("cannot encode the manifest: {error}"))
        })
    }

    /// Checks that every component exists as a regular file inside `root`.
    pub fn check_components(&self, root: &Path) -> Result<(), ToolchainError> {
        let canonical_root = root.canonicalize().map_err(|error| {
            ToolchainError::io(format!("cannot resolve `{}`", root.display()), error)
        })?;
        for (name, relative) in &self.components {
            let path = root.join(relative);
            let metadata = std::fs::metadata(&path).map_err(|error| ToolchainError::Component {
                name: name.clone(),
                path: relative.clone(),
                detail: format!("cannot read the component: {error}"),
            })?;
            if !metadata.is_file() {
                return Err(ToolchainError::Component {
                    name: name.clone(),
                    path: relative.clone(),
                    detail: "the component is not a regular file".to_owned(),
                });
            }
            let canonical = path.canonicalize().map_err(|error| {
                ToolchainError::io(format!("cannot resolve `{}`", path.display()), error)
            })?;
            if !canonical.starts_with(&canonical_root) {
                return Err(ToolchainError::Component {
                    name: name.clone(),
                    path: relative.clone(),
                    detail: "the component leaves the toolchain root".to_owned(),
                });
            }
        }
        Ok(())
    }
}

/// Normalizes a checksum and rejects anything that is not SHA-256 hex.
pub fn validate_checksum(value: &str) -> Result<String, ToolchainError> {
    let normalized = value.trim().to_ascii_lowercase();
    if normalized.len() != 64 || !normalized.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(ToolchainError::InvalidChecksum {
            value: value.to_owned(),
        });
    }
    Ok(normalized)
}

fn validate_relative(path: &Path) -> Result<(), ToolchainError> {
    if path.as_os_str().is_empty() {
        return Err(ToolchainError::manifest(
            "a toolchain component path cannot be empty",
        ));
    }
    if path.to_str().is_none() {
        return Err(ToolchainError::manifest(format!(
            "the toolchain component path `{}` is not valid UTF-8",
            path.display()
        )));
    }
    for component in path.components() {
        match component {
            Component::Normal(_) => {}
            Component::CurDir
            | Component::ParentDir
            | Component::RootDir
            | Component::Prefix(_) => {
                return Err(ToolchainError::manifest(format!(
                    "the toolchain component path `{}` must be relative and cannot contain `.` or `..`",
                    path.display()
                )));
            }
        }
    }
    Ok(())
}

fn push_field(message: &mut Vec<u8>, value: &str) {
    push_bytes(message, value.as_bytes());
}

fn push_bytes(message: &mut Vec<u8>, value: &[u8]) {
    message.extend((value.len() as u64).to_le_bytes());
    message.extend(value);
}

#[cfg(test)]
mod tests {
    use super::{ToolchainManifest, validate_checksum};

    fn checksum() -> String {
        "a".repeat(64)
    }

    fn manifest() -> ToolchainManifest {
        ToolchainManifest {
            channel: "stable".to_owned(),
            version: "0.1.0".to_owned(),
            checksum: checksum(),
            signature: None,
            components: [("sealc".to_owned(), "bin/sealc".into())]
                .into_iter()
                .collect(),
            targets: vec!["x86_64-unknown-linux-gnu".to_owned()],
        }
    }

    #[test]
    fn a_complete_manifest_is_valid() {
        let manifest = manifest();
        manifest.validate().expect("valid");
        assert!(manifest.declares_target("x86_64-unknown-linux-gnu"));
        assert!(!manifest.declares_target("aarch64-unknown-linux-gnu"));
    }

    #[test]
    fn checksums_must_be_sha256_hex() {
        assert_eq!(validate_checksum(&checksum()).expect("valid"), checksum());
        assert!(validate_checksum("abc").is_err());
        assert!(validate_checksum(&"z".repeat(64)).is_err());
    }

    #[test]
    fn malformed_manifests_are_rejected() {
        let mut missing_component = manifest();
        missing_component.components.clear();
        assert!(missing_component.validate().is_err());

        let mut missing_target = manifest();
        missing_target.targets.clear();
        assert!(missing_target.validate().is_err());

        let mut bad_path = manifest();
        bad_path
            .components
            .insert("sealc".to_owned(), "../sealc".into());
        assert!(bad_path.validate().is_err());

        let mut bad_version = manifest();
        bad_version.version = "latest".to_owned();
        assert!(bad_version.validate().is_err());

        let mut bad_target = manifest();
        bad_target.targets = vec!["mips-unknown-linux".to_owned()];
        assert!(bad_target.validate().is_err());
    }

    #[test]
    fn a_manifest_round_trips_through_toml() {
        let manifest = manifest();
        let text = manifest.to_toml().expect("serializes");
        let parsed = ToolchainManifest::parse(&text).expect("parses");
        assert_eq!(parsed, manifest);
    }

    #[test]
    fn unknown_fields_are_rejected() {
        let text = format!(
            "channel = \"stable\"\nversion = \"0.1.0\"\nchecksum = \"{}\"\ncomponents = {{ sealc = \"sealc\" }}\ntargets = [\"x86_64-unknown-linux-gnu\"]\nunknown = true\n",
            checksum()
        );
        assert!(ToolchainManifest::parse(&text).is_err());
    }
}
