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
    /// The object file produced by the frontend action.
    pub object: &'a Path,
    /// The executable to write.
    pub output: &'a Path,
    /// Runtime archive bytes linked as the final input.
    pub runtime: &'a [u8],
    /// Libraries passed to the linker in order.
    pub libraries: &'a [String],
    /// Directories searched for native libraries.
    pub search_paths: &'a [PathBuf],
    /// Extra linker arguments, in request order.
    pub link_args: &'a [OsString],
    /// An explicit linker program; `None` uses `SEAL_LINKER`, `CC`, or `cc`.
    pub linker: Option<&'a OsStr>,
}

/// Links `object` into `output`, replacing the destination atomically.
pub fn link_object(request: &ObjectLink<'_>) -> Result<(), CodegenError> {
    let inner = LinkRequest {
        object: request.object,
        runtime: Some(request.runtime),
        output: request.output,
        source_libraries: &[],
        libraries: request.libraries,
        search_paths: request.search_paths,
        link_args: request.link_args,
        linker: request.linker,
    };
    artifact::link_object(&inner)
}
