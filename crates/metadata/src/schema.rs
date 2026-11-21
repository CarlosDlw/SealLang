//! The JSON header stored in a `.sealmono` container.

use serde::{Deserialize, Serialize};

/// The only schema accepted by this compiler build.
pub const FORMAT_VERSION: u32 = 1;

/// One code generation unit stored in the container.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MonoModule {
    /// The zero-based unit index.
    pub cgu: u32,
    /// A stable module name.
    pub name: String,
    /// The blake3 digest of the module bitcode, in lowercase hex.
    pub digest: String,
    /// The exact payload size in bytes.
    pub size: u64,
    /// Number of defined functions in this unit.
    pub functions: u64,
}

/// The profile values that affect code generation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MonoProfile {
    /// LLVM optimization level, from 0 through 3.
    pub opt_level: u8,
    /// Whether DWARF information was emitted.
    pub debug: bool,
    /// Number of code generation units.
    pub codegen_units: u32,
}

/// One source file named by the compile unit.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MonoSourceFile {
    /// The remapped path stored in debug information.
    pub path: String,
    /// The blake3 digest of the source bytes.
    pub digest: String,
}

/// A size and layout summary for one type.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MonoLayout {
    /// The readable type name.
    pub name: String,
    /// The size in bytes.
    pub size: u64,
    /// The alignment in bytes.
    pub align: u64,
}

/// Counts and a stable digest of the analyzed program.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MonoSummary {
    /// Number of lowered type definitions.
    pub types: u64,
    /// Number of monomorphized instances.
    pub instances: u64,
    /// Number of vtables.
    pub vtables: u64,
    /// Number of drop glue items.
    pub drop_glues: u64,
    /// Number of statics.
    pub statics: u64,
    /// Number of constant allocations.
    pub constants: u64,
    /// A digest over the serialized summary and layouts.
    pub digest: String,
}

/// The complete header of one `.sealmono` artifact.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MonoManifest {
    /// Schema version; must equal [`FORMAT_VERSION`].
    pub version: u32,
    /// The LLVM target triple.
    pub target: String,
    /// The profile values.
    pub profile: MonoProfile,
    /// The entry function path, when the program has one.
    pub entry: Option<String>,
    /// Source files named by debug information.
    pub source_files: Vec<MonoSourceFile>,
    /// Type layouts visible to the program.
    pub layouts: Vec<MonoLayout>,
    /// Program size counts.
    pub summary: MonoSummary,
    /// The code generation units, ordered by index.
    pub modules: Vec<MonoModule>,
    /// A digest of the header before this field, used for reproducibility checks.
    pub build_id: String,
}

impl MonoManifest {
    /// Creates an empty v1 manifest for `target` with `codegen_units` units.
    #[must_use]
    pub fn new(target: impl Into<String>, codegen_units: u32) -> Self {
        Self {
            version: FORMAT_VERSION,
            target: target.into(),
            profile: MonoProfile {
                opt_level: 0,
                debug: false,
                codegen_units,
            },
            entry: None,
            source_files: Vec::new(),
            layouts: Vec::new(),
            summary: MonoSummary::default(),
            modules: Vec::new(),
            build_id: String::new(),
        }
    }

    /// Returns the number of units the manifest promises.
    #[must_use]
    pub const fn cgu_count(&self) -> u32 {
        self.profile.codegen_units
    }
}
