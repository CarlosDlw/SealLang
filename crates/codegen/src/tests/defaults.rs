//! Code generation for trait methods that use a default body.
//!
//! The default is a normal monomorphized instance. Static calls target it
//! directly, and a vtable entry stores its function pointer instead of a null
//! slot.

use super::support::{front_end, function_ir, lowered_module_ir, vtable_name};

const DEFAULT_SHAPE: &str = "\
struct Circle { radius: i32 }
trait Shape { func area(self: *Self) -> i32 { 7 } }
impl Shape for Circle {}
";

#[test]
fn static_default_call_is_direct() {
    let compiled = front_end(&format!(
        "{DEFAULT_SHAPE}\n\
         func main() -> i32 {{\n\
             let circle = Circle {{ radius: 1 }};\n\
             circle.area()\n\
         }}\n"
    ));
    let ir = lowered_module_ir(&compiled);
    let main = function_ir(&ir, "seal.main");
    assert!(
        main.contains("call i32 @seal.Shape.area."),
        "the call must target the default instance directly: {main}"
    );
    assert!(
        !main.contains("dyn.vtable"),
        "a concrete default call stays static: {main}"
    );
}

#[test]
fn vtable_default_entry_is_function_pointer() {
    let compiled = front_end(&format!(
        "{DEFAULT_SHAPE}\n\
         func upcast(circle: *Circle) -> *dyn Shape {{ circle }}\n"
    ));
    let name = vtable_name(&compiled);
    let ir = lowered_module_ir(&compiled);
    let global = ir
        .lines()
        .find(|line| line.contains(&name))
        .unwrap_or_else(|| panic!("missing vtable `{name}` in:\n{ir}"));
    assert!(
        global.contains("@seal.Shape.area."),
        "the vtable must point at the default instance: {global}"
    );
    assert!(
        !global.contains("null"),
        "a default slot must not be null: {global}"
    );
}
