//! Expression checking.

use seal_hir::{ExprKind, HirBlockId, HirExprId, Res};
use seal_span::Span;
use seal_symbol::Symbol;
use seal_ty::{TyId, TyKind};

use super::flow::Flow;
use super::{Checker, PlaceMode};
use crate::results::{PlaceProjection, ResolvedPlace};

/// The pieces of a field access needed to resolve and type it.
struct FieldAccess {
    base: HirExprId,
    name: Symbol,
    name_span: Span,
}

impl Checker<'_> {
    /// Checks `expr` against `expected`, reporting a mismatch at the expression.
    pub(super) fn check_expr(&mut self, expr: HirExprId, expected: TyId, flow: &mut Flow) {
        let found = self.infer_expr(expr, Some(expected), flow);
        self.mismatch(self.hir.expr(expr).span, expected, found);
    }

    /// Computes the type of `expr`, using `expected` as a hint where it changes
    /// inference (literals and conditional branches).
    pub(super) fn infer_expr(
        &mut self,
        expr: HirExprId,
        expected: Option<TyId>,
        flow: &mut Flow,
    ) -> TyId {
        self.infer_expr_mode(expr, expected, flow, PlaceMode::Read)
    }

    /// Types a place used as an assignment target: the place itself is not
    /// required to be initialized, because the assignment initializes it.
    pub(super) fn infer_place(&mut self, expr: HirExprId, flow: &mut Flow) -> TyId {
        self.infer_expr_mode(expr, None, flow, PlaceMode::Write)
    }

    fn infer_expr_mode(
        &mut self,
        expr: HirExprId,
        expected: Option<TyId>,
        flow: &mut Flow,
        mode: PlaceMode,
    ) -> TyId {
        let (span, kind) = {
            let expr = self.hir.expr(expr);
            (expr.span, expr.kind.clone())
        };

        let ty = match kind {
            ExprKind::Lit(literal) => self.literal_ty(literal, expected, span, false),
            ExprKind::Path(res) => self.check_path(expr, res, span, flow, mode),
            ExprKind::Borrow { operand } => self.check_borrow(operand, flow),
            ExprKind::Deref { operand } => {
                let ty = self.check_deref(operand, flow, span);
                if let Some(mut place) = self.results.place(operand).cloned() {
                    place.projections.push(PlaceProjection::Deref);
                    place.ty = ty;
                    self.results.record_place(expr, place);
                }
                ty
            }
            ExprKind::Field {
                base,
                name,
                name_span,
            } => self.check_field(
                expr,
                FieldAccess {
                    base,
                    name,
                    name_span,
                },
                span,
                flow,
                mode,
            ),
            ExprKind::Struct { adt, fields } => self.check_struct_literal(adt, &fields, flow, span),
            ExprKind::Unary { op, operand } => self.check_unary(op, operand, expected, flow, span),
            ExprKind::Binary { op, lhs, rhs } => {
                self.check_binary(op, lhs, rhs, expected, flow, span)
            }
            ExprKind::Assign { op, target, value } => {
                self.check_assign(op, target, value, flow, span)
            }
            ExprKind::Update {
                op,
                target,
                postfix: _,
            } => self.check_update(op, target, flow, span),
            ExprKind::Call { callee, args } => self.check_call(callee, &args, flow, span),
            ExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => self.check_if(condition, then_branch, else_branch, expected, flow, span),
            ExprKind::Error => self.error_ty(),
        };

        self.results.record_expr(expr, ty);
        ty
    }

    fn check_path(
        &mut self,
        expr: HirExprId,
        res: Res,
        span: Span,
        flow: &Flow,
        mode: PlaceMode,
    ) -> TyId {
        match res {
            Res::Local(local) => {
                let ty = self.local_type(local);
                self.results.record_place(
                    expr,
                    ResolvedPlace {
                        local,
                        root_ty: ty,
                        projections: Vec::new(),
                        ty,
                    },
                );
                if mode == PlaceMode::Read && !self.is_error(ty) {
                    self.require_initialized(expr, flow, span);
                }
                ty
            }
            Res::Function(_) => {
                self.report_function_as_value(span);
                self.error_ty()
            }
        }
    }

    fn check_field(
        &mut self,
        expr: HirExprId,
        access: FieldAccess,
        span: Span,
        flow: &mut Flow,
        mode: PlaceMode,
    ) -> TyId {
        let FieldAccess {
            base,
            name,
            name_span,
        } = access;
        // The base of a field is a place, so infer it without requiring the
        // whole place to be initialized: only the selected field is read.
        let base_ty = self.infer_expr_mode(base, None, flow, PlaceMode::Write);
        let (ty, field) = match self.hir.ty_ctxt().kind(base_ty) {
            TyKind::Adt(adt) => match self.hir.ty_ctxt().field_by_name(adt, name) {
                Some(field) => (field.ty, Some(field.def)),
                None => {
                    self.error(
                        seal_error_codes::E2007,
                        name_span,
                        format!(
                            "the type `{}` has no field `{}`",
                            self.ty_text(base_ty),
                            self.interner.resolve(name)
                        ),
                    )
                    .help("check the field name against the struct declaration")
                    .emit();
                    (self.error_ty(), None)
                }
            },
            TyKind::Ref(_) => {
                self.error(
                    seal_error_codes::E3015,
                    span,
                    "cannot access a field through a shared reference",
                )
                .help("dereference it first, as in `(*value).field`")
                .emit();
                (self.error_ty(), None)
            }
            TyKind::Error => (base_ty, None),
            _ => {
                self.error(
                    seal_error_codes::E3015,
                    span,
                    format!("a value of type `{}` has no fields", self.ty_text(base_ty)),
                )
                .help("only struct values have named fields")
                .emit();
                (self.error_ty(), None)
            }
        };

        if let (Some(field), Some(mut place)) = (field, self.results.place(base).cloned()) {
            place.projections.push(PlaceProjection::Field(field));
            place.ty = ty;
            self.results.record_place(expr, place);
        }
        if mode == PlaceMode::Read && !self.is_error(ty) {
            self.require_initialized(expr, flow, span);
        }
        ty
    }

    fn check_struct_literal(
        &mut self,
        definition: seal_def::DefId,
        fields: &[(seal_def::DefId, HirExprId)],
        flow: &mut Flow,
        span: Span,
    ) -> TyId {
        let Some(adt) = self.hir.adt(definition) else {
            return self.error_ty();
        };
        let ty = self.hir.ty_ctxt().adt_ty(adt);
        for (field, value) in fields {
            let field_ty = match self.hir.ty_ctxt().field(adt, *field) {
                Some(field) => field.ty,
                None => {
                    self.error(
                        seal_error_codes::E2007,
                        span,
                        "this struct literal names a field that no longer exists",
                    )
                    .emit();
                    self.error_ty()
                }
            };
            self.check_expr(*value, field_ty, flow);
        }
        ty
    }

    /// Checks that `condition` has type `bool`.
    pub(super) fn check_condition(&mut self, condition: HirExprId, flow: &mut Flow) {
        let ty = self.infer_expr(condition, Some(self.bool_ty()), flow);
        if !self.is_bool(ty) && !self.is_error(ty) && !self.is_never(ty) {
            self.error(
                seal_error_codes::E3003,
                self.hir.expr(condition).span,
                format!("expected `bool`, found `{}`", self.ty_text(ty)),
            )
            .help("conditions must have type `bool`")
            .emit();
        }
    }

    fn check_if(
        &mut self,
        condition: HirExprId,
        then_branch: HirBlockId,
        else_branch: Option<HirBlockId>,
        expected: Option<TyId>,
        flow: &mut Flow,
        span: Span,
    ) -> TyId {
        self.check_condition(condition, flow);

        let Some(else_branch) = else_branch else {
            let otherwise = flow.clone();
            let then_out = self.check_block(then_branch, None, flow.clone());
            let result = match expected {
                Some(expected) if expected != self.unit() && !self.is_error(expected) => {
                    self.mismatch(span, expected, self.unit());
                    if self.is_error(then_out.ty) {
                        self.error_ty()
                    } else {
                        expected
                    }
                }
                Some(expected) => {
                    if !self.is_unit(then_out.ty)
                        && !self.is_never(then_out.ty)
                        && !self.is_error(then_out.ty)
                    {
                        self.report_if_without_else(then_branch, then_out.ty);
                    }
                    expected
                }
                None => {
                    if !self.is_unit(then_out.ty)
                        && !self.is_never(then_out.ty)
                        && !self.is_error(then_out.ty)
                    {
                        self.report_if_without_else(then_branch, then_out.ty);
                    }
                    self.unit()
                }
            };
            flow.merge(then_out.flow, otherwise);
            return result;
        };

        let then_out = self.check_block(then_branch, expected, flow.clone());
        let else_expected = match expected {
            Some(expected) => Some(expected),
            None if self.is_never(then_out.ty) => None,
            None => Some(then_out.ty),
        };
        let else_out = self.check_block(else_branch, else_expected, flow.clone());
        flow.merge(then_out.flow, else_out.flow);

        if let Some(expected) = expected {
            if self.is_error(then_out.ty) || self.is_error(else_out.ty) {
                return self.error_ty();
            }
            return expected;
        }

        if self.is_never(then_out.ty) {
            else_out.ty
        } else if self.is_never(else_out.ty) {
            then_out.ty
        } else if self.is_error(then_out.ty) || self.is_error(else_out.ty) {
            self.error_ty()
        } else if then_out.ty == else_out.ty {
            then_out.ty
        } else {
            // `check_block` already reported the mismatch against `then_out.ty`.
            self.error_ty()
        }
    }

    fn report_if_without_else(&self, then_branch: HirBlockId, found: TyId) {
        self.error(
            seal_error_codes::E3001,
            self.hir.block(then_branch).span,
            format!(
                "`if` without `else` requires the branch to have type `()`, found `{}`",
                self.ty_text(found)
            ),
        )
        .help("add an `else` branch, or make the branch evaluate to `()`")
        .emit();
    }
}
