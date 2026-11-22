//! Inherent `impl str` registration, ownership, and receiver rules.

use seal_def::DefKind;

use crate::ReceiverKind;

use super::support::{Fixture, PackageSpec};

const CORE: &str = "core";

fn core_fixture(source: &'static str) -> Fixture {
    let files = Box::leak(vec![("core.sl", source)].into_boxed_slice());
    Fixture::from_packages(&[PackageSpec {
        name: CORE,
        files,
        exposed: &[],
        dependencies: &[],
    }])
}

fn user_fixture(source: &'static str) -> Fixture {
    Fixture::package(&[("main.sl", source)])
}

fn diagnostics_help(fixture: &Fixture, code: &str, fragment: &str) -> bool {
    fixture.diagnostics().iter().any(|diagnostic| {
        diagnostic.code().map(|code| code.as_str()) == Some(code)
            && diagnostic
                .helps()
                .iter()
                .any(|help| help.contains(fragment))
    })
}

#[test]
fn core_registers_shared_str_methods() {
    let fixture = core_fixture(
        "impl str { \
             public func is_empty(self: *str) -> bool { self.len() == 0 } \
         } \
         func main() -> bool { let text: *str = \"\"; text.is_empty() }",
    );
    let hir = fixture.lower_packages();
    fixture.assert_clean();

    let name = fixture.interner.intern("is_empty");
    let function = hir.str_method(name).expect("the method is registered");
    assert_eq!(hir.definitions().kind(function), DefKind::Function);
    assert_eq!(hir.receiver(function), Some(ReceiverKind::Shared));
    assert_eq!(hir.str_methods(name).len(), 1);
    assert!(hir.body_of(function).is_some());
}

#[test]
fn impl_str_is_rejected_outside_core() {
    let fixture = user_fixture(
        "impl str { public func is_empty(self: *str) -> bool { true } } \
         func main() {}",
    );
    let hir = fixture.lower_package();
    fixture.expect("E2082", "only be declared in the core package");

    let name = fixture.interner.intern("is_empty");
    assert!(hir.str_methods(name).is_empty());
}

#[test]
fn str_methods_reject_a_value_receiver() {
    let fixture =
        core_fixture("impl str { public func broken(self: str) -> bool { true } } func main() {}");
    let _ = fixture.lower_packages();
    fixture.expect("E2016", "not a valid receiver for `str`");
    assert!(
        diagnostics_help(&fixture, "E2016", "write `self: *str`"),
        "the diagnostic must document the only valid str receiver"
    );
}

#[test]
fn str_methods_reject_an_exclusive_receiver() {
    let fixture = core_fixture(
        "impl str { public func broken(self: *mut str) -> bool { true } } func main() {}",
    );
    let _ = fixture.lower_packages();
    fixture.expect("E2016", "not a valid receiver for `str`");
    assert!(
        diagnostics_help(&fixture, "E2016", "write `self: *str`"),
        "the diagnostic must document the only valid str receiver"
    );
}

#[test]
fn str_methods_require_a_receiver() {
    let fixture = core_fixture("impl str { public func empty() -> bool { true } } func main() {}");
    let _ = fixture.lower_packages();
    fixture.expect("E2016", "must take `self: *str`");
}

#[test]
fn impl_str_cannot_be_generic() {
    let fixture = core_fixture(
        "impl<T> str { public func is_empty(self: *str) -> bool { true } } func main() {}",
    );
    let _ = fixture.lower_packages();
    fixture.expect("E2056", "cannot declare generic parameters");
}

#[test]
fn impl_str_cannot_have_a_where_clause() {
    let fixture = core_fixture(
        "impl str where i32: Copy { \
             public func is_empty(self: *str) -> bool { true } \
         } \
         func main() {}",
    );
    let _ = fixture.lower_packages();
    fixture.expect("E2056", "cannot have a where clause");
}

#[test]
fn duplicate_str_methods_are_reported_and_only_the_first_is_kept() {
    let fixture = core_fixture(
        "impl str { public func is_empty(self: *str) -> bool { true } } \
         impl str { public func is_empty(self: *str) -> bool { false } } \
         func main() {}",
    );
    let hir = fixture.lower_packages();
    fixture.expect("E2017", "defined twice");

    let name = fixture.interner.intern("is_empty");
    assert_eq!(hir.str_methods(name).len(), 1);
    assert!(hir.str_method(name).is_some());
}
