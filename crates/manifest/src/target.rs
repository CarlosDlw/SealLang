//! Declared build targets.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::cfg::CfgSpec;

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
    /// A typed condition that activates the target.
    #[serde(default)]
    pub when: Option<CfgSpec>,
    /// Declared environment values for this target.
    #[serde(default)]
    pub env: std::collections::BTreeMap<String, String>,
    /// The profile this target uses by default.
    #[serde(default)]
    pub profile: Option<String>,
    /// The runtime policy (`default` or `none`).
    #[serde(default)]
    pub runtime: Option<String>,
    /// The panic policy (`abort` or `trap`).
    #[serde(default)]
    pub panic: Option<String>,
    /// The linker program.
    #[serde(default)]
    pub linker: Option<String>,
    /// The sysroot passed to the C toolchain.
    #[serde(default)]
    pub sysroot: Option<PathBuf>,
    /// The linker script.
    #[serde(default)]
    pub linker_script: Option<PathBuf>,
    /// Extra linker arguments declared for this target.
    #[serde(default)]
    pub link_args: Vec<String>,
    /// The artifact path relative to the target directory.
    #[serde(default)]
    pub output: Option<PathBuf>,
    /// Generator input globs; they are runtime data, not compiled sources.
    #[serde(default)]
    pub inputs: Vec<String>,
    /// Generator output globs, relative to the package root.
    #[serde(default)]
    pub outputs: Vec<String>,
    /// Generator targets whose outputs this target compiles.
    #[serde(default)]
    pub generated: Vec<String>,
    /// Native aliases linked into this target.
    #[serde(default)]
    pub native: Vec<String>,
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
            when: None,
            env: std::collections::BTreeMap::new(),
            profile: None,
            runtime: None,
            panic: None,
            linker: None,
            sysroot: None,
            linker_script: None,
            link_args: Vec::new(),
            output: None,
            inputs: Vec::new(),
            outputs: Vec::new(),
            generated: Vec::new(),
            native: Vec::new(),
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
            when: None,
            env: std::collections::BTreeMap::new(),
            profile: None,
            runtime: None,
            panic: None,
            linker: None,
            sysroot: None,
            linker_script: None,
            link_args: Vec::new(),
            output: None,
            inputs: Vec::new(),
            outputs: Vec::new(),
            generated: Vec::new(),
            native: Vec::new(),
        }
    }
}
