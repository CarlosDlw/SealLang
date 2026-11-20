//! Validation of external declarations.
//!
//! An external signature is an ordinary [`Signature`](crate::Signature) with
//! one extra fact: calling it always requires `unsafe`. Every type at the
//! boundary is checked by the shared FFI-safety predicate, so a parameter, a
//! return type, and an external static are accepted or rejected by one rule.

use seal_layout::LayoutCtxt;
use seal_span::Span;
use seal_ty::{FfiPosition, TyId, TyKind, safety};

use super::Checker;
use crate::results::Signature;

impl Checker<'_> {
    /// Builds the signature of every external function and validates the type
    /// of every external static.
    pub(super) fn build_externs(&mut self) {
        self.validate_fn_sigs();
        self.validate_extern_structs();
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

    /// Validates every declared C function signature.
    ///
    /// A signature is the pointee of a callable C function pointer, so each
    /// fixed parameter and the return type must be FFI-safe independently. A
    /// variadic signature must also have at least one fixed parameter.
    fn validate_fn_sigs(&mut self) {
        let uses = self.hir.fn_sigs().to_vec();
        for usage in uses {
            let Some(id) = self.hir.ty_ctxt().fn_sig_id(usage.ty) else {
                continue;
            };
            let signature = self.hir.ty_ctxt().fn_sig(id);
            if signature.variadic && signature.params.is_empty() {
                self.error(
                    seal_error_codes::E3084,
                    usage.span,
                    "a variadic C function signature needs at least one fixed parameter",
                )
                .help("declare the fixed parameters C always receives, then `...`")
                .emit();
            }
            for parameter in &signature.params {
                self.check_ffi_type(*parameter, FfiPosition::FunctionPointer, usage.span);
            }
            self.check_ffi_type(signature.ret, FfiPosition::Return, usage.span);
        }
    }

    /// Validates every field of every `extern struct`.
    ///
    /// An external aggregate is a C layout, so its fields must be C types with
    /// non-zero size. The check runs before bodies are type checked and names
    /// the offending field, not just the aggregate.
    fn validate_extern_structs(&mut self) {
        let structs = self.hir.extern_structs().to_vec();
        for adt in structs {
            let definition = self.hir.ty_ctxt().adt(adt).clone();
            let aggregate_span = self.hir.definitions().get(definition.def).span;
            if definition.fields.is_empty() {
                self.error(
                    seal_error_codes::E3085,
                    aggregate_span,
                    format!(
                        "the external struct `{}` has no fields",
                        self.ty_text(self.hir.ty_ctxt().adt_ty(adt))
                    ),
                )
                .help("declare at least one field with a C representation")
                .emit();
                continue;
            }
            for field in &definition.fields {
                let span = self.hir.definitions().get(field.def).span;
                let ty = field.ty;
                if matches!(self.hir.ty_ctxt().kind(ty), TyKind::Bool) {
                    self.error(
                        seal_error_codes::E3085,
                        span,
                        "a field of an external struct cannot be `bool`",
                    )
                    .help("use `c_bool` for the one-byte C boolean")
                    .emit();
                    continue;
                }
                if matches!(self.hir.ty_ctxt().kind(ty), TyKind::Opaque { .. }) {
                    self.error(
                        seal_error_codes::E3087,
                        span,
                        format!(
                            "the opaque type `{}` cannot be stored by value in an external struct",
                            self.ty_text(ty)
                        ),
                    )
                    .help("store a raw pointer to the opaque type instead")
                    .emit();
                    continue;
                }
                if self.zero_sized(ty) {
                    self.error(
                        seal_error_codes::E3085,
                        span,
                        format!(
                            "the field type `{}` has no storage in an external struct",
                            self.ty_text(ty)
                        ),
                    )
                    .help("remove the field or give it a non-zero C size")
                    .emit();
                    continue;
                }
                self.check_ffi_type(ty, FfiPosition::Field, span);
            }
        }
    }

    /// Returns `true` when `ty` occupies no bytes.
    fn zero_sized(&self, ty: TyId) -> bool {
        LayoutCtxt::new(self.hir.ty_ctxt(), self.pointer_width)
            .layout_of(ty)
            .is_ok_and(|layout| layout.size() == 0)
    }

    /// Reports `ty` when it cannot cross a C boundary at `position`.
    fn check_ffi_type(&mut self, ty: TyId, position: FfiPosition, span: Span) {
        if matches!(self.hir.ty_ctxt().kind(ty), TyKind::Error) {
            return;
        }
        let Err(reason) = safety(self.hir.ty_ctxt(), ty, position) else {
            return;
        };
        let text = self.ty_text(ty);
        if matches!(reason, seal_ty::NotFfiSafe::Opaque) {
            self.error(
                seal_error_codes::E3087,
                span,
                format!("`{text}` cannot cross a C boundary by value"),
            )
            .help(reason.suggestion())
            .emit();
            return;
        }
        self.error(
            seal_error_codes::E3084,
            span,
            format!("`{text}` is not FFI-safe: {}", reason.reason()),
        )
        .help(reason.suggestion())
        .emit();
    }
}
