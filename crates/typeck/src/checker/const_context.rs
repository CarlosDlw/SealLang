//! Restrictions inside constant contexts.
//!
//! A constant context is a `const`/`static` initializer or a `const func`
//! body. Such code may run during compilation, so it may not reach host state
//! or operations the evaluator cannot represent. The checks here are
//! conservative and syntactic: they reject a forbidden operation at its use
//! site, before the evaluator ever runs.

use seal_def::DefId;
use seal_error_codes as codes;
use seal_hir::HirExprId;
use seal_methods::IntrinsicOp;
use seal_span::Span;
use seal_ty::{TyId, TyKind};

use super::Checker;
use seal_hir::BodyKind;

/// How a constant context accesses a mutable static.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum StaticAccess {
    /// Reads the value or takes a shared reference.
    Read,
    /// Writes the value, takes an exclusive reference, or calls a mutating
    /// method on it.
    Write,
}

impl Checker<'_> {
    /// Returns `true` when the body runs in a constant context.
    pub(super) const fn in_const_context(&self) -> bool {
        self.const_initializer || self.const_fn
    }

    /// Checks a `const` or `static` declared type.
    ///
    /// The item-specific sized diagnostics (`E3056`/`E3057`) replace the
    /// generic storage error, because the declaration is a global value rather
    /// than a local or parameter.
    pub(super) fn value_decl_ty(&mut self, ty: TyId, span: Span, kind: BodyKind) -> TyId {
        if self.is_error(ty) {
            return ty;
        }
        if !self.is_sized(ty) {
            let (code, what) = match kind {
                BodyKind::Static { .. } => (codes::E3056, "static"),
                _ => (codes::E3057, "constant"),
            };
            self.error(
                code,
                span,
                format!(
                    "a {what} must have a `Sized` type, found `{}`",
                    self.ty_text(ty)
                ),
            )
            .help("store a reference instead, such as `*str` or `*[T]`")
            .emit();
            return self.error_ty();
        }
        self.storage_ty(ty, span, "value")
    }

    /// Reports a `*mut` mention in a declared constant-context signature.
    pub(super) fn note_declared_mut_ref(&mut self, ty: TyId, span: Span, is_const_fn: bool) {
        if self.mut_ref_reported || !self.contains_mut_ref(ty) {
            return;
        }
        self.mut_ref_reported = true;
        if is_const_fn {
            self.error(codes::E3062, span, "a `const func` cannot use `*mut T`")
                .help("const evaluation has no mutable references; use `*T` or a value")
                .emit();
        } else {
            self.error(
                codes::E4012,
                span,
                "a constant value cannot contain a mutable reference",
            )
            .help("a constant has no mutable storage; use `*T` or a value")
            .emit();
        }
    }

    /// Returns `true` when `ty` contains an exclusive reference.
    pub(super) fn contains_mut_ref(&self, ty: TyId) -> bool {
        let tcx = self.hir.ty_ctxt();
        match tcx.kind(ty) {
            TyKind::Ref { kind, pointee } => kind.is_exclusive() || self.contains_mut_ref(pointee),
            TyKind::Array { element, .. } | TyKind::Slice { element } => {
                self.contains_mut_ref(element)
            }
            TyKind::Adt { args, .. } | TyKind::Dyn { args, .. } => tcx
                .substs_args(args)
                .iter()
                .any(|ty| self.contains_mut_ref(*ty)),
            TyKind::Tuple(tuple) => tcx
                .tuple_elements(tuple)
                .iter()
                .any(|ty| self.contains_mut_ref(*ty)),
            _ => false,
        }
    }

    /// Reports a `*mut` type that a constant context cannot represent.
    ///
    /// A `const func` uses `E3062`; a value initializer uses `E4012`. Each body
    /// reports at most once, so one offending type does not cascade.
    pub(super) fn note_const_mut_ref(&mut self, ty: TyId, span: Span) {
        if self.mut_ref_reported || !self.in_const_context() || !self.contains_mut_ref(ty) {
            return;
        }
        self.mut_ref_reported = true;
        if self.const_fn {
            self.error(codes::E3062, span, "a `const func` cannot use `*mut T`")
                .help("const evaluation has no mutable references; use `*T` or a value")
                .emit();
        } else {
            self.error(
                codes::E4012,
                span,
                "a constant value cannot contain a mutable reference",
            )
            .help("a constant has no mutable storage; use `*T` or a value")
            .emit();
        }
    }

    /// Reports a non-constant call made from a constant context.
    pub(super) fn check_const_call(&mut self, callee: DefId, span: Span) {
        if !self.in_const_context() {
            return;
        }
        let is_const = self.hir.body_of(callee).is_some_and(|body| body.is_const);
        if is_const {
            return;
        }
        self.report_non_const_call(span);
    }

    /// Reports a call through a trait bound or a vtable.
    pub(super) fn report_const_dispatch(&self, span: Span) {
        if !self.in_const_context() {
            return;
        }
        if self.const_fn {
            self.error(
                codes::E4021,
                span,
                "a `const func` cannot use dynamic or trait-bound dispatch",
            )
            .help("call a concrete function instead; const evaluation has no vtable")
            .emit();
        } else {
            self.error(
                codes::E4016,
                span,
                "a constant context cannot use dynamic dispatch",
            )
            .help("call a concrete function instead; const evaluation has no vtable")
            .emit();
        }
    }

    /// Reports a custom iteration or indexing protocol in a constant context.
    pub(super) fn report_const_protocol(&self, span: Span) {
        if !self.in_const_context() {
            return;
        }
        if self.const_fn {
            self.error(
                codes::E4021,
                span,
                "a `const func` cannot use a custom protocol",
            )
            .help("use built-in indexing, ranges, arrays, or slices")
            .emit();
        } else {
            self.error(
                codes::E4017,
                span,
                "a constant context cannot use a custom protocol",
            )
            .help("use built-in indexing, ranges, arrays, or slices")
            .emit();
        }
    }

    /// Reports a read or write of a mutable static in a constant context.
    pub(super) fn report_const_static_access(&self, access: StaticAccess, span: Span) {
        if !self.in_const_context() {
            return;
        }
        if self.const_fn {
            self.error(
                codes::E4020,
                span,
                "a `const func` cannot access a mutable static",
            )
            .help("mutable global storage exists only at run time")
            .emit();
            return;
        }
        match access {
            StaticAccess::Read => {
                self.error(
                    codes::E4026,
                    span,
                    "a constant initializer cannot read a mutable static",
                )
                .help("read an immutable `static` or a `const` instead")
                .emit();
            }
            StaticAccess::Write => {
                self.error(
                    codes::E4015,
                    span,
                    "a constant context cannot write a static",
                )
                .help("global writes are not part of constant evaluation")
                .emit();
            }
        }
    }

    /// Checks a resolved method call against the constant-context rules.
    pub(super) fn check_const_method(&mut self, expr: HirExprId, receiver: HirExprId, span: Span) {
        if !self.in_const_context() {
            return;
        }
        let Some(method) = self.results.method(expr) else {
            return;
        };
        match &method.kind {
            crate::results::MethodKind::Intrinsic(id) => {
                let receiver_is_local = self
                    .results
                    .place(receiver)
                    .is_some_and(|place| place.root.is_local());
                if !const_intrinsic_allowed(id.def().intrinsic, receiver_is_local) {
                    self.error(
                        codes::E4022,
                        span,
                        format!(
                            "the intrinsic `{}` is not available in a constant context",
                            id.def().name
                        ),
                    )
                    .help("call it in ordinary run-time code instead")
                    .emit();
                }
            }
            crate::results::MethodKind::User { function, .. }
            | crate::results::MethodKind::Trait { function, .. } => {
                let is_const = self
                    .hir
                    .body_of(*function)
                    .is_some_and(|body| body.is_const);
                if !is_const {
                    self.report_non_const_call(span);
                }
            }
            crate::results::MethodKind::TraitBound { .. }
            | crate::results::MethodKind::Dynamic { .. } => {
                self.report_const_dispatch(span);
            }
        }
    }

    /// Reports a custom indexing protocol used in a constant context.
    pub(super) fn check_const_index(&mut self, expr: HirExprId, span: Span) {
        if !self.in_const_context() {
            return;
        }
        if matches!(
            self.results.index_protocol(expr),
            Some(crate::results::IndexProtocol::Custom { .. })
        ) {
            self.report_const_protocol(span);
        }
    }

    /// Reports a custom iteration protocol used in a constant context.
    pub(super) fn check_const_iteration(&mut self, source: HirExprId, span: Span) {
        if !self.in_const_context() {
            return;
        }
        if self.results.iteration(source).is_some() {
            self.report_const_protocol(span);
        }
    }

    fn report_non_const_call(&self, span: Span) {
        if self.const_fn {
            self.error(
                codes::E4018,
                span,
                "a `const func` cannot call a non-`const` function",
            )
            .help("declare the callee as `const func`, or call it at run time")
            .emit();
        } else {
            self.error(
                codes::E4011,
                span,
                "a constant context cannot call a non-`const` function",
            )
            .help("declare the callee as `const func`, or move the call to run time")
            .emit();
        }
    }
}

/// Returns whether an intrinsic may run during constant evaluation.
///
/// Scalar and sequence queries have no host state. A mutating sequence method
/// is only constant-evaluable when its receiver is a local place; a static or
/// constant receiver is rejected here and by the storage rules.
fn const_intrinsic_allowed(op: IntrinsicOp, receiver_is_local: bool) -> bool {
    match op {
        IntrinsicOp::Fill | IntrinsicOp::Swap | IntrinsicOp::Reverse => receiver_is_local,
        _ => true,
    }
}
