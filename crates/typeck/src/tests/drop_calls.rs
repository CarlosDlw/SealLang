//! Rejecting direct calls to a destructor.

use super::support::{check_clean, check_errors, function, tail};

const POINT: &str = "\
struct Point { x: i32 } \
impl Drop for Point { func drop(self: *mut Point) {} }";

#[test]
fn a_method_style_destructor_call_is_rejected() {
    let source =
        format!("{POINT} func main() {{ let mut point = Point {{ x: 0 }}; point.drop(); }}");
    let (fixture, _, _) = check_errors(&source);
    fixture.expect("E3065", "cannot be called directly");
}

#[test]
fn an_inherent_function_named_drop_is_still_callable() {
    let source = "\
struct Meter { value: i32 } \
impl Meter { func drop(self: *Meter) -> i32 { (*self).value } } \
func main() -> i32 { let meter = Meter { value: 7 }; meter.drop() }";
    let (fixture, hir, _) = check_clean(source);
    let (_, body) = function(&fixture, &hir, "main");
    let call = tail(&hir, body);
    assert!(matches!(
        hir.expr(call).kind,
        seal_hir::ExprKind::MethodCall { .. }
    ));
}

#[test]
fn a_qualified_destructor_call_is_rejected() {
    let source = format!(
        "{POINT} func main() {{ let mut point = Point {{ x: 0 }}; Drop::drop(&mut point); }}"
    );
    let (fixture, _, _) = check_errors(&source);
    fixture.expect("E3065", "cannot be called directly");
}
