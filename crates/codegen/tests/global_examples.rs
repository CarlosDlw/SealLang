//! Acceptance for the documented global-data examples.
//!
//! The examples are the public surface of the feature: `const` values, statics
//! with relocated pointers, and unsafe-gated mutable globals. Each one must
//! produce its documented result at `-O0` and `-O2`, and the artifact must place
//! the globals in the expected sections without constructors or thread-local
//! storage.

use std::fs;

use seal_codegen::Emit;
use seal_session::OptLevel;

mod support;

use support::{
    TempOutput, assert_no_thread_locals_or_constructors, compile, contains, is_read_only, run,
    section_of,
};

const CONST_STATIC: &str = include_str!("../../../examples/039_const_static.sl");
const GLOBAL_DATA: &str = include_str!("../../../examples/040_global_data.sl");
const GLOBAL_MUTATION: &str = include_str!("../../../examples/041_global_mutation.sl");

/// Compiles `source` at `O0` and `O2` and asserts the executable's exit code.
fn assert_result(source: &str, expected: i32, stem: &str) {
    for (opt_level, debug_info, suffix) in [
        (OptLevel::O0, true, format!("{stem}-debug")),
        (OptLevel::O2, false, format!("{stem}-release")),
    ] {
        let output = TempOutput::new(&suffix);
        compile(
            source,
            output.path(),
            Emit::Executable,
            opt_level,
            debug_info,
        )
        .unwrap_or_else(|error| panic!("{stem} failed to compile at {opt_level:?}: {error}"));
        assert_eq!(
            run(output.path()).code(),
            Some(expected),
            "{stem} produced the wrong result at {opt_level:?}"
        );
    }
}

/// Emits the assembly of `source` at `-O0`.
fn assembly(source: &str, stem: &str) -> String {
    let output = TempOutput::new(stem);
    compile(source, output.path(), Emit::Assembly, OptLevel::O0, false)
        .expect("the example emits assembly");
    fs::read_to_string(output.path()).expect("the assembly is readable")
}

#[test]
fn const_static_example_runs_at_o0_and_o2() {
    assert_result(CONST_STATIC, 42, "const-static-example");
}

#[test]
fn global_data_example_runs_at_o0_and_o2() {
    assert_result(GLOBAL_DATA, 42, "global-data-example");
}

#[test]
fn global_mutation_example_runs_at_o0_and_o2() {
    assert_result(GLOBAL_MUTATION, 42, "global-mutation-example");
}

#[test]
fn const_values_and_immutable_statics_are_read_only() {
    let assembly = assembly(CONST_STATIC, "const-static-example-sections");

    // A `const` read by value has no address in the language, so its evaluated
    // image is one private constant global.
    for symbol in [".Lseal.value.AREA", ".Lseal.value.main.TWICE_WIDTH"] {
        let section = section_of(&assembly, symbol)
            .unwrap_or_else(|| panic!("{symbol} is not in the assembly:\n{assembly}"));
        assert!(
            is_read_only(&section),
            "{symbol} landed in the writable section {section}"
        );
    }

    // A `static` keeps its own symbol and its own allocation.
    let section = section_of(&assembly, "seal.static.EDGE")
        .expect("an immutable static has a label in the assembly");
    assert!(
        is_read_only(&section),
        "an immutable static landed in {section}"
    );

    assert_no_thread_locals_or_constructors(&assembly);
}

#[test]
fn pointer_globals_land_in_read_only_relocated_data() {
    let assembly = assembly(GLOBAL_DATA, "global-data-example-sections");

    // An aggregate without pointers is pure data, so it needs no relocation.
    for symbol in ["seal.static.POINT", "seal.static.TABLE", "seal.static.UNIT"] {
        let section = section_of(&assembly, symbol)
            .unwrap_or_else(|| panic!("{symbol} is not in the assembly:\n{assembly}"));
        assert!(
            is_read_only(&section),
            "{symbol} landed in the writable section {section}"
        );
    }

    // A global that holds a pointer is read-only after relocation, which is
    // what `.data.rel.ro` provides.
    for symbol in [
        "seal.static.LABEL",
        "seal.static.POINT_REF",
        "seal.static.VIEW",
        "seal.static.SHAPE",
    ] {
        let section = section_of(&assembly, symbol)
            .unwrap_or_else(|| panic!("{symbol} is not in the assembly:\n{assembly}"));
        assert!(
            section.contains("data.rel.ro"),
            "{symbol} landed in {section}, which is not read-only-after-relocation"
        );
    }

    // The pointer fields are relocations against the string image and the
    // vtable, not copies of either.
    assert!(
        assembly.contains(".quad\tseal.str."),
        "a string field must be a relocation:\n{assembly}"
    );
    assert!(
        assembly.contains(".quad\t.Lseal.vtable."),
        "a trait-object static must point at a vtable:\n{assembly}"
    );

    assert_no_thread_locals_or_constructors(&assembly);
}

#[test]
fn mutable_globals_are_writable_and_zeroes_go_to_bss() {
    let assembly = assembly(GLOBAL_MUTATION, "global-mutation-example-sections");

    // A mutable static is real storage the program writes.
    let section = section_of(&assembly, "seal.static.TABLE")
        .expect("a non-zero mutable static has a label in the assembly");
    assert!(
        !is_read_only(&section),
        "a mutable static landed in the read-only section {section}"
    );

    // A zero-initialized mutable static is a local common symbol, which the
    // linker places in BSS without storing its zeros. That covers both a
    // file-scope static and a block-scope static.
    for symbol in ["seal.static.COUNTER", "seal.static.next_id.NEXT"] {
        assert!(
            assembly.contains(&format!(".comm\t{symbol},")),
            "{symbol} is not a zero-initialized common symbol:\n{assembly}"
        );
    }

    // An immutable static stays read-only even next to mutable globals.
    let section = section_of(&assembly, "seal.static.BASE")
        .expect("an immutable static has a label in the assembly");
    assert!(
        is_read_only(&section),
        "an immutable static landed in {section}"
    );

    assert_no_thread_locals_or_constructors(&assembly);
}

#[test]
fn objects_carry_the_same_sections_without_constructors() {
    for (source, stem, sections) in [
        (
            CONST_STATIC,
            "const-static-example-object",
            &[".rodata"][..],
        ),
        (
            GLOBAL_DATA,
            "global-data-example-object",
            &[".rodata", ".data.rel.ro"][..],
        ),
        (
            GLOBAL_MUTATION,
            "global-mutation-example-object",
            &[".rodata", ".data", ".bss"][..],
        ),
    ] {
        let output = TempOutput::new(stem);
        compile(source, output.path(), Emit::Object, OptLevel::O0, false)
            .unwrap_or_else(|error| panic!("{stem} failed to compile: {error}"));
        let object = fs::read(output.path()).expect("the object is readable");

        for section in sections {
            assert!(
                contains(&object, section.as_bytes()),
                "{stem} does not contain the section {section}"
            );
        }
        assert!(
            !contains(&object, b"llvm.global_ctors"),
            "{stem} must not emit a constructor table"
        );
        assert!(
            !contains(&object, b".tdata") && !contains(&object, b".tbss"),
            "{stem} must not emit thread-local storage"
        );
    }
}
