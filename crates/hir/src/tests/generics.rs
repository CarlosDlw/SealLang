//! Generic parameter registration, scopes, bounds, and impl records.

use seal_def::{DefId, DefKind};
use seal_ty::{Bound, ParamTyId, TraitRef, TyId, TyKind};

use crate::{ExprKind, ReceiverKind};

use super::support::{Fixture, assert_invariants, function, lower_clean, path_res, tail};

fn named_type(fixture: &Fixture, hir: &crate::Hir, name: &str, kind: DefKind) -> DefId {
    let symbol = fixture.interner.intern(name);
    hir.definitions()
        .iter_enumerated()
        .find(|(_, data)| data.kind == kind && data.name == symbol)
        .map(|(id, _)| id)
        .unwrap_or_else(|| panic!("no {kind:?} named `{name}`"))
}

fn param(fixture: &Fixture, hir: &crate::Hir, owner: DefId, name: &str) -> ParamTyId {
    let symbol = fixture.interner.intern(name);
    hir.generics(owner)
        .and_then(|generics| generics.param_by_name(symbol).map(|param| param.param))
        .unwrap_or_else(|| panic!("owner has no parameter `{name}`"))
}

fn method<'a>(
    fixture: &Fixture,
    hir: &'a crate::Hir,
    _owner: DefId,
    name: &str,
) -> (DefId, &'a crate::Body) {
    let symbol = fixture.interner.intern(name);
    let definition = hir
        .definitions()
        .iter_enumerated()
        .find(|(_, data)| data.kind == DefKind::Function && data.name == symbol)
        .map(|(id, _)| id)
        .unwrap_or_else(|| panic!("no method named `{name}`"));
    let body = hir.body_of(definition).expect("method body");
    (definition, body)
}

fn param_ty(hir: &crate::Hir, param: ParamTyId) -> TyId {
    hir.ty_ctxt().param_ty(param)
}

fn adt_args(hir: &crate::Hir, ty: TyId) -> Vec<TyId> {
    let TyKind::Adt { args, .. } = hir.ty_ctxt().kind(ty) else {
        panic!(
            "expected an instantiated aggregate, got {:?}",
            hir.ty_ctxt().kind(ty)
        );
    };
    hir.ty_ctxt().substs_args(args)
}

#[test]
fn a_free_function_parameter_is_visible_in_every_annotation() {
    let (fixture, hir) = lower_clean(
        "func wrap<T>(value: T, pointer: *T, nested: (T, [T; 2])) -> (T, *T) { (value, pointer) }",
    );
    let (definition, body) = function(&fixture, &hir, "wrap");
    let expected = param_ty(&hir, param(&fixture, &hir, definition, "T"));

    assert_eq!(body.params[0].ty, expected);
    let TyKind::Ref { pointee, .. } = hir.ty_ctxt().kind(body.params[1].ty) else {
        panic!("expected *T");
    };
    assert_eq!(pointee, expected);
    let TyKind::Tuple(tuple) = hir.ty_ctxt().kind(body.params[2].ty) else {
        panic!("expected a tuple");
    };
    let elements = hir.ty_ctxt().tuple_elements(tuple);
    assert_eq!(elements[0], expected);
    let TyKind::Array { element, .. } = hir.ty_ctxt().kind(elements[1]) else {
        panic!("expected [T; 2]");
    };
    assert_eq!(element, expected);
    let TyKind::Tuple(ret) = hir.ty_ctxt().kind(body.ret.expect("return type")) else {
        panic!("expected a tuple return");
    };
    assert_eq!(hir.ty_ctxt().tuple_elements(ret)[0], expected);
    assert_invariants(&fixture, &hir);
}

#[test]
fn struct_and_enum_parameters_are_visible_in_every_field() {
    let (fixture, hir) = lower_clean(
        "struct Pair<T, U> { first: T, second: *U, tuple: (T, U), array: [T; 2] } \
         enum Either<T> { Left(T), Right { value: *T } }",
    );
    let pair = named_type(&fixture, &hir, "Pair", DefKind::Struct);
    let t = param_ty(&hir, param(&fixture, &hir, pair, "T"));
    let u = param_ty(&hir, param(&fixture, &hir, pair, "U"));
    let adt = hir.adt(pair).expect("Pair has an aggregate");
    let fields = hir.ty_ctxt().adt(adt).fields;
    assert_eq!(fields[0].ty, t);
    let TyKind::Ref { pointee, .. } = hir.ty_ctxt().kind(fields[1].ty) else {
        panic!("expected *U");
    };
    assert_eq!(pointee, u);
    let TyKind::Tuple(tuple) = hir.ty_ctxt().kind(fields[2].ty) else {
        panic!("expected (T, U)");
    };
    assert_eq!(hir.ty_ctxt().tuple_elements(tuple), [t, u]);
    let TyKind::Array { element, .. } = hir.ty_ctxt().kind(fields[3].ty) else {
        panic!("expected [T; 2]");
    };
    assert_eq!(element, t);

    let either = named_type(&fixture, &hir, "Either", DefKind::Enum);
    let either_param = param_ty(&hir, param(&fixture, &hir, either, "T"));
    let adt = hir.adt(either).expect("Either has an aggregate");
    let variants = hir.ty_ctxt().adt(adt).variants;
    assert_eq!(variants[0].fields[0].ty, either_param);
    let TyKind::Ref { pointee, .. } = hir.ty_ctxt().kind(variants[1].fields[0].ty) else {
        panic!("expected *T");
    };
    assert_eq!(pointee, either_param);
    assert_invariants(&fixture, &hir);
}

#[test]
fn explicit_type_arguments_lower_to_instantiated_aggregates() {
    let (fixture, hir) = lower_clean(
        "struct Pair<T, U> { first: T, second: U } \
         func make<T, U>(value: Pair<T, U>, nested: Pair<Pair<T, U>, T>) {}",
    );
    let (definition, body) = function(&fixture, &hir, "make");
    let t = param_ty(&hir, param(&fixture, &hir, definition, "T"));
    let u = param_ty(&hir, param(&fixture, &hir, definition, "U"));
    assert_eq!(adt_args(&hir, body.params[0].ty), [t, u]);

    let outer = adt_args(&hir, body.params[1].ty);
    assert_eq!(outer[1], t);
    assert_eq!(adt_args(&hir, outer[0]), [t, u]);
    assert_invariants(&fixture, &hir);
}

#[test]
fn omitted_expression_arguments_stay_available_for_inference() {
    let (fixture, hir) = lower_clean(
        "struct Pair<T, U> { first: T, second: U } \
         func main() { let explicit = Pair::<i32, bool> { first: 1, second: true }; \
                       let omitted = Pair { first: 1, second: true }; }",
    );
    let (_, body) = function(&fixture, &hir, "main");
    let statements = hir.block(body.root).statements.clone();
    let explicit = match hir.stmt(statements[0]).kind.clone() {
        crate::StmtKind::Let { init, .. } => init.expect("initializer"),
        _ => panic!("expected let"),
    };
    let omitted = match hir.stmt(statements[1]).kind.clone() {
        crate::StmtKind::Let { init, .. } => init.expect("initializer"),
        _ => panic!("expected let"),
    };
    assert_eq!(
        hir.ty_ctxt()
            .substs_len(hir.expr_args(explicit).expect("explicit args")),
        2
    );
    assert_eq!(hir.expr_args(omitted), None);
    assert_invariants(&fixture, &hir);
}

#[test]
fn trait_self_and_parameters_shape_method_signatures() {
    let (fixture, hir) =
        lower_clean("trait Convert<T> { func convert(self: *Self, value: T) -> Self; }");
    let trait_def = named_type(&fixture, &hir, "Convert", DefKind::Trait);
    let declaration = hir.trait_definition(trait_def).expect("semantic trait");
    assert_eq!(declaration.generics.len(), 2);
    let self_param = declaration
        .generics
        .params
        .iter()
        .find(|param| param.is_self)
        .expect("implicit Self");
    let t = param(&fixture, &hir, trait_def, "T");
    let method = declaration
        .method(fixture.interner.intern("convert"))
        .expect("method");
    assert_eq!(method.signature.params.len(), 1);
    assert_eq!(method.signature.params[0], param_ty(&hir, t));
    assert_eq!(
        method.signature.ret,
        param_ty(&hir, self_param.param),
        "the return type keeps Self as a parameter"
    );
    assert_invariants(&fixture, &hir);
}

#[test]
fn generic_impl_headers_record_semantic_targets() {
    let (fixture, hir) = lower_clean(
        "struct Pair<T, U> { first: T, second: U } \
         trait Convert<T> { func convert(self: *Self) -> T; } \
         impl<T, U> Pair<T, U> { func first(self: *Self) -> T { (*self).first } } \
         impl<T, U> Convert<T> for Pair<T, U> { func convert(self: *Pair<T, U>) -> T { (*self).first } }",
    );
    let pair = named_type(&fixture, &hir, "Pair", DefKind::Struct);
    let impls = hir.impls_for(pair);
    assert_eq!(impls.len(), 2);
    assert_eq!(hir.inherent_impls().len(), 1);
    let inherent = &hir.inherent_impls()[0];
    assert_eq!(adt_args(&hir, inherent.self_ty).len(), 2);
    assert_eq!(inherent.generics.len(), 2);
    assert_eq!(inherent.methods.len(), 1);

    let trait_impls = hir.trait_impls();
    assert_eq!(trait_impls.len(), 1);
    let trait_impl = &trait_impls[0];
    assert_eq!(adt_args(&hir, trait_impl.trait_ref.self_ty).len(), 2);
    assert_eq!(hir.ty_ctxt().substs_len(trait_impl.trait_ref.args), 1);
    assert_eq!(trait_impl.generics.len(), 2);
    assert_invariants(&fixture, &hir);
}

#[test]
fn impl_methods_see_impl_parameters_and_self() {
    let (fixture, hir) = lower_clean(
        "struct Pair<T> { value: T } \
         impl<T> Pair<T> { func get(self: *Self) -> T { (*self).value } \
                           func duplicate<U>(self: *Self, value: U) -> (T, U) { ((*self).value, value) } }",
    );
    let pair = named_type(&fixture, &hir, "Pair", DefKind::Struct);
    let impl_def = hir.inherent_impls()[0].def;
    let t = param_ty(&hir, param(&fixture, &hir, impl_def, "T"));
    assert_ne!(t, param_ty(&hir, param(&fixture, &hir, pair, "T")));
    let (method, body) = method(&fixture, &hir, impl_def, "duplicate");
    let u = param_ty(&hir, param(&fixture, &hir, method, "U"));
    let TyKind::Tuple(ret) = hir.ty_ctxt().kind(body.ret.expect("return type")) else {
        panic!("expected tuple");
    };
    assert_eq!(hir.ty_ctxt().tuple_elements(ret), [t, u]);
    let u_impl = hir
        .flattened_generics(impl_def)
        .into_iter()
        .find(|param| fixture.symbol(param.name) == "T")
        .expect("impl T");
    assert_eq!(u_impl.param, param(&fixture, &hir, impl_def, "T"));
    assert_eq!(hir.receiver(method), Some(ReceiverKind::Shared));
    assert_invariants(&fixture, &hir);
}

#[test]
fn method_parameters_are_distinct_and_do_not_leak() {
    let (fixture, hir) = lower_clean(
        "struct Box<T> { value: T } \
         impl<T> Box<T> { func a<U>(self: *Self, value: U) -> U { value } \
                          func b<U>(self: *Self, value: U) -> U { value } }",
    );
    let impl_def = hir.inherent_impls()[0].def;
    let (a, _) = method(&fixture, &hir, impl_def, "a");
    let (b, _) = method(&fixture, &hir, impl_def, "b");
    assert_ne!(
        param(&fixture, &hir, a, "U"),
        param(&fixture, &hir, b, "U"),
        "each method owns its parameter"
    );
    assert_invariants(&fixture, &hir);
}

#[test]
fn two_declarations_of_the_same_parameter_are_rejected() {
    let fixture = Fixture::new("struct Pair<T, T> { first: T }");
    let _ = fixture.lower();
    fixture.expect("E2051", "declared twice");

    let fixture =
        Fixture::new("struct Box<T> { value: T } impl<T> Box<T> { func f<T>(self: *Self) {} }");
    let _ = fixture.lower();
    fixture.expect("E2051", "declared twice");
}

#[test]
fn expression_arguments_are_checked_for_arity_and_genericity() {
    let fixture = Fixture::new(
        "struct Pair<T> { value: T } \
         func identity<T>(value: T) -> T { value } \
         func main() { let a = Pair::<i32, bool> { value: 1 }; let b = identity::<i32, bool>(1); }",
    );
    let _ = fixture.lower();
    fixture.expect("E2052", "expected 1 type argument(s)");
    fixture.expect("E2052", "expected 1 type argument(s)");

    let fixture = Fixture::new("func identity() -> i32 { 1 } func main() { identity::<i32>(); }");
    let _ = fixture.lower();
    fixture.expect("E2053", "not generic");
}

#[test]
fn type_annotations_reject_wrong_and_missing_arguments_at_the_path() {
    let fixture = Fixture::new(
        "struct Pair<T, U> { first: T, second: U } \
         func f(value: Pair<i32>) {} \
         struct Point { x: i32 } func g(value: Point<i32>) {}",
    );
    let _ = fixture.lower();
    fixture.expect("E2052", "expected 2 type argument(s)");
    fixture.expect("E2053", "not generic");
}

#[test]
fn a_parameter_outside_its_scope_and_an_unknown_bound_parameter_are_distinct() {
    let fixture = Fixture::new("struct Box<T> { value: T } func f(value: Box<T>) {}");
    let _ = fixture.lower();
    fixture.expect("E2055", "not in scope");

    let fixture = Fixture::new("trait Bound<T> {} func f<U: Bound<Missing>>(value: U) {}");
    let _ = fixture.lower();
    fixture.expect("E2054", "cannot find type parameter `Missing`");
}

#[test]
fn bounds_resolve_traits_arity_and_reject_non_traits() {
    let fixture =
        Fixture::new("struct Point { x: i32 } trait Trait<T> {} func f<U: Point>(value: U) {}");
    let _ = fixture.lower();
    fixture.expect("E2060", "is not a trait");

    let fixture = Fixture::new("trait Trait<T> {} func f<U: Trait>(value: U) {}");
    let _ = fixture.lower();
    fixture.expect("E2052", "expected 1 type argument(s)");

    let fixture = Fixture::new("func f<U: Missing>(value: U) {}");
    let _ = fixture.lower();
    fixture.expect("E2057", "cannot find trait `Missing`");
}

#[test]
fn copy_and_trait_bounds_merge_and_deduplicate() {
    let (fixture, hir) = lower_clean(
        "trait Show<T> {} struct Point<T> { value: T } \
         func f<T: Copy + Show<i32>>(value: T) -> Point<T> where T: Copy { Point { value: value } }",
    );
    let (definition, _) = function(&fixture, &hir, "f");
    let generics = hir.generics(definition).expect("function generics");
    let bounds = &generics.params[0].bounds;
    assert_eq!(
        bounds
            .iter()
            .filter(|bound| matches!(bound, Bound::Copy))
            .count(),
        1
    );
    assert_eq!(
        bounds.len(),
        2,
        "Copy and Show<i32>, without a duplicate Copy"
    );
    assert!(matches!(bounds[1], Bound::Trait(TraitRef { .. })));
    assert!(hir.param_env(definition).has_copy(generics.params[0].param));
    assert_invariants(&fixture, &hir);
}

#[test]
fn a_where_clause_on_a_concrete_type_is_rejected() {
    let fixture = Fixture::new("trait Show {} struct Point {} func f() where Point: Show {}");
    let _ = fixture.lower();
    fixture.expect("E2062", "only constrain a type parameter");
}

#[test]
fn self_resolves_in_traits_and_impls_but_not_free_functions() {
    let (fixture, hir) = lower_clean(
        "struct Pair<T> { value: T } trait Convert<T> { func convert(self: *Self) -> T; } \
         impl<T> Convert<T> for Pair<T> { func convert(self: *Pair<T>) -> T { (*self).value } } \
         impl<T> Pair<T> { func value(self: *Self) -> T { (*self).value } }",
    );
    let pair = named_type(&fixture, &hir, "Pair", DefKind::Struct);
    let trait_impl = &hir.trait_impls()[0];
    assert_eq!(adt_args(&hir, trait_impl.trait_ref.self_ty).len(), 1);
    assert_eq!(
        hir.receiver(hir.inherent_impls()[0].methods[0].1),
        Some(ReceiverKind::Shared)
    );
    assert_eq!(hir.impls_for(pair).len(), 2);
    assert_invariants(&fixture, &hir);

    let fixture = Fixture::new("func f(value: Self) {}");
    let _ = fixture.lower();
    fixture.expect("E2059", "trait or impl context");
}

#[test]
fn a_type_parameter_is_not_a_value() {
    let fixture = Fixture::new("struct Box<T> { value: T } func f<T>() -> T { T }");
    let _ = fixture.lower();
    fixture.expect("E2010", "type parameter, not a value");
}

#[test]
fn every_generic_owner_has_generics_and_every_parameter_has_an_owner() {
    let (fixture, hir) = lower_clean(
        "struct Pair<T> { value: T } enum Maybe<T> { Some(T), None } \
         trait Show<T> { func show(self: *Self) -> T; } \
         impl<T> Pair<T> { func get<U>(self: *Self, value: U) -> U { value } }",
    );
    for (definition, data) in hir.definitions().iter_enumerated() {
        if matches!(
            data.kind,
            DefKind::Struct
                | DefKind::Enum
                | DefKind::Trait
                | DefKind::TraitMethod
                | DefKind::Impl
                | DefKind::Function
        ) {
            assert!(
                hir.generics(definition).is_some(),
                "{:?} `{}` has no generics entry",
                data.kind,
                fixture.symbol(data.name)
            );
        }
        if data.kind == DefKind::GenericParam {
            assert!(data.parent.is_some());
        }
    }
    let pair = named_type(&fixture, &hir, "Pair", DefKind::Struct);
    for parameter in hir.flattened_generics(pair) {
        assert_eq!(hir.ty_ctxt().param_data(parameter.param).owner, pair);
    }
}

#[test]
fn expression_paths_carry_explicit_function_arguments_and_none_when_omitted() {
    let (fixture, hir) = lower_clean(
        "func identity<T>(value: T) -> T { value } \
         func main() -> i32 { let explicit = identity::<i32>(1); identity(explicit) }",
    );
    let (_, body) = function(&fixture, &hir, "main");
    let statements = hir.block(body.root).statements.clone();
    let init = match hir.stmt(statements[0]).kind.clone() {
        crate::StmtKind::Let { init, .. } => init.expect("initializer"),
        _ => panic!("expected let"),
    };
    let ExprKind::Call { callee, .. } = hir.expr(init).kind.clone() else {
        panic!("expected call");
    };
    assert_eq!(
        hir.ty_ctxt()
            .substs_len(hir.expr_args(callee).expect("explicit arguments")),
        1
    );
    let call = tail(&hir, body);
    let ExprKind::Call { callee, .. } = hir.expr(call).kind.clone() else {
        panic!("expected tail call");
    };
    assert_eq!(hir.expr_args(callee), None);
    assert!(matches!(path_res(&hir, callee), crate::Res::Function(_)));
    assert_invariants(&fixture, &hir);
}

#[test]
fn a_trait_impl_method_is_checked_after_instantiating_its_parameters() {
    let fixture = Fixture::new(
        "struct Pair<T> { value: T } trait Convert<T> { func convert(self: *Self, value: T) -> T; } \
         impl<T> Convert<T> for Pair<T> { func convert(self: *Pair<T>, value: i32) -> T { value } }",
    );
    let _ = fixture.lower();
    fixture.expect("E2024", "parameter 1 has type `i32`");
}

#[test]
fn a_generic_trait_method_signature_matches_its_implementation() {
    let (fixture, hir) = lower_clean(
        "struct Point { x: i32 } trait Map<U> { func map<V>(self: *Self, value: U) -> V; } \
         impl<U> Map<U> for Point { func map<V>(self: *Point, value: U) -> V { value } }",
    );
    let point = named_type(&fixture, &hir, "Point", DefKind::Struct);
    let trait_impl = &hir.trait_impls()[0];
    assert_eq!(
        trait_impl.trait_ref.self_ty,
        hir.ty_ctxt().adt_ty(hir.adt(point).unwrap())
    );
    assert_invariants(&fixture, &hir);
}

#[test]
fn a_generic_trait_method_with_the_wrong_arity_is_rejected() {
    let fixture = Fixture::new(
        "struct Point { x: i32 } trait Map { func map<U>(self: *Self) -> U; } \
         impl Map for Point { func map<U, V>(self: *Point) -> U { self.x } }",
    );
    let _ = fixture.lower();
    fixture.expect("E2024", "generic parameter");
}

#[test]
fn a_method_parameter_does_not_leak_into_a_sibling_method() {
    let fixture = Fixture::new(
        "struct Box<T> { value: T } \
         impl<T> Box<T> { func a<U>(self: *Self) {} func b(self: *Self) -> U { (*self).value } }",
    );
    let _ = fixture.lower();
    fixture.expect("E2055", "not in scope");
}

#[test]
fn a_blanket_impl_records_a_parameter_self_type() {
    let (fixture, hir) = lower_clean("trait Show {} impl<T> Show for T {}");
    let trait_impl = &hir.trait_impls()[0];
    assert!(matches!(
        hir.ty_ctxt().kind(trait_impl.trait_ref.self_ty),
        TyKind::Param(_)
    ));
    assert_eq!(trait_impl.generics.len(), 1);
    assert_invariants(&fixture, &hir);
}

#[test]
fn self_in_impl_trait_arguments_is_the_impl_self_type() {
    let (fixture, hir) = lower_clean(
        "struct Pair<T> { value: T } trait Convert<T> {} impl<T> Convert<Self> for Pair<T> {}",
    );
    let trait_impl = &hir.trait_impls()[0];
    let argument = hir.ty_ctxt().substs_args(trait_impl.trait_ref.args)[0];
    assert_eq!(argument, trait_impl.trait_ref.self_ty);
    assert_invariants(&fixture, &hir);
}

#[test]
fn impl_bounds_are_visible_in_method_environments() {
    let (fixture, hir) = lower_clean(
        "struct Box<T> { value: T } impl<T: Copy> Box<T> { func get(self: *Self) -> T { (*self).value } }",
    );
    let method = hir.inherent_impls()[0].methods[0].1;
    let parameter = hir
        .flattened_generics(method)
        .into_iter()
        .find(|parameter| fixture.symbol(parameter.name) == "T")
        .expect("impl T")
        .param;
    assert!(hir.param_env(method).has_copy(parameter));
    assert_invariants(&fixture, &hir);
}

#[test]
fn a_method_where_clause_can_constrain_an_impl_parameter() {
    let (fixture, hir) = lower_clean(
        "struct Box<T> { value: T } \
         impl<T> Box<T> { func copy(self: *Self) -> T where T: Copy { (*self).value } }",
    );
    let method = hir.inherent_impls()[0].methods[0].1;
    let parameter = hir
        .flattened_generics(method)
        .into_iter()
        .find(|parameter| fixture.symbol(parameter.name) == "T")
        .expect("impl T")
        .param;
    assert!(hir.param_env(method).has_copy(parameter));
    assert_invariants(&fixture, &hir);
}

#[test]
fn a_supertrait_is_recorded_on_the_trait() {
    let (fixture, hir) = lower_clean("trait Base {} trait Child where Self: Base {}");
    let child = named_type(&fixture, &hir, "Child", DefKind::Trait);
    let declaration = hir.trait_definition(child).expect("semantic trait");
    assert_eq!(declaration.supertraits.len(), 1);
    assert_eq!(
        declaration.supertraits[0].trait_def,
        named_type(&fixture, &hir, "Base", DefKind::Trait)
    );
    assert_invariants(&fixture, &hir);
}

#[test]
fn self_in_an_inherent_method_return_is_the_impl_type() {
    let (fixture, hir) = lower_clean(
        "struct Box<T> { value: T } impl<T> Box<T> { func same(self: *Self) -> Self { *self } }",
    );
    let method = hir.inherent_impls()[0].methods[0].1;
    let body = hir.body_of(method).expect("body");
    assert_eq!(body.ret, Some(hir.inherent_impls()[0].self_ty));
    assert_invariants(&fixture, &hir);
}

#[test]
fn a_type_parameter_may_shadow_a_nominal_type_name() {
    let (fixture, hir) = lower_clean("struct T { value: i32 } func f<T>(value: T) -> T { value }");
    let (definition, body) = function(&fixture, &hir, "f");
    assert!(matches!(
        hir.ty_ctxt().kind(body.params[0].ty),
        TyKind::Param(_)
    ));
    assert_eq!(
        hir.ty_ctxt()
            .param_data(match hir.ty_ctxt().kind(body.params[0].ty) {
                TyKind::Param(param) => param,
                _ => unreachable!(),
            })
            .owner,
        definition
    );
    assert_invariants(&fixture, &hir);
}

#[test]
fn an_inherent_impl_rejects_structural_targets_while_a_trait_impl_accepts_them() {
    let fixture = Fixture::new("trait Show {} impl Show for i32 {} impl i32 { func f() {} }");
    let _ = fixture.lower();
    fixture.expect("E2056", "not a nominal type");
}

#[test]
fn a_generic_impl_can_target_a_non_nominal_sized_type_for_a_trait() {
    let (fixture, hir) =
        lower_clean("trait Show {} impl Show for (i32, bool) {} impl Show for [i32; 2] {}");
    let trait_impls = hir.trait_impls();
    assert_eq!(trait_impls.len(), 2);
    assert!(matches!(
        hir.ty_ctxt().kind(trait_impls[0].trait_ref.self_ty),
        TyKind::Tuple(_)
    ));
    assert!(matches!(
        hir.ty_ctxt().kind(trait_impls[1].trait_ref.self_ty),
        TyKind::Array { .. }
    ));
    assert_invariants(&fixture, &hir);
}

#[test]
fn explicit_arguments_on_a_non_generic_type_are_rejected() {
    let fixture = Fixture::new("struct Point { x: i32 } func f(value: Point<i32>) {}");
    let _ = fixture.lower();
    fixture.expect("E2053", "not generic");
}

#[test]
fn where_bounds_on_parameters_are_recorded_in_the_environment() {
    let (fixture, hir) = lower_clean("trait Show {} func f<T>(value: T) where T: Show + Copy {}");
    let (definition, _) = function(&fixture, &hir, "f");
    let parameter = param(&fixture, &hir, definition, "T");
    let env = hir.param_env(definition);
    assert!(env.has_copy(parameter));
    assert!(
        env.bounds(parameter)
            .iter()
            .any(|bound| matches!(bound, Bound::Trait(_)))
    );
    assert_invariants(&fixture, &hir);
}

#[test]
fn duplicate_type_arguments_on_a_trait_path_use_the_argument_span() {
    let fixture = Fixture::new("trait Show<T> {} func f<U: Show<i32, bool>>(value: U) {}");
    let _ = fixture.lower();
    fixture.expect("E2052", "expected 1 type argument(s)");
}

#[test]
fn generic_arguments_on_a_non_generic_trait_are_rejected() {
    let fixture = Fixture::new("trait Show {} func f<U: Show<i32>>(value: U) {}");
    let _ = fixture.lower();
    fixture.expect("E2053", "not generic");
}

#[test]
fn a_substs_id_is_shared_by_equal_argument_lists() {
    let (fixture, hir) = lower_clean(
        "struct Pair<T, U> { first: T, second: U } \
         func f(a: Pair<i32, bool>, b: Pair<i32, bool>) {}",
    );
    let (_, body) = function(&fixture, &hir, "f");
    let first = match hir.ty_ctxt().kind(body.params[0].ty) {
        TyKind::Adt { args, .. } => args,
        _ => panic!("expected Pair"),
    };
    let second = match hir.ty_ctxt().kind(body.params[1].ty) {
        TyKind::Adt { args, .. } => args,
        _ => panic!("expected Pair"),
    };
    assert_eq!(first, second);
    assert!(hir.ty_ctxt().substs_len_total() >= 1);
    assert_invariants(&fixture, &hir);
}
