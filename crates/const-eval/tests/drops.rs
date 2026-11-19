//! Dropping a value while evaluating a constant.
//!
//! The tests use the real `impl Drop` surface: HIR records the destructor, MIR
//! lowering inserts the drop site, and constant evaluation rejects it before any
//! destructor could run. Constructing such a value and storing it in a `const`
//! or `static` is allowed, because no scope releases it at compile time.

mod support;

use support::{Fixture, clean};

const TOKEN: &str = "\
struct Token { tag: i32 } \
impl Drop for Token { func drop(self: *mut Token) {} }";

#[test]
fn a_constant_of_a_drop_type_is_allowed() {
    clean(&format!("{TOKEN} const VALUE: Token = Token {{ tag: 1 }};"));
}

#[test]
fn dropping_a_local_in_a_constant_initializer_is_reported() {
    let fixture = Fixture::compile(&format!(
        "{TOKEN} const VALUE: i32 = {{ let token = Token {{ tag: 1 }}; 1 }};"
    ));
    fixture.expect_code("E4028");
}

#[test]
fn dropping_a_local_in_a_const_function_is_reported() {
    let fixture = Fixture::compile(&format!(
        "{TOKEN} \
         const func compute() -> i32 {{ let token = Token {{ tag: 1 }}; 1 }} \
         const VALUE: i32 = compute();"
    ));
    fixture.expect_code("E4028");
}

#[test]
fn an_explicit_drop_in_a_const_function_is_reported() {
    let fixture = Fixture::compile(&format!(
        "{TOKEN} \
         const func compute() -> i32 {{ \
            let token = Token {{ tag: 1 }}; \
            drop(token); \
            1 \
         }} \
         const VALUE: i32 = compute();"
    ));
    fixture.expect_code("E4028");
}

#[test]
fn a_static_of_a_drop_type_is_allowed() {
    clean(&format!(
        "{TOKEN} static VALUE: Token = Token {{ tag: 1 }};"
    ));
}

#[test]
fn a_const_function_returning_a_drop_value_is_allowed() {
    clean(&format!(
        "{TOKEN} const func make() -> Token {{ Token {{ tag: 1 }} }}"
    ));
}
