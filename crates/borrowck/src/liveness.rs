//! Backward liveness over the control-flow graph.

use seal_mir::Body;

use crate::access;

pub(crate) use crate::access::LocalSet;

/// The set of locals live at each statement.
pub(crate) struct Liveness {
    /// `before[block][statement]` holds the locals live before that statement.
    pub(crate) before: Vec<Vec<LocalSet>>,
    /// `after[block]` holds the locals live at the block's terminator, after
    /// its last statement.
    pub(crate) after: Vec<LocalSet>,
}

/// Computes liveness by iterating the union of successor states until stable.
///
/// A local is live at a point when some path from that point reads it before
/// defining it. Blocks that no edge reaches end with empty sets, which is the
/// least solution and never makes a dead loan look active.
pub(crate) fn analyze(body: &Body) -> Liveness {
    let block_count = body.blocks.len();
    let mut live_in = vec![LocalSet::default(); block_count];

    loop {
        let mut changed = false;
        for (block_id, block) in body.blocks.iter_enumerated() {
            let mut live = LocalSet::default();
            for successor in block.terminator.kind.successors() {
                live.extend(live_in[successor.index() as usize].iter().copied());
            }
            access::terminator_uses(&block.terminator.kind, &mut live);

            for statement in block.statements.iter().rev() {
                if let Some(definition) = access::statement_def(statement) {
                    live.remove(&definition);
                }
                access::statement_uses(statement, &mut live);
            }

            if live != live_in[block_id.index() as usize] {
                live_in[block_id.index() as usize] = live;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }

    let mut before = Vec::with_capacity(block_count);
    let mut after = Vec::with_capacity(block_count);
    for block in body.blocks.iter() {
        let mut live = LocalSet::default();
        for successor in block.terminator.kind.successors() {
            live.extend(live_in[successor.index() as usize].iter().copied());
        }
        access::terminator_uses(&block.terminator.kind, &mut live);
        after.push(live.clone());

        let mut row = vec![LocalSet::default(); block.statements.len()];
        for (position, statement) in block.statements.iter().enumerate().rev() {
            if let Some(definition) = access::statement_def(statement) {
                live.remove(&definition);
            }
            access::statement_uses(statement, &mut live);
            row[position] = live.clone();
        }
        before.push(row);
    }

    Liveness { before, after }
}
