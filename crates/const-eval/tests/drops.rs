//! Dropping a value while evaluating a constant.
//!
//! Lowering does not emit drop sites yet, so the tests add the statement to the
//! constant's body before evaluation runs.

mod support;

use seal_def::{DefId, DefKind};
use seal_hir::Hir;
use seal_mir::{Body, DropKind, Mir, Place, Statement};
use seal_span::Span;
use support::Fixture;

/// Returns the definition of the only item of `kind` in the file.
fn only(hir: &Hir, kind: DefKind) -> DefId {
    hir.definitions()
        .iter_enumerated()
        .find(|(_, data)| data.kind == kind)
        .map(|(id, _)| id)
        .unwrap_or_else(|| panic!("the source declares no {kind:?}"))
}

/// Appends a drop of the return place to the body owned by `owner`.
fn drop_return_place(mir: &mut Mir, owner: DefId) {
    let body = mir
        .bodies
        .iter_mut()
        .find(|body| body.owner == owner)
        .expect("the definition has a body");
    let entry = body.entry;
    body.blocks[entry].statements.push(Statement::drop(
        Place::local(Body::return_local()),
        DropKind::Scope,
        Span::default(),
    ));
}

#[test]
fn dropping_a_value_with_a_destructor_is_reported() {
    let source = "struct Pair { a: i32 }\n\n\
                  func destroy(value: *mut Pair) {}\n\n\
                  const VALUE: Pair = Pair { a: 1 };";
    let fixture = Fixture::compile_edited(source, |hir, mir| {
        let tcx = hir.ty_ctxt();
        let pair = hir
            .adt(only(hir, DefKind::Struct))
            .expect("the definition is an aggregate");
        // A destructor is the only way a value becomes non-trivial at compile
        // time; registering one is what the surface will do once `impl Drop`
        // exists.
        let previous = tcx.set_drop_method(pair, only(hir, DefKind::Function));
        assert!(
            previous.is_none(),
            "the aggregate starts without a destructor"
        );
        drop_return_place(mir, only(hir, DefKind::Const));
    });
    fixture.expect_code("E4028");
}

#[test]
fn dropping_a_value_without_a_destructor_is_allowed() {
    let source = "const VALUE: i32 = 7;";
    let fixture = Fixture::compile_edited(source, |hir, mir| {
        drop_return_place(mir, only(hir, DefKind::Const));
    });
    assert!(
        fixture.codes().is_empty(),
        "a drop that runs no destructor is a no-op, got {:?}",
        fixture.codes()
    );
}
