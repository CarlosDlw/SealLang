//! Runs the heap ABI contract tests against the embedded runtime archive.
//!
//! The harness is C because the ABI is a C boundary. It is compiled with the
//! host C toolchain, linked against the exact archive bytes embedded in the
//! compiler, and executed as a real process.

use std::env;
use std::fs;
use std::path::Path;
use std::process::Command;

mod support;

use support::cc::{Compiler, TempDirectory, run};

#[test]
fn the_heap_abi_satisfies_its_contract() {
    let manifest = env!("CARGO_MANIFEST_DIR");
    let temp = TempDirectory::new("heap");
    let compiler = Compiler::discover();
    let msvc = compiler.is_msvc();
    let archive = temp
        .path()
        .join(if msvc { "seal_rt.lib" } else { "seal_rt.a" });
    fs::write(&archive, seal_codegen::runtime_archive()).expect("the runtime archive is written");

    let source = Path::new(manifest).join("tests/runtime/heap.c");
    let include = Path::new(manifest).join("runtime/include");
    let object = temp.path().join(if msvc { "heap.obj" } else { "heap.o" });
    compiler.compile_object(&source, Some(&include), &object);

    let executable = temp.path().join(if cfg!(windows) {
        "heap_test.exe"
    } else {
        "heap_test"
    });
    compiler.link(&[&object], &[&archive], &executable);

    run(&mut Command::new(&executable));
}
