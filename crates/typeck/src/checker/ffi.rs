//! Validation of external declarations.
//!
//! An external signature is an ordinary [`Signature`](crate::Signature) with
//! one extra fact: calling it always requires `unsafe`. Every type at the
//! boundary is checked by the shared FFI-safety predicate, so a parameter, a
//! return type, and an external static are accepted or rejected by one rule.

use seal_span::Span;
use seal_ty::{FfiPosition, TyId, safety};

use super::Checker;
use crate::results::Signature;

impl Checker<'_> {
    /// Builds the signature of every external function and validates the type
    /// of every external static.
    pub(super) fn build_externs(&mut self) {
        let functions: Vec<_> = self.hir.extern_functions().to_vec();
        for definition in functions {
            let Some(data) = self.hir.extern_function(definition).cloned() else {
                continue;
            };
            let definition_span = self.hir.definitions().get(definition).span;
            for param in &data.params {
                self.results.record_local(param.def, param.ty);
                let span = self.hir.definitions().get(param.def).span;
                self.check_ffi_type(param.ty, FfiPosition::Parameter, span);
            }
            if let Some(ret) = data.ret {
                self.check_ffi_type(ret, FfiPosition::Return, definition_span);
            }
            let params = data.params.iter().map(|param| param.ty).collect();
            let ret = data.ret.unwrap_or_else(|| self.unit());
            self.results.insert_signature(
                definition,
                Signature {
                    params,
                    ret,
                    is_unsafe: true,
                    generics: Vec::new(),
                },
            );
        }

        let statics: Vec<_> = self.hir.extern_statics().to_vec();
        for definition in statics {
            let Some(data) = self.hir.extern_static(definition).cloned() else {
                continue;
            };
            let span = self.hir.definitions().get(definition).span;
            self.check_ffi_type(data.ty, FfiPosition::Static, span);
        }
    }

    /// Reports `ty` when it cannot cross a C boundary at `position`.
    fn check_ffi_type(&mut self, ty: TyId, position: FfiPosition, span: Span) {
        let Err(reason) = safety(self.hir.ty_ctxt(), ty, position) else {
            return;
        };
        let text = self.ty_text(ty);
        self.error(
            seal_error_codes::E3084,
            span,
            format!("`{text}` is not FFI-safe: {}", reason.reason()),
        )
        .help(reason.suggestion())
        .emit();
    }
}
