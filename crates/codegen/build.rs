//! Builds the native runtime archive embedded in the compiler.
//!
//! The archive is created with `cc` directly instead of `compile`: `compile`
//! would emit a `cargo:rustc-link-lib` directive and link the runtime into the
//! compiler binary. The only native artifact this project produces is the
//! archive that the backend copies next to generated executables.

use std::env;
use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=runtime/seal_rt.c");

    let manifest = PathBuf::from(
        env::var_os("CARGO_MANIFEST_DIR").expect("Cargo always sets CARGO_MANIFEST_DIR"),
    );
    let source = manifest.join("runtime/seal_rt.c");

    let mut build = cc::Build::new();
    build
        .file(&source)
        .std("c11")
        .opt_level(2)
        .pic(true)
        .warnings(true)
        .flag_if_supported("-Wall")
        .flag_if_supported("-Wextra")
        .flag_if_supported("-Wpedantic")
        .flag_if_supported("-fvisibility=hidden")
        .flag_if_supported("-fno-unwind-tables")
        .flag_if_supported("-fno-asynchronous-unwind-tables");

    let objects = build.compile_intermediates();
    let archive = build.create_archive("seal_rt", &objects);
    println!("cargo:rustc-env=SEAL_RUNTIME_ARCHIVE={}", archive.display());
}
