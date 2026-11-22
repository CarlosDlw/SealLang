//! Collecting the loans of a body.

use std::cell::Cell;

use seal_hir::Hir;
use seal_index::Idx;
use seal_mir::{Body, Rvalue, StatementKind};
use seal_ty::TyKind;

use crate::holds::Holds;
use crate::id::LoanId;
use crate::loan::Loan;
use crate::origins::{self, OriginSet, Origins};

/// Collects every `&place` or `&mut place` and its position in the body.
///
/// The returned table is indexed by block and statement, matching how the
/// dataflow passes replay the body.
pub(crate) fn collect(hir: &Hir, body: &Body) -> (Vec<Loan>, Vec<Vec<Option<LoanId>>>) {
    let mut loans = Vec::new();
    let mut positions = Vec::with_capacity(body.blocks.len());

    for block in body.blocks.iter() {
        let mut row = vec![None; block.statements.len()];
        for (position, statement) in block.statements.iter().enumerate() {
            let StatementKind::Assign { rvalue, .. } = &statement.kind else {
                continue;
            };
            let (place, kind) = match rvalue {
                Rvalue::Ref { place, kind } => (place.clone(), *kind),
                Rvalue::OwnedView {
                    owner,
                    kind: view_kind,
                    ..
                } => (owner.clone(), *view_kind),
                Rvalue::SliceRef { base, kind, .. } => {
                    let place = if matches!(
                        hir.ty_ctxt().kind(origins::place_ty(hir, body, base)),
                        TyKind::Ref { .. }
                    ) {
                        base.clone().deref()
                    } else {
                        base.clone()
                    };
                    (place, *kind)
                }
                Rvalue::StrView { base, .. } => {
                    let place = if matches!(
                        hir.ty_ctxt().kind(origins::place_ty(hir, body, base)),
                        TyKind::Ref { .. }
                    ) {
                        base.clone().deref()
                    } else {
                        base.clone()
                    };
                    (place, seal_ty::RefKind::Shared)
                }
                Rvalue::TraitObjectReborrow { place, to, .. } => {
                    (place.clone(), reborrow_kind(hir, *to))
                }
                _ => continue,
            };
            let id = LoanId::from_usize(loans.len());
            loans.push(Loan {
                place,
                span: statement.span,
                kind,
                keys: OriginSet::default(),
                parents: Vec::new(),
                last_use: None,
                reported: Cell::new(false),
            });
            row[position] = Some(id);
        }
        positions.push(row);
    }

    (loans, positions)
}

/// Returns the mutability of a trait-object reborrow's destination.
pub(crate) fn reborrow_kind(hir: &Hir, ty: seal_ty::TyId) -> seal_ty::RefKind {
    match hir.ty_ctxt().kind(ty) {
        TyKind::Ref { kind, .. } => kind,
        _ => panic!("a trait-object reborrow must target a pointer; this is a compiler bug"),
    }
}

/// Fills in each loan's storage keys and authorizing loans.
///
/// Keys are resolved with the origin state before the borrow statement, and
/// parents with the may-hold state at the same point. Together they decide
/// which live loans a later access conflicts with.
pub(crate) fn resolve(
    body: &Body,
    loans: &mut [Loan],
    loan_at: &[Vec<Option<LoanId>>],
    origins: &Origins,
    holds: &Holds,
    const_locals: &origins::ConstLocals,
) {
    for (block_index, block) in body.blocks.iter().enumerate() {
        for (position, statement) in block.statements.iter().enumerate() {
            let Some(id) = loan_at[block_index][position] else {
                continue;
            };
            let StatementKind::Assign { rvalue, .. } = &statement.kind else {
                continue;
            };
            if !matches!(
                rvalue,
                Rvalue::Ref { .. }
                    | Rvalue::OwnedView { .. }
                    | Rvalue::SliceRef { .. }
                    | Rvalue::StrView { .. }
                    | Rvalue::TraitObjectReborrow { .. }
            ) {
                continue;
            }
            let loan = &loans[id.index() as usize];
            let place = loan.place.clone();
            let loan = &mut loans[id.index() as usize];
            loan.keys =
                origins::keys_of(&origins.before[block_index][position], &place, const_locals);
            let mut parents: Vec<LoanId> = origins::first_deref_base(&place)
                .map(|base| {
                    holds.before[block_index][position]
                        .at(&base)
                        .into_iter()
                        .collect()
                })
                .unwrap_or_default();
            parents.sort_by_key(|parent| parent.index());
            parents.dedup();
            loan.parents = parents;
        }
    }
}
