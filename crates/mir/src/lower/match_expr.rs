//! Lowering `match` expressions.
//!
//! The scrutinee is evaluated once into a temporary. Each arm is then a chain of
//! tests over that temporary. An or-pattern is expanded into alternatives before
//! lowering, so every alternative has one concrete sequence of field places.
//! Bindings are assigned only after the whole pattern matched: assigning a
//! non-`Copy` field earlier would move it out of the scrutinee even when a later
//! test failed and a later arm needed the value.
//!
//! A guard runs after the pattern matched but before the arm commits. Only
//! `Copy` bindings are assigned before the guard; type checking rejects a guard
//! that reads any other binding.

use seal_ast::PatternLiteral;
use seal_def::DefId;
use seal_hir::{HirPatternId, MatchArm, PatternKind};
use seal_span::Span;
use seal_ty::{AdtId, TyId, TyKind, VariantIdx};

use super::BodyLowering;
use crate::body::{LocalDecl, LocalKind};
use crate::id::{BasicBlockId, LocalId};
use crate::operand::{Constant, Operand};
use crate::place::Place;
use crate::rvalue::{BinOp, Rvalue};
use crate::terminator::{SwitchTarget, TerminatorKind};

/// A pattern with or-patterns expanded and every enum and struct resolved.
#[derive(Clone, Debug)]
pub(super) struct FlatPattern {
    kind: FlatPatternKind,
    span: Span,
}

#[derive(Clone, Debug)]
enum FlatPatternKind {
    Wildcard,
    Binding {
        def: DefId,
        pattern: Option<Box<FlatPattern>>,
    },
    Literal(PatternLiteral),
    Range {
        start: PatternLiteral,
        end: PatternLiteral,
        inclusive: bool,
    },
    Variant {
        adt: AdtId,
        variant: VariantIdx,
        fields: Vec<(DefId, FlatPattern)>,
    },
    Struct {
        adt: AdtId,
        fields: Vec<(DefId, FlatPattern)>,
    },
    Tuple(Vec<FlatPattern>),
    Array {
        prefix: Vec<FlatPattern>,
        suffix: Vec<FlatPattern>,
        has_rest: bool,
    },
}

/// The fixed parts of an array or slice pattern.
struct ArrayParts<'a> {
    prefix: &'a [FlatPattern],
    suffix: &'a [FlatPattern],
    has_rest: bool,
    span: Span,
}

impl BodyLowering<'_> {
    /// Lowers one `match` expression.
    pub(super) fn lower_match(
        &mut self,
        scrutinee: seal_hir::HirExprId,
        arms: &[MatchArm],
        dest: Option<Place>,
        span: Span,
    ) {
        let ty = self.expr_ty(scrutinee);
        let subject = self.new_temp(ty, span);
        self.lower_expr_into(scrutinee, Some(subject.clone()));

        let start = self
            .current
            .expect("a match expression follows a reachable scrutinee");
        let join = self.new_block();
        let dead = self.new_block();
        let saved = self.current;
        self.current = Some(dead);
        self.terminate(TerminatorKind::Unreachable, span);
        self.current = saved;

        let mut entry = start;
        for (index, arm) in arms.iter().enumerate() {
            let last = index + 1 == arms.len();
            let next_arm = if last { dead } else { self.new_block() };
            let owned = self.declare_pattern_bindings(arm.pattern);

            let mut alternatives = self.expand_pattern(arm.pattern);
            if alternatives.is_empty() {
                self.current = Some(entry);
                self.terminate_goto(next_arm, span);
                entry = next_arm;
                continue;
            }
            let mut fails = Vec::with_capacity(alternatives.len().saturating_sub(1));
            for _ in 0..alternatives.len().saturating_sub(1) {
                fails.push(self.new_block());
            }

            let mut alternative_entry = entry;
            for (position, alternative) in alternatives.drain(..).enumerate() {
                let fail = fails.get(position).copied().unwrap_or(next_arm);
                let mut bindings = Vec::new();
                self.current = Some(alternative_entry);
                // The bindings live as long as the arm: they are dropped when it
                // ends, after the locals its body declared.
                self.push_scope(owned.clone());
                self.lower_flat_pattern(&alternative, subject.clone(), ty, fail, &mut bindings);

                if let Some(guard) = arm.guard {
                    self.lower_guarded_arm(&bindings, guard, fail, arm.span);
                } else {
                    self.bind_all(&bindings, arm.span);
                }
                self.lower_arm_body(arm.body, dest.clone(), join, arm.span);
                self.pop_scope(arm.span);

                alternative_entry = fail;
            }

            self.current = Some(next_arm);
            entry = next_arm;
        }

        self.current = Some(join);
    }

    /// Lowers one arm body and routes it to the join block.
    fn lower_arm_body(
        &mut self,
        body: seal_hir::HirExprId,
        dest: Option<Place>,
        join: BasicBlockId,
        span: Span,
    ) {
        self.lower_expr_into(body, dest);
        if self.current.is_some() {
            self.terminate_goto(join, span);
        }
    }

    /// Emits binding assignments for an unguarded arm.
    pub(super) fn bind_all(&mut self, bindings: &[(DefId, Place)], span: Span) {
        for (definition, place) in bindings {
            self.bind_one(*definition, place.clone(), span);
        }
    }

    fn bind_one(&mut self, definition: DefId, place: Place, span: Span) {
        let ty = self.expr_or_local_ty(definition);
        let local = *self
            .local_of
            .get(&definition)
            .expect("every pattern binding received a MIR local");
        let operand = self.place_operand(place, ty);
        self.assign_operand(Place::local(local), operand, span);
    }

    fn lower_guarded_arm(
        &mut self,
        bindings: &[(DefId, Place)],
        guard: seal_hir::HirExprId,
        fail: BasicBlockId,
        span: Span,
    ) {
        for (definition, place) in bindings {
            let ty = self.expr_or_local_ty(*definition);
            if self.hir.ty_ctxt().is_copy(ty) {
                self.bind_one(*definition, place.clone(), span);
            }
        }

        let guard_operand = self.lower_operand(guard);
        let pass = self.new_block();
        self.terminate(
            TerminatorKind::SwitchInt {
                discriminant: guard_operand,
                targets: vec![SwitchTarget {
                    value: 1,
                    target: pass,
                }],
                otherwise: fail,
            },
            span,
        );
        self.current = Some(pass);
        for (definition, place) in bindings {
            let ty = self.expr_or_local_ty(*definition);
            if !self.hir.ty_ctxt().is_copy(ty) {
                self.bind_one(*definition, place.clone(), span);
            }
        }
    }

    /// Declares a MIR local per pattern binding and returns them in order.
    ///
    /// The caller hands the returned locals to the scope that owns them: a
    /// pattern binding is dropped when the block or arm that bound it ends.
    pub(super) fn declare_pattern_bindings(&mut self, pattern: HirPatternId) -> Vec<LocalId> {
        let mut definitions = Vec::new();
        self.collect_binding_defs(pattern, &mut definitions);
        let mut locals = Vec::with_capacity(definitions.len());
        for definition in definitions {
            if let Some(local) = self.local_of.get(&definition) {
                locals.push(*local);
                continue;
            }
            let data = self.hir.definitions().get(definition);
            let ty = self.expr_or_local_ty(definition);
            let local = self.push_local(LocalDecl {
                ty,
                name: Some(data.name),
                kind: LocalKind::Binding,
                span: data.span,
            });
            self.local_of.insert(definition, local);
            locals.push(local);
        }
        locals
    }

    fn collect_binding_defs(&self, pattern: HirPatternId, out: &mut Vec<DefId>) {
        match &self.hir.pattern(pattern).kind {
            PatternKind::Binding { def, pattern, .. } => {
                out.push(*def);
                if let Some(pattern) = pattern {
                    self.collect_binding_defs(*pattern, out);
                }
            }
            PatternKind::Variant { fields, .. } | PatternKind::Struct { fields, .. } => {
                for (_, field) in fields {
                    self.collect_binding_defs(*field, out);
                }
            }
            PatternKind::Tuple(elements) => {
                for element in elements {
                    self.collect_binding_defs(*element, out);
                }
            }
            PatternKind::Array { prefix, suffix, .. } => {
                for element in prefix.iter().chain(suffix) {
                    self.collect_binding_defs(*element, out);
                }
            }
            PatternKind::Or(alternatives) => {
                for alternative in alternatives {
                    self.collect_binding_defs(*alternative, out);
                }
            }
            PatternKind::Wildcard
            | PatternKind::Literal(_)
            | PatternKind::Range { .. }
            | PatternKind::Error => {}
        }
    }

    /// Expands or-patterns into one flat pattern per alternative combination.
    pub(super) fn expand_pattern(&self, pattern: HirPatternId) -> Vec<FlatPattern> {
        let span = self.hir.pattern(pattern).span;
        match &self.hir.pattern(pattern).kind {
            PatternKind::Wildcard | PatternKind::Error => vec![FlatPattern {
                kind: FlatPatternKind::Wildcard,
                span,
            }],
            PatternKind::Binding { def, pattern, .. } => vec![FlatPattern {
                kind: FlatPatternKind::Binding {
                    def: *def,
                    pattern: pattern.map(|pattern| {
                        Box::new(self.expand_pattern(pattern).into_iter().next().unwrap_or(
                            FlatPattern {
                                kind: FlatPatternKind::Wildcard,
                                span,
                            },
                        ))
                    }),
                },
                span,
            }],
            PatternKind::Literal(literal) => vec![FlatPattern {
                kind: FlatPatternKind::Literal(*literal),
                span,
            }],
            PatternKind::Range {
                start,
                end,
                inclusive,
            } => vec![FlatPattern {
                kind: FlatPatternKind::Range {
                    start: *start,
                    end: *end,
                    inclusive: *inclusive,
                },
                span,
            }],
            PatternKind::Struct { adt, fields } => {
                let Some(adt_id) = self.hir.adt(*adt) else {
                    return vec![FlatPattern {
                        kind: FlatPatternKind::Wildcard,
                        span,
                    }];
                };
                let mut combinations: Vec<Vec<(DefId, FlatPattern)>> = vec![Vec::new()];
                for (field, child) in fields {
                    let options = self.expand_pattern(*child);
                    let mut next = Vec::new();
                    for prefix in &combinations {
                        for option in &options {
                            let mut combination = prefix.clone();
                            combination.push((*field, option.clone()));
                            next.push(combination);
                        }
                    }
                    combinations = next;
                }
                combinations
                    .into_iter()
                    .map(|fields| FlatPattern {
                        kind: FlatPatternKind::Struct {
                            adt: adt_id,
                            fields,
                        },
                        span,
                    })
                    .collect()
            }
            PatternKind::Tuple(elements) => self
                .expand_sequence(elements)
                .into_iter()
                .map(|fields| FlatPattern {
                    kind: FlatPatternKind::Tuple(fields),
                    span,
                })
                .collect(),
            PatternKind::Array {
                prefix,
                suffix,
                has_rest,
            } => {
                let prefixes = self.expand_sequence(prefix);
                let suffixes = self.expand_sequence(suffix);
                let mut combinations = Vec::new();
                for prefix in &prefixes {
                    for suffix in &suffixes {
                        combinations.push((prefix.clone(), suffix.clone()));
                    }
                }
                combinations
                    .into_iter()
                    .map(|(prefix, suffix)| FlatPattern {
                        kind: FlatPatternKind::Array {
                            prefix,
                            suffix,
                            has_rest: *has_rest,
                        },
                        span,
                    })
                    .collect()
            }
            PatternKind::Variant {
                adt,
                variant,
                fields,
            } => {
                let Some(adt) = self.hir.adt(*adt) else {
                    return vec![FlatPattern {
                        kind: FlatPatternKind::Wildcard,
                        span,
                    }];
                };
                let mut combinations: Vec<Vec<(DefId, FlatPattern)>> = vec![Vec::new()];
                for (field, child) in fields {
                    let options = self.expand_pattern(*child);
                    let mut next = Vec::new();
                    for prefix in &combinations {
                        for option in &options {
                            let mut combination = prefix.clone();
                            combination.push((*field, option.clone()));
                            next.push(combination);
                        }
                    }
                    combinations = next;
                }
                combinations
                    .into_iter()
                    .map(|fields| FlatPattern {
                        kind: FlatPatternKind::Variant {
                            adt,
                            variant: *variant,
                            fields,
                        },
                        span,
                    })
                    .collect()
            }
            PatternKind::Or(alternatives) => alternatives
                .iter()
                .flat_map(|alternative| self.expand_pattern(*alternative))
                .collect(),
        }
    }

    /// Expands a sequence of patterns into every alternative combination.
    fn expand_sequence(&self, patterns: &[HirPatternId]) -> Vec<Vec<FlatPattern>> {
        let mut combinations: Vec<Vec<FlatPattern>> = vec![Vec::new()];
        for pattern in patterns {
            let options = self.expand_pattern(*pattern);
            let mut next = Vec::new();
            for prefix in &combinations {
                for option in &options {
                    let mut combination = prefix.clone();
                    combination.push(option.clone());
                    next.push(combination);
                }
            }
            combinations = next;
        }
        combinations
    }

    /// Emits the tests for one flat alternative.
    ///
    /// On success the current block is open and every binding path has been
    /// recorded; on failure control jumps to `fail`.
    pub(super) fn lower_flat_pattern(
        &mut self,
        pattern: &FlatPattern,
        place: Place,
        ty: TyId,
        fail: BasicBlockId,
        bindings: &mut Vec<(DefId, Place)>,
    ) {
        match &pattern.kind {
            FlatPatternKind::Wildcard => {}
            FlatPatternKind::Binding { def, pattern } => {
                bindings.push((*def, place.clone()));
                if let Some(pattern) = pattern {
                    self.lower_flat_pattern(pattern, place, ty, fail, bindings);
                }
            }
            FlatPatternKind::Literal(literal) => {
                self.emit_comparison(place, BinOp::Eq, *literal, ty, fail, pattern.span);
            }
            FlatPatternKind::Range {
                start,
                end,
                inclusive,
            } => {
                self.emit_comparison(place.clone(), BinOp::Ge, *start, ty, fail, pattern.span);
                let op = if *inclusive { BinOp::Le } else { BinOp::Lt };
                self.emit_comparison(place, op, *end, ty, fail, pattern.span);
            }
            FlatPatternKind::Variant {
                adt,
                variant,
                fields,
            } => {
                let place = self.deref_pattern_place(place, ty);
                let Some(definition) = self.hir.ty_ctxt().variant(*adt, *variant) else {
                    self.current = Some(fail);
                    return;
                };
                let tag_ty = self
                    .hir
                    .ty_ctxt()
                    .lookup(TyKind::Uint(seal_ty::UintTy::U32))
                    .expect("HIR lowering interns every primitive type");
                let tag = self.new_temp(tag_ty, pattern.span);
                self.assign_rvalue(
                    tag.clone(),
                    Rvalue::Discriminant(place.clone()),
                    pattern.span,
                );
                let success = self.new_block();
                self.terminate(
                    TerminatorKind::SwitchInt {
                        discriminant: Operand::Copy(tag),
                        targets: vec![SwitchTarget {
                            value: u128::from(variant.index()),
                            target: success,
                        }],
                        otherwise: fail,
                    },
                    pattern.span,
                );
                self.current = Some(success);

                for (field, child) in fields {
                    let Some(field_def) = definition.field(*field) else {
                        self.current = Some(fail);
                        return;
                    };
                    let field_ty = field_def.ty;
                    let field_place = place.clone().variant_field(*variant, *field);
                    self.lower_flat_pattern(child, field_place, field_ty, fail, bindings);
                }
            }
            FlatPatternKind::Struct { adt, fields } => {
                let place = self.deref_pattern_place(place, ty);
                let definition = self.hir.ty_ctxt().adt(*adt);
                for (field, child) in fields {
                    let Some(field_def) = definition.field(*field) else {
                        self.current = Some(fail);
                        return;
                    };
                    let field_ty = field_def.ty;
                    let field_place = place.clone().field(*field);
                    self.lower_flat_pattern(child, field_place, field_ty, fail, bindings);
                }
            }
            FlatPatternKind::Tuple(fields) => {
                let mut place = place;
                let mut ty = ty;
                while let TyKind::Ref { pointee, .. } = self.hir.ty_ctxt().kind(ty) {
                    place = place.deref();
                    ty = pointee;
                }
                let elements = match self.hir.ty_ctxt().kind(ty) {
                    TyKind::Tuple(tuple) => self.hir.ty_ctxt().tuple_elements(tuple),
                    TyKind::Unit => Vec::new(),
                    _ => {
                        self.current = Some(fail);
                        return;
                    }
                };
                if elements.len() != fields.len() {
                    self.current = Some(fail);
                    return;
                }
                for (index, (child, element_ty)) in fields.iter().zip(elements).enumerate() {
                    let field_place = place.clone().tuple_field(index as u32);
                    self.lower_flat_pattern(child, field_place, element_ty, fail, bindings);
                }
            }
            FlatPatternKind::Array {
                prefix,
                suffix,
                has_rest,
            } => {
                let parts = ArrayParts {
                    prefix,
                    suffix,
                    has_rest: *has_rest,
                    span: pattern.span,
                };
                match self.hir.ty_ctxt().kind(ty) {
                    TyKind::Array { .. } => {
                        self.lower_array_pattern(&parts, place, ty, fail, bindings);
                    }
                    TyKind::Ref { pointee, .. } => {
                        let TyKind::Slice { element } = self.hir.ty_ctxt().kind(pointee) else {
                            self.current = Some(fail);
                            return;
                        };
                        self.lower_slice_pattern(&parts, place, element, fail, bindings);
                    }
                    _ => {
                        self.current = Some(fail);
                    }
                }
            }
        }
    }

    /// Removes the references a struct or variant pattern matches through.
    fn deref_pattern_place(&self, mut place: Place, mut ty: TyId) -> Place {
        while let TyKind::Ref { pointee, .. } = self.hir.ty_ctxt().kind(ty) {
            place = place.deref();
            ty = pointee;
        }
        place
    }

    /// Lowers an array pattern over a fixed-size array value.
    fn lower_array_pattern(
        &mut self,
        parts: &ArrayParts<'_>,
        place: Place,
        ty: TyId,
        fail: BasicBlockId,
        bindings: &mut Vec<(DefId, Place)>,
    ) {
        let TyKind::Array { element, length } = self.hir.ty_ctxt().kind(ty) else {
            self.current = Some(fail);
            return;
        };
        for (index, child) in parts.prefix.iter().enumerate() {
            self.lower_array_element(child, place.clone(), element, index as u64, fail, bindings);
        }
        let offset = length.saturating_sub(parts.suffix.len() as u64);
        for (position, child) in parts.suffix.iter().enumerate() {
            self.lower_array_element(
                child,
                place.clone(),
                element,
                offset + position as u64,
                fail,
                bindings,
            );
        }
    }

    /// Lowers an array pattern over a slice reference.
    ///
    /// The pattern tests the runtime length first, then reads the fixed prefix
    /// from the start and the fixed suffix from the end. The test guarantees
    /// every index is in bounds, so the element projections need no further
    /// check.
    fn lower_slice_pattern(
        &mut self,
        parts: &ArrayParts<'_>,
        place: Place,
        element: TyId,
        fail: BasicBlockId,
        bindings: &mut Vec<(DefId, Place)>,
    ) {
        let usize_ty = self.usize_ty();
        let length = self.new_temp(usize_ty, parts.span);
        self.assign_rvalue(
            length.clone(),
            Rvalue::Len {
                slice: Operand::Copy(place.clone()),
            },
            parts.span,
        );
        let fixed = parts.prefix.len() + parts.suffix.len();
        if !parts.has_rest {
            let success = self.new_block();
            self.terminate(
                TerminatorKind::SwitchInt {
                    discriminant: Operand::Copy(length.clone()),
                    targets: vec![SwitchTarget {
                        value: fixed as u128,
                        target: success,
                    }],
                    otherwise: fail,
                },
                parts.span,
            );
            self.current = Some(success);
        } else if fixed > 0 {
            let bool_ty = self
                .hir
                .ty_ctxt()
                .lookup(TyKind::Bool)
                .expect("HIR lowering interns every primitive type");
            let enough = self.new_temp(bool_ty, parts.span);
            self.assign_rvalue(
                enough.clone(),
                Rvalue::Binary {
                    op: BinOp::Ge,
                    lhs: Operand::Copy(length.clone()),
                    rhs: Operand::Constant(Constant::Int {
                        value: fixed as u128,
                        ty: usize_ty,
                    }),
                    span: parts.span,
                },
                parts.span,
            );
            let success = self.new_block();
            self.terminate(
                TerminatorKind::SwitchInt {
                    discriminant: Operand::Copy(enough),
                    targets: vec![SwitchTarget {
                        value: 1,
                        target: success,
                    }],
                    otherwise: fail,
                },
                parts.span,
            );
            self.current = Some(success);
        }

        let slice = place.deref();
        for (index, child) in parts.prefix.iter().enumerate() {
            self.lower_array_element(child, slice.clone(), element, index as u64, fail, bindings);
        }
        for (position, child) in parts.suffix.iter().enumerate() {
            let back = (parts.suffix.len() - position) as u64;
            let index = self.new_temp(usize_ty, parts.span);
            self.assign_rvalue(
                index.clone(),
                Rvalue::Binary {
                    op: BinOp::Sub,
                    lhs: Operand::Copy(length.clone()),
                    rhs: Operand::Constant(Constant::Int {
                        value: back as u128,
                        ty: usize_ty,
                    }),
                    span: parts.span,
                },
                parts.span,
            );
            let element_place = slice.clone().index(index.expect_local(), None);
            self.lower_flat_pattern(child, element_place, element, fail, bindings);
        }
    }

    /// Projects one fixed element of an array pattern and lowers its pattern.
    fn lower_array_element(
        &mut self,
        child: &FlatPattern,
        place: Place,
        element: TyId,
        index: u64,
        fail: BasicBlockId,
        bindings: &mut Vec<(DefId, Place)>,
    ) {
        let span = child.span;
        let usize_ty = self
            .hir
            .ty_ctxt()
            .lookup(TyKind::Uint(seal_ty::UintTy::Usize))
            .expect("HIR lowering interns every primitive type");
        let index_place = self.new_temp(usize_ty, span);
        self.assign_rvalue(
            index_place.clone(),
            Rvalue::Use(Operand::Constant(Constant::Int {
                value: index as u128,
                ty: usize_ty,
            })),
            span,
        );
        let element_place = place.index(index_place.expect_local(), Some(index));
        self.lower_flat_pattern(child, element_place, element, fail, bindings);
    }

    /// Compares the value at `place` with a pattern literal and continues only
    /// when the comparison holds.
    fn emit_comparison(
        &mut self,
        place: Place,
        op: BinOp,
        literal: PatternLiteral,
        ty: TyId,
        fail: BasicBlockId,
        span: Span,
    ) {
        let lhs = self.place_operand(place, ty);
        let rhs = Operand::Constant(self.pattern_constant(literal, ty));
        let bool_ty = self
            .hir
            .ty_ctxt()
            .lookup(TyKind::Bool)
            .expect("HIR lowering interns every primitive type");
        let matched = self.new_temp(bool_ty, span);
        self.assign_rvalue(matched.clone(), Rvalue::Binary { op, lhs, rhs, span }, span);
        let success = self.new_block();
        self.terminate(
            TerminatorKind::SwitchInt {
                discriminant: Operand::Copy(matched),
                targets: vec![SwitchTarget {
                    value: 1,
                    target: success,
                }],
                otherwise: fail,
            },
            span,
        );
        self.current = Some(success);
    }

    fn pattern_constant(&self, literal: PatternLiteral, ty: TyId) -> Constant {
        match literal {
            PatternLiteral::Int {
                value, negative, ..
            } => {
                let value = if negative {
                    value.wrapping_neg()
                } else {
                    value
                };
                Constant::Int { value, ty }
            }
            PatternLiteral::Float { value, .. } => Constant::Float { value, ty },
            PatternLiteral::Char(value) => Constant::Char(value),
            PatternLiteral::Bool(value) => Constant::Bool(value),
        }
    }

    /// Declares a `for` pattern's bindings once, before the loop, and returns
    /// their locals.
    ///
    /// The loop body owns them: every iteration binds them again and leaving the
    /// body drops the previous iteration's values.
    pub(super) fn declare_for_pattern(&mut self, pattern: HirPatternId) -> Vec<LocalId> {
        self.declare_pattern_bindings(pattern)
    }

    /// Lowers one iteration's pattern, jumping to `success` when it matches and
    /// to `fail` when an alternative does not.
    ///
    /// Type checking proves the pattern irrefutable, but a failed alternative
    /// still has a defined destination: the increment block skips the body, so
    /// error recovery cannot produce an invalid loop.
    pub(super) fn lower_for_pattern(
        &mut self,
        pattern: HirPatternId,
        place: Place,
        ty: TyId,
        fail: BasicBlockId,
        success: BasicBlockId,
        span: Span,
    ) {
        let alternatives = self.expand_pattern(pattern);
        if alternatives.is_empty() {
            self.terminate_goto(fail, span);
            return;
        }
        let mut entries = Vec::new();
        for _ in 0..alternatives.len().saturating_sub(1) {
            entries.push(self.new_block());
        }
        for (index, alternative) in alternatives.iter().enumerate() {
            if index > 0 {
                self.current = entries.get(index - 1).copied();
            }
            let alternative_fail = entries.get(index).copied().unwrap_or(fail);
            let mut bindings = Vec::new();
            self.lower_flat_pattern(
                alternative,
                place.clone(),
                ty,
                alternative_fail,
                &mut bindings,
            );
            self.bind_all(&bindings, span);
            self.terminate_goto(success, span);
        }
    }
}
