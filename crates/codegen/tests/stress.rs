//! Monomorphization stress and determinism tests.
//!
//! A program with many distinct instances must compile once per instance, stay
//! bounded in size, and produce byte-identical output for identical input.

mod support;

use seal_codegen::Emit;
use seal_session::OptLevel;

use support::{TempOutput, compile, run};

/// Builds a program with one `mark<[i32; N]>` instance per `length`.
fn many_instances(lengths: std::ops::RangeInclusive<usize>) -> String {
    let mut source =
        String::from("func mark<T>() -> i32 { 1 }\nfunc main() -> i32 {\n  let mut total = 0;\n");
    for length in lengths {
        source.push_str(&format!("  total = total + mark::<[i32; {length}]>();\n"));
    }
    source.push_str("  total\n}\n");
    source
}

#[test]
fn one_hundred_instances_compile_and_run_at_o0_and_o2() {
    let source = many_instances(1..=100);
    for opt_level in [OptLevel::O0, OptLevel::O2] {
        let output = TempOutput::new("stress-100");
        compile(&source, output.path(), Emit::Executable, opt_level, false)
            .expect("one hundred instances compile");
        assert_eq!(run(output.path()).code(), Some(100));
        let size = std::fs::metadata(output.path())
            .expect("artifact exists")
            .len();
        assert!(
            size < 1024 * 1024,
            "one hundred instances produced {size} bytes; expected a bounded executable"
        );
    }
}

#[test]
fn one_hundred_instances_get_distinct_symbols() {
    let source = many_instances(1..=100);
    let output = TempOutput::new("stress-symbols");
    compile(&source, output.path(), Emit::Assembly, OptLevel::O0, false)
        .expect("assembly is emitted");
    let text = std::fs::read_to_string(output.path()).expect("assembly is readable");
    let mut symbols = std::collections::BTreeSet::new();
    let prefix = "seal.mark.";
    let mut rest = text.as_str();
    while let Some(index) = rest.find(prefix) {
        let tail = &rest[index + prefix.len()..];
        let end = tail
            .find(|character: char| !character.is_ascii_hexdigit())
            .unwrap_or(tail.len());
        symbols.insert(format!("{prefix}{}", &tail[..end]));
        rest = &tail[end..];
    }
    assert_eq!(
        symbols.len(),
        100,
        "one symbol per distinct array length, got {symbols:?}"
    );
}

#[test]
fn identical_sources_produce_byte_identical_objects() {
    let source = many_instances(1..=5);
    let first = TempOutput::new("determinism-first");
    let second = TempOutput::new("determinism-second");
    compile(&source, first.path(), Emit::Object, OptLevel::O0, false)
        .expect("the first object is emitted");
    compile(&source, second.path(), Emit::Object, OptLevel::O0, false)
        .expect("the second object is emitted");
    let first_bytes = std::fs::read(first.path()).expect("the first object is readable");
    let second_bytes = std::fs::read(second.path()).expect("the second object is readable");
    assert_eq!(
        first_bytes, second_bytes,
        "the same source must produce the same object"
    );
}
