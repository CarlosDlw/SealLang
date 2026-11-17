//! End-to-end tests for monomorphized code generation.
//!
//! Every test compiles the same generic fixture through the full pipeline and
//! asserts the language-level result, the produced symbols, or the debug
//! metadata. The fixture declares one type parameter per item kind and uses
//! every instance at least once.

mod support;

use seal_codegen::Emit;
use seal_session::OptLevel;

use support::{TempOutput, compile, contains, run};

const PROGRAM: &str = include_str!("fixtures/generic_program.sl");

fn symbols(text: &str) -> Vec<String> {
    let bytes = text.as_bytes();
    let mut symbols = Vec::new();
    let mut index = 0;
    while index + 4 < bytes.len() {
        if &bytes[index..index + 4] == b"seal" && bytes[index + 4] == b'.' {
            let start = index;
            let mut end = index + 5;
            while end < bytes.len()
                && (bytes[end].is_ascii_alphanumeric() || matches!(bytes[end], b'.' | b'_'))
            {
                end += 1;
            }
            symbols.push(text[start..end].to_owned());
            index = end;
        } else {
            index += 1;
        }
    }
    symbols.sort();
    symbols.dedup();
    symbols
}

#[test]
fn generic_program_produces_the_language_result_at_o0_and_o2() {
    for (opt_level, debug_info, suffix) in [
        (OptLevel::O0, true, "generic-debug"),
        (OptLevel::O2, false, "generic-release"),
        (OptLevel::O2, true, "generic-release-debug"),
    ] {
        let output = TempOutput::new(suffix);
        compile(
            PROGRAM,
            output.path(),
            Emit::Executable,
            opt_level,
            debug_info,
        )
        .expect("code generation succeeds");
        assert_eq!(
            run(output.path()).code(),
            Some(39),
            "wrong result for {suffix}"
        );
    }
}

#[test]
fn generic_instances_get_distinct_stable_symbols() {
    let first = TempOutput::new("symbols-first");
    let second = TempOutput::new("symbols-second");
    compile(PROGRAM, first.path(), Emit::Assembly, OptLevel::O0, false)
        .expect("assembly is emitted");
    compile(PROGRAM, second.path(), Emit::Assembly, OptLevel::O0, false)
        .expect("assembly is emitted");
    let first_text = std::fs::read_to_string(first.path()).expect("assembly is readable");
    let second_text = std::fs::read_to_string(second.path()).expect("assembly is readable");

    let first_symbols = symbols(&first_text);
    let second_symbols = symbols(&second_text);
    assert_eq!(
        first_symbols, second_symbols,
        "symbols must be stable across builds"
    );

    let identities: Vec<&String> = first_symbols
        .iter()
        .filter(|symbol| symbol.starts_with("seal.identity."))
        .collect();
    assert_eq!(
        identities.len(),
        2,
        "expected `identity<i32>` and `identity<bool>`, found {identities:?}"
    );
    assert_ne!(identities[0], identities[1], "instances must not collide");

    for expected in [
        "seal.main",
        "seal.describe.",
        "seal.Pair.show",
        "seal.Box.new.",
        "seal.Box.get.",
    ] {
        assert!(
            first_symbols.iter().any(|symbol| symbol.contains(expected)),
            "missing symbol `{expected}` in {first_symbols:?}"
        );
    }
}

#[test]
fn generic_trait_calls_are_direct_calls_without_a_vtable() {
    let output = TempOutput::new("generic-direct-calls");
    compile(PROGRAM, output.path(), Emit::Assembly, OptLevel::O0, false)
        .expect("assembly is emitted");
    let text = std::fs::read_to_string(output.path()).expect("assembly is readable");
    assert!(
        text.contains("seal.Pair.show"),
        "the bound call must target the impl directly"
    );
    for forbidden in ["vtable", "dictionary", "witness"] {
        assert!(
            !text.contains(forbidden),
            "static dispatch must not emit `{forbidden}`"
        );
    }
}

#[test]
fn generic_debug_information_names_each_instance() {
    let output = TempOutput::new("generic-dwarf");
    compile(PROGRAM, output.path(), Emit::Executable, OptLevel::O0, true)
        .expect("code generation succeeds");
    let bytes = std::fs::read(output.path()).expect("artifact is readable");

    for expected in [
        "identity<i32>",
        "identity<bool>",
        "describe<Pair<i32, bool>>",
        "Pair<Pair<i32, bool>, i32>",
        "Option<i32>",
        "Matrix<i32>",
        "Node<i32>",
    ] {
        assert!(
            contains(&bytes, expected.as_bytes()),
            "debug information is missing `{expected}`"
        );
    }
}

#[test]
fn generic_copy_bound_lets_a_value_be_used_after_a_call() {
    let source = "func keep<T: Copy>(value: T) -> T {\n  value\n}\n\n\
                  func main() -> i32 {\n  let base = keep(4);\n  \
                  let copy = keep::<i32>(base);\n  base + copy\n}\n";
    for opt_level in [OptLevel::O0, OptLevel::O2] {
        let output = TempOutput::new("generic-copy");
        compile(source, output.path(), Emit::Executable, opt_level, false)
            .expect("code generation succeeds");
        assert_eq!(run(output.path()).code(), Some(8));
    }
}

#[test]
fn generic_object_output_contains_instance_symbols() {
    let output = TempOutput::new("generic-object");
    compile(PROGRAM, output.path(), Emit::Object, OptLevel::O2, false)
        .expect("an object is emitted");
    let bytes = std::fs::read(output.path()).expect("artifact is readable");
    for expected in ["seal.identity.", "seal.describe.", "seal.main"] {
        assert!(
            contains(&bytes, expected.as_bytes()),
            "object is missing `{expected}`"
        );
    }
}
