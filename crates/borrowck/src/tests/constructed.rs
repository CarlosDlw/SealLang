//! Initialization and move checks on hand-built MIR.
//!
//! No v0.1 source type is non-`Copy`, so lowering never emits a `Move`; these
//! bodies exercise the analysis directly, exactly as aggregates will when they
//! arrive.

use seal_def::DefId;
use seal_hir::Hir;
use seal_index::IndexVec;
use seal_mir::{
    BasicBlock, BasicBlockId, Body, Constant, LocalDecl, LocalId, LocalKind, Operand, Place,
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

fn body(
    owner: DefId,
    locals: Vec<LocalDecl>,
    statements: Vec<Statement>,
    arg_count: usize,
) -> Body {
    Body {
        owner,
        kind: seal_mir::BodyKind::Function,
        is_const: false,
        arg_count,
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

#[test]
fn using_a_moved_local_is_rejected() {
    let fixture = Fixture::new("func main() {}\n");
    let (hir, _) = fixture.lower();
    let owner = main_owner(&hir);
    let int = int_type(&hir);
    let locals = vec![
        local(int, LocalKind::Return),
        local(int, LocalKind::Arg),
        local(int, LocalKind::Temp),
    ];
    let statements = vec![
        assign(
            local_id(2),
            Rvalue::Use(Operand::Move(Place::local(local_id(1)))),
        ),
        assign(
            local_id(0),
            Rvalue::Use(Operand::Copy(Place::local(local_id(1)))),
        ),
    ];
    fixture.check_body(&hir, &body(owner, locals, statements, 1));
    fixture.expect("E5005", "moved");
}

#[test]
fn moving_a_borrowed_local_is_rejected() {
    let fixture = Fixture::new("func main() {}\n");
    let (hir, _) = fixture.lower();
    let owner = main_owner(&hir);
    let int = int_type(&hir);
    let reference = hir.ty_ctxt().ref_ty(int);
    let locals = vec![
        local(reference, LocalKind::Return),
        local(int, LocalKind::Arg),
        local(reference, LocalKind::Temp),
        local(int, LocalKind::Temp),
    ];
    let statements = vec![
        assign(
            local_id(2),
            Rvalue::Ref {
                place: Place::local(local_id(1)),
                kind: seal_ty::RefKind::Shared,
            },
        ),
        assign(
            local_id(3),
            Rvalue::Use(Operand::Move(Place::local(local_id(1)))),
        ),
        assign(
            local_id(0),
            Rvalue::Use(Operand::Copy(Place::local(local_id(2)))),
        ),
    ];
    fixture.check_body(&hir, &body(owner, locals, statements, 1));
    fixture.expect("E5002", "borrowed");
}

#[test]
fn reading_an_uninitialized_local_is_rejected() {
    let fixture = Fixture::new("func main() {}\n");
    let (hir, _) = fixture.lower();
    let owner = main_owner(&hir);
    let int = int_type(&hir);
    let locals = vec![
        local(int, LocalKind::Return),
        local(int, LocalKind::Binding),
    ];
    let statements = vec![assign(
        local_id(0),
        Rvalue::Use(Operand::Copy(Place::local(local_id(1)))),
    )];
    fixture.check_body(&hir, &body(owner, locals, statements, 0));
    fixture.expect("E5003", "uninitialized");
}

#[test]
fn assigning_in_time_is_allowed() {
    let fixture = Fixture::new("func main() {}\n");
    let (hir, _) = fixture.lower();
    let owner = main_owner(&hir);
    let int = int_type(&hir);
    let locals = vec![
        local(int, LocalKind::Return),
        local(int, LocalKind::Binding),
    ];
    let statements = vec![
        assign(
            local_id(1),
            Rvalue::Use(Operand::Constant(Constant::Int { value: 7, ty: int })),
        ),
        assign(
            local_id(0),
            Rvalue::Use(Operand::Copy(Place::local(local_id(1)))),
        ),
    ];
    fixture.check_body(&hir, &body(owner, locals, statements, 0));
    fixture.assert_clean();
}

#[test]
fn reassigning_after_a_move_is_allowed() {
    let fixture = Fixture::new("func main() {}\n");
    let (hir, _) = fixture.lower();
    let owner = main_owner(&hir);
    let int = int_type(&hir);
    let locals = vec![
        local(int, LocalKind::Return),
        local(int, LocalKind::Arg),
        local(int, LocalKind::Temp),
    ];
    let statements = vec![
        assign(
            local_id(2),
            Rvalue::Use(Operand::Move(Place::local(local_id(1)))),
        ),
        assign(
            local_id(1),
            Rvalue::Use(Operand::Constant(Constant::Int { value: 1, ty: int })),
        ),
        assign(
            local_id(0),
            Rvalue::Use(Operand::Copy(Place::local(local_id(1)))),
        ),
    ];
    fixture.check_body(&hir, &body(owner, locals, statements, 1));
    fixture.assert_clean();
}
