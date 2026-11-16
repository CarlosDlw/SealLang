//! Options for one code generation request.

use std::path::PathBuf;

use seal_session::OptLevel;

/// The artifact a code generation request produces.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Emit {
    /// A relocatable object file.
    Object,
    /// Target assembly text.
    Assembly,
    /// A linked native executable.
    #[default]
    Executable,
}

impl Emit {
    /// Returns the flag spelling of the artifact kind.
    #[inline]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Object => "obj",
            Self::Assembly => "asm",
            Self::Executable => "exe",
        }
    }
}

/// Everything the backend needs to produce one artifact.
///
/// The output path is explicit rather than derived from the input: the driver
/// owns file naming, and code generation stays deterministic for a given
/// option set.
#[derive(Clone, Debug)]
pub struct CodegenOptions {
    /// The artifact to produce.
    pub emit: Emit,
    /// Where the artifact is written.
    pub output: PathBuf,
    /// Optimization level.
    pub opt_level: OptLevel,
    /// Emit DWARF debug information.
    pub debug_info: bool,
    /// Module name, used for the LLVM module and the debug producer.
    pub crate_name: Option<String>,
}

impl CodegenOptions {
    /// Creates options for `emit`, writing the artifact to `output`.
    pub fn new(emit: Emit, output: impl Into<PathBuf>) -> Self {
        Self {
            emit,
            output: output.into(),
            opt_level: OptLevel::O0,
            debug_info: false,
            crate_name: None,
        }
    }
}
