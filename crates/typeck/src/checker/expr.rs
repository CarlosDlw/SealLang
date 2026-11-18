//! Expression checking.

use seal_ast::Literal;
use seal_hir::{ExprKind, HirBlockId, HirExprId, HirPatternId, Res};
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

/// One struct literal under construction.
struct StructLiteral<'a> {
    expr: HirExprId,
    definition: seal_def::DefId,
    explicit: Option<seal_ty::SubstsId>,
    fields: &'a [(seal_def::DefId, HirExprId)],
    expected: Option<TyId>,
    span: Span,
}

/// One enum variant literal under construction.
struct VariantLiteral<'a> {
    expr: HirExprId,
    definition: seal_def::DefId,
    variant: seal_ty::VariantIdx,
    explicit: Option<seal_ty::SubstsId>,
    fields: &'a [(seal_def::DefId, HirExprId)],
    expected: Option<TyId>,
    span: Span,
}

impl Checker<'_> {
    /// Checks `expr` against `expected`, reporting a mismatch at the expression.
    ///
    /// Every expected-type boundary routes through here, so a trait-object
    /// coercion is selected exactly once and recorded on the expression.
    pub(super) fn check_expr(&mut self, expr: HirExprId, expected: TyId, flow: &mut Flow) {
        self.check_expr_against(expr, expected, flow);
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

    /// Rejects reading a non-`Copy` pointee by value through a reference.
    ///
    /// A move would leave the storage the reference reaches without a value,
    /// and this function does not own that storage.
    pub(super) fn report_move_out_of_reference(&mut self, pointee: TyId, span: Span) {
        self.error(
            seal_error_codes::E3018,
            span,
            format!(
                "cannot move out of a reference to `{}`",
                self.ty_text(pointee)
            ),
        )
        .help("borrow the pointee instead, as in `&*reference`, or read its fields one by one")
        .emit();
    }

    pub(super) fn infer_expr_mode(
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
        // A parameter may already have been bound by an expected return type;
        // passing the resolved type lets literals adopt it instead of falling
        // back to their default and then failing unification.
        let expected = expected.map(|expected| self.resolve(expected));

        let ty = match kind {
            ExprKind::Lit(literal) => self.literal_ty(literal, expected, span, false),
            ExprKind::Path { res, .. } => self.check_path(expr, res, span, flow, mode),
            ExprKind::Borrow { operand, kind } => {
                self.check_borrow(expr, operand, kind, expected, flow)
            }
            ExprKind::Deref { operand } => {
                let ty = self.check_deref(operand, flow, span);
                let deref = self.reference_kind(operand);
                if let Some(mut place) = self.results.place(operand).cloned() {
                    place.projections.push(PlaceProjection::Deref);
                    place.ty = ty;
                    place.deref = deref;
                    self.results.record_place(expr, place);
                }
                if mode == PlaceMode::Read && !self.is_error(ty) && !self.is_copy(ty) {
                    self.report_move_out_of_reference(ty, span);
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
            ExprKind::TupleField {
                base,
                index,
                index_span,
            } => self.check_tuple_field(expr, base, index, index_span, flow, mode),
            ExprKind::Tuple { elements } => self.check_tuple(&elements, expected, flow),
            ExprKind::Struct { adt, args, fields } => self.check_struct_literal(
                StructLiteral {
                    expr,
                    definition: adt,
                    explicit: args,
                    fields: &fields,
                    expected,
                    span,
                },
                flow,
            ),
            ExprKind::Variant {
                adt,
                variant,
                args,
                fields,
            } => self.check_variant_literal(
                VariantLiteral {
                    expr,
                    definition: adt,
                    variant,
                    explicit: args,
                    fields: &fields,
                    expected,
                    span,
                },
                flow,
            ),
            ExprKind::Array { elements } => self.check_array(&elements, expected, flow, span),
            ExprKind::ArrayRepeat { value, length } => {
                self.check_array_repeat(value, length, expected, flow, span)
            }
            ExprKind::Index { base, index } => {
                self.check_index(expr, base, index, span, flow, mode)
            }
            ExprKind::Slice { base, start, end } => self.check_slice(base, start, end, span, flow),
            ExprKind::SliceRef {
                base,
                start,
                end,
                kind,
            } => self.check_slice_ref(base, start, end, kind, span, flow),
            ExprKind::Len { operand } => self.check_len(operand, span, flow),
            ExprKind::Unary { op, operand } => self.check_unary(op, operand, expected, flow, span),
            ExprKind::Binary { op, lhs, rhs } => {
                self.check_binary(op, lhs, rhs, expected, flow, span)
            }
            ExprKind::Cast { operand, .. } => {
                // Numeric cast checking lands with the cast diagnostic phase;
                // the operand is still inferred so its own errors surface.
                let _ = self.infer_expr(operand, None, flow);
                self.error_ty()
            }
            ExprKind::Assign { op, target, value } => {
                self.check_assign(op, target, value, flow, span)
            }
            ExprKind::Update {
                op,
                target,
                postfix: _,
            } => self.check_update(op, target, flow, span),
            ExprKind::Call { callee, args } => {
                self.check_call(expr, callee, &args, expected, flow, span)
            }
            ExprKind::MethodCall {
                receiver,
                name,
                name_span,
                generic_args,
                args,
            } => self.check_method_call(
                super::method::MethodCall {
                    expr,
                    receiver,
                    name,
                    name_span,
                    generic_args,
                    args: &args,
                    span,
                },
                expected,
                flow,
            ),
            ExprKind::Match { scrutinee, arms } => {
                self.check_match(scrutinee, &arms, expected, flow, span)
            }
            ExprKind::Block(block) => {
                let outcome = self.check_block(block, expected, flow.clone());
                *flow = outcome.flow;
                outcome.ty
            }
            ExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => self.check_if(condition, then_branch, else_branch, expected, flow, span),
            ExprKind::IfLet {
                pattern,
                scrutinee,
                then_branch,
                else_branch,
            } => self.check_if_let(
                (pattern, scrutinee),
                then_branch,
                else_branch,
                expected,
                flow,
                span,
            ),
            ExprKind::Error => self.error_ty(),
        };

        let ty = self.resolve_deep(ty);
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
                let moved_in_guard = self
                    .guard_moves
                    .as_mut()
                    .is_some_and(|moves| moves.remove(&local));
                if moved_in_guard && !self.is_error(ty) {
                    self.error(
                        seal_error_codes::E2045,
                        span,
                        format!(
                            "the guard uses `{}`, but its type `{}` is not `Copy`",
                            self.def_name(local),
                            self.ty_text(ty)
                        ),
                    )
                    .help("move the check into the arm body, or match a `Copy` field")
                    .emit();
                }
                self.results.record_place(
                    expr,
                    ResolvedPlace {
                        local,
                        root_ty: ty,
                        projections: Vec::new(),
                        ty,
                        deref: None,
                    },
                );
                if matches!(mode, PlaceMode::Read | PlaceMode::Borrow) && !self.is_error(ty) {
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
            TyKind::Adt { adt, args } => match self.hir.ty_ctxt().field_by_name(adt, name) {
                Some(field) => (
                    self.hir.ty_ctxt().substitute(field.ty, args),
                    Some(field.def),
                ),
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
            TyKind::Ref { .. } => {
                self.error(
                    seal_error_codes::E3015,
                    span,
                    "cannot access a field through a reference",
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
        if matches!(mode, PlaceMode::Read | PlaceMode::Borrow) && !self.is_error(ty) {
            self.require_initialized(expr, flow, span);
        }
        ty
    }

    fn check_tuple_field(
        &mut self,
        expr: HirExprId,
        base: HirExprId,
        index: u32,
        index_span: Span,
        flow: &mut Flow,
        mode: PlaceMode,
    ) -> TyId {
        let base_ty = self.infer_expr_mode(base, None, flow, PlaceMode::Write);
        let (ty, projected) = match self.hir.ty_ctxt().kind(base_ty) {
            TyKind::Tuple(tuple) => {
                let arity = self.hir.ty_ctxt().tuple_len(tuple);
                match self.hir.ty_ctxt().tuple_element(tuple, index as usize) {
                    Some(element) => (element, true),
                    None => {
                        self.report_tuple_index(index, arity, index_span);
                        (self.error_ty(), false)
                    }
                }
            }
            TyKind::Unit => {
                self.error(
                    seal_error_codes::E3036,
                    index_span,
                    "the unit value `()` has no tuple elements",
                )
                .help("the empty tuple cannot be indexed")
                .emit();
                (self.error_ty(), false)
            }
            TyKind::Ref { .. } => {
                self.error(
                    seal_error_codes::E3015,
                    index_span,
                    "cannot access a tuple element through a reference",
                )
                .help("dereference it first, as in `(*value).0`")
                .emit();
                (self.error_ty(), false)
            }
            TyKind::Error => (base_ty, false),
            _ => {
                self.error(
                    seal_error_codes::E3015,
                    index_span,
                    format!(
                        "a value of type `{}` has no tuple elements",
                        self.ty_text(base_ty)
                    ),
                )
                .help("numeric field access is only valid on tuples")
                .emit();
                (self.error_ty(), false)
            }
        };

        if projected && let Some(mut place) = self.results.place(base).cloned() {
            place.projections.push(PlaceProjection::TupleField(index));
            place.ty = ty;
            self.results.record_place(expr, place);
        }
        if matches!(mode, PlaceMode::Read | PlaceMode::Borrow) && !self.is_error(ty) {
            self.require_initialized(expr, flow, index_span);
        }
        ty
    }

    fn report_tuple_index(&self, index: u32, arity: usize, span: Span) {
        self.error(
            seal_error_codes::E3036,
            span,
            format!("tuple index `.{index}` is out of bounds for a tuple with {arity} element(s)"),
        )
        .help(if arity == 0 {
            "the unit value `()` has no elements".to_owned()
        } else {
            format!("valid indices are `0` through `{}`", arity - 1)
        })
        .emit();
    }

    fn check_tuple(
        &mut self,
        elements: &[HirExprId],
        expected: Option<TyId>,
        flow: &mut Flow,
    ) -> TyId {
        let expected_elements =
            expected.and_then(|expected| match self.hir.ty_ctxt().kind(expected) {
                TyKind::Tuple(tuple) => {
                    let declared = self.hir.ty_ctxt().tuple_elements(tuple);
                    (declared.len() == elements.len()).then_some(declared)
                }
                TyKind::Unit if elements.is_empty() => Some(Vec::new()),
                _ => None,
            });

        for (position, element) in elements.iter().enumerate() {
            match expected_elements
                .as_ref()
                .and_then(|types| types.get(position))
            {
                Some(expected) => self.check_expr(*element, *expected, flow),
                None => {
                    self.infer_expr(*element, None, flow);
                }
            }
        }

        if elements.is_empty() {
            return self.unit();
        }
        let types: Vec<TyId> = elements
            .iter()
            .map(|element| {
                self.results.expr_ty(*element).unwrap_or_else(|| {
                    panic!("a checked tuple element has no type; this is a compiler bug")
                })
            })
            .collect();
        self.hir.ty_ctxt().tuple_ty(&types)
    }

    fn check_array(
        &mut self,
        elements: &[HirExprId],
        expected: Option<TyId>,
        flow: &mut Flow,
        span: Span,
    ) -> TyId {
        let expected_parts = expected.and_then(|ty| self.array_parts(ty));
        if elements.is_empty() {
            if let Some((element, length)) = expected_parts {
                let expected = expected.expect("an array with a known element type");
                if length != 0 {
                    self.mismatch(span, expected, self.array_ty(element, 0));
                    return self.error_ty();
                }
                return expected;
            }
            if let Some(expected) = expected {
                self.mismatch(span, expected, self.array_ty(self.error_ty(), 0));
                return self.error_ty();
            }
            self.error(
                seal_error_codes::E3019,
                span,
                "cannot infer the element type of an empty array",
            )
            .help("annotate the binding with an array type, such as `[i32; 0]`")
            .emit();
            return self.error_ty();
        }

        let element_expected = expected_parts.map(|(element, _)| element);
        let first = match element_expected {
            Some(expected) => self.check_expr_against(elements[0], expected, flow),
            None => self.infer_expr(elements[0], None, flow),
        };
        let element_ty = element_expected.unwrap_or(first);
        for element in &elements[1..] {
            self.check_expr(*element, element_ty, flow);
        }

        let actual = self.array_ty(element_ty, elements.len() as u64);
        if let Some((_, length)) = expected_parts
            && length != elements.len() as u64
        {
            self.mismatch(span, expected.expect("an expected array type"), actual);
            return self.error_ty();
        }
        if expected.is_none() && self.array_exceeds_address_space(actual) {
            self.report_array_too_large(span, actual, "array literal");
            return self.error_ty();
        }
        actual
    }

    fn check_array_repeat(
        &mut self,
        value: HirExprId,
        length: u64,
        expected: Option<TyId>,
        flow: &mut Flow,
        span: Span,
    ) -> TyId {
        let expected_parts = expected.and_then(|ty| self.array_parts(ty));
        let element_expected = expected_parts.map(|(element, _)| element);
        let value_ty = match element_expected {
            Some(expected) => self.check_expr_against(value, expected, flow),
            None => self.infer_expr(value, None, flow),
        };
        if !self.is_error(value_ty) && !self.is_copy(value_ty) {
            self.error(
                seal_error_codes::E3020,
                self.hir.expr(value).span,
                format!(
                    "cannot repeat a value of type `{}` because it is not `Copy`",
                    self.ty_text(value_ty)
                ),
            )
            .help("build each element separately, or use a `Copy` element type")
            .emit();
        }

        let actual = self.array_ty(value_ty, length);
        if let Some((_, expected_length)) = expected_parts
            && expected_length != length
        {
            self.mismatch(span, expected.expect("an expected array type"), actual);
            return self.error_ty();
        }
        if expected.is_none() && self.array_exceeds_address_space(actual) {
            self.report_array_too_large(span, actual, "array repeat");
            return self.error_ty();
        }
        actual
    }

    fn check_index(
        &mut self,
        expr: HirExprId,
        base: HirExprId,
        index: HirExprId,
        span: Span,
        flow: &mut Flow,
        mode: PlaceMode,
    ) -> TyId {
        let mut base_ty = self.infer_expr_mode(base, None, flow, PlaceMode::Write);
        let mut auto_deref = None;
        if let TyKind::Ref { kind, pointee } = self.hir.ty_ctxt().kind(base_ty)
            && matches!(
                self.hir.ty_ctxt().kind(pointee),
                TyKind::Array { .. } | TyKind::Slice { .. }
            )
        {
            auto_deref = Some(kind);
            base_ty = pointee;
        }

        let (element, length, dynamic_length) = match self.hir.ty_ctxt().kind(base_ty) {
            TyKind::Array { element, length } => (element, length, false),
            TyKind::Slice { element } => (element, 0, true),
            _ => {
                self.check_expr(index, self.usize_ty(), flow);
                if !self.is_error(base_ty) {
                    self.error(
                        seal_error_codes::E3021,
                        span,
                        format!("cannot index a value of type `{}`", self.ty_text(base_ty)),
                    )
                    .help("only arrays and slices can be indexed")
                    .emit();
                }
                return self.error_ty();
            }
        };

        let index_ty = self.infer_expr(index, Some(self.usize_ty()), flow);
        if index_ty != self.usize_ty() && !self.is_error(index_ty) {
            self.mismatch(self.hir.expr(index).span, self.usize_ty(), index_ty);
        }

        let constant = if dynamic_length {
            None
        } else {
            self.constant_index(index, length)
        };

        if mode == PlaceMode::Read
            && auto_deref.is_some()
            && !self.is_error(element)
            && !self.is_copy(element)
        {
            self.report_move_out_of_reference(element, span);
        }

        if let Some(mut place) = self.results.place(base).cloned() {
            if let Some(kind) = auto_deref {
                place.projections.push(PlaceProjection::Deref);
                place.deref = Some(kind);
            }
            place.projections.push(PlaceProjection::Index {
                index,
                constant,
                length,
                dynamic_length,
            });
            place.ty = element;
            self.results.record_place(expr, place);
        }
        if matches!(mode, PlaceMode::Read | PlaceMode::Borrow)
            && !self.is_error(element)
            && index_ty == self.usize_ty()
        {
            self.require_initialized(expr, flow, span);
        }
        element
    }

    /// Returns the constant value of an index, reporting an out-of-range literal.
    fn constant_index(&self, index: HirExprId, length: u64) -> Option<u64> {
        let index_span = self.hir.expr(index).span;
        let ExprKind::Lit(Literal::Int {
            value, ref suffix, ..
        }) = self.hir.expr(index).kind
        else {
            return None;
        };
        if !suffix.is_none_or(|suffix| self.interner.resolve(suffix) == "usize") {
            return None;
        }
        if value >= u128::from(length) {
            self.error(
                seal_error_codes::E3022,
                index_span,
                format!("array index `{value}` is out of bounds for an array of length {length}"),
            )
            .help(if length == 0 {
                "an array of length 0 has no valid indices"
            } else {
                "the last valid index is `length - 1`"
            })
            .emit();
            None
        } else {
            Some(value as u64)
        }
    }

    fn check_struct_literal(&mut self, literal: StructLiteral<'_>, flow: &mut Flow) -> TyId {
        let StructLiteral {
            expr,
            definition,
            explicit,
            fields,
            expected,
            span,
        } = literal;
        let Some(adt) = self.hir.adt(definition) else {
            return self.error_ty();
        };
        let generics = self.aggregate_generics(definition);
        let args = self.aggregate_arguments(adt, &generics, explicit, expected);
        let substs = self.hir.ty_ctxt().substs(&args);
        for (field, value) in fields {
            let field_ty = match self.hir.ty_ctxt().field(adt, *field) {
                Some(field) => self.substitute(field.ty, substs),
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
        let ty = self.hir.ty_ctxt().adt_ty_with(adt, substs);
        if let Some(expected) = expected {
            self.mismatch(span, expected, ty);
        }
        let Some(args) =
            self.finish_aggregate_instantiation(expr, definition, &generics, &args, span)
        else {
            return self.error_ty();
        };
        self.hir
            .ty_ctxt()
            .adt_ty_with(adt, self.hir.ty_ctxt().substs(&args))
    }

    /// Checks a constructed enum variant against its declared fields.
    fn check_variant_literal(&mut self, literal: VariantLiteral<'_>, flow: &mut Flow) -> TyId {
        let VariantLiteral {
            expr,
            definition,
            variant,
            explicit,
            fields,
            expected,
            span,
        } = literal;
        let Some(adt) = self.hir.adt(definition) else {
            return self.error_ty();
        };
        let Some(variant_def) = self.hir.ty_ctxt().variant(adt, variant) else {
            self.error(
                seal_error_codes::E2032,
                span,
                "this enum variant no longer exists",
            )
            .emit();
            return self.error_ty();
        };
        let generics = self.aggregate_generics(definition);
        let args = self.aggregate_arguments(adt, &generics, explicit, expected);
        let substs = self.hir.ty_ctxt().substs(&args);
        for (field, value) in fields {
            let field_ty = match variant_def.field(*field) {
                Some(field) => self.substitute(field.ty, substs),
                None => {
                    self.error(
                        seal_error_codes::E2007,
                        span,
                        "this variant literal names a field that no longer exists",
                    )
                    .emit();
                    self.error_ty()
                }
            };
            self.check_expr(*value, field_ty, flow);
        }
        let ty = self.hir.ty_ctxt().adt_ty_with(adt, substs);
        if let Some(expected) = expected {
            self.mismatch(span, expected, ty);
        }
        let Some(args) =
            self.finish_aggregate_instantiation(expr, definition, &generics, &args, span)
        else {
            return self.error_ty();
        };
        self.hir
            .ty_ctxt()
            .adt_ty_with(adt, self.hir.ty_ctxt().substs(&args))
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
        let start = flow.clone();

        let Some(else_branch) = else_branch else {
            let then_out = self.check_block(then_branch, None, start.clone());
            return self.finish_if_without_else(
                "if",
                (then_branch, then_out),
                expected,
                flow,
                start,
                span,
            );
        };

        let then_out = self.check_block(then_branch, expected, start.clone());
        let else_expected = match expected {
            Some(expected) => Some(expected),
            None if self.is_never(then_out.ty) => None,
            None => Some(then_out.ty),
        };
        let else_out = self.check_block(else_branch, else_expected, start);
        self.finish_if_with_else(then_out, else_out, expected, flow)
    }

    /// Checks an `if let` expression.
    ///
    /// The scrutinee is evaluated once. The pattern may be refutable; when it
    /// matches, its bindings are initialized in the then branch only. The else
    /// branch starts from the state before the pattern, because no binding is
    /// available when the match fails.
    fn check_if_let(
        &mut self,
        (pattern, scrutinee): (HirPatternId, HirExprId),
        then_branch: HirBlockId,
        else_branch: Option<HirBlockId>,
        expected: Option<TyId>,
        flow: &mut Flow,
        span: Span,
    ) -> TyId {
        let scrutinee_ty = self.infer_expr(scrutinee, None, flow);
        let start = flow.clone();
        let mut matched = start.clone();
        self.check_pattern(pattern, scrutinee_ty, &mut matched);

        let Some(else_branch) = else_branch else {
            let then_out = self.check_block(then_branch, None, matched);
            return self.finish_if_without_else(
                "if let",
                (then_branch, then_out),
                expected,
                flow,
                start,
                span,
            );
        };

        let then_out = self.check_block(then_branch, expected, matched);
        let else_expected = match expected {
            Some(expected) => Some(expected),
            None if self.is_never(then_out.ty) => None,
            None => Some(then_out.ty),
        };
        let else_out = self.check_block(else_branch, else_expected, start);
        self.finish_if_with_else(then_out, else_out, expected, flow)
    }

    /// Combines the branches of a conditional that has no `else`, where the
    /// missing branch produces `()`.
    fn finish_if_without_else(
        &mut self,
        construct: &str,
        (then_branch, then_out): (HirBlockId, super::stmt::BlockOutcome),
        expected: Option<TyId>,
        flow: &mut Flow,
        otherwise: Flow,
        span: Span,
    ) -> TyId {
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
                    self.report_if_without_else(construct, then_branch, then_out.ty);
                }
                expected
            }
            None => {
                if !self.is_unit(then_out.ty)
                    && !self.is_never(then_out.ty)
                    && !self.is_error(then_out.ty)
                {
                    self.report_if_without_else(construct, then_branch, then_out.ty);
                }
                self.unit()
            }
        };
        flow.merge(then_out.flow, otherwise);
        result
    }

    /// Combines the branches of a conditional that has an `else`.
    fn finish_if_with_else(
        &mut self,
        then_out: super::stmt::BlockOutcome,
        else_out: super::stmt::BlockOutcome,
        expected: Option<TyId>,
        flow: &mut Flow,
    ) -> TyId {
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

    fn report_if_without_else(&self, construct: &str, then_branch: HirBlockId, found: TyId) {
        self.error(
            seal_error_codes::E3001,
            self.hir.block(then_branch).span,
            format!(
                "`{construct}` without `else` requires the branch to have type `()`, found `{}`",
                self.ty_text(found)
            ),
        )
        .help("add an `else` branch, or make the branch evaluate to `()`")
        .emit();
    }
}
