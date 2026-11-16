//! Collecting the loans of a body.

use seal_index::Idx;
use seal_mir::{Body, Rvalue, StatementKind};

use crate::holds::Holds;
use crate::id::LoanId;
use crate::loan::Loan;
use crate::origins::{self, OriginSet, Origins};

/// Collects every `&place` or `&mut place` and its position in the body.
///
/// The returned table is indexed by block and statement, matching how the
/// dataflow passes replay the body.
pub(crate) fn collect(body: &Body) -> (Vec<Loan>, Vec<Vec<Option<LoanId>>>) {
    let mut loans = Vec::new();
    let mut positions = Vec::with_capacity(body.blocks.len());

    for block in body.blocks.iter() {
        let mut row = vec![None; block.statements.len()];
        for (position, statement) in block.statements.iter().enumerate() {
            let StatementKind::Assign { rvalue, .. } = &statement.kind;
            let Rvalue::Ref { place, kind } = rvalue else {
                continue;
            };
            let id = LoanId::from_usize(loans.len());
            loans.push(Loan {
                place: place.clone(),
                span: statement.span,
                kind: *kind,
                keys: OriginSet::default(),
                parents: Vec::new(),
            });
            row[position] = Some(id);
        }
        positions.push(row);
    }

    (loans, positions)
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
) {
    for (block_index, block) in body.blocks.iter().enumerate() {
        for (position, statement) in block.statements.iter().enumerate() {
            let Some(id) = loan_at[block_index][position] else {
                continue;
            };
            let StatementKind::Assign {
                rvalue: Rvalue::Ref { place, .. },
                ..
            } = &statement.kind
            else {
                continue;
            };
            let loan = &mut loans[id.index() as usize];
            loan.keys = origins::keys_of(&origins.before[block_index][position], place);
            let mut parents: Vec<LoanId> = origins::first_deref_base(place)
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
