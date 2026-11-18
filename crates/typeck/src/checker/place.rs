//! Places: assignment, compound assignment, increment/decrement, and the
//! initialization checks that read a place.

use seal_ast::{AssignOp, UpdateOp};
use seal_error_codes as codes;
use seal_hir::HirExprId;
use seal_span::Span;
use seal_ty::{InitState, TyId};

use super::Checker;
use super::flow::Flow;
use crate::results::{PlaceProjection, PlaceRoot, ResolvedPlace};

impl Checker<'_> {
    pub(super) fn check_assign(
        &mut self,
        op: AssignOp,
        target: HirExprId,
        value: HirExprId,
        flow: &mut Flow,
        span: Span,
    ) -> TyId {
        let target_span = self.hir.expr(target).span;
        if self.hir.place(target).is_none() {
            self.report_bad_target(target, target_span);
            self.results.record_expr(target, self.error_ty());
            self.infer_expr(value, None, flow);
            return self.error_ty();
        }

        let ty = self.infer_place(target, flow);
        let resolved = self.results.place(target).cloned();
        let Some(root) = resolved.as_ref().map(|place| place.root) else {
            self.check_expr(value, ty, flow);
            return self.unit();
        };
        let plain = op == AssignOp::Assign;

        // A write through a reference needs `*mut T`: the last dereference is
        // the one that reaches the destination. `*T` is read-only (D4).
        if let Some(kind) = self.deref_kind(target) {
            if !kind.is_exclusive() {
                self.report_shared_write(target_span);
                self.check_expr(value, ty, flow);
                return self.unit();
            }
            if !plain {
                self.check_assign_op(op, ty, span);
            }
            self.check_expr(value, ty, flow);
            return self.unit();
        }

        let mutable = self.root_is_writable(root);
        let whole = resolved.as_ref().is_none_or(ResolvedPlace::is_whole);
        let initialized = resolved
            .as_ref()
            .is_some_and(|place| self.place_state(place, flow) == InitState::Initialized);

        // A runtime index cannot prove which element is written, so the whole
        // array must already be initialized; otherwise the write would leave
        // the other elements in an unknown state.
        if plain
            && resolved
                .as_ref()
                .is_some_and(ResolvedPlace::has_dynamic_index)
            && !initialized
        {
            self.report_place_uninitialized(resolved.as_ref(), target_span);
            self.check_expr(value, ty, flow);
            return self.unit();
        }

        // D9: an uninitialized immutable binding may be initialized once by a
        // whole assignment. Field writes always require `mut`, because they do
        // not initialize the binding as a whole.
        if root.is_local() && !mutable && (!plain || !whole || initialized) {
            let name = self.def_name(root.def());
            self.error(
                codes::E3004,
                target_span,
                format!("cannot assign to `{name}`, as it is not declared as mutable"),
            )
            .help("declare it with `let mut` to allow assignment")
            .emit();
        }
        if !plain {
            if !initialized {
                self.report_place_uninitialized(resolved.as_ref(), target_span);
            }
            self.check_assign_op(op, ty, span);
        }

        self.check_expr(value, ty, flow);
        if let Some(place) = resolved
            && place.root.is_local()
            && !place.contains_custom_index()
        {
            flow.assign(place.root.def(), &place.path());
        }
        self.unit()
    }

    pub(super) fn check_update(
        &mut self,
        op: UpdateOp,
        target: HirExprId,
        flow: &mut Flow,
        span: Span,
    ) -> TyId {
        let target_span = self.hir.expr(target).span;
        if self.hir.place(target).is_none() {
            self.report_bad_target(target, target_span);
            self.results.record_expr(target, self.error_ty());
            return self.error_ty();
        }

        let ty = self.infer_place(target, flow);
        let resolved = self.results.place(target).cloned();
        let Some(root) = resolved.as_ref().map(|place| place.root) else {
            return self.unit();
        };

        if let Some(kind) = self.deref_kind(target) {
            if !kind.is_exclusive() {
                self.report_shared_write(target_span);
                return self.unit();
            }
            self.check_update_numeric(op, ty, span);
            return self.unit();
        }

        if root.is_local() && !self.root_is_writable(root) {
            let name = self.def_name(root.def());
            self.error(
                codes::E3004,
                target_span,
                format!("cannot assign to `{name}`, as it is not declared as mutable"),
            )
            .help("declare it with `let mut` to allow assignment")
            .emit();
        }
        if !resolved
            .as_ref()
            .is_some_and(|place| self.place_state(place, flow) == InitState::Initialized)
        {
            self.report_place_uninitialized(resolved.as_ref(), target_span);
        }
        self.check_update_numeric(op, ty, span);

        if let Some(place) = resolved
            && place.root.is_local()
            && !place.contains_custom_index()
        {
            flow.assign(place.root.def(), &place.path());
        }
        self.unit()
    }

    /// Returns `true` when the root definition permits writes.
    pub(super) fn root_is_writable(&self, root: PlaceRoot) -> bool {
        match root {
            PlaceRoot::Local(def) | PlaceRoot::Static(def) => {
                self.hir.definitions().is_mutable(def)
            }
            PlaceRoot::Const(_) => false,
        }
    }

    /// Requires a place expression to be initialized before it is read.
    pub(super) fn require_initialized(&mut self, expr: HirExprId, flow: &Flow, span: Span) {
        let Some(place) = self.results.place(expr).cloned() else {
            return;
        };
        if place.contains_deref() || !place.root.is_local() {
            return;
        }
        if self.place_state(&place, flow) != InitState::Initialized {
            let text = self.place_text(&place);
            self.error(
                codes::E3011,
                span,
                format!("`{text}` is possibly uninitialized"),
            )
            .help(format!("assign a value to `{text}` before reading it"))
            .emit();
        }
    }

    /// Returns the initialization state of a resolved place.
    pub(super) fn place_state(&self, place: &ResolvedPlace, flow: &Flow) -> InitState {
        if !place.root.is_local() {
            return InitState::Initialized;
        }
        flow.state(
            place.root.def(),
            &place.path(),
            place.root_ty,
            self.hir.ty_ctxt(),
        )
    }

    /// Renders a place as written, for diagnostics.
    pub(super) fn place_text(&self, place: &ResolvedPlace) -> String {
        let mut text = self.def_name(place.root.def()).to_owned();
        for projection in &place.projections {
            match projection {
                PlaceProjection::Field(field) => {
                    text.push('.');
                    text.push_str(self.def_name(*field));
                }
                PlaceProjection::TupleField(index) => {
                    text.push('.');
                    text.push_str(&index.to_string());
                }
                PlaceProjection::Index {
                    constant: Some(index),
                    ..
                } => {
                    text.push_str(&format!("[{index}]"));
                }
                PlaceProjection::Index { constant: None, .. }
                | PlaceProjection::CustomIndex { .. } => text.push_str("[..]"),
                PlaceProjection::Deref => {}
            }
        }
        text
    }

    fn report_place_uninitialized(&self, place: Option<&ResolvedPlace>, span: Span) {
        let text = place
            .map(|place| self.place_text(place))
            .unwrap_or_else(|| "this place".to_owned());
        self.error(
            codes::E3011,
            span,
            format!("`{text}` is possibly uninitialized"),
        )
        .help(format!("assign a value to `{text}` before reading it"))
        .emit();
    }

    fn check_update_numeric(&self, op: UpdateOp, ty: TyId, span: Span) {
        if let seal_ty::TyKind::Param(param) = self.hir.ty_ctxt().kind(ty) {
            let name = self
                .interner
                .resolve(self.hir.ty_ctxt().param_data(param).name);
            self.error(
                codes::E3041,
                span,
                format!(
                    "operator `{}` is not available for the type parameter `{name}`",
                    op.as_str()
                ),
            )
            .help("add a bound that provides the operation, or make the function concrete")
            .emit();
            return;
        }
        if !self.is_numeric(ty) && !self.is_error(ty) {
            self.error(
                codes::E3002,
                span,
                format!(
                    "`{}` requires a numeric place, found `{}`",
                    op.as_str(),
                    self.ty_text(ty)
                ),
            )
            .help("increment and decrement work on integer and floating-point places")
            .emit();
        }
    }

    fn report_shared_write(&self, span: Span) {
        self.error(
            codes::E3014,
            span,
            "cannot write through a shared reference",
        )
        .help("`*T` is read-only; use a `*mut T` reference to write through it")
        .emit();
    }

    fn report_bad_target(&self, target: HirExprId, span: Span) {
        if self.is_function_target(target) {
            self.error(codes::E3005, span, "cannot assign to a function")
                .help("only local bindings are assignable")
                .emit();
        } else if !self.is_error_target(target) {
            self.error(codes::E3005, span, "invalid assignment target")
                .help("assign to a local binding or one of its fields")
                .emit();
        }
    }

    fn is_function_target(&self, target: HirExprId) -> bool {
        matches!(
            self.hir.expr(target).kind,
            seal_hir::ExprKind::Path {
                res: seal_hir::Res::Function(_),
                ..
            }
        )
    }

    fn is_error_target(&self, target: HirExprId) -> bool {
        matches!(self.hir.expr(target).kind, seal_hir::ExprKind::Error)
    }
}
