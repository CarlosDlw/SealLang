//! Statics and `const func` at runtime.
//!
//! A `static` is initialized before the program starts and never released, so
//! its destructor must never run. A `const func` may construct and return a
//! value whose type declares a destructor; only a runtime scope releases it.

use seal_codegen::Emit;
use seal_session::OptLevel;

mod support;

use support::{TempOutput, compile, run};

const PROGRAM: &str = "\
struct Token { tag: i32 } \
impl Drop for Token { \
    func drop(self: *mut Token) { unsafe { COUNT = COUNT + 1; } } \
} \
static VALUE: Token = Token { tag: 7 }; \
static mut COUNT: i32 = 0;";

#[test]
fn a_static_of_a_drop_type_is_initialized_without_a_destructor() {
    let source = format!(
        "{PROGRAM} \
         func main() -> i32 {{ \
            let view: *Token = &VALUE; \
            (*view).tag \
         }}"
    );
    assert_returns(&source, 7);
}

#[test]
fn reading_a_static_does_not_drop_it() {
    let source = format!(
        "{PROGRAM} \
         func main() -> i32 {{ \
            let view: *Token = &VALUE; \
            let tag = (*view).tag; \
            unsafe {{ COUNT = 1; }} \
            tag + unsafe {{ COUNT }} \
         }}"
    );
    assert_returns(&source, 8);
}

#[test]
fn a_const_function_returning_a_drop_value_is_released_by_the_runtime_scope() {
    let source = format!(
        "{PROGRAM} \
         const func make() -> Token {{ Token {{ tag: 1 }} }} \
         func release() {{ let token = make(); }} \
         func main() -> i32 {{ release(); unsafe {{ COUNT }} }}"
    );
    assert_returns(&source, 1);
}

/// Compiles `source` at both optimization levels and asserts the exit status.
fn assert_returns(source: &str, expected: i32) {
    for opt_level in [OptLevel::O0, OptLevel::O2] {
        let output = TempOutput::new("drop-static");
        compile(source, output.path(), Emit::Executable, opt_level, false)
            .unwrap_or_else(|error| panic!("compilation failed at {opt_level:?}: {error}"));
        assert_eq!(
            run(output.path()).code(),
            Some(expected),
            "unexpected result at {opt_level:?}"
        );
    }
}
