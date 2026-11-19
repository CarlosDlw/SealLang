//! Runtime behavior of `impl Drop` destructors.
//!
//! Each scenario compiles and runs a real program at `-O0` and `-O2`, so the
//! observable effect is the value the program computes, not the MIR or the
//! emitted IR. `COUNT` folds the tag of every destructor call, which makes both
//! the number and the order of the calls visible in one integer.
//!
//! A process exit status is one byte, so every expected value stays below 256.

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
fn a_destructor_runs_when_the_value_goes_out_of_scope() {
    let source = format!(
        "{TOKEN} \
         func release() {{ let token = Token {{ tag: 1 }}; }} \
         func main() -> i32 {{ release(); unsafe {{ COUNT }} }}"
    );
    assert_returns(&source, 1);
}

#[test]
fn locals_drop_in_reverse_declaration_order() {
    let source = format!(
        "{TOKEN} \
         func release() {{ \
            let first = Token {{ tag: 1 }}; \
            let second = Token {{ tag: 2 }}; \
         }} \
         func main() -> i32 {{ release(); unsafe {{ COUNT }} }}"
    );
    assert_returns(&source, 21);
}

#[test]
fn a_nested_struct_drops_its_fields_in_declaration_order() {
    let source = format!(
        "{TOKEN} \
         struct Pair {{ first: Token, second: Token }} \
         func release() {{ \
            let pair = Pair {{ first: Token {{ tag: 1 }}, second: Token {{ tag: 2 }} }}; \
         }} \
         func main() -> i32 {{ release(); unsafe {{ COUNT }} }}"
    );
    assert_returns(&source, 12);
}

#[test]
fn the_destructor_runs_before_the_fields_it_owns() {
    let source = format!(
        "{TOKEN} \
         struct Outer {{ head: Token, tail: Token }} \
         impl Drop for Outer {{ \
            func drop(self: *mut Outer) {{ unsafe {{ COUNT = COUNT * 10 + 1; }} }} \
         }} \
         func release() {{ \
            let value = Outer {{ head: Token {{ tag: 2 }}, tail: Token {{ tag: 3 }} }}; \
         }} \
         func main() -> i32 {{ release(); unsafe {{ COUNT }} }}"
    );
    assert_returns(&source, 123);
}

#[test]
fn moving_a_value_does_not_duplicate_the_destructor() {
    let source = format!(
        "{TOKEN} \
         func release() {{ \
            let token = Token {{ tag: 1 }}; \
            let moved = token; \
            let inner = moved; \
         }} \
         func main() -> i32 {{ release(); unsafe {{ COUNT }} }}"
    );
    assert_returns(&source, 1);
}

#[test]
fn a_parameter_drops_at_the_end_of_the_callee() {
    let source = format!(
        "{TOKEN} \
         func take(value: Token) {{}} \
         func release() {{ let token = Token {{ tag: 1 }}; take(token); }} \
         func main() -> i32 {{ release(); unsafe {{ COUNT }} }}"
    );
    assert_returns(&source, 1);
}

#[test]
fn a_returned_value_drops_in_the_caller() {
    let source = format!(
        "{TOKEN} \
         func make() -> Token {{ Token {{ tag: 1 }} }} \
         func release() {{ let value = make(); }} \
         func main() -> i32 {{ release(); unsafe {{ COUNT }} }}"
    );
    assert_returns(&source, 1);
}

#[test]
fn an_early_return_runs_the_pending_destructors() {
    let source = format!(
        "{TOKEN} \
         func release() {{ let token = Token {{ tag: 1 }}; return; }} \
         func main() -> i32 {{ release(); unsafe {{ COUNT }} }}"
    );
    assert_returns(&source, 1);
}

#[test]
fn a_loop_body_drops_its_value_on_every_iteration() {
    let source = format!(
        "{TOKEN} \
         func release() {{ \
            let mut index = 0; \
            while index < 3 {{ \
                let token = Token {{ tag: 1 }}; \
                index += 1; \
            }} \
         }} \
         func main() -> i32 {{ release(); unsafe {{ COUNT }} }}"
    );
    assert_returns(&source, 111);
}

#[test]
fn break_drops_the_loop_body_value() {
    let source = format!(
        "{TOKEN} \
         func release() {{ \
            let mut index = 0; \
            while index < 3 {{ \
                let token = Token {{ tag: 4 }}; \
                break; \
            }} \
         }} \
         func main() -> i32 {{ release(); unsafe {{ COUNT }} }}"
    );
    assert_returns(&source, 4);
}

#[test]
fn continue_drops_the_loop_body_value() {
    let source = format!(
        "{TOKEN} \
         func release() {{ \
            let mut index = 0; \
            while index < 3 {{ \
                let token = Token {{ tag: 1 }}; \
                index += 1; \
                continue; \
            }} \
         }} \
         func main() -> i32 {{ release(); unsafe {{ COUNT }} }}"
    );
    assert_returns(&source, 111);
}

#[test]
fn an_enum_drops_only_the_active_variant() {
    let source = format!(
        "{TOKEN} \
         enum Slot {{ Value(Token), Empty }} \
         func release() {{ let slot = Slot::Value(Token {{ tag: 1 }}); }} \
         func main() -> i32 {{ release(); unsafe {{ COUNT }} }}"
    );
    assert_returns(&source, 1);
}

#[test]
fn a_generic_type_runs_its_destructor_for_every_instantiation() {
    let source = format!(
        "{TOKEN} \
         struct Wrapper<T> {{ value: T }} \
         impl<T> Drop for Wrapper<T> {{ \
            func drop(self: *mut Wrapper<T>) {{ unsafe {{ COUNT = COUNT + 1; }} }} \
         }} \
         func release() {{ \
            let owned = Wrapper {{ value: Token {{ tag: 1 }} }}; \
            let scalar = Wrapper {{ value: 7 }}; \
         }} \
         func main() -> i32 {{ release(); unsafe {{ COUNT }} }}"
    );
    // The two locals drop in reverse declaration order: `scalar` adds one,
    // `owned` runs its destructor and then drops its `Token` field.
    assert_returns(&source, 21);
}

#[test]
fn an_array_drops_its_elements_in_index_order() {
    let source = format!(
        "{TOKEN} \
         func release() {{ \
            let values = [Token {{ tag: 1 }}, Token {{ tag: 2 }}, Token {{ tag: 3 }}]; \
         }} \
         func main() -> i32 {{ release(); unsafe {{ COUNT }} }}"
    );
    assert_returns(&source, 123);
}

#[test]
fn a_for_loop_drops_the_item_and_the_remaining_elements_once() {
    let source = format!(
        "{TOKEN} \
         func release() {{ \
            for item in [Token {{ tag: 1 }}, Token {{ tag: 2 }}, Token {{ tag: 3 }}] {{ \
                break; \
            }} \
         }} \
         func main() -> i32 {{ release(); unsafe {{ COUNT }} }}"
    );
    assert_returns(&source, 123);
}

#[test]
fn a_complete_for_loop_drops_each_item_once() {
    let source = format!(
        "{TOKEN} \
         func release() {{ \
            for item in [Token {{ tag: 1 }}, Token {{ tag: 2 }}] {{ }} \
         }} \
         func main() -> i32 {{ release(); unsafe {{ COUNT }} }}"
    );
    assert_returns(&source, 12);
}

#[test]
fn a_destructor_may_write_its_own_field() {
    let source = format!(
        "{TOKEN} \
         func release() {{ let token = Token {{ tag: 1 }}; }} \
         func main() -> i32 {{ release(); unsafe {{ COUNT }} }}"
    );
    assert_returns(&source, 1);

    let source = "\
struct Cell { value: i32 } \
impl Drop for Cell { \
    func drop(self: *mut Cell) { \
        (*self).value = (*self).value + 1; \
        unsafe { COUNT = (*self).value; } \
    } \
} \
static mut COUNT: i32 = 0; \
func release() { let cell = Cell { value: 1 }; } \
func main() -> i32 { release(); unsafe { COUNT } }";
    assert_returns(source, 2);
}

#[test]
fn an_empty_destructor_still_compiles_and_runs() {
    let source = "\
struct Marker { value: i32 } \
impl Drop for Marker { func drop(self: *mut Marker) {} } \
static mut COUNT: i32 = 0; \
func release() { \
    let marker = Marker { value: 1 }; \
    unsafe { COUNT = COUNT + marker.value; } \
} \
func main() -> i32 { release(); unsafe { COUNT } }";
    assert_returns(source, 1);
}

/// Compiles `source` at both optimization levels and asserts the exit status.
fn assert_returns(source: &str, expected: i32) {
    for opt_level in [OptLevel::O0, OptLevel::O2] {
        let output = TempOutput::new("drop-destructor");
        compile(source, output.path(), Emit::Executable, opt_level, false)
            .unwrap_or_else(|error| panic!("compilation failed at {opt_level:?}: {error}"));
        assert_eq!(
            run(output.path()).code(),
            Some(expected),
            "unexpected result at {opt_level:?}"
        );
    }
}
