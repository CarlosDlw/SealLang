//! Options for one code generation request.

use std::ffi::OsString;
use std::path::PathBuf;

use seal_def::DefId;
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
    /// A `.sealmono` archive of per-CGU LLVM bitcode.
    Mono,
    /// A static C library containing the object and runtime.
    StaticLib,
    /// A shared C library containing the object and runtime.
    Cdylib,
}

impl Emit {
    /// Returns the flag spelling of the artifact kind.
    #[inline]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Object => "obj",
            Self::Assembly => "asm",
            Self::Executable => "exe",
            Self::Mono => "mono",
            Self::StaticLib => "staticlib",
            Self::Cdylib => "cdylib",
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
    /// Optional runtime archive used instead of the embedded host archive.
    ///
    /// This is required to link an executable for a target whose architecture
    /// or operating system differs from the compiler host. The caller is
    /// responsible for supplying an archive built for that target.
    pub runtime_archive: Option<PathBuf>,
    /// Native libraries named on the command line, in request order.
    pub libraries: Vec<String>,
    /// Directories searched for native libraries.
    pub search_paths: Vec<PathBuf>,
    /// Arguments passed to the linker unchanged.
    pub link_args: Vec<OsString>,
    /// An explicitly selected entry function.
    ///
    /// `None` keeps the single-file convention of looking for `main`.
    pub entry: Option<DefId>,
    /// Number of code generation units to produce for [`Emit::Mono`].
    pub cgu_count: u32,
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
            runtime_archive: None,
            libraries: Vec::new(),
            search_paths: Vec::new(),
            link_args: Vec::new(),
            entry: None,
            cgu_count: 1,
        }
    }
}
