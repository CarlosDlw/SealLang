//! Deprecation warnings for method calls and field accesses.
//!
//! These uses are resolved by type checking, after HIR lowering. Struct
//! literals and patterns warn during lowering; the checker only handles the two
//! selections that need an inferred receiver or aggregate type.

use seal_diagnostics::Level;

use super::support::Fixture;

fn deprecated_warnings(fixture: &Fixture) -> Vec<seal_diagnostics::Diagnostic> {
    fixture
        .diagnostics()
        .into_iter()
        .filter(|diagnostic| {
            diagnostic.level() == Level::Warning
                && diagnostic.lint() == Some(seal_diagnostics::DEPRECATED)
        })
        .collect()
}

#[test]
fn field_access_and_inherent_method_calls_warn_once() {
    let fixture = Fixture::new(
        "struct S { deprecated(since: \"0.2.0\") old: i32 }\n\
         impl S {\n\
           deprecated(since: \"0.3.0\") func old_method(self: *S) -> i32 { (*self).old }\n\
         }\n\
         func main() {\n\
           let value = S { old: 1 };\n\
           let field = value.old;\n\
           let called = value.old_method();\n\
           let _ = field + called;\n\
         }",
    );
    let _ = fixture.check(64);
    let warnings = deprecated_warnings(&fixture);
    let old_field = warnings
        .iter()
        .filter(|warning| warning.message().contains("`old`"))
        .count();
    let old_method = warnings
        .iter()
        .filter(|warning| warning.message().contains("`old_method`"))
        .count();
    assert_eq!(old_field, 2, "one literal use and one access: {warnings:?}");
    assert_eq!(old_method, 1, "one call: {warnings:?}");
    assert!(warnings.iter().any(|warning| {
        warning.message().contains("`old_method`")
            && warning
                .labels()
                .iter()
                .any(|label| label.message == "this use is deprecated")
    }));
}

#[test]
fn a_method_call_inside_its_own_deprecated_body_does_not_warn() {
    let fixture = Fixture::new(
        "struct S {}\n\
         impl S {\n\
           deprecated(since: \"0.2.0\") func old(self: *S) { (*self).old(); }\n\
         }\n\
         func main() {}",
    );
    let _ = fixture.check(64);
    assert!(deprecated_warnings(&fixture).is_empty());
}

#[test]
fn a_trait_method_warns_when_called_on_an_implementation() {
    let fixture = Fixture::new(
        "struct S {}\n\
         trait T { deprecated(since: \"0.2.0\") func old(self: *Self) -> i32; }\n\
         impl T for S { func old(self: *S) -> i32 { 1 } }\n\
         func main() {\n\
           let value = S {};\n\
           let result = value.old();\n\
           let _ = result;\n\
         }",
    );
    let _ = fixture.check(64);
    let warnings = deprecated_warnings(&fixture);
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings[0].message().contains("`old`"));
    assert_eq!(warnings[0].lint(), Some(seal_diagnostics::DEPRECATED));
}
