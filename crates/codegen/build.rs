//! Builds the native runtime archive embedded in the compiler.
//!
//! The archive is created with `cc` directly instead of `compile`: `compile`
//! would emit a `cargo:rustc-link-lib` directive and link the runtime into the
//! compiler binary. The only native artifact this project produces is the
//! archive that the backend copies next to generated executables.

use std::env;
use std::path::PathBuf;

/// Native runtime translation units, in archive order.
const SOURCES: &[&str] = &[
    "runtime/core/exit.c",
    "runtime/core/io.c",
    "runtime/core/panic.c",
    "runtime/heap/alloc.c",
    "runtime/heap/system.c",
];

/// Shared ABI and backend headers included by the runtime translation units.
const HEADERS: &[&str] = &["runtime/include/seal_rt.h", "runtime/heap/system.h"];

fn main() {
    for source in SOURCES.iter().chain(HEADERS.iter()) {
        println!("cargo:rerun-if-changed={source}");
    }

    let manifest = PathBuf::from(
        env::var_os("CARGO_MANIFEST_DIR").expect("Cargo always sets CARGO_MANIFEST_DIR"),
    );
    let include = manifest.join("runtime/include");

    let mut build = cc::Build::new();
    build
        .include(&include)
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

    for source in SOURCES {
        build.file(manifest.join(source));
    }

    let objects = build.compile_intermediates();
    let archive = build.create_archive("seal_rt", &objects);
    println!("cargo:rustc-env=SEAL_RUNTIME_ARCHIVE={}", archive.display());
}
