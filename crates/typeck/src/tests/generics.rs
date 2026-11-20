//! Generic bodies, inference, bounds, and trait solving.

use seal_hir::ExprKind;

use crate::MethodKind;

use super::support::{
    check_clean, check_errors, expr_ty, function, let_binding, local_ty, statements, tail,
    ty_text_with,
};

fn instantiation_args(
    hir: &seal_hir::Hir,
    results: &crate::TypeckResults,
    expr: seal_hir::HirExprId,
) -> Vec<seal_ty::TyId> {
    let instance = results
        .instantiation(expr)
        .expect("the expression has an instantiation");
    hir.ty_ctxt().substs_args(instance.args)
}

const PAIR: &str = "\
struct Pair<T, U> { first: T, second: U } \
impl<T, U> Pair<T, U> { \
    func new(first: T, second: U) -> Pair<T, U> { Pair { first: first, second: second } } \
    func first(self: *Self) -> T { (*self).first } \
    func with<V>(self: *Self, value: V) -> (T, V) { ((*self).first, value) } \
    func make<A, B>(first: A, second: B) -> Pair<A, B> { Pair { first: first, second: second } } \
}";

#[test]
fn a_generic_function_infers_its_argument_from_the_parameter() {
    let source = "func identity<T>(value: T) -> T { value } \
                  func main() -> i32 { identity(1) }";
    let (fixture, hir, results) = check_clean(source);
    let (_, body) = function(&fixture, &hir, "main");
    let call = tail(&hir, body);
    assert_eq!(ty_text_with(&fixture, &hir, expr_ty(&results, call)), "i32");
    assert_eq!(instantiation_args(&hir, &results, call).len(), 1);
}

#[test]
fn a_generic_raw_pointer_parameter_infers_its_pointee() {
    let source = "func identity<T>(value: *raw T) -> *raw T { value } \
                  func main() -> *raw i32 { \
                      let number = 41; \
                      let pointer = &number as *raw i32; \
                      identity(pointer) \
                  }";
    let (fixture, hir, results) = check_clean(source);
    let (_, body) = function(&fixture, &hir, "main");
    let call = tail(&hir, body);
    assert_eq!(
        ty_text_with(&fixture, &hir, expr_ty(&results, call)),
        "*raw i32"
    );
    let args = instantiation_args(&hir, &results, call);
    assert_eq!(args.len(), 1);
    assert_eq!(
        args[0],
        hir.ty_ctxt()
            .lookup(seal_ty::TyKind::Int(seal_ty::IntTy::I32))
            .expect("i32 is interned")
    );
}

#[test]
fn the_expected_raw_pointer_return_type_infers_the_pointee() {
    let source = "func make<T>() -> *raw T { make::<T>() } \
                  func caller() -> *raw bool { make() } \
                  func main() -> i32 { 0 }";
    let (fixture, hir, results) = check_clean(source);
    let (_, body) = function(&fixture, &hir, "caller");
    let call = tail(&hir, body);
    assert_eq!(
        ty_text_with(&fixture, &hir, expr_ty(&results, call)),
        "*raw bool"
    );
    let args = instantiation_args(&hir, &results, call);
    assert_eq!(args.len(), 1);
    assert_eq!(
        args[0],
        hir.ty_ctxt()
            .lookup(seal_ty::TyKind::Bool)
            .expect("bool is interned")
    );
}

#[test]
fn the_expected_return_type_guides_argument_inference() {
    let source = "func identity<T>(value: T) -> T { value } \
                  func main() -> i64 { identity(1) }";
    let (fixture, hir, results) = check_clean(source);
    let (_, body) = function(&fixture, &hir, "main");
    let call = tail(&hir, body);
    assert_eq!(ty_text_with(&fixture, &hir, expr_ty(&results, call)), "i64");
    assert_eq!(
        instantiation_args(&hir, &results, call)[0],
        hir.ty_ctxt()
            .lookup(seal_ty::TyKind::Int(seal_ty::IntTy::I64))
            .expect("i64 is interned")
    );
}

#[test]
fn explicit_type_arguments_are_exact() {
    let source = "func identity<T>(value: T) -> T { value } \
                  func main() -> u8 { identity::<u8>(1) }";
    let (fixture, hir, results) = check_clean(source);
    let (_, body) = function(&fixture, &hir, "main");
    assert_eq!(
        ty_text_with(&fixture, &hir, expr_ty(&results, tail(&hir, body))),
        "u8"
    );
}

#[test]
fn explicit_type_arguments_control_literal_range_checking() {
    let source = "func identity<T>(value: T) -> T { value } \
                  func main() { identity::<u8>(300); }";
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3009", "out of range for `u8`");
}

#[test]
fn a_wrong_explicit_argument_count_is_reported_once() {
    let source = "func identity<T>(value: T) -> T { value } \
                  func main() -> i32 { identity::<i32, u8>(1) }";
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E2052", "expected 1 type argument");
}

#[test]
fn an_unconstrained_return_parameter_is_inferred_from_the_expected_type() {
    let source = "func make<T>() -> T { make::<T>() } \
                  func main() -> i32 { make() }";
    let (fixture, hir, results) = check_clean(source);
    let (_, body) = function(&fixture, &hir, "main");
    assert_eq!(
        ty_text_with(&fixture, &hir, expr_ty(&results, tail(&hir, body))),
        "i32"
    );
}

#[test]
fn an_unconstrained_parameter_without_context_is_e3038() {
    let source = "func make<T>() -> T { make::<T>() } func main() { make(); }";
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3038", "cannot infer type parameter `T`");
}

#[test]
fn a_struct_literal_infers_every_argument() {
    let source = "struct Pair<T, U> { first: T, second: U } \
                  func main() { let pair = Pair { first: 1, second: true }; }";
    let (fixture, hir, results) = check_clean(source);
    let (_, body) = function(&fixture, &hir, "main");
    let local = let_binding(&hir, statements(&hir, body)[0]);
    assert_eq!(
        ty_text_with(&fixture, &hir, local_ty(&results, local)),
        "Pair<i32, bool>"
    );
}

#[test]
fn repeated_parameters_share_one_inferred_argument() {
    let source = "struct Pair<T> { first: T, second: T } \
                  func main() { let pair = Pair { first: 1, second: 2 }; }";
    let (fixture, hir, results) = check_clean(source);
    let (_, body) = function(&fixture, &hir, "main");
    let local = let_binding(&hir, statements(&hir, body)[0]);
    assert_eq!(
        ty_text_with(&fixture, &hir, local_ty(&results, local)),
        "Pair<i32>"
    );
}

#[test]
fn an_explicit_struct_literal_checks_field_types() {
    let source = "struct Pair<T, U> { first: T, second: U } \
                  func main() { let pair: Pair<i32, bool> = Pair::<i32, bool> { first: 1, second: 2 }; }";
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3001", "mismatched types");
}

#[test]
fn an_empty_generic_literal_without_context_is_e3038() {
    let source = "struct Empty<T> {} func main() { let empty = Empty {}; }";
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3038", "cannot infer type parameter `T`");
}

#[test]
fn an_enum_variant_infers_from_its_fields() {
    let source = "enum Option<T> { Some(T), None } \
                  func main() -> i32 { \
                      match Option::Some(1) { Option::Some(value) => value, Option::None => 0 } \
                  }";
    check_clean(source);
}

#[test]
fn an_ambiguous_unit_variant_needs_an_expected_type() {
    let source = "enum Option<T> { Some(T), None } \
                  func main() { let value: Option<i32> = Option::None; }";
    check_clean(source);
}

#[test]
fn a_bare_unit_variant_is_e3038() {
    let source = "enum Option<T> { Some(T), None } func main() { let value = Option::None; }";
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3038", "cannot infer type parameter `T`");
}

#[test]
fn an_explicit_unit_variant_is_complete() {
    let source = "enum Option<T> { Some(T), None } \
                  func main() -> i32 { \
                      match Option::<i32>::None { Option::Some(value) => value, Option::None => 0 } \
                  }";
    check_clean(source);
}

#[test]
fn a_generic_variant_reports_the_instantiated_field_type() {
    let source = "enum Option<T> { Some(T), None } \
                  func main() { let value: Option<bool> = Option::Some(1); }";
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3001", "mismatched types");
}

#[test]
fn matching_a_generic_enum_keeps_the_parameter_rigid() {
    let source = "enum Option<T> { Some(T), None } \
                  func get<T>(value: Option<T>, fallback: T) -> T { \
                      match value { Option::Some(inner) => inner, Option::None => fallback } \
                  }";
    check_clean(source);
}

#[test]
fn a_generic_inherent_method_infers_from_its_arguments() {
    let source =
        format!("{PAIR} func main() -> i32 {{ let pair = Pair::new(1, true); pair.first() }}");
    check_clean(&source);
}

#[test]
fn a_method_parameter_is_inferred_from_an_argument() {
    let source = format!(
        "{PAIR} func inferred() -> (i32, f64) {{ Pair::new(1, true).with(2.0) }} \
         func main() -> i32 {{ Pair::new(1, true).first() }}"
    );
    check_clean(&source);
}

#[test]
fn explicit_method_type_arguments_are_checked() {
    let source = format!(
        "{PAIR} func inferred() -> (i32, u8) {{ Pair::new(1, true).with::<u8>(2) }} \
         func main() -> i32 {{ Pair::new(1, true).first() }}"
    );
    check_clean(&source);
}

#[test]
fn a_method_type_argument_count_mismatch_is_e3044() {
    let source =
        format!("{PAIR} func main() -> i32 {{ Pair::new(1, true).with::<u8, bool>(2); 0 }}");
    let (fixture, _, _) = check_errors(&source);
    fixture.expect("E3044", "takes 1 type argument");
}

#[test]
fn an_associated_function_infers_impl_arguments_from_expected_type() {
    let source = format!(
        "{PAIR} func main() -> i32 {{ \
            let pair: Pair<i32, bool> = Pair::make(1, true); pair.first() \
        }}"
    );
    check_clean(&source);
}

#[test]
fn explicit_associated_type_arguments_select_the_impl() {
    let source =
        format!("{PAIR} func main() -> i32 {{ Pair::<i32, bool>::make(1, true).first() }}");
    check_clean(&source);
}

#[test]
fn a_bound_method_resolves_symbolically() {
    let source = "\
        trait Show { func show(self: *Self) -> i32; } \
        struct Point { x: i32 } \
        impl Show for Point { func show(self: *Point) -> i32 { (*self).x } } \
        func display<T: Show>(value: *T) -> i32 { value.show() } \
        func main() -> i32 { let point = Point { x: 1 }; display(&point) }";
    let (fixture, hir, results) = check_clean(source);
    let (_, body) = function(&fixture, &hir, "display");
    let call = tail(&hir, body);
    assert!(matches!(
        results.method(call).expect("method resolved").kind,
        MethodKind::TraitBound { .. }
    ));
    let instance = results
        .instantiation(call)
        .expect("the symbolic call is recorded");
    assert_eq!(
        hir.definitions().kind(instance.def),
        seal_def::DefKind::TraitMethod
    );
}

#[test]
fn two_bounds_with_the_same_method_are_ambiguous() {
    let source = "\
        trait A { func name(self: *Self) -> i32; } \
        trait B { func name(self: *Self) -> i32; } \
        func value<T: A + B>(input: *T) -> i32 { input.name() }";
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3040", "more than one bound");
}

#[test]
fn a_missing_bound_method_is_reported() {
    let source = "func value<T>(input: *T) -> i32 { input.missing() }";
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3027", "no method named `missing`");
}

#[test]
fn a_trait_bound_is_proved_for_a_concrete_impl() {
    let source = "\
        trait Show { } \
        struct Point { x: i32 } \
        impl Show for Point { } \
        func need<T: Show>(value: T) { } \
        func main() { need(Point { x: 1 }); }";
    check_clean(source);
}

#[test]
fn a_missing_trait_bound_is_e3039() {
    let source = "trait Show { } func need<T: Show>(value: T) { } func main() { need(1); }";
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3039", "trait bound `T: Show`");
}

#[test]
fn a_blanket_impl_satisfies_a_bound() {
    let source = "\
        trait Show { } impl<T> Show for T { } \
        func need<T: Show>(value: T) { } func main() { need(1); }";
    check_clean(source);
}

#[test]
fn a_conditional_copy_impl_is_checked() {
    let source = "\
        struct Pair<T> { first: T, second: T } \
        trait Show { } impl<T: Copy> Show for Pair<T> { } \
        func need<T: Show>(value: T) { } \
        func good() { need(Pair { first: 1, second: 2 }); } \
        struct Token { value: i32 } \
        func bad() { need(Pair { first: Token { value: 1 }, second: Token { value: 2 } }); }";
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3039", "not satisfied");
}

#[test]
fn a_copy_bound_rejects_a_non_copy_argument() {
    let source = "struct Token { value: i32 } \
                  func dup<T: Copy>(value: T) -> (T, T) { (value, value) } \
                  func main() { dup(Token { value: 1 }); }";
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3043", "does not satisfy the `Copy` bound of `T`");
}

#[test]
fn a_copy_bound_accepts_a_scalar() {
    let source = "func dup<T: Copy>(value: T) -> (T, T) { (value, value) } \
                  func main() { dup(1); }";
    check_clean(source);
}

#[test]
fn a_supertrait_method_is_visible() {
    let source = "\
        trait Base { func base(self: *Self) -> i32; } \
        trait Child where Self: Base { } \
        func value<T: Child>(input: *T) -> i32 { input.base() }";
    check_clean(source);
}

#[test]
fn a_supertrait_satisfies_a_supertrait_goal() {
    let source = "\
        trait Base { } \
        trait Child where Self: Base { } \
        func need<T: Base>(value: T) { } \
        func pass<T: Child>(value: T) { need(value); }";
    check_clean(source);
}

#[test]
fn nested_bound_arguments_use_the_complete_substitution() {
    let source = "\
        trait Wrapper<T> { } \
        struct Boxed<T> { value: T } \
        impl Wrapper<i32> for Boxed<i32> { } \
        func need<T: Wrapper<U>, U: Copy>(value: T, other: U) { } \
        func main() { need(Boxed { value: 1 }, 2); }";
    check_clean(source);
}

#[test]
fn a_self_referential_bound_reports_the_recursion_limit() {
    let source = "\
        trait Loop { } impl<T: Loop> Loop for T { } \
        func need<T: Loop>(value: T) { } func main() { need(1); }";
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3042", "recursion limit");
}

#[test]
fn a_type_parameter_rejects_arithmetic_with_e3041() {
    let source = "func bad<T>(value: T) -> T { value + value }";
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3041", "type parameter `T`");
}

#[test]
fn a_type_parameter_rejects_field_and_index_access_without_panicking() {
    let source = "func field<T>(value: T) { value.x; }";
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3015", "has no fields");

    let source = "func index<T>(value: T) { value[0]; }";
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3021", "cannot index");
}

#[test]
fn a_generic_function_with_explicit_arguments_is_a_value() {
    let source = "func identity<T>(value: T) -> T { value } \
                  func main() { let function = identity::<i32>; }";
    check_clean(source);
}

#[test]
fn generic_types_work_in_arrays_and_nested_aggregates() {
    let source = "\
        struct Pair<T, U> { first: T, second: U } \
        func nested<T, U>(values: [Pair<Pair<T, U>, T>; 2]) -> Pair<T, U> { \
            values[0].first \
        }";
    check_clean(source);
}

#[test]
fn a_generic_struct_pattern_uses_the_instantiated_field_types() {
    let source = "\
        struct Pair<T> { value: T } \
        enum Maybe<T> { Some(Pair<T>), None } \
        func value<T>(input: Maybe<T>) -> T { \
            match input { Maybe::Some(pair) => pair.value, Maybe::None => input_value(input) } \
        } \
        func input_value<T>(input: Maybe<T>) -> T { input_value(input) }";
    check_clean(source);
}

#[test]
fn independent_parameters_infer_separately() {
    let source = "func zip<A, B>(first: A, second: B) -> (A, B) { (first, second) } \
                  func main() -> (i32, bool) { zip(1, true) }";
    let (fixture, hir, results) = check_clean(source);
    let (_, body) = function(&fixture, &hir, "main");
    let call = tail(&hir, body);
    assert_eq!(
        ty_text_with(&fixture, &hir, expr_ty(&results, call)),
        "(i32, bool)"
    );
    assert_eq!(instantiation_args(&hir, &results, call).len(), 2);
}

#[test]
fn a_generic_impl_body_sees_the_instantiated_self_type() {
    let source = "\
        struct Boxed<T> { value: T } \
        impl<T> Boxed<T> { \
            func into(self: Self) -> T { self.value } \
            func set(self: *mut Self, value: T) { (*self).value = value; } \
        } \
        func main() -> i32 { \
            let mut boxed = Boxed { value: 1 }; \
            boxed.set(2); \
            boxed.into() \
        }";
    check_clean(source);
}

#[test]
fn an_inherent_method_wins_over_a_matching_trait_impl() {
    let source = "\
        trait Show { func show(self: *Self) -> i32; } \
        struct Pair<T> { value: T } \
        impl<T> Show for Pair<T> { func show(self: *Pair<T>) -> i32 { 0 } } \
        impl<T> Pair<T> { func show(self: *Self) -> i32 { 1 } } \
        func read<T>(pair: *Pair<T>) -> i32 { pair.show() }";
    let (fixture, hir, results) = check_clean(source);
    let (_, body) = function(&fixture, &hir, "read");
    let call = tail(&hir, body);
    assert!(matches!(
        results.method(call).expect("method resolved").kind,
        MethodKind::User { .. }
    ));
}

#[test]
fn a_parameter_cannot_be_constructed_from_a_literal() {
    let source = "func evil<T>() -> T { 0 }";
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3001", "mismatched types");
}

#[test]
fn generic_aggregates_check_in_tuples_slices_and_fields() {
    let source = "\
        struct Pair<T, U> { first: T, second: U } \
        enum Maybe<T> { Some(T), None } \
        struct Holder<T> { value: Maybe<T>, items: [Maybe<T>; 2] } \
        func nested<T, U>(value: (Pair<T, U>, *[Pair<T, U>])) -> T { value.0.first } \
        func hold<T>(holder: Holder<T>) -> Maybe<T> { holder.items[0] } \
        func input<T>(holder: Holder<T>) -> Maybe<T> { holder.value }";
    check_clean(source);
}

#[test]
fn two_method_parameters_infer_from_arguments_and_return() {
    let source = "\
        struct Pair<T, U> { first: T, second: U } \
        impl<T, U> Pair<T, U> { \
            func new(first: T, second: U) -> Pair<T, U> { Pair { first: first, second: second } } \
            func combine<V, W>(self: *Self, first: V, second: W) -> Pair<V, W> { \
                Pair { first: first, second: second } \
            } \
        } \
        func main() -> bool { \
            let pair = Pair::new(1, true).combine(2.0, true); pair.second \
        }";
    check_clean(source);
}

#[test]
fn a_type_parameter_rejects_other_operators_with_e3041() {
    let source = "func less<T>(value: T) -> bool { value < value }";
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3041", "type parameter `T`");

    let source = "func bits<T>(value: T) -> T { value & value }";
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3041", "type parameter `T`");
}

#[test]
fn a_wrong_trait_argument_does_not_satisfy_a_bound() {
    let source = "\
        trait Wrapper<T> { } \
        struct Boxed<T> { value: T } \
        impl Wrapper<i32> for Boxed<i32> { } \
        func need<T: Wrapper<bool>>(value: T) { } \
        func main() { need(Boxed { value: 1 }); }";
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3039", "Wrapper<bool>");
}

#[test]
fn pointers_to_a_parameter_are_usable_without_a_copy_bound() {
    let source = "func alias<T>(value: *T) -> (*T, *T) { (value, value) } \
                  func exclusive<T>(value: *mut T) -> *mut T { value }";
    check_clean(source);
}

#[test]
fn a_generic_function_needs_explicit_arguments_as_a_value() {
    let source = "func identity<T>(value: T) -> T { value } \
                  func consume(value: i32) { } \
                  func main() { consume(identity); }";
    let (fixture, _, _) = check_errors(source);
    fixture.expect("E3099", "needs explicit type arguments");
}

#[test]
fn every_inferred_call_records_a_complete_instantiation() {
    let source = "func identity<T>(value: T) -> T { value } \
                  func main() -> i32 { identity(1) }";
    let (fixture, hir, results) = check_clean(source);
    let (_, body) = function(&fixture, &hir, "main");
    let call = tail(&hir, body);
    let instance = results.instantiation(call).expect("complete instance");
    assert_eq!(hir.ty_ctxt().substs_args(instance.args).len(), 1);
    assert!(matches!(hir.expr(call).kind, ExprKind::Call { .. }));
}
