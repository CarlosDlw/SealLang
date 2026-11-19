//! Runtime behavior of the built-in `drop(value)`.
//!
//! Every scenario compiles and runs a real program at `-O0` and `-O2`; the
//! expected value stays below 256 because a process exit status is one byte.

use seal_codegen::Emit;
use seal_session::OptLevel;

mod support;

use support::{TempOutput, compile, run};

/// A value whose tag is folded into a global counter when it is dropped.
const TOKEN: &str = "\
struct Token { tag: i32 } \
impl Drop for Token { \
    func drop(self: *mut Token) { \
        unsafe { COUNT = COUNT * 10 + (*self).tag; } \
    } \
} \
static mut COUNT: i32 = 0;";

#[test]
fn drop_runs_the_destructor_before_the_scope_ends() {
    let source = format!(
        "{TOKEN} \
         func release() {{ \
            let token = Token {{ tag: 1 }}; \
            drop(token); \
            unsafe {{ COUNT = COUNT * 10 + 9; }} \
         }} \
         func main() -> i32 {{ release(); unsafe {{ COUNT }} }}"
    );
    assert_returns(&source, 19);
}

#[test]
fn a_dropped_value_is_not_dropped_again_at_scope_end() {
    let source = format!(
        "{TOKEN} \
         func release() {{ \
            let mut token = Token {{ tag: 1 }}; \
            drop(token); \
            token = Token {{ tag: 2 }}; \
         }} \
         func main() -> i32 {{ release(); unsafe {{ COUNT }} }}"
    );
    assert_returns(&source, 12);
}

#[test]
fn dropping_a_temporary_runs_its_destructor_once() {
    let source = format!(
        "{TOKEN} \
         func make() -> Token {{ Token {{ tag: 1 }} }} \
         func release() {{ drop(make()); }} \
         func main() -> i32 {{ release(); unsafe {{ COUNT }} }}"
    );
    assert_returns(&source, 1);
}

#[test]
fn dropping_a_shared_reference_runs_nothing() {
    let source = format!(
        "{TOKEN} \
         func release() {{ \
            let token = Token {{ tag: 1 }}; \
            drop(&token); \
         }} \
         func main() -> i32 {{ release(); unsafe {{ COUNT }} }}"
    );
    assert_returns(&source, 1);
}

#[test]
fn dropping_a_pointer_runs_nothing() {
    let source = format!(
        "{TOKEN} \
         func release() {{ \
            let token = Token {{ tag: 1 }}; \
            let pointer: *Token = &token; \
            drop(pointer); \
         }} \
         func main() -> i32 {{ release(); unsafe {{ COUNT }} }}"
    );
    assert_returns(&source, 1);
}

#[test]
fn dropping_copy_values_is_a_no_op() {
    let source = "\
func main() -> i32 { \
    drop(1); \
    let value = 5; \
    drop(value); \
    7 \
}";
    assert_returns(source, 7);
}

#[test]
fn drop_inside_a_loop_runs_on_every_iteration() {
    let source = format!(
        "{TOKEN} \
         func release() {{ \
            let mut index = 0; \
            while index < 3 {{ \
                let token = Token {{ tag: 1 }}; \
                drop(token); \
                index += 1; \
            }} \
         }} \
         func main() -> i32 {{ release(); unsafe {{ COUNT }} }}"
    );
    assert_returns(&source, 111);
}

/// Compiles `source` at both optimization levels and asserts the exit status.
fn assert_returns(source: &str, expected: i32) {
    for opt_level in [OptLevel::O0, OptLevel::O2] {
        let output = TempOutput::new("drop-builtin");
        compile(source, output.path(), Emit::Executable, opt_level, false)
            .unwrap_or_else(|error| panic!("compilation failed at {opt_level:?}: {error}"));
        assert_eq!(
            run(output.path()).code(),
            Some(expected),
            "unexpected result at {opt_level:?}"
        );
    }
}
