//! Lowering `for` loops.
//!
//! Every form lowers to one counted loop: a condition block, a block that
//! matches the pattern and binds its names, the body, and an increment block.
//! `continue` targets the increment, so a skipped iteration still advances the
//! source; `break` targets the exit.
//!
//! The iterable is evaluated once before the loop. An array is moved into a
//! temporary and its elements are read by index; a reference to an array or a
//! slice is kept in a temporary and each element is reborrowed with the same
//! shared or exclusive kind as the source.

use seal_hir::{ForSource, HirBlockId, HirExprId, HirPatternId};
use seal_span::Span;
use seal_ty::{RefKind, TyId, TyKind};

use super::BodyLowering;
use crate::operand::{Constant, Operand};
use crate::place::Place;
use crate::rvalue::{BinOp, Rvalue};
use crate::terminator::{SwitchTarget, TerminatorKind};

/// How a sequence source exposes its elements.
struct SequenceSource {
    /// The element type the pattern matches.
    subject: TyId,
    /// The reference kind used to borrow each element, if any.
    reference: Option<RefKind>,
    /// The loop length: a constant for arrays, a temporary for slices.
    length: Operand,
}

impl BodyLowering<'_> {
    /// Lowers one `for` statement.
    pub(super) fn lower_for(
        &mut self,
        pattern: HirPatternId,
        source: ForSource,
        body: HirBlockId,
        span: Span,
    ) {
        self.declare_for_pattern(pattern);
        match source {
            ForSource::Range {
                start,
                end,
                inclusive,
            } => self.lower_for_range(pattern, start, end, inclusive, body, span),
            ForSource::Expr(iterable) => self.lower_for_sequence(pattern, iterable, body, span),
        }
    }

    /// Lowers `for pattern in start..end` and `start..=end`.
    fn lower_for_range(
        &mut self,
        pattern: HirPatternId,
        start: HirExprId,
        end: HirExprId,
        inclusive: bool,
        body: HirBlockId,
        span: Span,
    ) {
        let ty = self.expr_ty(start);
        let current = self.new_temp(ty, span);
        self.lower_expr_into(start, Some(current.clone()));
        let end_place = self.new_temp(ty, span);
        self.lower_expr_into(end, Some(end_place.clone()));

        let condition = self.new_block();
        let bind = self.new_block();
        let body_block = self.new_block();
        let increment = self.new_block();
        let exit = self.new_block();

        self.terminate_goto(condition, span);
        self.current = Some(condition);
        let op = if inclusive { BinOp::Le } else { BinOp::Lt };
        let test = self.compare(
            Operand::Copy(current.clone()),
            Operand::Copy(end_place.clone()),
            op,
            span,
        );
        self.terminate(
            TerminatorKind::SwitchInt {
                discriminant: test,
                targets: vec![SwitchTarget {
                    value: 1,
                    target: bind,
                }],
                otherwise: exit,
            },
            span,
        );

        self.current = Some(bind);
        self.lower_for_pattern(pattern, current.clone(), ty, increment, body_block, span);

        self.current = Some(body_block);
        self.loops.push(super::LoopTargets {
            break_to: exit,
            continue_to: increment,
        });
        self.lower_block(body, None);
        self.loops.pop();
        if self.current.is_some() {
            self.terminate_goto(increment, span);
        }

        self.current = Some(increment);
        if inclusive {
            // `current <= end` ran, so equality is the last iteration. Exiting
            // before adding one keeps `current + 1` from overflowing at the
            // maximum value of the type.
            let last = self.compare(
                Operand::Copy(current.clone()),
                Operand::Copy(end_place.clone()),
                BinOp::Eq,
                span,
            );
            let advance = self.new_block();
            self.terminate(
                TerminatorKind::SwitchInt {
                    discriminant: last,
                    targets: vec![SwitchTarget {
                        value: 1,
                        target: exit,
                    }],
                    otherwise: advance,
                },
                span,
            );
            self.current = Some(advance);
        }
        let one = Operand::Constant(self.constant_one(ty));
        self.assign_rvalue(
            current.clone(),
            Rvalue::Binary {
                op: BinOp::Add,
                lhs: Operand::Copy(current),
                rhs: one,
                span,
            },
            span,
        );
        self.terminate_goto(condition, span);

        self.current = Some(exit);
    }

    /// Lowers `for pattern in array`, `&array`, `&mut array`, or a slice
    /// reference.
    fn lower_for_sequence(
        &mut self,
        pattern: HirPatternId,
        iterable: HirExprId,
        body: HirBlockId,
        span: Span,
    ) {
        let collection_ty = self.expr_ty(iterable);
        let collection = self.new_temp(collection_ty, span);
        self.lower_expr_into(iterable, Some(collection.clone()));
        let source = self.sequence_source(collection_ty, collection.clone(), span);

        let index_ty = self.usize_ty();
        let index = self.new_temp(index_ty, span);
        self.assign_constant(
            index.clone(),
            Constant::Int {
                value: 0,
                ty: index_ty,
            },
            span,
        );

        let condition = self.new_block();
        let bind = self.new_block();
        let body_block = self.new_block();
        let increment = self.new_block();
        let exit = self.new_block();

        self.terminate_goto(condition, span);
        self.current = Some(condition);
        let test = self.compare(
            Operand::Copy(index.clone()),
            source.length.clone(),
            BinOp::Lt,
            span,
        );
        self.terminate(
            TerminatorKind::SwitchInt {
                discriminant: test,
                targets: vec![SwitchTarget {
                    value: 1,
                    target: bind,
                }],
                otherwise: exit,
            },
            span,
        );

        self.current = Some(bind);
        let index_local = index.local;
        let element_place = if source.reference.is_some() {
            collection.clone().deref()
        } else {
            collection.clone()
        }
        .index(index_local, None);
        let subject = match source.reference {
            Some(kind) => {
                let reference = self.new_temp(source.subject, span);
                self.assign_rvalue(
                    reference.clone(),
                    Rvalue::Ref {
                        place: element_place,
                        kind,
                    },
                    span,
                );
                reference
            }
            None => element_place,
        };
        self.lower_for_pattern(
            pattern,
            subject,
            source.subject,
            increment,
            body_block,
            span,
        );

        self.current = Some(body_block);
        self.loops.push(super::LoopTargets {
            break_to: exit,
            continue_to: increment,
        });
        self.lower_block(body, None);
        self.loops.pop();
        if self.current.is_some() {
            self.terminate_goto(increment, span);
        }

        self.current = Some(increment);
        let one = Operand::Constant(self.constant_one(index_ty));
        self.assign_rvalue(
            index.clone(),
            Rvalue::Binary {
                op: BinOp::Add,
                lhs: Operand::Copy(index),
                rhs: one,
                span,
            },
            span,
        );
        self.terminate_goto(condition, span);

        self.current = Some(exit);
    }

    /// Classifies an array or reference and produces its length operand.
    fn sequence_source(&mut self, ty: TyId, collection: Place, span: Span) -> SequenceSource {
        match self.hir.ty_ctxt().kind(ty) {
            TyKind::Array { element, length } => SequenceSource {
                subject: element,
                reference: None,
                length: self.constant_length(length),
            },
            TyKind::Ref { kind, pointee } => match self.hir.ty_ctxt().kind(pointee) {
                TyKind::Array { element, length } => SequenceSource {
                    subject: self.hir.ty_ctxt().ref_ty_with(kind, element),
                    reference: Some(kind),
                    length: self.constant_length(length),
                },
                TyKind::Slice { element } => {
                    let length = self.new_temp(self.usize_ty(), span);
                    self.assign_rvalue(
                        length.clone(),
                        Rvalue::Len {
                            slice: Operand::Copy(collection),
                        },
                        span,
                    );
                    SequenceSource {
                        subject: self.hir.ty_ctxt().ref_ty_with(kind, element),
                        reference: Some(kind),
                        length: Operand::Copy(length),
                    }
                }
                _ => unreachable!("type checking rejects a non-sequence reference"),
            },
            _ => unreachable!("type checking rejects a non-iterable source"),
        }
    }

    /// Compares two operands and returns the boolean temp holding the result.
    fn compare(&mut self, lhs: Operand, rhs: Operand, op: BinOp, span: Span) -> Operand {
        let bool_ty = self
            .hir
            .ty_ctxt()
            .lookup(TyKind::Bool)
            .expect("HIR lowering interns every primitive type");
        let condition = self.new_temp(bool_ty, span);
        self.assign_rvalue(
            condition.clone(),
            Rvalue::Binary { op, lhs, rhs, span },
            span,
        );
        Operand::Copy(condition)
    }
}
