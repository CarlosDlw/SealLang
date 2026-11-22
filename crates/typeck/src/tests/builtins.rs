//! Control-flow, equality, and introspection built-ins.

use super::support::{Fixture, check_clean, check_errors, expect_ty, tail};

#[test]
fn introspection_queries_have_their_documented_result_types() {
    let (fixture, hir, results) = check_clean(
        "func main() -> i32 {\n\
           let name: *str = type_name::<i32>();\n\
           let value = 1;\n\
           let same: *str = type_of(value);\n\
           let size: usize = size_of::<i64>();\n\
           let align: usize = align_of::<i64>();\n\
           if name == same && size == 8 && align == 8 { 0 } else { 1 }\n\
         }",
    );
    let function = hir
        .body_of(
            hir.definitions()
                .iter_enumerated()
                .find(|(_, data)| fixture.symbol(data.name) == "main")
                .map(|(id, _)| id)
                .expect("main exists"),
        )
        .expect("main has a body");
    let query = tail(&hir, function);
    expect_ty(&hir, super::support::expr_ty(&results, query), "i32");
}

#[test]
fn panic_and_exit_diverge_from_any_return_type() {
    check_clean(
        "func fail() -> i32 { panic(\"failure\") }\n\
         func stop() -> bool { exit(3) }\n\
         func main() -> i32 { 0 }",
    );
}

#[test]
fn panic_requires_a_string_reference() {
    let (fixture, _, _) = check_errors("func main() { panic(1); }");
    fixture.expect("E3001", "mismatched types");
}

#[test]
fn exit_requires_an_i32_status() {
    let (fixture, _, _) = check_errors("func main() { exit(1u8); }");
    fixture.expect("E3001", "mismatched types");
}

#[test]
fn layout_queries_reject_unsized_types() {
    let (size, _, _) = check_errors("func main() { let _ = size_of::<str>(); }");
    size.expect("E3010", "size query");
    let (align, _, _) = check_errors("func main() { let _ = align_of::<[u8]>(); }");
    align.expect("E3010", "alignment query");
}

#[test]
fn type_name_accepts_unsized_types() {
    check_clean(
        "func name() -> *str { type_name::<str>() }\n\
         func main() -> i32 { 0 }",
    );
}

#[test]
fn introspection_rejects_value_arguments_and_missing_type_arguments() {
    // The rejected value argument is lowered and therefore stays in the HIR
    // arena, but the surrounding error expression gives it no type. Check the
    // diagnostic directly instead of asserting the clean-tree invariant.
    let with_value = Fixture::new("func main() { let _ = size_of::<u8>(1); }");
    with_value.check(64);
    with_value.expect("E3008", "takes no value arguments");
    let missing = Fixture::new("func main() { let _ = type_name(); }");
    missing.check(64);
    missing.expect("E2052", "expected 1 type argument");
}

#[test]
fn a_non_generic_builtin_rejects_type_arguments() {
    let (fixture, _, _) = check_errors("func main() { panic::<i32>(\"no\"); }");
    fixture.expect("E2053", "not generic");
}

#[test]
fn equality_assertions_are_scoped_to_test_builds() {
    let (fixture, _, _) = check_errors("func main() { assert_eq(1, 1); assert_ne(1, 2); }");
    fixture.expect("E8105", "available only in test and benchmark builds");
}

#[test]
fn user_items_shadow_every_builtin_name() {
    check_clean(
        "func panic(value: i32) -> i32 { value }\n\
         func type_name(value: i32) -> i32 { value }\n\
         func main() -> i32 { panic(1) + type_name(2) }",
    );
}

#[test]
fn type_of_reads_without_moving_its_operand() {
    check_clean(
        "struct Token { tag: i32 }\n\
         func main() -> i32 {\n\
           let token = Token { tag: 7 };\n\
           let name = type_of(token);\n\
           if name == \"Token\" { token.tag } else { 0 }\n\
         }",
    );
}

#[test]
fn generic_type_queries_accept_a_type_parameter() {
    check_clean(
        "func label<T>() -> *str { type_name::<T>() }\n\
         func bytes<T>() -> usize { size_of::<T>() }\n\
         func main() -> i32 { if label::<bool>() == \"bool\" && bytes::<i32>() == 4 { 0 } else { 1 } }",
    );
}
