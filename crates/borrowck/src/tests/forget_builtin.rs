//! `mem::forget` in the ownership analysis.
//!
//! The statement is built on top of a checked body, exactly like the drop
//! statement tests: forgetting consumes a place like a move, so the ordinary
//! move and borrow rules apply afterwards.

use seal_def::DefKind;
use seal_hir::Hir;
use seal_index::IndexVec;
use seal_mir::{
    BasicBlock, BasicBlockId, Body, BodyKind, LocalDecl, LocalId, LocalKind, Operand, Place,
    Rvalue, Statement, Terminator, TerminatorKind,
};
use seal_span::Span;
use seal_ty::{IntTy, TyId, TyKind};

use super::support::{Fixture, main_owner};

fn local_id(index: u32) -> LocalId {
    LocalId::from_u32(index).expect("test locals have small indices")
}

fn local(ty: TyId, kind: LocalKind) -> LocalDecl {
    LocalDecl {
        ty,
        name: None,
        kind,
        span: Span::default(),
    }
}

fn assign(place: LocalId, rvalue: Rvalue) -> Statement {
    Statement::assign(Place::local(place), rvalue, Span::default())
}

fn body(owner: seal_def::DefId, locals: Vec<LocalDecl>, statements: Vec<Statement>) -> Body {
    Body {
        owner,
        kind: BodyKind::Function,
        is_const: false,
        arg_count: 1,
        locals: IndexVec::from_raw(locals),
        blocks: IndexVec::from_raw(vec![BasicBlock {
            statements,
            terminator: Terminator {
                kind: TerminatorKind::Return,
                span: Span::default(),
            },
        }]),
        entry: BasicBlockId::from_u32(0).expect("zero is a valid block"),
    }
}

fn int_type(hir: &Hir) -> TyId {
    hir.ty_ctxt()
        .lookup(TyKind::Int(IntTy::I32))
        .expect("HIR lowering interns `i32`")
}

fn owned_fixture(source: &str) -> (Fixture, Hir, TyId) {
    let fixture = Fixture::new(source);
    let (hir, _) = fixture.lower();
    let owned = hir
        .definitions()
        .iter_enumerated()
        .find(|(_, data)| data.kind == DefKind::Struct && fixture.symbol(data.name) == "Pair")
        .map(|(id, _)| {
            let adt = hir.adt(id).expect("the definition is an aggregate");
            hir.ty_ctxt().adt_ty(adt)
        })
        .expect("the fixture declares `Pair`");
    (fixture, hir, owned)
}

#[test]
fn forgetting_a_moved_value_is_rejected() {
    let (fixture, hir, owned) = owned_fixture("struct Pair { a: i32 }\n\nfunc main() {}\n");
    let owner = main_owner(&hir);
    let locals = vec![
        local(owned, LocalKind::Return),
        local(owned, LocalKind::Arg),
    ];
    let statements = vec![
        assign(
            local_id(0),
            Rvalue::Use(Operand::Move(Place::local(local_id(1)))),
        ),
        Statement::forget(Place::local(local_id(1)), Span::default()),
    ];
    fixture.check_body(&hir, &body(owner, locals, statements));
    fixture.expect("E5005", "moved");
}

#[test]
fn forgetting_a_borrowed_value_is_rejected() {
    let (fixture, hir, owned) = owned_fixture("struct Pair { a: i32 }\n\nfunc main() {}\n");
    let owner = main_owner(&hir);
    let reference = hir.ty_ctxt().ref_ty(owned);
    let locals = vec![
        local(owned, LocalKind::Return),
        local(owned, LocalKind::Arg),
        local(reference, LocalKind::Temp),
    ];
    let statements = vec![
        assign(
            local_id(2),
            Rvalue::Ref {
                place: Place::local(local_id(1)),
                kind: seal_ty::RefKind::Shared,
            },
        ),
        Statement::forget(Place::local(local_id(1)), Span::default()),
        assign(
            local_id(0),
            Rvalue::Use(Operand::Copy(Place::local(local_id(2)))),
        ),
    ];
    fixture.check_body(&hir, &body(owner, locals, statements));
    fixture.expect("E5002", "borrowed");
}

#[test]
fn forgetting_a_copy_value_does_not_move_it() {
    let (fixture, hir, _) = owned_fixture("struct Pair { a: i32 }\n\nfunc main() {}\n");
    let owner = main_owner(&hir);
    let int = int_type(&hir);
    let locals = vec![local(int, LocalKind::Return), local(int, LocalKind::Arg)];
    let statements = vec![
        Statement::forget(Place::local(local_id(1)), Span::default()),
        assign(
            local_id(0),
            Rvalue::Use(Operand::Copy(Place::local(local_id(1)))),
        ),
    ];
    fixture.check_body(&hir, &body(owner, locals, statements));
    fixture.assert_clean();
}
