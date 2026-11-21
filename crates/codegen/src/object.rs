//! Linking a previously generated object file.
//!
//! The split build keeps code generation and linking as separate actions. This
//! module is the only public surface of that split: the frontend writes an
//! object, the build layer supplies the runtime archive and native libraries,
//! and the linker runs with the same argument rules a single-shot executable
//! uses.

use std::ffi::OsStr;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use crate::artifact::{self, LinkRequest};
use crate::error::CodegenError;

/// One request to link an existing object into an executable.
#[derive(Clone, Debug)]
pub struct ObjectLink<'a> {
    /// The object files produced by the code generation actions, in order.
    pub objects: &'a [PathBuf],
    /// The executable to write.
    pub output: &'a Path,
    /// Runtime archive bytes linked as the final input.
    ///
    /// `None` is a freestanding link: no runtime archive is written or passed
    /// to the linker.
    pub runtime: Option<&'a [u8]>,
    /// Libraries passed to the linker in order.
    pub libraries: &'a [String],
    /// Directories searched for native libraries.
    pub search_paths: &'a [PathBuf],
    /// Extra linker arguments, in request order.
    pub link_args: &'a [OsString],
    /// An explicit linker program; `None` uses `SEAL_LINKER`, `CC`, or `cc`.
    pub linker: Option<&'a OsStr>,
    /// Whether to produce a shared library instead of an executable.
    pub shared: bool,
    /// Whether this is a freestanding link (`-nostdlib`, `_start`, exit stub).
    pub freestanding: bool,
    /// The target triple used to compile the freestanding support stub.
    pub target: &'a str,
    /// The linker script, when the target declares one.
    pub linker_script: Option<&'a Path>,
    /// The sysroot passed to the C toolchain.
    pub sysroot: Option<&'a Path>,
    /// Timestamp exposed to the linker for reproducible outputs.
    pub source_date_epoch: u64,
    /// Whether to request deterministic linker metadata.
    pub reproducible: bool,
}

/// One request to archive existing objects and the runtime.
#[derive(Clone, Debug)]
pub struct ObjectArchive<'a> {
    /// The object files produced by the code generation actions, in order.
    pub objects: &'a [PathBuf],
    /// The static library to write.
    pub output: &'a Path,
    /// Runtime archive bytes linked into the library.
    ///
    /// `None` archives only the generated objects.
    pub runtime: Option<&'a [u8]>,
}

/// Archives `objects` and the runtime into `output` atomically.
pub fn archive_objects(request: &ObjectArchive<'_>) -> Result<(), CodegenError> {
    let objects: Vec<&Path> = request.objects.iter().map(PathBuf::as_path).collect();
    artifact::archive_files(&objects, request.runtime, request.output)
}

/// Links `objects` into `output`, replacing the destination atomically.
pub fn link_objects(request: &ObjectLink<'_>) -> Result<(), CodegenError> {
    let objects: Vec<&Path> = request.objects.iter().map(PathBuf::as_path).collect();
    let inner = LinkRequest {
        objects: &objects,
        runtime: request.runtime,
        output: request.output,
        shared: request.shared,
        source_libraries: &[],
        libraries: request.libraries,
        search_paths: request.search_paths,
        link_args: request.link_args,
        linker: request.linker,
        freestanding: request.freestanding,
        target: request.target,
        linker_script: request.linker_script,
        sysroot: request.sysroot,
        source_date_epoch: request.source_date_epoch,
        reproducible: request.reproducible,
    };
    artifact::link_object(&inner)
}
