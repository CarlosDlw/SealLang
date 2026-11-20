//! Parameter definitions shared by functions, trait methods, and closures.

use seal_def::{DefData, DefId, DefKind};
use seal_error_codes as codes;
use seal_span::Span;
use seal_symbol::Symbol;
use seal_ty::TyId;

use super::Lowering;
use crate::body::ParamData;

impl Lowering<'_> {
    /// Defines one parameter of `owner` and installs it in the current scope.
    ///
    /// The type is already lowered: functions lower it from the declaration,
    /// and a closure without an annotation passes the error placeholder that
    /// type checking replaces with the expected signature.
    pub(super) fn define_param(
        &mut self,
        owner: DefId,
        name: Symbol,
        name_span: Span,
        span: Span,
        ty: TyId,
    ) -> ParamData {
        let definition = self.define(DefData {
            kind: DefKind::Param,
            name,
            name_span,
            span,
            parent: Some(owner),
            mutable: false,
        });

        if let Some(first) = self.scopes.current(name) {
            let first_span = self.hir.definitions().get(first).name_span;
            self.error(
                codes::E2001,
                name_span,
                format!(
                    "the parameter `{}` is declared twice",
                    self.interner.resolve(name)
                ),
            )
            .secondary(first_span, "first declared here")
            .help("rename one of the parameters")
            .emit();
        } else {
            self.scopes.insert(name, definition);
        }

        ParamData {
            def: definition,
            ty,
        }
    }
}
