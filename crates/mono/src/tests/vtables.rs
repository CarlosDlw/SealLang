//! Trait-object vtable collection during monomorphization.

use seal_index::Idx;
use seal_mir::{Rvalue, StatementKind};
use seal_ty::{IntTy, TraitRef};

use crate::collector::Limits;
use crate::tests::support::{Fixture, collect_clean, names};
use crate::{MonoProgram, VtableId, VtableKey, instance_name, vtable_hash, vtable_name};

const SHAPE: &str = "\
struct Circle { radius: i32 } \
trait Shape { func area(self: *Self) -> i32; } \
impl Shape for Circle { func area(self: *Circle) -> i32 { (*self).radius } }";

const TWO_TYPES: &str = "\
struct Circle { radius: i32 } \
struct Square { side: i32 } \
trait Shape { func area(self: *Self) -> i32; } \
impl Shape for Circle { func area(self: *Circle) -> i32 { (*self).radius } } \
impl Shape for Square { func area(self: *Square) -> i32 { (*self).side } } \
func both(first: *Circle, second: *Square) { \
    let a: *dyn Shape = first; \
    let b: *dyn Shape = second; \
}";

fn instance_named<'a>(
    fixture: &Fixture,
    hir: &seal_hir::Hir,
    program: &'a MonoProgram,
    name: &str,
) -> &'a crate::MonoMir {
    program
        .instances()
        .iter()
        .find(|instance| instance_name(hir, fixture.interner(), instance.instance) == name)
        .unwrap_or_else(|| {
            panic!(
                "no instance named `{name}`, have {:?}",
                names(fixture, hir, program)
            )
        })
}

fn rvalue(body: &seal_mir::Body, predicate: impl Fn(&Rvalue) -> bool) -> &Rvalue {
    body.blocks
        .iter()
        .flat_map(|block| &block.statements)
        .find_map(|statement| {
            let StatementKind::Assign { rvalue, .. } = &statement.kind else {
                return None;
            };
            predicate(rvalue).then_some(rvalue)
        })
        .expect("the body contains the expected rvalue")
}

fn entry_names(fixture: &Fixture, hir: &seal_hir::Hir, program: &MonoProgram) -> Vec<Vec<String>> {
    program
        .vtables()
        .iter()
        .map(|table| {
            table
                .entries
                .iter()
                .map(|entry| instance_name(hir, fixture.interner(), *entry))
                .collect()
        })
        .collect()
}

#[test]
fn one_concrete_type_and_trait_produce_one_vtable() {
    let source = format!("{SHAPE} func upcast(circle: *Circle) -> *dyn Shape {{ circle }}");
    let (fixture, hir, _mir, program) = collect_clean(&source);
    assert_eq!(program.vtables().len(), 1);
    let table = program.vtable(VtableId::from_usize(0));
    assert_eq!(
        vtable_name(&hir, fixture.interner(), &table.key),
        "Circle as Shape"
    );
}

#[test]
fn two_concrete_types_produce_two_vtables() {
    let (fixture, hir, _mir, program) = collect_clean(TWO_TYPES);
    assert_eq!(program.vtables().len(), 2);
    assert_eq!(
        entry_names(&fixture, &hir, &program),
        [vec!["area".to_owned()], vec!["area".to_owned()]]
    );
}

#[test]
fn two_traits_for_one_concrete_type_produce_two_vtables() {
    let source = "\
        struct Circle { radius: i32 } \
        trait Shape { func area(self: *Self) -> i32; } \
        trait Named { func name(self: *Self) -> i32; } \
        impl Shape for Circle { func area(self: *Circle) -> i32 { (*self).radius } } \
        impl Named for Circle { func name(self: *Circle) -> i32 { (*self).radius } } \
        func shape(value: *Circle) -> *dyn Shape { value } \
        func named(value: *Circle) -> *dyn Named { value }";
    let (_fixture, _hir, _mir, program) = collect_clean(source);
    assert_eq!(program.vtables().len(), 2);
    assert_ne!(
        program
            .vtable(VtableId::from_usize(0))
            .key
            .trait_ref
            .trait_def,
        program
            .vtable(VtableId::from_usize(1))
            .key
            .trait_ref
            .trait_def
    );
}

#[test]
fn shared_and_exclusive_coercions_share_one_vtable() {
    let source = "\
        struct Circle { radius: i32 } \
        trait Shape { \
            func area(self: *Self) -> i32; \
            func set(self: *mut Self, value: i32); \
        } \
        impl Shape for Circle { \
            func area(self: *Circle) -> i32 { (*self).radius } \
            func set(self: *mut Circle, value: i32) { (*self).radius = value; } \
        } \
        func shared(value: *Circle) -> *dyn Shape { value } \
        func exclusive(value: *mut Circle) -> *mut dyn Shape { value }";
    let (_fixture, _hir, _mir, program) = collect_clean(source);
    assert_eq!(program.vtables().len(), 1);
    assert_eq!(program.vtable(VtableId::from_usize(0)).entries.len(), 2);
}

#[test]
fn repeated_coercions_reuse_one_vtable() {
    let source = format!(
        "{SHAPE} func upcast(first: *Circle, second: *Circle) {{ \
             let a: *dyn Shape = first; \
             let b: *dyn Shape = second; \
         }}"
    );
    let (_fixture, _hir, _mir, program) = collect_clean(&source);
    assert_eq!(program.vtables().len(), 1);
}

#[test]
fn generic_trait_arguments_produce_distinct_vtables() {
    let source = "\
        struct Meter { value: i32 } \
        trait Convert<T> { func convert(self: *Self, value: T) -> T; } \
        impl Convert<i32> for Meter { \
            func convert(self: *Meter, value: i32) -> i32 { value } \
        } \
        impl Convert<bool> for Meter { \
            func convert(self: *Meter, value: bool) -> bool { value } \
        } \
        func as_int(value: *Meter) -> *dyn Convert<i32> { value } \
        func as_bool(value: *Meter) -> *dyn Convert<bool> { value }";
    let (fixture, hir, _mir, program) = collect_clean(source);
    assert_eq!(program.vtables().len(), 2);
    let int = program.vtable(VtableId::from_usize(0));
    let boolean = program.vtable(VtableId::from_usize(1));
    assert_eq!(trait_arg_names(&fixture, &hir, &int.key), ["i32"]);
    assert_eq!(trait_arg_names(&fixture, &hir, &boolean.key), ["bool"]);
}

fn trait_arg_names(fixture: &Fixture, hir: &seal_hir::Hir, key: &VtableKey) -> Vec<String> {
    hir.ty_ctxt()
        .substs_args(key.trait_ref.args)
        .iter()
        .map(|argument| {
            hir.ty_ctxt()
                .display_with(*argument, fixture.interner())
                .to_string()
        })
        .collect()
}

#[test]
fn vtable_entries_follow_the_layout_order() {
    let source = "\
        struct Circle { radius: i32 } \
        trait Shape { \
            func area(self: *Self) -> i32; \
            func scale(self: *mut Self, factor: i32); \
            func name(self: *Self) -> i32; \
        } \
        impl Shape for Circle { \
            func area(self: *Circle) -> i32 { (*self).radius } \
            func scale(self: *mut Circle, factor: i32) { (*self).radius = factor; } \
            func name(self: *Circle) -> i32 { 1 } \
        } \
        func upcast(value: *Circle) -> *dyn Shape { value }";
    let (fixture, hir, _mir, program) = collect_clean(source);
    let names = entry_names(&fixture, &hir, &program);
    assert_eq!(
        names,
        [vec![
            "area".to_owned(),
            "scale".to_owned(),
            "name".to_owned()
        ]]
    );
}

#[test]
fn supertrait_methods_are_collected_without_direct_calls() {
    let source = "\
        struct Point { x: i32 } \
        trait Root { func root(self: *Self) -> i32; } \
        trait Leaf: Root { func leaf(self: *Self) -> i32; } \
        impl Root for Point { func root(self: *Point) -> i32 { (*self).x } } \
        impl Leaf for Point { func leaf(self: *Point) -> i32 { (*self).x } } \
        func upcast(value: *Point) -> *dyn Leaf { value }";
    let (fixture, hir, _mir, program) = collect_clean(source);
    assert_eq!(program.vtables().len(), 1);
    assert_eq!(
        entry_names(&fixture, &hir, &program),
        [vec!["leaf".to_owned(), "root".to_owned()]]
    );
    assert!(program.instances().iter().all(|instance| {
        !instance
            .body
            .blocks
            .iter()
            .flat_map(|block| &block.statements)
            .any(|statement| {
                matches!(
                    statement.kind,
                    StatementKind::Assign {
                        rvalue: Rvalue::DynamicCall { .. },
                        ..
                    }
                )
            })
    }));
}

#[test]
fn supertrait_entry_uses_the_declaring_trait_instance() {
    let source = "\
        struct Mark { value: i32 } \
        trait Root<T> { func root(self: *Self, value: T) -> T; } \
        trait Leaf: Root<i32> { func leaf(self: *Self) -> i32; } \
        impl Root<i32> for Mark { func root(self: *Mark, value: i32) -> i32 { value } } \
        impl Leaf for Mark { func leaf(self: *Mark) -> i32 { (*self).value } } \
        func upcast(value: *Mark) -> *dyn Leaf { value }";
    let (fixture, hir, _mir, program) = collect_clean(source);
    assert_eq!(program.vtables().len(), 1);
    assert_eq!(
        entry_names(&fixture, &hir, &program),
        [vec!["leaf".to_owned(), "root".to_owned()]]
    );
}

#[test]
fn diamond_supertraits_do_not_duplicate_instances() {
    let source = "\
        struct Base { value: i32 } \
        trait Root { func root(self: *Self) -> i32; } \
        trait Left: Root { func left(self: *Self) -> i32; } \
        trait Right: Root { func right(self: *Self) -> i32; } \
        trait Leaf: Left + Right { func leaf(self: *Self) -> i32; } \
        impl Root for Base { func root(self: *Base) -> i32 { (*self).value } } \
        impl Left for Base { func left(self: *Base) -> i32 { 1 } } \
        impl Right for Base { func right(self: *Base) -> i32 { 2 } } \
        impl Leaf for Base { func leaf(self: *Base) -> i32 { 3 } } \
        func upcast(value: *Base) -> *dyn Leaf { value }";
    let (fixture, hir, _mir, program) = collect_clean(source);
    assert_eq!(program.vtables().len(), 1);
    assert_eq!(
        entry_names(&fixture, &hir, &program),
        [vec![
            "leaf".to_owned(),
            "left".to_owned(),
            "root".to_owned(),
            "right".to_owned()
        ]]
    );
    assert_eq!(
        names(&fixture, &hir, &program)
            .iter()
            .filter(|name| name.as_str() == "root")
            .count(),
        1
    );
}

#[test]
fn mono_substitutes_dynamic_call_arguments() {
    let source = "\
        struct Meter { value: i32 } \
        trait Convert<T> { func convert(self: *Self, value: T) -> T; } \
        impl Convert<i32> for Meter { \
            func convert(self: *Meter, value: i32) -> i32 { value } \
        } \
        func use<U>(object: *dyn Convert<U>, value: U) -> U { object.convert(value) } \
        func main() -> i32 { \
            let meter = Meter { value: 1 }; \
            let object: *dyn Convert<i32> = &meter; \
            use(object, 2) \
        }";
    let (fixture, hir, _mir, program) = collect_clean(source);
    let use_i32 = instance_named(&fixture, &hir, &program, "use<i32>");
    let Rvalue::DynamicCall {
        object_trait_ref,
        arg_tys,
        ret_ty,
        ..
    } = rvalue(&use_i32.body, |rvalue| {
        matches!(rvalue, Rvalue::DynamicCall { .. })
    })
    else {
        unreachable!();
    };
    assert_eq!(
        hir.ty_ctxt()
            .display_with(*ret_ty, fixture.interner())
            .to_string(),
        "i32"
    );
    assert_eq!(arg_tys.len(), 1);
    assert_eq!(
        hir.ty_ctxt()
            .display_with(arg_tys[0], fixture.interner())
            .to_string(),
        "i32"
    );
    assert_eq!(
        hir.ty_ctxt().substs_args(object_trait_ref.args),
        [hir.ty_ctxt().int_ty(IntTy::I32)]
    );
}

#[test]
fn mono_substitutes_make_trait_object_types() {
    let source = format!(
        "{SHAPE} func upcast<T: Shape>(value: *T) -> *dyn Shape {{ value }} \
         func main() {{ let circle = Circle {{ radius: 1 }}; upcast(&circle); }}"
    );
    let (fixture, hir, _mir, program) = collect_clean(&source);
    let upcast = instance_named(&fixture, &hir, &program, "upcast<Circle>");
    let Rvalue::MakeTraitObject {
        source_ty,
        target_ty,
        ..
    } = rvalue(&upcast.body, |rvalue| {
        matches!(rvalue, Rvalue::MakeTraitObject { .. })
    })
    else {
        unreachable!();
    };
    assert_eq!(
        hir.ty_ctxt()
            .display_with(*source_ty, fixture.interner())
            .to_string(),
        "*Circle"
    );
    assert_eq!(
        hir.ty_ctxt()
            .display_with(*target_ty, fixture.interner())
            .to_string(),
        "*dyn Shape"
    );
}

#[test]
fn vtable_ids_are_stable_across_collections() {
    let (first_fixture, first_hir, _mir, first) = collect_clean(TWO_TYPES);
    let (second_fixture, second_hir, _mir, second) = collect_clean(TWO_TYPES);
    assert_eq!(first.vtables().len(), second.vtables().len());
    for index in 0..first.vtables().len() {
        let id = VtableId::from_usize(index);
        let key = first.vtable(id).key;
        assert_eq!(second.vtable_id(key), Some(id));
        assert_eq!(
            vtable_name(&first_hir, first_fixture.interner(), &key),
            vtable_name(&second_hir, second_fixture.interner(), &key)
        );
    }
}

#[test]
fn vtable_fingerprints_are_stable_across_collections() {
    let (first_fixture, first_hir, _mir, first) = collect_clean(TWO_TYPES);
    let (second_fixture, second_hir, _mir, second) = collect_clean(TWO_TYPES);
    for index in 0..first.vtables().len() {
        let id = VtableId::from_usize(index);
        assert_eq!(
            vtable_hash(&first_hir, &first.vtable(id).key),
            vtable_hash(&second_hir, &second.vtable(id).key)
        );
    }
    let _ = (&first_fixture, &second_fixture);
}

#[test]
fn a_vtable_entry_uses_the_default_when_the_impl_omits_the_method() {
    let source = "\
        struct Circle { radius: i32 } \
        trait Shape { func area(self: *Self) -> i32 { 7 } } \
        impl Shape for Circle {} \
        func upcast(circle: *Circle) -> *dyn Shape { circle }";
    let (fixture, hir, _mir, program) = collect_clean(source);
    assert_eq!(program.vtables().len(), 1);
    let table = program.vtable(VtableId::from_usize(0));
    assert_eq!(
        entry_names(&fixture, &hir, &program),
        [vec!["area<Circle>".to_owned()]]
    );
    assert_eq!(table.entries[0].def, {
        hir.definitions()
            .iter_enumerated()
            .find(|(_, data)| fixture.symbol(data.name) == "area")
            .map(|(id, _)| id)
            .expect("area exists")
    });
    assert!(table.drop.is_none());
    program.validate(&hir).expect("the default vtable is valid");
}

#[test]
fn a_vtable_mixes_overrides_and_defaults_in_layout_order() {
    let source = "\
        struct Circle { radius: i32 } \
        trait Shape { \
            func area(self: *Self) -> i32; \
            func name(self: *Self) -> i32 { 9 } \
        } \
        impl Shape for Circle { func area(self: *Circle) -> i32 { (*self).radius } } \
        func upcast(circle: *Circle) -> *dyn Shape { circle }";
    let (fixture, hir, _mir, program) = collect_clean(source);
    assert_eq!(
        entry_names(&fixture, &hir, &program),
        [vec!["area".to_owned(), "name<Circle>".to_owned()]]
    );
    let table = program.vtable(VtableId::from_usize(0));
    assert_eq!(
        hir.definitions().kind(table.entries[0].def),
        seal_def::DefKind::Function
    );
    assert_eq!(
        hir.definitions().kind(table.entries[1].def),
        seal_def::DefKind::TraitMethod
    );
    program.validate(&hir).expect("the mixed vtable is valid");
}

#[test]
fn a_supertrait_default_fills_its_vtable_slot() {
    let source = "\
        struct Point { x: i32 } \
        trait Root { func root(self: *Self) -> i32 { 5 } } \
        trait Leaf: Root { func leaf(self: *Self) -> i32; } \
        impl Root for Point {} \
        impl Leaf for Point { func leaf(self: *Point) -> i32 { (*self).x } } \
        func upcast(point: *Point) -> *dyn Leaf { point }";
    let (fixture, hir, _mir, program) = collect_clean(source);
    assert_eq!(
        entry_names(&fixture, &hir, &program),
        [vec!["leaf".to_owned(), "root<Point>".to_owned()]]
    );
    let table = program.vtable(VtableId::from_usize(0));
    assert_eq!(
        hir.definitions().kind(table.entries[1].def),
        seal_def::DefKind::TraitMethod
    );
    program
        .validate(&hir)
        .expect("the supertrait default is valid");
}

#[test]
fn a_generic_default_vtable_entry_carries_the_trait_arguments() {
    let source = "\
        struct Meter { value: i32 } \
        trait Convert<T> { \
            func make(self: *Self) -> T; \
            func twice(self: *Self) -> T { self.make() } \
        } \
        impl Convert<i32> for Meter {\
            func make(self: *Meter) -> i32 { (*self).value }\
        } \
        func upcast(meter: *Meter) -> *dyn Convert<i32> { meter }";
    let (fixture, hir, _mir, program) = collect_clean(source);
    assert_eq!(
        entry_names(&fixture, &hir, &program),
        [vec!["make".to_owned(), "twice<Meter, i32>".to_owned()]]
    );
    let table = program.vtable(VtableId::from_usize(0));
    let arguments: Vec<String> = hir
        .ty_ctxt()
        .substs_args(table.entries[1].args)
        .iter()
        .map(|argument| {
            hir.ty_ctxt()
                .display_with(*argument, fixture.interner())
                .to_string()
        })
        .collect();
    assert_eq!(arguments, ["Meter", "i32"]);
    program
        .validate(&hir)
        .expect("the generic default is valid");
}

#[test]
fn missing_impl_reports_e7005_without_an_ice() {
    let fixture = Fixture::new(
        "trait Show { func show(self: *Self) -> i32; } \
         func take(value: *dyn Show) { }",
    );
    let (hir, _mir) = fixture.frontend();
    let use_site = hir.dyn_types()[0];
    let (trait_def, args) = hir
        .ty_ctxt()
        .dyn_parts(use_site.ty)
        .expect("the parameter uses a trait object");
    let key = VtableKey {
        concrete: hir.ty_ctxt().int_ty(IntTy::I32),
        trait_ref: TraitRef::new(trait_def, hir.ty_ctxt().int_ty(IntTy::I32), args),
    };
    let error = crate::vtable::build_vtable(&hir, fixture.interner(), key)
        .expect_err("i32 does not implement Show");
    assert!(matches!(error, crate::MonoError::MissingImpl { .. }));
    crate::vtable::report(fixture.dcx(), use_site.span, &error);
    fixture.expect("E7005", "no implementation");
}

#[test]
fn vtable_and_method_limits_are_enforced() {
    let vtable_limit = Fixture::new(TWO_TYPES);
    let (_hir, _mir, program) = vtable_limit.lower_with(Limits {
        depth: 64,
        instances: 4096,
        vtables: 1,
        glues: 4096,
        glue_depth: 64,
    });
    vtable_limit.expect("E7006", "vtable limit");
    assert_eq!(program.vtables().len(), 1);

    let method_limit = Fixture::new(&format!(
        "{SHAPE} func upcast(circle: *Circle) -> *dyn Shape {{ circle }}"
    ));
    let (_hir, _mir, program) = method_limit.lower_with(Limits {
        depth: 64,
        instances: 1,
        vtables: 64,
        glues: 4096,
        glue_depth: 64,
    });
    method_limit.expect("E7004", "instance limit");
    assert_eq!(program.instances().len(), 1);
}
