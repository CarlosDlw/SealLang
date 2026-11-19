//! Lowering `for` loops.
//!
//! Built-in ranges and sequences lower to one counted loop: a condition block,
//! a block that matches the pattern and binds its names, the body, and the
//! advance of the counter. A range advances after the body; a sequence advances
//! at the start of the body, so its index always names the first element that
//! was *not* taken and the exit can drop exactly the elements left behind.
//! `break` targets the exit and `continue` targets the condition, so neither
//! skips an element.
//!
//! The iterable is evaluated once before the loop. An array is moved into a
//! temporary and indexed through an exclusive borrow of that temporary, so a
//! non-`Copy` element can be read without the initialization analysis treating
//! the whole array as moved on every iteration. The loop owns that temporary:
//! the elements it did not take are dropped by `DropRemaining` on the exit, and
//! no scope drop would run for the array. A reference to an array or a slice is
//! kept in a temporary and each element is reborrowed with the same shared or
//! exclusive kind as the source, which leaves every element to its owner.
//!
//! A custom source records protocol calls during type checking. Its loop has a
//! condition, a bind block that calls `next`, the body, and an exit, with no
//! increment: `continue` returns to the condition so only `has_next` decides
//! whether another item is produced.

use crate::id::LocalId;
use seal_hir::{ForSource, HirBlockId, HirExprId, HirPatternId};
use seal_span::Span;
use seal_ty::{RefKind, TyId, TyKind};
use seal_typeck::IterationProtocol;

use super::BodyLowering;
use crate::operand::{Constant, Operand};
use crate::place::Place;
use crate::rvalue::{BinOp, Rvalue};
use crate::statement::Statement;
use crate::terminator::{SwitchTarget, TerminatorKind};

/// How a sequence source exposes its elements.
struct SequenceSource {
    /// The element type the pattern matches.
    subject: TyId,
    /// The reference kind used to borrow each element, if any.
    reference: Option<RefKind>,
    /// The array or slice place that the index is applied to.
    base: Place,
    /// The loop length: a constant for arrays, a temporary for slices.
    length: Operand,
}

/// The parts every `for` lowering shares.
struct ForParts<'a> {
    /// The pattern matched against each item.
    pattern: HirPatternId,
    /// The loop body.
    body: HirBlockId,
    /// The span of the whole statement.
    span: Span,
    /// The locals the pattern binds, owned by the body block.
    bindings: &'a [LocalId],
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
        let bindings = self.declare_for_pattern(pattern);
        let parts = ForParts {
            pattern,
            body,
            span,
            bindings: &bindings,
        };
        match source {
            ForSource::Range {
                start,
                end,
                inclusive,
            } => self.lower_for_range(start, end, inclusive, &parts),
            ForSource::Expr(iterable) => {
                if let Some(protocol) = self.typeck.iteration(iterable).cloned() {
                    self.lower_for_protocol(iterable, protocol, &parts);
                } else {
                    self.lower_for_sequence(iterable, &parts);
                }
            }
        }
    }

    /// Lowers a custom `IntoIterator` or direct-`Iterator` loop.
    ///
    /// The source is evaluated once. `IntoIterator::iter` runs before the first
    /// condition when present; otherwise the source temporary is the iterator.
    /// The shared `has_next` borrow ends before the exclusive `next` borrow, and
    /// the body returns directly to the condition so `continue` checks the next
    /// item without calling `next` twice.
    fn lower_for_protocol(
        &mut self,
        iterable: HirExprId,
        protocol: IterationProtocol,
        parts: &ForParts<'_>,
    ) {
        let span = parts.span;
        let source_ty = self.expr_ty(iterable);
        let source = self.new_temp(source_ty, span);
        self.lower_expr_into(iterable, Some(source.clone()));
        let iterator = match protocol.iter.as_ref() {
            Some(iter) => {
                let receiver =
                    self.lower_place_receiver(source.clone(), source_ty, &iter.method, span);
                let iterator = self.new_temp(protocol.iterator_ty, span);
                self.lower_protocol_call(
                    iter,
                    receiver,
                    Vec::new(),
                    vec![span],
                    Some(iterator.clone()),
                    span,
                );
                iterator
            }
            None => source,
        };

        let condition = self.new_block();
        let bind = self.new_block();
        let body_block = self.new_block();
        let exit = self.new_block();

        self.terminate_goto(condition, span);
        self.current = Some(condition);
        let test = self.new_temp(protocol.has_next.method.ret_ty, span);
        let receiver = self.lower_place_receiver(
            iterator.clone(),
            protocol.iterator_ty,
            &protocol.has_next.method,
            span,
        );
        self.lower_protocol_call(
            &protocol.has_next,
            receiver,
            Vec::new(),
            vec![span],
            Some(test.clone()),
            span,
        );
        self.terminate(
            TerminatorKind::SwitchInt {
                discriminant: Operand::Copy(test),
                targets: vec![SwitchTarget {
                    value: 1,
                    target: bind,
                }],
                otherwise: exit,
            },
            span,
        );

        self.current = Some(bind);
        let receiver = self.lower_place_receiver(
            iterator.clone(),
            protocol.iterator_ty,
            &protocol.next.method,
            span,
        );
        let item = self.new_temp(protocol.item_ty, span);
        self.lower_protocol_call(
            &protocol.next,
            receiver,
            Vec::new(),
            vec![span],
            Some(item.clone()),
            span,
        );
        self.lower_for_pattern(
            parts.pattern,
            item,
            protocol.item_ty,
            condition,
            body_block,
            span,
        );

        self.current = Some(body_block);
        let depth = self.scopes.len();
        self.loops.push(super::LoopTargets {
            break_to: exit,
            continue_to: condition,
            depth,
        });
        self.lower_block_with(parts.body, None, parts.bindings.to_vec());
        self.loops.pop();
        if self.current.is_some() {
            self.terminate_goto(condition, span);
        }

        self.current = Some(exit);
    }

    /// Lowers `for pattern in start..end` and `start..=end`.
    fn lower_for_range(
        &mut self,
        start: HirExprId,
        end: HirExprId,
        inclusive: bool,
        parts: &ForParts<'_>,
    ) {
        let span = parts.span;
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
        self.lower_for_pattern(
            parts.pattern,
            current.clone(),
            ty,
            increment,
            body_block,
            span,
        );

        self.current = Some(body_block);
        let depth = self.scopes.len();
        self.loops.push(super::LoopTargets {
            break_to: exit,
            continue_to: increment,
            depth,
        });
        self.lower_block_with(parts.body, None, parts.bindings.to_vec());
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
    fn lower_for_sequence(&mut self, iterable: HirExprId, parts: &ForParts<'_>) {
        let span = parts.span;
        let collection_ty = self.expr_ty(iterable);
        let collection = self.new_temp(collection_ty, span);
        self.lower_expr_into(iterable, Some(collection.clone()));
        // A by-value loop owns its array: it drops the elements the body did not
        // take when it exits, so the statement must not drop the array itself.
        let owned = matches!(self.hir.ty_ctxt().kind(collection_ty), TyKind::Array { .. });
        if owned && let Some(local) = collection.local_id() {
            self.hand_over_temp(local);
        }
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
        let index_local = index.expect_local();
        let element_place = source.base.clone().index(index_local, None);
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
            parts.pattern,
            subject,
            source.subject,
            condition,
            body_block,
            span,
        );

        self.current = Some(body_block);
        // The pattern read the element with the index it was given, and the
        // index advances here, before the body runs. It therefore always names
        // the first element that was *not* taken, so leaving the body from
        // anywhere — `break` included — still drops exactly the unconsumed tail.
        let one = Operand::Constant(self.constant_one(index_ty));
        self.assign_rvalue(
            index.clone(),
            Rvalue::Binary {
                op: BinOp::Add,
                lhs: Operand::Copy(index.clone()),
                rhs: one,
                span,
            },
            span,
        );
        let depth = self.scopes.len();
        self.loops.push(super::LoopTargets {
            break_to: exit,
            continue_to: condition,
            depth,
        });
        self.lower_block_with(parts.body, None, parts.bindings.to_vec());
        self.loops.pop();
        if self.current.is_some() {
            self.terminate_goto(condition, span);
        }

        self.current = Some(exit);
        if owned && self.hir.ty_ctxt().needs_drop(collection_ty) {
            self.push_statement(Statement::drop_remaining(
                collection,
                Operand::Copy(index),
                span,
            ));
        }
    }

    /// Classifies an array or reference and produces its length operand.
    fn sequence_source(&mut self, ty: TyId, collection: Place, span: Span) -> SequenceSource {
        match self.hir.ty_ctxt().kind(ty) {
            TyKind::Array { element, length } => {
                // Move elements out through an exclusive borrow of the
                // temporary. A direct index move is dynamic, and the
                // initialization analysis would mark the whole array moved on
                // the first iteration, rejecting the second.
                let pointer_ty = self.hir.ty_ctxt().ref_ty_with(RefKind::Exclusive, ty);
                let pointer = self.new_temp(pointer_ty, span);
                self.assign_rvalue(
                    pointer.clone(),
                    Rvalue::Ref {
                        place: collection,
                        kind: RefKind::Exclusive,
                    },
                    span,
                );
                SequenceSource {
                    subject: element,
                    reference: None,
                    base: pointer.deref(),
                    length: self.constant_length(length),
                }
            }
            TyKind::Ref { kind, pointee } => match self.hir.ty_ctxt().kind(pointee) {
                TyKind::Array { element, length } => SequenceSource {
                    subject: self.hir.ty_ctxt().ref_ty_with(kind, element),
                    reference: Some(kind),
                    base: collection.deref(),
                    length: self.constant_length(length),
                },
                TyKind::Slice { element } => {
                    let length = self.new_temp(self.usize_ty(), span);
                    self.assign_rvalue(
                        length.clone(),
                        Rvalue::Len {
                            slice: Operand::Copy(collection.clone()),
                        },
                        span,
                    );
                    SequenceSource {
                        subject: self.hir.ty_ctxt().ref_ty_with(kind, element),
                        reference: Some(kind),
                        base: collection.deref(),
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
