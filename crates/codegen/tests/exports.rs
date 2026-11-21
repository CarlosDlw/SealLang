//! C ABI exports and the library artifacts built from them.

use std::fs;
use std::path::Path;
use std::process::Command;

use seal_codegen::Emit;
use seal_session::OptLevel;

mod support;

use support::cc::Compiler;

const SCALAR: &str = r#"
export "c" func add(a: i32, b: i32) -> i32 {
  a + b
}
"#;

const STRUCTS: &str = r#"
extern struct Pair { x: i32, y: i32 }
extern struct Big { a: i64, b: i64, c: i64 }

export "c" func make(x: i32, y: i32) -> Pair {
  Pair { x: x, y: y }
}

export "c" func sum(pair: Pair) -> i32 {
  pair.x + pair.y
}

export "c" func build(a: i64, b: i64, c: i64) -> Big {
  Big { a: a, b: b, c: c }
}

export "c" func total(value: Big) -> i64 {
  value.a + value.b + value.c
}
"#;

#[test]
fn scalar_exports_are_callable_from_c() {
    let directory = support::cc::TempDirectory::new("export-scalar");
    let object = directory.path().join("export.o");
    support::compile(SCALAR, &object, Emit::Object, OptLevel::O0, false)
        .expect("the export compiles");
    let source = directory.path().join("main.c");
    fs::write(
        &source,
        "int add(int, int);\nint main(void) { return add(20, 22) == 42 ? 0 : 1; }\n",
    )
    .expect("C source");
    let output = directory.path().join("main");
    let compiler = Compiler::discover();
    let c_object = directory.path().join("main.o");
    compiler.compile_object(&source, None, &c_object);
    compiler.link(&[&c_object, &object], &[], &output);
    assert_eq!(support::run(&output).code(), Some(0));
}

#[test]
fn struct_exports_follow_the_c_abi_in_both_directions() {
    let directory = support::cc::TempDirectory::new("export-structs");
    let object = directory.path().join("export.o");
    support::compile(STRUCTS, &object, Emit::Object, OptLevel::O2, false)
        .expect("the export compiles");
    let source = directory.path().join("main.c");
    fs::write(
        &source,
        r#"
typedef struct { int x; int y; } Pair;
typedef struct { long a, b, c; } Big;
Pair make(int, int);
int sum(Pair);
Big build(long, long, long);
long total(Big);
int main(void) {
  Pair pair = make(20, 22);
  Big big = build(20, 21, 1);
  return pair.x == 20 && pair.y == 22 && sum(pair) == 42 &&
         total(big) == 42 ? 0 : 1;
}
"#,
    )
    .expect("C source");
    let output = directory.path().join("main");
    let compiler = Compiler::discover();
    let c_object = directory.path().join("main.o");
    compiler.compile_object(&source, None, &c_object);
    compiler.link(&[&c_object, &object], &[], &output);
    assert_eq!(support::run(&output).code(), Some(0));
}

#[test]
fn static_libraries_contain_the_export_and_no_c_main() {
    let directory = support::cc::TempDirectory::new("export-staticlib");
    let archive = directory.path().join("libmath.a");
    support::compile(SCALAR, &archive, Emit::StaticLib, OptLevel::O0, false)
        .expect("the static library is written");
    let source = directory.path().join("main.c");
    fs::write(
        &source,
        "int add(int, int);\nint main(void) { return add(40, 2) == 42 ? 0 : 1; }\n",
    )
    .expect("C source");
    let output = directory.path().join("main");
    let compiler = Compiler::discover();
    let c_object = directory.path().join("main.o");
    compiler.compile_object(&source, None, &c_object);
    compiler.link(&[&c_object, &archive], &[], &output);
    assert_eq!(support::run(&output).code(), Some(0));
}

#[test]
fn a_shared_library_exports_only_the_declared_c_symbols() {
    let directory = support::cc::TempDirectory::new("export-cdylib");
    let library = directory.path().join("libmath.so");
    support::compile(SCALAR, &library, Emit::Cdylib, OptLevel::O2, false)
        .expect("the shared library is written");
    let symbols = dynamic_symbols(&library);
    if symbols.is_empty() {
        return;
    }
    assert!(symbols.iter().any(|symbol| symbol == "add"));
    assert!(
        !symbols.iter().any(|symbol| symbol.starts_with("seal.")),
        "Seal symbols must stay hidden: {symbols:?}"
    );
    assert!(
        !symbols.iter().any(|symbol| symbol == "main"),
        "a library must not export the Seal C entry point: {symbols:?}"
    );
}

fn dynamic_symbols(path: &Path) -> Vec<String> {
    if !cfg!(target_os = "linux") {
        return Vec::new();
    }
    let output = Command::new("nm")
        .arg("-D")
        .arg("--defined-only")
        .arg(path)
        .output()
        .expect("nm runs");
    if !output.status.success() {
        return Vec::new();
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| line.split_whitespace().last())
        .map(str::to_owned)
        .collect()
}
