//! Drop statements in the ownership analysis.
//!
//! Lowering does not emit drop sites yet, so these tests build the statements
//! on top of a checked program. The types are aggregates because a user-defined
//! type is the only owned, non-`Copy` value the language has today.

use seal_def::DefKind;
use seal_hir::Hir;
use seal_index::IndexVec;
use seal_mir::{
    BasicBlock, BasicBlockId, Body, BodyKind, Constant, DropKind, LocalDecl, LocalId, LocalKind,
    Operand, Place, Rvalue, Statement, Terminator, TerminatorKind,
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

fn drop(place: LocalId, kind: DropKind) -> Statement {
    Statement::drop(Place::local(place), kind, Span::default())
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

/// Returns the type of the aggregate named `name`.
fn struct_type(fixture: &Fixture, hir: &Hir, name: &str) -> TyId {
    let def = hir
        .definitions()
        .iter_enumerated()
        .find(|(_, data)| data.kind == DefKind::Struct && fixture.symbol(data.name) == name)
        .map(|(id, _)| id)
        .unwrap_or_else(|| panic!("no struct named `{name}`"));
    let adt = hir.adt(def).expect("the definition is an aggregate");
    hir.ty_ctxt().adt_ty(adt)
}

/// Lowers a program that declares an owned type the tests can use.
fn owned_fixture(source: &str) -> (Fixture, Hir, TyId) {
    let fixture = Fixture::new(source);
    let (hir, _) = fixture.lower();
    let owned = struct_type(&fixture, &hir, "Pair");
    (fixture, hir, owned)
}

#[test]
fn dropping_a_borrowed_value_is_rejected() {
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
        drop(local_id(1), DropKind::Scope),
        assign(
            local_id(0),
            Rvalue::Use(Operand::Copy(Place::local(local_id(2)))),
        ),
    ];
    fixture.check_body(&hir, &body(owner, locals, statements));
    fixture.expect("E5002", "borrowed");
}

#[test]
fn dropping_a_moved_value_is_rejected() {
    let (fixture, hir, owned) = owned_fixture("struct Pair { a: i32 }\n\nfunc main() {}\n");
    let owner = main_owner(&hir);
    let int = int_type(&hir);
    let locals = vec![
        local(int, LocalKind::Return),
        local(owned, LocalKind::Arg),
        local(owned, LocalKind::Temp),
    ];
    let statements = vec![
        assign(
            local_id(0),
            Rvalue::Use(Operand::Constant(Constant::Int { value: 0, ty: int })),
        ),
        assign(
            local_id(2),
            Rvalue::Use(Operand::Move(Place::local(local_id(1)))),
        ),
        drop(local_id(1), DropKind::Explicit),
    ];
    fixture.check_body(&hir, &body(owner, locals, statements));
    fixture.expect("E5005", "moved");
}

#[test]
fn a_scope_drop_of_a_moved_value_reports_nothing() {
    let (fixture, hir, owned) = owned_fixture("struct Pair { a: i32 }\n\nfunc main() {}\n");
    let owner = main_owner(&hir);
    let int = int_type(&hir);
    let locals = vec![
        local(int, LocalKind::Return),
        local(owned, LocalKind::Arg),
        local(owned, LocalKind::Temp),
    ];
    let statements = vec![
        assign(
            local_id(0),
            Rvalue::Use(Operand::Constant(Constant::Int { value: 0, ty: int })),
        ),
        assign(
            local_id(2),
            Rvalue::Use(Operand::Move(Place::local(local_id(1)))),
        ),
        // A value moved out before the end of its scope is exactly what a drop
        // site has to tolerate: deciding whether the value is still there is
        // drop elaboration's job, not the owner's.
        drop(local_id(1), DropKind::Scope),
    ];
    fixture.check_body(&hir, &body(owner, locals, statements));
    fixture.assert_clean();
}

#[test]
fn an_overwrite_drop_leaves_the_value_initialized() {
    let (fixture, hir, owned) = owned_fixture("struct Pair { a: i32 }\n\nfunc main() {}\n");
    let owner = main_owner(&hir);
    let locals = vec![
        local(owned, LocalKind::Return),
        local(owned, LocalKind::Arg),
    ];
    let statements = vec![
        drop(local_id(1), DropKind::Overwrite),
        // Reading the place afterwards is only meaningful while the overwrite
        // drop left the value in place for the assignment that follows it.
        assign(
            local_id(0),
            Rvalue::Use(Operand::Copy(Place::local(local_id(1)))),
        ),
    ];
    fixture.check_body(&hir, &body(owner, locals, statements));
    fixture.assert_clean();
}

#[test]
fn a_drop_of_a_copy_value_does_not_move_it() {
    let (fixture, hir, _) = owned_fixture("struct Pair { a: i32 }\n\nfunc main() {}\n");
    let owner = main_owner(&hir);
    let int = int_type(&hir);
    let locals = vec![local(int, LocalKind::Return), local(int, LocalKind::Arg)];
    let statements = vec![
        drop(local_id(1), DropKind::Scope),
        assign(
            local_id(0),
            Rvalue::Use(Operand::Copy(Place::local(local_id(1)))),
        ),
    ];
    fixture.check_body(&hir, &body(owner, locals, statements));
    fixture.assert_clean();
}

#[test]
fn dropping_the_rest_of_a_borrowed_array_is_rejected() {
    let (fixture, hir, owned) = owned_fixture("struct Pair { a: i32 }\n\nfunc main() {}\n");
    let owner = main_owner(&hir);
    let tcx = hir.ty_ctxt();
    let array = tcx.array_ty(owned, 2);
    let reference = tcx.ref_ty(array);
    let int = int_type(&hir);
    let locals = vec![
        local(int, LocalKind::Return),
        local(array, LocalKind::Arg),
        local(reference, LocalKind::Temp),
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
        Statement::drop_remaining(
            Place::local(local_id(1)),
            Operand::Constant(Constant::Int { value: 0, ty: int }),
            Span::default(),
        ),
        // Copies of a reference keep its loan alive, so the borrow is still
        // live where the array is consumed.
        assign(
            local_id(3),
            Rvalue::Use(Operand::Copy(Place::local(local_id(2)))),
        ),
        assign(
            local_id(0),
            Rvalue::Use(Operand::Constant(Constant::Int { value: 0, ty: int })),
        ),
    ];
    fixture.check_body(&hir, &body(owner, locals, statements));
    fixture.expect("E5002", "borrowed");
}

#[test]
fn dropping_the_rest_of_an_owned_array_is_allowed() {
    let (fixture, hir, owned) = owned_fixture("struct Pair { a: i32 }\n\nfunc main() {}\n");
    let owner = main_owner(&hir);
    let tcx = hir.ty_ctxt();
    let array = tcx.array_ty(owned, 2);
    let int = int_type(&hir);
    let locals = vec![local(int, LocalKind::Return), local(array, LocalKind::Arg)];
    let statements = vec![
        Statement::drop_remaining(
            Place::local(local_id(1)),
            Operand::Constant(Constant::Int { value: 1, ty: int }),
            Span::default(),
        ),
        assign(
            local_id(0),
            Rvalue::Use(Operand::Constant(Constant::Int { value: 0, ty: int })),
        ),
    ];
    fixture.check_body(&hir, &body(owner, locals, statements));
    fixture.assert_clean();
}
