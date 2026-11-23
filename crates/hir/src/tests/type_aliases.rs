//! Transparent type aliases: declaration, expansion, paths, and recovery.

use seal_def::{DefId, DefKind};
use seal_diagnostics::Level;
use seal_ty::{IntTy, TyKind};

use super::support::{Fixture, function, lower_clean, tail};
use crate::{ExprKind, Hir, PatternKind, StmtKind};

fn alias_def(fixture: &Fixture, hir: &Hir, name: &str) -> DefId {
    hir.definitions()
        .iter_enumerated()
        .find(|(_, data)| data.kind == DefKind::TypeAlias && fixture.symbol(data.name) == name)
        .map(|(id, _)| id)
        .unwrap_or_else(|| panic!("no type alias named `{name}`"))
}

#[test]
fn a_primitive_alias_has_the_target_identity() {
    let (fixture, hir) = lower_clean("type Id = i32; func f(value: Id) -> i32 { value }");
    let alias = alias_def(&fixture, &hir, "Id");
    let target = hir
        .type_alias_target(alias)
        .expect("the alias target was lowered");
    let (_, body) = function(&fixture, &hir, "f");
    assert_eq!(body.params[0].ty, target);
    assert_eq!(body.ret.expect("return type"), target);
    assert_eq!(hir.ty_ctxt().kind(target), TyKind::Int(IntTy::I32));
}

#[test]
fn an_alias_can_be_declared_after_its_use() {
    let (fixture, hir) = lower_clean("func f(value: Later) -> i32 { value } type Later = i32;");
    let alias = alias_def(&fixture, &hir, "Later");
    let target = hir.type_alias_target(alias).expect("lowered");
    let (_, body) = function(&fixture, &hir, "f");
    assert_eq!(body.params[0].ty, target);
}

#[test]
fn aliases_chain_without_adding_a_type_layer() {
    let (fixture, hir) = lower_clean("type A = B; type B = i32; func f(value: A) -> i32 { value }");
    let a = alias_def(&fixture, &hir, "A");
    let b = alias_def(&fixture, &hir, "B");
    assert_eq!(hir.type_alias_target(a), hir.type_alias_target(b));
    assert_eq!(
        hir.ty_ctxt()
            .kind(hir.type_alias_target(a).expect("target")),
        TyKind::Int(IntTy::I32)
    );
}

#[test]
fn tuple_aliases_expand_inside_annotations_and_fields() {
    let (fixture, hir) = lower_clean(
        "type Pair = (i32, bool);\n\
         struct Holder { value: Pair }\n\
         func f(value: Pair) -> (i32, bool) { value }",
    );
    let alias = alias_def(&fixture, &hir, "Pair");
    let target = hir.type_alias_target(alias).expect("target");
    let (_, body) = function(&fixture, &hir, "f");
    assert_eq!(body.params[0].ty, target);
    assert_eq!(body.ret.expect("return type"), target);
}

#[test]
fn generic_aliases_substitute_their_parameters() {
    let (fixture, hir) =
        lower_clean("type Pair<T> = (T, T); func f(value: Pair<i32>) -> (i32, i32) { value }");
    let alias = alias_def(&fixture, &hir, "Pair");
    let target = hir.type_alias_target(alias).expect("target");
    assert!(
        matches!(hir.ty_ctxt().kind(target), TyKind::Tuple(_)),
        "the unsubstituted target is a tuple"
    );
    let (_, body) = function(&fixture, &hir, "f");
    assert_eq!(
        hir.ty_ctxt().kind(body.params[0].ty),
        hir.ty_ctxt().kind(body.ret.expect("return type"))
    );
}

#[test]
fn aliases_compose_with_nested_generic_arguments() {
    let (fixture, hir) = lower_clean(
        "type Pair<T> = (T, T);\n\
         type Nested<T> = Pair<Pair<T>>;\n\
         func f(value: Nested<i32>) -> (i32, i32) { value.0 }",
    );
    let aliases = ["Pair", "Nested"];
    for name in aliases {
        assert!(
            hir.type_alias_target(alias_def(&fixture, &hir, name))
                .is_some()
        );
    }
}

#[test]
fn an_alias_can_name_a_struct_constructor() {
    let (fixture, hir) = lower_clean(
        "struct Point { x: i32, y: i32 }\n\
         type P = Point;\n\
         func main() -> i32 { let point = P { x: 1, y: 2 }; point.x }",
    );
    let (_, body) = function(&fixture, &hir, "main");
    let StmtKind::Let { init, .. } = &hir.stmt(hir.block(body.root).statements[0]).kind else {
        panic!("expected a let");
    };
    let ExprKind::Struct { adt, .. } = &hir.expr(init.expect("initializer")).kind else {
        panic!("expected a struct literal");
    };
    assert_eq!(fixture.symbol(hir.definitions().name(*adt)), "Point");
}

#[test]
fn an_alias_can_name_a_struct_pattern() {
    let (fixture, hir) = lower_clean(
        "struct Point { x: i32, y: i32 }\n\
         type P = Point;\n\
         func main(point: P) -> i32 { match point { P { x, y } => x + y } }",
    );
    let (_, body) = function(&fixture, &hir, "main");
    let ExprKind::Match { arms, .. } = &hir.expr(tail(&hir, body)).kind else {
        panic!("expected a match expression");
    };
    let PatternKind::Struct { adt, .. } = &hir.pattern(arms[0].pattern).kind else {
        panic!("expected a struct pattern");
    };
    assert_eq!(fixture.symbol(hir.definitions().name(*adt)), "Point");
}

#[test]
fn an_alias_can_name_an_enum_variant_and_method_path() {
    let (fixture, hir) = lower_clean(
        "enum Maybe { Some(i32), None }\n\
         type M = Maybe;\n\
         func main() -> i32 { let value = M::Some(1); match value { M::Some(x) => x, M::None => 0 } }",
    );
    let (_, body) = function(&fixture, &hir, "main");
    let StmtKind::Let { init, .. } = &hir.stmt(hir.block(body.root).statements[0]).kind else {
        panic!("expected a let");
    };
    let ExprKind::Variant { adt, .. } = &hir.expr(init.expect("initializer")).kind else {
        panic!("expected a variant call");
    };
    assert_eq!(fixture.symbol(hir.definitions().name(*adt)), "Maybe");
}

#[test]
fn an_alias_can_be_the_target_of_an_inherent_impl() {
    let (fixture, hir) = lower_clean(
        "struct Point { x: i32 }\n\
         type P = Point;\n\
         impl P { func origin() -> P { P { x: 0 } } }\n\
         func main() -> i32 { let point = P::origin(); point.x }",
    );
    let (_, body) = function(&fixture, &hir, "main");
    let _ = tail(&hir, body);
    assert!(fixture.diagnostics().is_empty());
}

#[test]
fn aliases_expand_in_every_type_constructor() {
    let (fixture, hir) = lower_clean(
        "trait Show {}\n\
         type R = *i32;\n\
         type W = *mut i32;\n\
         type P = *raw i32;\n\
         type A = [i32; 3];\n\
         type S = [i32];\n\
         type F = func(i32) -> i32;\n\
         type D = *dyn Show;\n\
         type U = ();\n\
         func main() {}",
    );
    let target = |name: &str| {
        hir.type_alias_target(alias_def(&fixture, &hir, name))
            .expect("lowered target")
    };
    assert!(matches!(
        hir.ty_ctxt().kind(target("R")),
        TyKind::Ref { .. }
    ));
    assert!(matches!(
        hir.ty_ctxt().kind(target("W")),
        TyKind::Ref { .. }
    ));
    assert!(matches!(
        hir.ty_ctxt().kind(target("P")),
        TyKind::RawPtr { .. }
    ));
    assert!(matches!(
        hir.ty_ctxt().kind(target("A")),
        TyKind::Array { length: 3, .. }
    ));
    assert!(matches!(
        hir.ty_ctxt().kind(target("S")),
        TyKind::Slice { .. }
    ));
    assert!(matches!(hir.ty_ctxt().kind(target("F")), TyKind::FnPtr(_)));
    assert!(matches!(
        hir.ty_ctxt().kind(target("D")),
        TyKind::Ref { .. }
    ));
    assert!(matches!(hir.ty_ctxt().kind(target("U")), TyKind::Unit));
}

#[test]
fn a_generic_alias_names_a_generic_constructor_through_the_alias_chain() {
    let (fixture, hir) = lower_clean(
        "struct Pair<T> { first: T, second: T }\n\
         type Alias<T> = Pair<T>;\n\
         type Concrete = Alias<i32>;\n\
         func main() -> i32 {\n\
           let explicit = Concrete { first: 1, second: 2 };\n\
           let inferred = Alias { first: 3, second: 4 };\n\
           explicit.first + inferred.second\n\
         }",
    );
    let (_, body) = function(&fixture, &hir, "main");
    let _ = tail(&hir, body);
    assert!(
        fixture.diagnostics().is_empty(),
        "{:?}",
        fixture.diagnostics()
    );
}

#[test]
fn an_alias_can_name_the_target_of_a_trait_impl() {
    let (fixture, hir) = lower_clean(
        "trait Show { func show(self: *Self) -> i32; }\n\
         struct Point { x: i32 }\n\
         type P = Point;\n\
         impl Show for P { func show(self: *P) -> i32 { self.x } }\n\
         func main() -> i32 { let point = P { x: 1 }; point.show() }",
    );
    let (_, body) = function(&fixture, &hir, "main");
    let _ = tail(&hir, body);
    assert!(
        fixture.diagnostics().is_empty(),
        "{:?}",
        fixture.diagnostics()
    );
}

#[test]
fn self_is_not_available_in_a_file_scope_alias() {
    let fixture = Fixture::new("type A = Self; func main() {}");
    let _ = fixture.lower();
    fixture.expect("E2059", "only allowed in a trait or impl context");
}

#[test]
fn a_duplicate_alias_is_rejected() {
    let fixture = Fixture::new("type A = i32; type A = u32; func main() {}");
    let _ = fixture.lower();
    fixture.expect("E2005", "defined twice");
}

#[test]
fn an_alias_cannot_share_the_type_namespace_with_a_struct() {
    let fixture = Fixture::new("struct A { x: i32 } type A = i32; func main() {}");
    let _ = fixture.lower();
    fixture.expect("E2005", "defined twice");
}

#[test]
fn an_alias_cannot_share_the_type_namespace_with_a_trait() {
    let fixture = Fixture::new("type A = i32; trait A {} func main() {}");
    let _ = fixture.lower();
    fixture.expect("E2005", "defined twice");
}

#[test]
fn an_unknown_alias_target_is_reported_once() {
    let fixture = Fixture::new("type A = Missing; func f(value: A) {}");
    let _ = fixture.lower();
    fixture.expect("E2002", "cannot find type");
}

#[test]
fn a_missing_generic_argument_list_is_reported() {
    let fixture = Fixture::new("type Pair<T> = (T, T); func f(value: Pair) {}");
    let _ = fixture.lower();
    fixture.expect("E2052", "expected 1 type argument");
}

#[test]
fn arguments_on_a_non_generic_alias_are_reported() {
    let fixture = Fixture::new("type Id = i32; func f(value: Id<u8>) {}");
    let _ = fixture.lower();
    fixture.expect("E2053", "not generic");
}

#[test]
fn a_direct_alias_cycle_is_rejected() {
    let fixture = Fixture::new("type A = A; func main() {}");
    let _ = fixture.lower();
    fixture.expect("E2095", "expands recursively");
}

#[test]
fn a_mutual_alias_cycle_is_rejected() {
    let fixture = Fixture::new("type A = B; type B = A; func main() {}");
    let _ = fixture.lower();
    fixture.expect("E2095", "expands recursively");
}

#[test]
fn bounds_on_an_alias_are_rejected() {
    let fixture = Fixture::new("type A<T: Copy> = T; func main() {}");
    let _ = fixture.lower();
    fixture.expect("E2096", "cannot constrain");

    let fixture = Fixture::new("type A<T> where T: Copy = T; func main() {}");
    let _ = fixture.lower();
    fixture.expect("E2096", "cannot constrain");
}

#[test]
fn a_public_alias_to_a_private_target_is_rejected() {
    let fixture = Fixture::package(&[(
        "app.sl",
        "module app;\n\
         internal struct Secret { internal value: i32 }\n\
         public type Leak = Secret;",
    )]);
    let _ = fixture.lower_package_exposing(&["app"]);
    fixture.expect("E3107", "exposes the non-public type `Secret`");
}

#[test]
fn a_public_alias_to_a_public_target_is_part_of_the_api() {
    let fixture = Fixture::package(&[("app.sl", "module app;\npublic type Id = i32;")]);
    let hir = fixture.lower_package_exposing(&["app"]);
    fixture.assert_clean();
    let alias = alias_def(&fixture, &hir, "Id");
    assert!(hir.is_public_api(alias));
    assert!(hir.public_items().contains(&alias));
    assert!(hir.type_alias_target(alias).is_some());
}

#[test]
fn an_imported_alias_resolves_through_the_type_namespace() {
    let fixture = Fixture::package(&[
        ("main.sl", "use api::Id; func f(value: Id) -> i32 { value }"),
        ("api.sl", "module api;\npublic type Id = i32;"),
    ]);
    let hir = fixture.lower_package_exposing(&["api", "root"]);
    fixture.assert_clean();
    let alias = alias_def(&fixture, &hir, "Id");
    let target = hir.type_alias_target(alias).expect("lowered");
    let (_, body) = function(&fixture, &hir, "f");
    assert_eq!(body.params[0].ty, target);
}

#[test]
fn a_cfg_disabled_alias_does_not_exist() {
    let fixture = Fixture::new("cfg(test) type Hidden = i32; func f(value: Hidden) {}");
    let _ = fixture.lower();
    fixture.expect("E2002", "cannot find type");
}

#[test]
fn a_deprecated_alias_warns_at_its_use() {
    let fixture = Fixture::new(
        "deprecated(since: \"0.2.0\", replacement: \"Current\")\n\
         type Old = i32;\n\
         func f(value: Old) {}",
    );
    let _ = fixture.lower();
    let warnings: Vec<_> = fixture
        .diagnostics()
        .into_iter()
        .filter(|diagnostic| {
            diagnostic.level() == Level::Warning
                && diagnostic.lint() == Some(seal_diagnostics::DEPRECATED)
        })
        .collect();
    assert!(
        warnings
            .iter()
            .any(|warning| warning.message().contains("`Old`")),
        "{warnings:?}"
    );
}
