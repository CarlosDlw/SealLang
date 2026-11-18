//! Trait-object code generation tests.
//!
//! These tests lower complete modules, so they inspect the real fat pointer,
//! vtable globals, indirect calls, `noalias` filtering, and debug metadata.

use seal_def::DefId;
use seal_ty::IntTy;

use super::support::{
    Compiled, front_end, function_ir, lowered_debug_module_ir, lowered_module_ir, module_ir,
    resolve_vtable, vtable_name,
};

const SHAPE: &str = "\
trait Shape { func area(self: *Self) -> i32; }
struct Circle { radius: i32 }
struct Square { side: i32 }
impl Shape for Circle { func area(self: *Circle) -> i32 { (*self).radius * (*self).radius } }
impl Shape for Square { func area(self: *Square) -> i32 { (*self).side * (*self).side } }
";

/// Compiles a program with one shared coercion and a dynamic call.
fn shape_program() -> Compiled {
    front_end(&format!(
        "{SHAPE}\n\
         func upcast(circle: *Circle) -> *dyn Shape {{ circle }}\n\
         func dispatch(shape: *dyn Shape) -> i32 {{ shape.area() }}\n\
         func main() -> i32 {{ let circle = Circle {{ radius: 3 }}; dispatch(upcast(&circle)) }}\n"
    ))
}

#[test]
fn dyn_pointer_is_two_pointer_words() {
    let ir = module_ir(&front_end(
        "trait Marker { }\n\
         func identity(value: *dyn Marker) -> *dyn Marker { value }\n",
    ));
    assert!(
        ir.contains("declare { ptr, ptr } @seal.identity({ ptr, ptr })"),
        "the trait-object ABI must be two pointers: {ir}"
    );
}

#[test]
fn vtable_header_has_drop_size_and_align() {
    let compiled = shape_program();
    let name = vtable_name(&compiled);
    let ir = lowered_module_ir(&compiled);
    let global = ir
        .lines()
        .find(|line| line.contains(&name))
        .unwrap_or_else(|| panic!("missing vtable `{name}` in:\n{ir}"));
    assert!(global.contains("@seal.drop.noop"), "{global}");
    assert!(global.contains("i64 4"), "{global}");
}

#[test]
fn vtable_method_slots_follow_the_layout() {
    let compiled = front_end(
        "trait Root { func root(self: *Self) -> i32; }\n\
         trait Leaf: Root { func leaf(self: *Self) -> i32; }\n\
         struct Value { value: i32 }\n\
         impl Root for Value { func root(self: *Value) -> i32 { (*self).value } }\n\
         impl Leaf for Value { func leaf(self: *Value) -> i32 { (*self).value + 1 } }\n\
         func upcast(value: *Value) -> *dyn Leaf { value }\n",
    );
    let name = vtable_name(&compiled);
    let ir = lowered_module_ir(&compiled);
    let global = ir
        .lines()
        .find(|line| line.contains(&name))
        .unwrap_or_else(|| panic!("missing vtable `{name}` in:\n{ir}"));
    let leaf = global.find("@seal.Value.leaf").expect("leaf slot");
    let root = global.find("@seal.Value.root").expect("root slot");
    assert!(
        leaf < root,
        "principal methods precede supertrait methods: {global}"
    );
}

#[test]
fn vtable_is_a_constant_global() {
    let compiled = shape_program();
    let name = vtable_name(&compiled);
    let ir = lowered_module_ir(&compiled);
    let global = ir
        .lines()
        .find(|line| line.contains(&name))
        .unwrap_or_else(|| panic!("missing vtable `{name}` in:\n{ir}"));
    assert!(global.contains("private unnamed_addr constant"), "{global}");
}

#[test]
fn trait_object_construction_uses_data_and_vtable() {
    let compiled = shape_program();
    let name = vtable_name(&compiled);
    let ir = lowered_module_ir(&compiled);
    let upcast = function_ir(&ir, "seal.upcast");
    assert!(upcast.contains("insertvalue { ptr, ptr }"), "{upcast}");
    assert!(upcast.contains(&name), "{upcast}");
}

#[test]
fn trait_object_coercion_does_not_call_an_allocator() {
    let compiled = shape_program();
    let ir = lowered_module_ir(&compiled);
    let upcast = function_ir(&ir, "seal.upcast");
    assert!(
        !upcast.contains("call "),
        "coercion must not call: {upcast}"
    );
    assert!(!upcast.contains("malloc"), "{upcast}");
}

#[test]
fn dynamic_dispatch_uses_an_indirect_call() {
    let compiled = shape_program();
    let ir = lowered_module_ir(&compiled);
    let dispatch = function_ir(&ir, "seal.dispatch");
    assert!(dispatch.contains("call i32 %"), "{dispatch}");
    assert!(
        !dispatch.contains("@seal.Circle.area"),
        "dispatch must not call a concrete method directly: {dispatch}"
    );
}

#[test]
fn missing_vtable_is_an_internal_error_not_a_null_global() {
    let compiled = shape_program();
    let tcx = compiled.hir.ty_ctxt();
    let shape = compiled
        .session
        .interner()
        .lookup("Shape")
        .and_then(|name| compiled.hir.trait_def(name))
        .expect("the fixture declares Shape");
    let target = tcx.ref_ty(tcx.dyn_ty(shape, tcx.empty_substs()));
    let source = tcx.ref_ty(tcx.int_ty(IntTy::I32));

    let error = resolve_vtable(&compiled, source, target).expect_err("`i32` has no Shape vtable");
    assert!(
        matches!(error, crate::CodegenError::Internal(_)),
        "unexpected error: {error:?}"
    );
}

#[test]
fn dynamic_call_passes_and_returns_aggregates() {
    let compiled = front_end(
        "struct Pair { left: i32, right: i32 }\n\
         trait Build { func build(self: *Self, seed: i32) -> Pair; }\n\
         struct Source { value: i32 }\n\
         impl Build for Source {\n\
           func build(self: *Source, seed: i32) -> Pair {\n\
             Pair { left: (*self).value + seed, right: seed }\n\
           }\n\
         }\n\
         func dispatch(source: *dyn Build) -> i32 {\n\
           let pair = source.build(2);\n\
           pair.left + pair.right\n\
         }\n",
    );
    let ir = lowered_module_ir(&compiled);
    let dispatch = function_ir(&ir, "seal.dispatch");
    assert!(
        dispatch.contains("call %seal.adt"),
        "the aggregate return type must survive the indirect call: {dispatch}"
    );
}

#[test]
fn dynamic_call_with_multiple_arguments() {
    let compiled = front_end(
        "trait Combine { func combine(self: *Self, a: i32, b: i32) -> i32; }\n\
         struct Value { value: i32 }\n\
         impl Combine for Value {\n\
           func combine(self: *Value, a: i32, b: i32) -> i32 { (*self).value + a + b }\n\
         }\n\
         func dispatch(value: *dyn Combine) -> i32 { value.combine(1, 2) }\n",
    );
    let ir = lowered_module_ir(&compiled);
    let dispatch = function_ir(&ir, "seal.dispatch");
    let call = dispatch
        .lines()
        .find(|line| line.contains("call i32 %"))
        .unwrap_or_else(|| panic!("missing indirect call: {dispatch}"));
    assert_eq!(call.matches("i32").count(), 3, "{call}");
}

#[test]
fn empty_trait_vtable_has_only_the_header() {
    let compiled = front_end(
        "trait Empty { }\n\
         struct Value { value: i32 }\n\
         impl Empty for Value { }\n\
         func upcast(value: *Value) -> *dyn Empty { value }\n",
    );
    let name = vtable_name(&compiled);
    let ir = lowered_module_ir(&compiled);
    let global = ir
        .lines()
        .find(|line| line.contains(&name))
        .unwrap_or_else(|| panic!("missing vtable `{name}` in:\n{ir}"));
    assert!(
        global.contains("constant { ptr, i64, i64 }"),
        "an empty trait keeps the three-slot header: {global}"
    );
}

#[test]
fn exclusive_receiver_reborrows_shared() {
    let compiled = front_end(
        "trait Read { func read(self: *Self) -> i32; }\n\
         struct Counter { value: i32 }\n\
         impl Read for Counter { func read(self: *Counter) -> i32 { (*self).value } }\n\
         func read(object: *mut dyn Read) -> i32 { object.read() }\n",
    );
    let ir = lowered_module_ir(&compiled);
    let read = function_ir(&ir, "seal.read");
    assert!(read.contains("dyn.data"), "{read}");
    assert!(read.contains("dyn.vtable"), "{read}");
    assert!(read.contains("dynamic_call"), "{read}");
}

#[test]
fn exclusive_reborrow_preserves_both_fields() {
    let compiled = front_end(
        "trait Read { func read(self: *Self) -> i32; }\n\
         struct Counter { value: i32 }\n\
         impl Read for Counter { func read(self: *Counter) -> i32 { (*self).value } }\n\
         func shared(object: *mut dyn Read) -> *dyn Read { &*object }\n",
    );
    let ir = lowered_module_ir(&compiled);
    let shared = function_ir(&ir, "seal.shared");
    assert_eq!(
        shared.matches("extractvalue { ptr, ptr }").count(),
        2,
        "{shared}"
    );
    assert_eq!(
        shared.matches("insertvalue { ptr, ptr }").count(),
        2,
        "{shared}"
    );
}

#[test]
fn vtable_methods_do_not_get_noalias() {
    let compiled = front_end(
        "trait Bump { func bump(self: *mut Self); }\n\
         struct Counter { value: i32 }\n\
         impl Bump for Counter { func bump(self: *mut Counter) { (*self).value += 1; } }\n\
         func upcast(counter: *mut Counter) -> *mut dyn Bump { counter }\n\
         func run(counter: *mut dyn Bump) { counter.bump(); }\n",
    );
    let method = method_def(&compiled, "bump");
    assert!(
        compiled.facts.noalias_functions().contains(&method),
        "the fixture must prove the method exclusive before filtering"
    );
    let ir = module_ir(&compiled);
    let method = function_ir(&ir, "seal.Counter.bump");
    assert!(
        !method.contains("noalias"),
        "a vtable-reachable method must not be `noalias`: {method}"
    );
}

#[test]
fn dyn_fat_pointer_gets_no_pointer_attributes() {
    let ir = module_ir(&front_end(
        "trait Marker { }\n\
         func skip(shared: *dyn Marker, exclusive: *mut dyn Marker) -> i32 { 0 }\n",
    ));
    assert!(!ir.contains("readonly"), "unexpected `readonly` in:\n{ir}");
    assert!(!ir.contains("noalias"), "unexpected `noalias` in:\n{ir}");
}

#[test]
fn debug_info_describes_the_dyn_pointer() {
    let compiled = shape_program();
    let ir = lowered_debug_module_ir(&compiled);
    assert!(ir.contains("DW_TAG_member"), "{ir}");
    assert!(ir.contains("name: \"data\""), "{ir}");
    assert!(ir.contains("name: \"vtable\""), "{ir}");
}

/// Returns the implementation method named `name` that borrow checking proved
/// exclusive. The trait declaration shares the name but has no body facts.
fn method_def(compiled: &Compiled, name: &str) -> DefId {
    let symbol = compiled
        .session
        .interner()
        .lookup(name)
        .unwrap_or_else(|| panic!("`{name}` is never interned"));
    let mut found = compiled
        .hir
        .definitions()
        .iter_enumerated()
        .filter(|(_, definition)| definition.name == symbol)
        .map(|(id, _)| id)
        .filter(|id| compiled.facts.noalias_functions().contains(id));
    let first = found
        .next()
        .unwrap_or_else(|| panic!("`{name}` has no exclusive implementation"));
    assert!(found.next().is_none(), "`{name}` is ambiguous");
    first
}
