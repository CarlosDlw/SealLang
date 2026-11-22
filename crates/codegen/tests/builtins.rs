//! Runtime behavior of the control-flow, equality, and introspection built-ins.
//!
//! Every successful scenario is compiled and run at `-O0` and `-O2`. The panic
//! and exit scenarios also inspect the process status and captured diagnostics.

use seal_codegen::Emit;
use seal_session::OptLevel;

mod support;

use support::{TempOutput, compile, run_captured};

/// Compiles `source` at both optimization levels and asserts the exit status.
fn assert_returns(source: &str, expected: i32) {
    for opt_level in [OptLevel::O0, OptLevel::O2] {
        let output = TempOutput::new("builtins");
        compile(source, output.path(), Emit::Executable, opt_level, false)
            .unwrap_or_else(|error| panic!("compilation failed at {opt_level:?}: {error}"));
        let result = run_captured(output.path());
        assert_eq!(
            result.status.code(),
            Some(expected),
            "unexpected result at {opt_level:?}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
}

#[test]
fn introspection_reports_concrete_generic_types_and_layouts() {
    let source = "\
struct Mixed { byte: u8, word: u64 } \
func label<T>() -> *str { type_name::<T>() } \
func bytes<T>() -> usize { size_of::<T>() } \
func alignment<T>() -> usize { align_of::<T>() } \
func identity<T>(value: T) -> *str { type_of(value) } \
func main() -> i32 { \
  let mut score = 0; \
  if type_name::<i32>() == \"i32\" { score += 1; } \
  if type_name::<[u8; 4]>() == \"[u8; 4]\" { score += 2; } \
  if label::<u8>() == \"u8\" { score += 4; } \
  if type_of(score) == \"i32\" { score += 8; } \
  if identity::<bool>(true) == \"bool\" { score += 16; } \
  if bytes::<Mixed>() == 16 { score += 32; } \
  if alignment::<Mixed>() == 8 { score += 64; } \
  if bytes::<u64>() == 8 && alignment::<u64>() == 8 { score += 128; } \
  score \
}";
    assert_returns(source, 255);
}

#[test]
fn type_of_reads_without_moving_or_dropping_its_operand() {
    let source = "\
struct Token { tag: i32 } \
impl Drop for Token { \
  func drop(self: *mut Token) { unsafe { COUNT = COUNT * 10 + (*self).tag; } } \
} \
static mut COUNT: i32 = 0; \
func main() -> i32 { \
  let token = Token { tag: 7 }; \
  if type_of(token) == \"Token\" { drop(token); } \
  unsafe { COUNT } \
}";
    assert_returns(source, 7);
}

#[test]
fn type_of_evaluates_its_operand_for_effects() {
    let source = "\
static mut COUNT: i32 = 0; \
func bump() -> i32 { unsafe { COUNT += 1; } 1 } \
func main() -> i32 { \
  let name = type_of(bump()); \
  if name == \"i32\" { unsafe { COUNT } } else { 0 } \
}";
    assert_returns(source, 1);
}

#[test]
fn type_queries_are_available_in_constant_initializers() {
    let source = "\
const NAME: *str = type_name::<i32>(); \
const SIZE: usize = size_of::<u64>(); \
const ALIGN: usize = align_of::<u64>(); \
func main() -> i32 { \
  if NAME == \"i32\" && SIZE == 8 && ALIGN == 8 { 0 } else { 1 } \
}";
    assert_returns(source, 0);
}

#[test]
fn panic_reports_a_computed_message_and_source_location() {
    let source = "func main() -> i32 {\n  let reason: *str = \"boom\";\n  panic(reason)\n}";
    for opt_level in [OptLevel::O0, OptLevel::O2] {
        let output = TempOutput::new("panic-builtin");
        compile(source, output.path(), Emit::Executable, opt_level, false)
            .unwrap_or_else(|error| panic!("compilation failed at {opt_level:?}: {error}"));
        let result = run_captured(output.path());
        assert_eq!(result.status.code(), Some(101));
        let stderr = String::from_utf8_lossy(&result.stderr);
        assert!(
            stderr.contains("panic: boom at <test.sl>:3:3"),
            "unexpected stderr at {opt_level:?}: {stderr}"
        );
    }
}

#[test]
fn panic_accepts_a_literal_message() {
    let source = "func main() -> i32 { panic(\"literal\") }";
    let output = TempOutput::new("panic-literal");
    compile(source, output.path(), Emit::Executable, OptLevel::O0, false)
        .expect("compilation succeeds");
    let result = run_captured(output.path());
    assert_eq!(result.status.code(), Some(101));
    assert!(String::from_utf8_lossy(&result.stderr).contains("panic: literal at <test.sl>:1:22"));
}

#[test]
fn exit_uses_the_supplied_process_status() {
    let source = "func main() -> i32 { let code = 7; exit(code) }";
    assert_returns(source, 7);
}

#[test]
fn exit_status_is_not_truncated_by_the_language() {
    let source = "func main() -> i32 { let code: i32 = 200; exit(code) }";
    assert_returns(source, 200);
}
