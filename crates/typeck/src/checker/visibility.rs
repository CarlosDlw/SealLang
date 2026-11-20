//! Access control for fields and methods.
//!
//! A path that names an item is checked during lowering, where it already
//! resolves to one definition. A field or method is selected only after the
//! receiver's type is known, so these checks live in the checker. They reuse
//! the same visibility predicate as lowering, and the current module comes from
//! the function whose body is being checked.

use seal_def::{DefId, Visibility};
use seal_error_codes as codes;
use seal_span::Span;

use super::Checker;

impl Checker<'_> {
    /// Reports `def` when the body being checked cannot name it.
    ///
    /// Returns `true` when the definition is visible. A body with no owner
    /// cannot be attributed to a module, so it is not rejected; every function,
    /// method, and default body is checked with its owner set.
    pub(super) fn require_item_visible(&mut self, def: DefId, span: Span) -> bool {
        let Some(owner) = self.owner else {
            return true;
        };
        let from = self.hir.module_of_def(owner);
        if self.hir.is_visible_from(def, from) {
            return true;
        }
        let visibility = self.hir.definitions().visibility(def);
        let help = if visibility == Visibility::Public {
            "make the module part of the package API, or use the item inside the package"
        } else {
            "make the item `internal` or `public`, or keep the use in a module that can see it"
        };
        self.error(
            codes::E2072,
            span,
            format!(
                "the item is `{}` and is not visible here",
                visibility.as_str()
            ),
        )
        .help(help)
        .emit();
        false
    }
}
