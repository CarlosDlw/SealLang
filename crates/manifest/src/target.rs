//! Declared build targets.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// The kind of artifact a target produces.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TargetKind {
    /// A native executable.
    Bin,
    /// The package library; it exposes the public API.
    Lib,
    /// A test executable.
    Test,
    /// A benchmark executable.
    Bench,
    /// An example executable.
    Example,
    /// A host executable that generates sources.
    Generator,
    /// A static C ABI library.
    Staticlib,
    /// A dynamic C ABI library.
    Cdylib,
}

impl TargetKind {
    /// Returns the stable spelling used in plans and metadata.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Bin => "bin",
            Self::Lib => "lib",
            Self::Test => "test",
            Self::Bench => "bench",
            Self::Example => "example",
            Self::Generator => "generator",
            Self::Staticlib => "staticlib",
            Self::Cdylib => "cdylib",
        }
    }

    /// Returns whether the target needs a function entry point.
    pub const fn needs_entry(self) -> bool {
        matches!(self, Self::Bin | Self::Example | Self::Generator)
    }
}

/// One `[targets.<name>]` table.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct TargetManifest {
    /// The artifact kind.
    pub kind: TargetKind,
    /// The entry function path, such as `root::main`.
    #[serde(default)]
    pub entry: Option<String>,
    /// Extra source globs for this target.
    #[serde(default)]
    pub sources: Vec<String>,
    /// Source globs included in this target only.
    #[serde(default)]
    pub include: Vec<String>,
    /// The library target whose sources this target reuses.
    #[serde(default)]
    pub reuse: Option<String>,
    /// Features required before this target is built.
    #[serde(default)]
    pub required_features: Vec<String>,
    /// The profile this target uses by default.
    #[serde(default)]
    pub profile: Option<String>,
    /// The runtime policy (`default` or `none`).
    #[serde(default)]
    pub runtime: Option<String>,
    /// The linker program.
    #[serde(default)]
    pub linker: Option<String>,
    /// The linker script.
    #[serde(default)]
    pub linker_script: Option<PathBuf>,
    /// The artifact path relative to the target directory.
    #[serde(default)]
    pub output: Option<PathBuf>,
}

impl TargetManifest {
    /// Creates a default binary target.
    pub fn bin() -> Self {
        Self {
            kind: TargetKind::Bin,
            entry: Some("root::main".to_owned()),
            sources: Vec::new(),
            include: Vec::new(),
            reuse: None,
            required_features: Vec::new(),
            profile: None,
            runtime: None,
            linker: None,
            linker_script: None,
            output: None,
        }
    }

    /// Creates a default library target.
    pub fn lib() -> Self {
        Self {
            kind: TargetKind::Lib,
            entry: None,
            sources: Vec::new(),
            include: Vec::new(),
            reuse: None,
            required_features: Vec::new(),
            profile: None,
            runtime: None,
            linker: None,
            linker_script: None,
            output: None,
        }
    }
}
