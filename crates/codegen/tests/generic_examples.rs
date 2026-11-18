//! End-to-end tests for the generic language examples.
//!
//! Each example documents its result in a comment and must produce it at every
//! optimization level, and must emit an object and assembly without an entry
//! point dependency. The source is included at compile time, so a broken example
//! fails the build instead of the test run.

mod support;

use seal_codegen::Emit;
use seal_session::OptLevel;

use support::{TempOutput, compile, run};

struct Example {
    name: &'static str,
    source: &'static str,
    result: i32,
}

const EXAMPLES: [Example; 5] = [
    Example {
        name: "027_generic_functions.sl",
        source: include_str!("../../../examples/027_generic_functions.sl"),
        result: 42,
    },
    Example {
        name: "028_generic_structs.sl",
        source: include_str!("../../../examples/028_generic_structs.sl"),
        result: 45,
    },
    Example {
        name: "029_generic_enums.sl",
        source: include_str!("../../../examples/029_generic_enums.sl"),
        result: 42,
    },
    Example {
        name: "030_generic_traits.sl",
        source: include_str!("../../../examples/030_generic_traits.sl"),
        result: 42,
    },
    Example {
        name: "031_generic_recursion.sl",
        source: include_str!("../../../examples/031_generic_recursion.sl"),
        result: 42,
    },
];

#[test]
fn generic_examples_produce_their_documented_result_at_o0_and_o2() {
    for example in &EXAMPLES {
        for opt_level in [OptLevel::O0, OptLevel::O2] {
            let output = TempOutput::new(example.name);
            compile(
                example.source,
                output.path(),
                Emit::Executable,
                opt_level,
                false,
            )
            .unwrap_or_else(|error| panic!("{}: {error}", example.name));
            assert_eq!(
                run(output.path()).code(),
                Some(example.result),
                "{} returned the wrong value",
                example.name
            );
        }
    }
}

#[test]
fn an_inline_generic_default_runs_at_o0_and_o2() {
    let source = "\
        struct Wrapper<T> { value: T }\n\
        trait Convert<T> {\n\
            func value(self: *Self) -> T;\n\
            func doubled(self: *Self) -> T { self.value() }\n\
        }\n\
        impl Convert<i32> for Wrapper<i32> {\n\
            func value(self: *Wrapper<i32>) -> i32 { (*self).value * 2 }\n\
        }\n\
        func main() -> i32 {\n\
            let wrapper = Wrapper { value: 21 };\n\
            wrapper.doubled()\n\
        }\n";
    for opt_level in [OptLevel::O0, OptLevel::O2] {
        let output = TempOutput::new("inline-generic-default");
        compile(source, output.path(), Emit::Executable, opt_level, false)
            .expect("the inline generic default compiles");
        assert_eq!(run(output.path()).code(), Some(42));
    }
}

#[test]
fn generic_examples_emit_objects_and_assembly() {
    for example in &EXAMPLES {
        for emit in [Emit::Object, Emit::Assembly] {
            let output = TempOutput::new(example.name);
            compile(example.source, output.path(), emit, OptLevel::O2, false)
                .unwrap_or_else(|error| panic!("{}: {error}", example.name));
            let bytes = std::fs::read(output.path())
                .unwrap_or_else(|error| panic!("{} is readable: {error}", example.name));
            assert!(
                !bytes.is_empty(),
                "{} emitted an empty artifact",
                example.name
            );
            if emit == Emit::Assembly {
                let text = String::from_utf8_lossy(&bytes);
                assert!(
                    text.contains("seal.main"),
                    "{} is missing its entry point symbol",
                    example.name
                );
            }
        }
    }
}
