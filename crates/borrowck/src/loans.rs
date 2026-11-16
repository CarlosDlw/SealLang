//! Collecting the loans of a body.

use seal_index::Idx;
use seal_mir::{Body, Rvalue, StatementKind};

use crate::id::LoanId;
use crate::loan::Loan;

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
            });
            row[position] = Some(id);
        }
        positions.push(row);
    }

    (loans, positions)
}
