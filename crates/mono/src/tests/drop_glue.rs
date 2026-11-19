//! Drop glue collection during monomorphization.

use seal_hir::Hir;
use seal_index::Idx;
use seal_ty::TyId;

use crate::collector::Limits;
use crate::tests::support::{Fixture, collect_clean};
use crate::{DropGlueEntry, GlueChildren, MonoItem, MonoProgram, VtableId};

const TOKEN: &str = "\
struct Token { value: i32 } \
func drop_token(value: *Token) { }";

/// Returns the type of the local named `name` in the instance named `caller`.
fn local_ty(fixture: &Fixture, hir: &Hir, program: &MonoProgram, caller: &str, name: &str) -> TyId {
    let instance = program
        .instances()
        .iter()
        .find(|instance| crate::instance_name(hir, fixture.interner(), instance.instance) == caller)
        .unwrap_or_else(|| {
            panic!(
                "no instance named `{caller}`, have {:?}",
                program
                    .instances()
                    .iter()
                    .map(|instance| crate::instance_name(
                        hir,
                        fixture.interner(),
                        instance.instance
                    ))
                    .collect::<Vec<_>>()
            )
        });
    instance
        .body
        .locals
        .iter()
        .find(|declaration| {
            declaration
                .name
                .is_some_and(|symbol| fixture.symbol(symbol) == name)
        })
        .unwrap_or_else(|| panic!("no local named `{name}` in `{caller}`"))
        .ty
}

/// Returns the collected glue for the local named `name` in `caller`.
fn glue_of_local<'a>(
    fixture: &Fixture,
    hir: &Hir,
    program: &'a MonoProgram,
    caller: &str,
    name: &str,
) -> &'a DropGlueEntry {
    let ty = local_ty(fixture, hir, program, caller, name);
    let id = program
        .drop_glue_id(ty)
        .unwrap_or_else(|| panic!("`{name}` has no collected drop glue"));
    program.drop_glue(id)
}

/// Returns the rendered name of the glue's destructor instance.
fn destructor_name(fixture: &Fixture, hir: &Hir, glue: &DropGlueEntry) -> String {
    let destructor = glue
        .glue
        .destructor
        .expect("the type declares a destructor");
    crate::instance_name(hir, fixture.interner(), destructor)
}

#[test]
fn a_program_without_drops_or_trait_objects_collects_no_glue() {
    let (_fixture, _hir, _mir, program) =
        collect_clean("func main() -> i32 { let value = 1; value }");
    assert!(program.drop_glues().is_empty());
    assert!(
        !program
            .items()
            .iter()
            .any(|item| matches!(item, MonoItem::DropGlue(_)))
    );
}

#[test]
fn a_generic_body_specialized_for_a_trivial_type_collects_no_glue() {
    let (_fixture, _hir, _mir, program) =
        collect_clean("func consume<T>(value: T) { } func main() { consume(1); }");
    assert!(program.drop_glues().is_empty());
}

#[test]
fn a_type_with_a_destructor_gets_one_glue_naming_it() {
    let source = format!("{TOKEN} func main() {{ let token = Token {{ value: 1 }}; }}");
    let fixture = Fixture::new(&source);
    let (hir, _mir, program) = fixture.lower_with_destructors(&[("Token", "drop_token")]);
    fixture.assert_clean();
    let glue = glue_of_local(&fixture, &hir, &program, "main", "token");
    let destructor = glue.glue.destructor.expect("Token declares a destructor");
    assert_eq!(
        crate::instance_name(&hir, fixture.interner(), destructor),
        "drop_token"
    );
    assert!(hir.ty_ctxt().substs_args(destructor.args).is_empty());
    assert!(program.id_of(destructor).is_some());
    assert_eq!(glue.glue.children, GlueChildren::None);
}

#[test]
fn a_generic_destructor_is_instantiated_with_the_aggregate_arguments() {
    let source = "\
struct Holder<T> { value: T } \
func drop_holder<T>(value: *Holder<T>) { } \
func main() { let holder = Holder { value: 1 }; }";
    let fixture = Fixture::new(source);
    let (hir, _mir, program) = fixture.lower_with_destructors(&[("Holder", "drop_holder")]);
    fixture.assert_clean();
    let glue = glue_of_local(&fixture, &hir, &program, "main", "holder");
    assert_eq!(destructor_name(&fixture, &hir, glue), "drop_holder<i32>");
}

#[test]
fn a_container_without_a_destructor_lists_its_droppable_field() {
    // Drop elaboration splits a top-level drop of a plain struct into one drop
    // per field, so the struct only gets its own glue where the language drops
    // it as a unit: here as an enum variant field.
    let source = format!(
        "{TOKEN} struct Pair {{ first: Token }} \
         enum Wrapper {{ Pair(Pair) }} \
         func main() {{ let wrapper = Wrapper::Pair(Pair {{ first: Token {{ value: 1 }} }}); }}"
    );
    let fixture = Fixture::new(&source);
    let (hir, _mir, program) = fixture.lower_with_destructors(&[("Token", "drop_token")]);
    fixture.assert_clean();
    let wrapper = glue_of_local(&fixture, &hir, &program, "main", "wrapper");
    let GlueChildren::Variants(variants) = &wrapper.glue.children else {
        panic!(
            "an enum glue lists variants, got {:?}",
            wrapper.glue.children
        );
    };
    let pair = program
        .drop_glue_id(variants[0][0])
        .expect("the variant field has its own glue");
    let pair = program.drop_glue(pair);
    assert!(pair.glue.destructor.is_none());
    let GlueChildren::Fields(fields) = &pair.glue.children else {
        panic!("a struct glue lists fields, got {:?}", pair.glue.children);
    };
    assert_eq!(fields.len(), 1);
    let child = program
        .drop_glue_id(fields[0])
        .expect("the field has its own glue");
    assert_eq!(
        destructor_name(&fixture, &hir, program.drop_glue(child)),
        "drop_token"
    );
}

#[test]
fn an_enum_glue_drops_only_the_active_variant_fields() {
    let source = format!(
        "{TOKEN} enum Message {{ Empty, Text {{ token: Token }} }} \
         func main() {{ let message = Message::Text {{ token: Token {{ value: 1 }} }}; }}"
    );
    let fixture = Fixture::new(&source);
    let (hir, _mir, program) = fixture.lower_with_destructors(&[("Token", "drop_token")]);
    fixture.assert_clean();
    let glue = glue_of_local(&fixture, &hir, &program, "main", "message");
    assert!(glue.glue.destructor.is_none());
    let GlueChildren::Variants(variants) = &glue.glue.children else {
        panic!("an enum glue lists variants, got {:?}", glue.glue.children);
    };
    assert_eq!(variants.len(), 2);
    assert!(variants[0].is_empty());
    assert_eq!(variants[1].len(), 1);
    assert!(program.drop_glue_id(variants[1][0]).is_some());
}

#[test]
fn an_array_glue_drops_every_element() {
    let source = format!(
        "{TOKEN} func main() {{ \
         let items = [Token {{ value: 1 }}, Token {{ value: 2 }}, Token {{ value: 3 }}]; }}"
    );
    let fixture = Fixture::new(&source);
    let (hir, _mir, program) = fixture.lower_with_destructors(&[("Token", "drop_token")]);
    fixture.assert_clean();
    let glue = glue_of_local(&fixture, &hir, &program, "main", "items");
    let GlueChildren::Elements { element, count } = &glue.glue.children else {
        panic!("an array glue lists elements, got {:?}", glue.glue.children);
    };
    assert_eq!(*count, 3);
    let child = program
        .drop_glue_id(*element)
        .expect("the element has its own glue");
    assert_eq!(
        destructor_name(&fixture, &hir, program.drop_glue(child)),
        "drop_token"
    );
}

#[test]
fn a_tuple_glue_skips_elements_that_need_no_destructor() {
    // Like a plain struct, a top-level tuple is dropped element by element, so
    // the tuple glue appears where the value is dropped as a unit.
    let source = format!(
        "{TOKEN} enum Holder {{ Value((Token, i32)) }} \
         func main() {{ let holder = Holder::Value((Token {{ value: 1 }}, 2)); }}"
    );
    let fixture = Fixture::new(&source);
    let (hir, _mir, program) = fixture.lower_with_destructors(&[("Token", "drop_token")]);
    fixture.assert_clean();
    let holder = glue_of_local(&fixture, &hir, &program, "main", "holder");
    let GlueChildren::Variants(variants) = &holder.glue.children else {
        panic!(
            "an enum glue lists variants, got {:?}",
            holder.glue.children
        );
    };
    let tuple = program
        .drop_glue_id(variants[0][0])
        .expect("the variant field has its own glue");
    let GlueChildren::Fields(fields) = &program.drop_glue(tuple).glue.children else {
        panic!(
            "a tuple glue lists fields, got {:?}",
            program.drop_glue(tuple)
        );
    };
    assert_eq!(fields.len(), 1);
    assert!(program.drop_glue_id(fields[0]).is_some());
}

#[test]
fn two_drop_sites_of_the_same_type_share_one_glue() {
    let source = format!(
        "{TOKEN} func first(token: Token) {{ }} \
         func main() {{ let a = Token {{ value: 1 }}; let b = Token {{ value: 2 }}; }}"
    );
    let fixture = Fixture::new(&source);
    let (_hir, _mir, program) = fixture.lower_with_destructors(&[("Token", "drop_token")]);
    fixture.assert_clean();
    assert_eq!(program.drop_glues().len(), 1);
    let items = program
        .items()
        .iter()
        .filter(|item| matches!(item, MonoItem::DropGlue(_)))
        .count();
    assert_eq!(items, 1);
}

#[test]
fn a_vtable_concrete_type_has_drop_glue() {
    let source = format!(
        "{TOKEN} \
         trait Show {{ func show(self: *Self) -> i32; }} \
         impl Show for Token {{ func show(self: *Token) -> i32 {{ (*self).value }} }} \
         func upcast(token: *Token) -> *dyn Show {{ token }} \
         func main() {{ let token = Token {{ value: 1 }}; let object = upcast(&token); }}"
    );
    let fixture = Fixture::new(&source);
    let (_hir, _mir, program) = fixture.lower_with_destructors(&[("Token", "drop_token")]);
    fixture.assert_clean();
    let table = program.vtable(VtableId::from_usize(0));
    assert!(program.drop_glue_id(table.key.concrete).is_some());
}

#[test]
fn type_hash_distinguishes_instantiations() {
    let source = "\
struct Pair<T> { value: T } \
func main() { let numbers = Pair { value: 1 }; let flags = Pair { value: true }; }";
    let (fixture, hir, _mir, program) = collect_clean(source);
    let numbers = local_ty(&fixture, &hir, &program, "main", "numbers");
    let flags = local_ty(&fixture, &hir, &program, "main", "flags");
    assert_ne!(
        crate::type_hash(&hir, numbers),
        crate::type_hash(&hir, flags)
    );
    assert_eq!(
        crate::type_hash(&hir, numbers),
        crate::type_hash(&hir, numbers)
    );
}

#[test]
fn the_glue_limit_is_reported_with_the_type() {
    let source = format!("{TOKEN} func main() {{ let token = Token {{ value: 1 }}; }}");
    let fixture = Fixture::new(&source);
    let (_hir, _mir, program) = fixture.lower_with_limits_and_destructors(
        Limits {
            depth: 64,
            instances: 4096,
            vtables: 4096,
            glues: 0,
            glue_depth: 64,
        },
        &[("Token", "drop_token")],
    );
    fixture.expect("E7008", "Token");
    assert!(program.drop_glues().is_empty());
}

#[test]
fn the_glue_depth_limit_is_reported_for_a_nested_type() {
    let source = format!(
        "{TOKEN} enum Outer {{ Value(Token) }} \
         func main() {{ let outer = Outer::Value(Token {{ value: 1 }}); }}"
    );
    let fixture = Fixture::new(&source);
    let (_hir, _mir, _program) = fixture.lower_with_limits_and_destructors(
        Limits {
            depth: 64,
            instances: 4096,
            vtables: 4096,
            glues: 4096,
            glue_depth: 0,
        },
        &[("Token", "drop_token")],
    );
    fixture.expect("E7008", "nesting depth");
}
