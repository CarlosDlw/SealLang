//! Drop glue emission.
//!
//! Each test lowers a whole program and inspects the resulting IR: the glue
//! symbol a vtable names, the destructor a glue calls, the order of field drops,
//! the variant switch, and the element loop. `impl Drop` is not part of the
//! surface yet, so the fixtures register a destructor before MIR lowering, the
//! same way the monomorphization tests do.

use super::support::{
    front_end, front_end_with_destructors, function_ir, lowered_module_ir, vtable_name,
};

const TOKEN: &str = "\
struct Token { value: i32 }\n\
func drop_token(value: *Token) { }\n";

/// Returns the symbol that starts right after `marker`.
fn symbol_at(line: &str, marker: &str) -> Option<String> {
    let start = line.find(marker)? + marker.len();
    let rest = &line[start..];
    let end = rest
        .find(|character: char| {
            !(character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-'))
        })
        .unwrap_or(rest.len());
    (!rest[..end].is_empty()).then(|| rest[..end].to_owned())
}

/// Returns the body of every drop glue function in `ir`.
fn glue_functions(ir: &str) -> Vec<String> {
    ir.lines()
        .filter(|line| line.starts_with("define internal void @seal.drop.glue."))
        .filter_map(|line| symbol_at(line, "@"))
        .map(|name| function_ir(ir, &name))
        .collect()
}

#[test]
fn a_program_without_droppable_types_emits_no_glue() {
    let ir = lowered_module_ir(&front_end("func main() -> i32 { 1 }"));
    assert!(!ir.contains("seal.drop.glue"), "{ir}");
    assert!(!ir.contains("seal.drop.noop"), "{ir}");
}

#[test]
fn a_vtable_first_slot_points_at_the_concrete_glue() {
    let compiled = front_end(
        "trait Shape { func area(self: *Self) -> i32; }\n\
         struct Circle { radius: i32 }\n\
         impl Shape for Circle { func area(self: *Circle) -> i32 { (*self).radius } }\n\
         func upcast(circle: *Circle) -> *dyn Shape { circle }\n",
    );
    let name = vtable_name(&compiled);
    let ir = lowered_module_ir(&compiled);
    let global = ir
        .lines()
        .find(|line| line.contains(&name))
        .unwrap_or_else(|| panic!("missing vtable `{name}` in:\n{ir}"));
    let glue = symbol_at(global, "@seal.drop.glue.").expect("the first slot names a glue");
    assert!(!global.contains("seal.drop.noop"), "{global}");
    let body = function_ir(&ir, &format!("seal.drop.glue.{glue}"));
    assert!(body.contains("ret void"), "{body}");
    assert!(
        !body.contains("call"),
        "a type that needs no drop calls nothing: {body}"
    );
}

#[test]
fn a_drop_statement_calls_the_place_glue_and_its_destructor() {
    let compiled = front_end_with_destructors(
        &format!("{TOKEN}func main() {{ let token = Token {{ value: 1 }}; }}"),
        &[("Token", "drop_token")],
    );
    let ir = lowered_module_ir(&compiled);
    let main = function_ir(&ir, "seal.main");
    let glue = symbol_at(&main, "@seal.drop.glue.").expect("the drop statement calls a glue");
    let body = function_ir(&ir, &format!("seal.drop.glue.{glue}"));
    assert!(
        body.contains("call {") && body.contains("@seal.drop_token"),
        "{body}"
    );
}

#[test]
fn a_struct_glue_drops_fields_in_declaration_order() {
    let compiled = front_end_with_destructors(
        &format!(
            "{TOKEN}struct Pair {{ first: Token, second: Token }}\n\
             enum Wrapper {{ Value(Pair) }}\n\
             func main() {{ let wrapper = Wrapper::Value(Pair {{ first: Token {{ value: 1 }}, second: Token {{ value: 2 }} }}); }}"
        ),
        &[("Token", "drop_token")],
    );
    let ir = lowered_module_ir(&compiled);
    let pair = glue_functions(&ir)
        .into_iter()
        .find(|body| body.contains("Pair"))
        .unwrap_or_else(|| panic!("missing the Pair glue in:\n{ir}"));
    assert_eq!(
        pair.matches("call void @seal.drop.glue.").count(),
        2,
        "{pair}"
    );
    let first = pair.find("i32 0").expect("first field projection");
    let second = pair.find("i32 1").expect("second field projection");
    assert!(first < second, "{pair}");
}

#[test]
fn an_enum_glue_switches_on_the_tag_and_drops_the_active_variant() {
    let compiled = front_end_with_destructors(
        &format!(
            "{TOKEN}enum Wrapper {{ Empty, Value(Token) }}\n\
             func main() {{ let wrapper = Wrapper::Value(Token {{ value: 1 }}); }}"
        ),
        &[("Token", "drop_token")],
    );
    let ir = lowered_module_ir(&compiled);
    let wrapper = glue_functions(&ir)
        .into_iter()
        .find(|body| body.contains("switch i"))
        .unwrap_or_else(|| panic!("missing the enum glue in:\n{ir}"));
    assert!(wrapper.contains("drop.variant0"), "{wrapper}");
    assert!(wrapper.contains("drop.variant1"), "{wrapper}");
    assert!(wrapper.contains("unreachable"), "{wrapper}");
    assert_eq!(
        wrapper.matches("call void @seal.drop.glue.").count(),
        1,
        "{wrapper}"
    );
    assert_eq!(wrapper.matches("ret void").count(), 2, "{wrapper}");
}

#[test]
fn an_array_glue_drops_elements_in_a_loop() {
    let compiled = front_end_with_destructors(
        &format!(
            "{TOKEN}func main() {{ let items = [Token {{ value: 1 }}, Token {{ value: 2 }}]; }}"
        ),
        &[("Token", "drop_token")],
    );
    let ir = lowered_module_ir(&compiled);
    let array = glue_functions(&ir)
        .into_iter()
        .find(|body| body.contains("drop.check"))
        .unwrap_or_else(|| panic!("missing the array glue in:\n{ir}"));
    assert_eq!(
        array.matches("call void @seal.drop.glue.").count(),
        1,
        "an array glue calls the element glue once, in a loop: {array}"
    );
    assert!(array.contains("drop.body"), "{array}");
    assert!(array.contains("drop.done"), "{array}");
    assert!(array.contains("icmp ult"), "{array}");
}

#[test]
fn a_remaining_drop_loops_from_the_kept_index() {
    let compiled = front_end_with_destructors(
        &format!(
            "{TOKEN}func main() {{ let items = [Token {{ value: 1 }}, Token {{ value: 2 }}]; for item in items {{ }} }}"
        ),
        &[("Token", "drop_token")],
    );
    let ir = lowered_module_ir(&compiled);
    let main = function_ir(&ir, "seal.main");
    assert!(main.contains("drop.check"), "{main}");
    assert!(main.contains("drop.done"), "{main}");
    assert!(main.contains("icmp ult"), "{main}");
    let loop_body = main
        .split("drop.body:")
        .nth(1)
        .expect("the remaining-drop statement emits a loop body");
    assert!(
        loop_body.contains("call void @seal.drop.glue."),
        "the loop body calls the element glue: {main}"
    );
}
