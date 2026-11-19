//! Deciding whether a place definitely holds no value any more.
//!
//! `needs_drop`-style initialization facts answer "is every path initialized
//! here?", which is what a use needs. Drop elaboration needs the opposite
//! question for one place: "is there definitely nothing left to drop?". A moved
//! value is gone on the path that moved it, but the merged state after an `if`
//! is only `Moved` when some path moved it, so the lattice in
//! [`crate::init`] cannot answer it.
//!
//! This module runs one boolean must-analysis per queried place: a block starts
//! *dead* only when every predecessor ends dead, so the answer is a definite
//! fact and a dead site can be removed. Unreachable code is treated as alive,
//! which keeps a statement instead of silently deleting it.

use std::collections::VecDeque;

use seal_ty::PathElem;

use crate::body::{Body, LocalKind};
use crate::init;
use crate::place::{Place, PlaceRoot};
use crate::statement::{DropKind, Statement, StatementKind};

use super::moved_places;

/// The dead state before each statement of every block.
pub(crate) struct DeadInfo {
    before: Vec<Vec<bool>>,
}

impl DeadInfo {
    /// Returns whether the place was dead before the statement at `position`.
    ///
    /// # Panics
    ///
    /// Panics when the position does not belong to the body, which cannot
    /// happen for a statement taken from that body.
    pub(crate) fn before(&self, block: usize, position: usize) -> bool {
        self.before[block][position]
    }

    fn all_alive(body: &Body) -> Self {
        Self {
            before: body
                .blocks
                .iter()
                .map(|block| vec![false; block.statements.len()])
                .collect(),
        }
    }
}

/// Computes where `place` definitely holds no value.
///
/// A place behind a reference is always alive: the storage it names belongs to
/// the caller or to a static, and it is initialized before it can be reached.
/// A global root is alive for the same reason.
pub(crate) fn analyze(body: &Body, place: &Place) -> DeadInfo {
    let Some((root, path)) = init::path_of(place) else {
        return DeadInfo::all_alive(body);
    };
    if !matches!(place.root, PlaceRoot::Local(_)) {
        return DeadInfo::all_alive(body);
    }
    let context = Context { root, path };

    let entry_dead = body.local(root).kind != LocalKind::Arg;
    let mut in_states: Vec<Option<bool>> = vec![None; body.blocks.len()];
    in_states[body.entry.index() as usize] = Some(entry_dead);
    let mut queue = VecDeque::from([body.entry]);

    while let Some(block) = queue.pop_front() {
        let mut dead = in_states[block.index() as usize].expect("a queued block has a state");
        for statement in &body.block(block).statements {
            dead = transfer(&context, statement, dead);
        }
        for successor in body.block(block).terminator.kind.successors() {
            let slot = &mut in_states[successor.index() as usize];
            let changed = match slot {
                Some(existing) if *existing && !dead => {
                    *slot = Some(false);
                    true
                }
                Some(_) => false,
                None => {
                    *slot = Some(dead);
                    true
                }
            };
            if changed {
                queue.push_back(successor);
            }
        }
    }

    let mut before = Vec::with_capacity(body.blocks.len());
    for (block_id, block) in body.blocks.iter_enumerated() {
        let mut dead = in_states[block_id.index() as usize].unwrap_or(false);
        let mut row = Vec::with_capacity(block.statements.len());
        for statement in &block.statements {
            row.push(dead);
            dead = transfer(&context, statement, dead);
        }
        before.push(row);
    }
    DeadInfo { before }
}

/// The place every statement is interpreted against.
struct Context {
    root: crate::id::LocalId,
    path: Vec<PathElem>,
}

/// What an access to a place does to the queried place.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Effect {
    /// The access leaves no value in the queried place.
    Dead,
    /// The access may leave a value, or a part of one, in the queried place.
    Alive,
}

impl Context {
    /// Returns how accessing `place` affects the queried place.
    fn effect(&self, place: &Place) -> Option<Effect> {
        let (root, path) = init::path_of(place)?;
        if root != self.root {
            return None;
        }
        if starts_with(&self.path, &path) {
            // The access covers the queried place.
            Some(Effect::Dead)
        } else if starts_with(&path, &self.path) {
            // The access leaves part of the queried place behind.
            Some(Effect::Alive)
        } else {
            None
        }
    }
}

/// Returns `true` when `prefix` is a prefix of `path`, including equality.
fn starts_with(path: &[PathElem], prefix: &[PathElem]) -> bool {
    path.len() >= prefix.len() && path.starts_with(prefix)
}

/// Applies one statement to the queried place.
///
/// A move or a consuming drop makes it dead when it covers the place as a
/// whole, and merely alive when it takes a part of it. An assignment makes the
/// destination alive: after the store there is a value again.
fn transfer(context: &Context, statement: &Statement, dead: bool) -> bool {
    match &statement.kind {
        StatementKind::Assign { place, rvalue } => {
            let mut dead = dead;
            for moved in moved_places(rvalue) {
                if let Some(effect) = context.effect(moved) {
                    dead = effect == Effect::Dead;
                }
            }
            if context.effect(place).is_some() {
                dead = false;
            }
            dead
        }
        StatementKind::Drop { place, kind } => {
            if *kind == DropKind::Overwrite {
                return dead;
            }
            match context.effect(place) {
                Some(effect) => effect == Effect::Dead,
                None => dead,
            }
        }
        // The statement consumes what is left of the array, which keeps part of
        // it and may still hold a value in the queried place.
        StatementKind::DropRemaining { place, .. } => match context.effect(place) {
            Some(_) => false,
            None => dead,
        },
    }
}
