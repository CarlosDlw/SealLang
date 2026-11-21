//! Cross-field validation for a `.sealmono` artifact.

use std::collections::BTreeSet;

use crate::error::MetadataError;
use crate::schema::{FORMAT_VERSION, MonoManifest};

/// Validates a manifest against its payloads.
pub(crate) fn validate(manifest: &MonoManifest, modules: &[Vec<u8>]) -> Result<(), MetadataError> {
    if manifest.version != FORMAT_VERSION {
        return Err(MetadataError::new(format!(
            "unsupported header version {}; this compiler writes version {FORMAT_VERSION}",
            manifest.version
        )));
    }
    if manifest.target.trim().is_empty() {
        return Err(MetadataError::new("the target triple is empty"));
    }
    if manifest.cgu_count() == 0 {
        return Err(MetadataError::new(
            "the artifact requests zero code generation units",
        ));
    }
    if manifest.profile.opt_level > 3 {
        return Err(MetadataError::new(format!(
            "the optimization level {} is outside 0..=3",
            manifest.profile.opt_level
        )));
    }
    if manifest.modules.len() != manifest.cgu_count() as usize {
        return Err(MetadataError::new(format!(
            "the header promises {} units but lists {}",
            manifest.cgu_count(),
            manifest.modules.len()
        )));
    }
    if manifest.modules.len() != modules.len() {
        return Err(MetadataError::new(format!(
            "the header lists {} units but the container has {}",
            manifest.modules.len(),
            modules.len()
        )));
    }
    let mut seen = BTreeSet::new();
    for (position, module) in manifest.modules.iter().enumerate() {
        if module.cgu as usize != position {
            return Err(MetadataError::new(format!(
                "module {position} names CGU {}",
                module.cgu
            )));
        }
        if !seen.insert(module.cgu) {
            return Err(MetadataError::new(format!(
                "CGU {} appears more than once",
                module.cgu
            )));
        }
        if module.name.trim().is_empty() {
            return Err(MetadataError::new(format!(
                "CGU {} has an empty name",
                module.cgu
            )));
        }
        let bytes = &modules[position];
        if bytes.is_empty() {
            return Err(MetadataError::new(format!(
                "CGU {} has an empty payload",
                module.cgu
            )));
        }
        if module.size != bytes.len() as u64 {
            return Err(MetadataError::new(format!(
                "CGU {} declares {} bytes but has {}",
                module.cgu,
                module.size,
                bytes.len()
            )));
        }
        let digest = super::format::digest(bytes);
        if module.digest != digest {
            return Err(MetadataError::new(format!(
                "CGU {} has digest {} but its payload hashes to {digest}",
                module.cgu, module.digest
            )));
        }
    }
    Ok(())
}
