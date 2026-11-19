//! Structural rules for drop statements.
//!
//! Lowering does not emit drop sites yet, so these tests build the statements
//! they need on top of a lowered body. That is also the only way to exercise
//! the consumers of a statement shape before a program can produce one.

use seal_def::DefKind;
use seal_span::Span;
use seal_ty::{Instance, IntTy, TyId, TyKind};

use crate::body::Body;
use crate::id::BasicBlockId;
use crate::operand::{Constant, Operand};
use crate::place::{Place, PlaceRoot};
use crate::statement::{DropKind, Statement, StatementKind};
use crate::validate::ValidationError;
use crate::{LocalDecl, LocalKind, instantiate_body};

use super::support::{definition, function, local_named, lower_clean};

/// Appends `statement` to the entry block of `body`.
fn push(body: &mut Body, statement: Statement) -> BasicBlockId {
    let block = body.entry;
    body.blocks[block].statements.push(statement);
    block
}

/// Declares an unnamed temporary of `ty`, the way lowering does.
fn temporary(ty: TyId) -> LocalDecl {
    LocalDecl {
        ty,
        name: None,
        kind: LocalKind::Temp,
        span: Span::default(),
    }
}

#[test]
fn a_drop_of_a_local_passes_validation() {
    let (fixture, hir, mir) = lower_clean("func main() { let value = 1; }");
    let mut body = function(&fixture, &hir, &mir, "main").clone();
    let value = local_named(&fixture, &body, "value");
    push(
        &mut body,
        Statement::drop(Place::local(value), DropKind::Scope, Span::default()),
    );
    // `i32` needs no destructor. Which values get a drop site is lowering's
    // decision, so validation accepts the statement either way: repeating
    // `needs_drop` here would let the two policies disagree.
    body.validate_with(&hir)
        .expect("a drop of a local is well formed");
}

#[test]
fn a_drop_of_a_static_names_the_global() {
    let source = "static COUNTER: i32 = 0;\n\nfunc main() {}\n";
    let (fixture, hir, mir) = lower_clean(source);
    let counter = definition(&fixture, &hir, DefKind::Static, "COUNTER");
    let mut body = function(&fixture, &hir, &mir, "main").clone();
    let mut place = Place::local(Body::return_local());
    place.root = PlaceRoot::Static(counter);
    let block = push(
        &mut body,
        Statement::drop(place, DropKind::Scope, Span::default()),
    );
    assert_eq!(
        body.validate_with(&hir),
        Err(ValidationError::DropOfGlobal {
            block,
            def: counter
        })
    );
}

#[test]
fn a_drop_remaining_of_a_non_array_is_rejected() {
    let (fixture, hir, mir) = lower_clean("func main() { let value = 1; }");
    let mut body = function(&fixture, &hir, &mir, "main").clone();
    let value = local_named(&fixture, &body, "value");
    let block = push(
        &mut body,
        Statement::drop_remaining(
            Place::local(value),
            Operand::Copy(Place::local(value)),
            Span::default(),
        ),
    );
    assert_eq!(
        body.validate_with(&hir),
        Err(ValidationError::DropRemainingOnNonArray { block })
    );
}

#[test]
fn a_drop_remaining_of_an_array_passes_validation() {
    let source = "func main() { let items: [i32; 2] = [1, 2]; let index: i32 = 0; }";
    let (fixture, hir, mir) = lower_clean(source);
    let mut body = function(&fixture, &hir, &mir, "main").clone();
    let items = local_named(&fixture, &body, "items");
    let index = local_named(&fixture, &body, "index");
    push(
        &mut body,
        Statement::drop_remaining(
            Place::local(items),
            Operand::Copy(Place::local(index)),
            Span::default(),
        ),
    );
    body.validate_with(&hir)
        .expect("dropping the rest of an array is well formed");
}

#[test]
fn instantiation_keeps_drop_statements() {
    let source = "func consume<T>(value: T) {}\n\nfunc main() { consume(1) }\n";
    let (fixture, hir, mir) = lower_clean(source);
    let mut body = function(&fixture, &hir, &mir, "consume").clone();
    let tcx = hir.ty_ctxt();
    let value = local_named(&fixture, &body, "value");
    let parameter = body.local(value).ty;
    let array = body.locals.push(temporary(tcx.array_ty(parameter, 2)));
    let array_place = Place::local(array);
    push(
        &mut body,
        Statement::drop(Place::local(value), DropKind::Scope, Span::default()),
    );
    push(
        &mut body,
        Statement::drop_remaining(
            array_place.clone(),
            // The operand carries a parameter type so substitution has
            // something to rewrite. A real range start is always concrete; this
            // is a unit test of the substitution path, not of a valid program.
            Operand::Constant(Constant::Int {
                value: 1,
                ty: parameter,
            }),
            Span::default(),
        ),
    );

    let int = tcx
        .lookup(TyKind::Int(IntTy::I32))
        .expect("HIR lowering interns `i32`");
    let args = tcx.substs(&[int]);
    let instantiated = instantiate_body(&hir, &body, Instance::new(body.owner, args));
    assert_eq!(
        instantiated.local(array).ty,
        tcx.array_ty(int, 2),
        "the synthetic array local is specialized"
    );
    let statements = &instantiated.blocks[instantiated.entry].statements;
    let drop = statements
        .iter()
        .find_map(|statement| match &statement.kind {
            StatementKind::Drop { place, kind } => Some((place, kind)),
            _ => None,
        })
        .expect("the instantiated body keeps its drop");
    assert_eq!(*drop.0, Place::local(value), "the drop keeps its place");
    assert_eq!(*drop.1, DropKind::Scope, "the drop keeps its kind");
    let from = statements
        .iter()
        .find_map(|statement| match &statement.kind {
            StatementKind::DropRemaining { place, from } => Some((place, from)),
            _ => None,
        })
        .expect("the instantiated body keeps its drop-remaining");
    assert_eq!(*from.0, array_place, "the statement keeps its array place");
    let Operand::Constant(Constant::Int { ty, .. }) = from.1 else {
        panic!("expected a constant range start, found {:?}", from.1);
    };
    assert_eq!(*ty, int, "the range operand is specialized");
}
