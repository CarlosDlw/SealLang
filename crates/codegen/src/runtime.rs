//! The native runtime archive embedded in the compiler.
//!
//! The archive is built from the runtime's C11 translation units under
//! `runtime/core/` and `runtime/heap/` for the architecture and operating
//! system of the compiler itself. Objects and assembly for another target do
//! not need it, but linking an executable does, so cross-target executables are
//! rejected with a clear artifact error instead of failing later in the
//! linker. The heap objects are pulled in only when an allocation symbol is
//! referenced.

use seal_target::{TargetSpec, host_triple};

/// Runtime archive bytes built by `build.rs`.
pub(crate) const ARCHIVE: &[u8] = include_bytes!(env!("SEAL_RUNTIME_ARCHIVE"));

/// Returns whether the embedded runtime can be linked for `target`.
///
/// The comparison deliberately ignores the vendor and environment components:
/// `x86_64-unknown-linux-gnu` and `x86_64-unknown-linux-musl` share the runtime
/// ABI and the small set of libc symbols the runtime uses. Architecture and OS
/// determine calling convention, pointer width, and the available process API.
pub(crate) fn supports(target: &TargetSpec) -> bool {
    let host = host_triple();
    target.triple().arch() == host.arch() && target.triple().os() == host.os()
}

#[cfg(test)]
mod tests {
    use super::{ARCHIVE, supports};
    use seal_target::host_triple;

    #[test]
    fn the_embedded_archive_is_present_and_matches_the_host() {
        assert!(!ARCHIVE.is_empty(), "the runtime archive is empty");
        assert!(
            supports(&host_triple().spec()),
            "the host runtime is not usable for the host target"
        );
    }
}
