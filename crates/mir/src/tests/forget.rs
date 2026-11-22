//! MIR lowering of the compiler-backed `mem::forget`.
//!
//! The public wrapper lives in `core`, so these fixtures are lowered as the
//! reserved `core` package. Calls through the private `__forget` builtin exercise
//! the same lowering path as the public wrapper body.

use crate::place::{Place, PlaceRoot};
use crate::statement::{Statement, StatementKind};
use crate::validate::ValidationError;
use crate::{LocalDecl, LocalId, LocalKind};
use seal_span::Span;
use seal_ty::{TyId, TyKind};

use super::support::{definition, function, lower_clean_in_core};

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

fn forget_places(body: &crate::Body) -> Vec<&crate::Place> {
    body.blocks
        .iter()
        .flat_map(|block| &block.statements)
        .filter_map(|statement| match &statement.kind {
            StatementKind::Forget { place } => Some(place),
            _ => None,
        })
        .collect()
}

#[test]
fn forgetting_a_non_copy_value_emits_a_forget_statement() {
    let source = "\
module mem; \
struct Pair { value: i32 } \
func consume(pair: Pair) { __forget(pair); }";
    let (fixture, hir, mir) = lower_clean_in_core(source);
    let body = function(&fixture, &hir, &mir, "consume");
    assert_eq!(forget_places(body).len(), 1);
    assert!(forget_places(body)[0].local_id().is_some());
}

#[test]
fn forgetting_a_copy_value_emits_no_statement() {
    let source = "module mem;\nfunc consume(value: i32) { __forget(value); }";
    let (fixture, hir, mir) = lower_clean_in_core(source);
    let body = function(&fixture, &hir, &mir, "consume");
    assert!(forget_places(body).is_empty());
}

#[test]
fn forgetting_a_constant_materializes_it_before_the_forget() {
    let source = "\
module mem; \
struct Pair { value: i32 } \
const VALUE: Pair = Pair { value: 1 }; \
func consume() { __forget(VALUE); }";
    let (fixture, hir, mir) = lower_clean_in_core(source);
    let body = function(&fixture, &hir, &mir, "consume");
    let places = forget_places(body);
    assert_eq!(places.len(), 1);
    assert!(
        matches!(places[0].root, PlaceRoot::Local(local) if body.local(local).kind == LocalKind::Temp),
        "a constant must be copied into a temporary before it is forgotten"
    );
}

#[test]
fn a_function_value_call_keeps_the_wrapper_free_of_drops() {
    let source = "\
module mem; \
struct Token { value: i32 } \
func forget<T>(value: T) { __forget(value); } \
func main() { let release: func(Token) = forget::<Token>; release(Token { value: 1 }); }";
    let (fixture, hir, mut mir) = lower_clean_in_core(source);
    crate::elaborate_drops(&hir, &mut mir).expect("drop elaboration succeeds");

    let wrapper = function(&fixture, &hir, &mir, "forget");
    assert_eq!(forget_places(wrapper).len(), 1);
    assert!(
        !wrapper.blocks.iter().any(|block| block
            .statements
            .iter()
            .any(|statement| matches!(statement.kind, StatementKind::Drop { .. }))),
        "the wrapper must not drop its parameter"
    );

    let main = function(&fixture, &hir, &mir, "main");
    assert!(
        main.blocks
            .iter()
            .any(|block| block.statements.iter().any(|statement| {
                matches!(
                    &statement.kind,
                    StatementKind::Assign {
                        rvalue: crate::Rvalue::Call {
                            target: crate::CallTarget::FnPtr { .. },
                            ..
                        },
                        ..
                    }
                )
            })),
        "a call whose result is discarded must still be emitted"
    );
    assert!(
        !main.blocks.iter().any(|block| block
            .statements
            .iter()
            .any(|statement| matches!(statement.kind, StatementKind::Drop { .. }))),
        "the moved argument must not be dropped by the caller"
    );
}

#[test]
fn a_forget_of_an_unsized_place_is_rejected() {
    let source = "module mem;\nfunc main() {}";
    let (fixture, hir, mir) = lower_clean_in_core(source);
    let str_ty = hir
        .ty_ctxt()
        .lookup(TyKind::Str)
        .expect("the test interner knows `str`");
    let mut body = function(&fixture, &hir, &mir, "main").clone();
    body.locals.push(local(str_ty, LocalKind::Arg));
    body.arg_count = 1;
    let block = body.entry;
    body.blocks[block].statements.push(Statement::forget(
        Place::local(local_id(1)),
        Default::default(),
    ));
    assert_eq!(
        body.validate_with(&hir),
        Err(ValidationError::MoveFromUnsizedPlace { block })
    );
}

#[test]
fn a_forget_of_a_global_is_rejected() {
    let source = "module mem;\nstatic VALUE: i32 = 0;\nfunc main() {}";
    let (fixture, hir, mir) = lower_clean_in_core(source);
    let value = definition(&fixture, &hir, seal_def::DefKind::Static, "VALUE");
    let mut body = function(&fixture, &hir, &mir, "main").clone();
    let block = body.entry;
    body.blocks[block].statements.push(Statement::forget(
        crate::Place::static_root(value),
        Default::default(),
    ));
    assert_eq!(
        body.validate_with(&hir),
        Err(ValidationError::DropOfGlobal { block, def: value })
    );
}
