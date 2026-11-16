//! Call-site aliasing and exclusive arguments.

use seal_def::DefId;
use seal_hir::Hir;
use seal_mir::Mir;
use seal_ty::{RefKind, TyKind};

use super::support::{Fixture, check_clean, check_errors};

/// Returns the function that declares an exclusive-reference parameter.
fn exclusive_owner(hir: &Hir, mir: &Mir) -> DefId {
    mir.bodies()
        .iter()
        .find(|body| {
            body.args().any(|argument| {
                matches!(
                    hir.ty_ctxt().kind(argument.ty),
                    TyKind::Ref {
                        kind: RefKind::Exclusive,
                        ..
                    }
                )
            })
        })
        .expect("the test declares an exclusive-reference parameter")
        .owner
}

#[test]
fn two_shared_arguments_may_alias() {
    check_clean(
        "func sum(a: *i32, b: *i32) -> i32 {\n  *a + *b\n}\n\n\
         func main() -> i32 {\n  let x = 1;\n  let r = &x;\n  sum(r, r)\n}\n",
    );
}

#[test]
fn exclusive_arguments_must_name_distinct_storage() {
    let fixture = check_errors(
        "func take(a: *mut i32, b: *mut i32) {}\n\n\
         func main() -> i32 {\n  let mut x = 1;\n  let r = &mut x;\n  take(r, r);\n  0\n}\n",
    );
    fixture.expect("E5010", "may name the same storage");
}

#[test]
fn an_aggregate_argument_counts_as_its_reference_fields() {
    let fixture = check_errors(
        "struct Holder {\n  p: *mut i32,\n}\n\n\
         func take(holder: Holder, other: *mut i32) {}\n\n\
         func main() -> i32 {\n  let mut x = 1;\n  let r = &mut x;\n  let holder = Holder { p: r };\n  take(holder, r);\n  0\n}\n",
    );
    fixture.expect("E5010", "may name the same storage");
}

#[test]
fn an_exclusive_argument_conflicts_with_a_live_reborrow() {
    let fixture = check_errors(
        "func take(p: *mut i32) {}\n\n\
         func main() -> i32 {\n  let mut x = 1;\n  let r = &mut x;\n  let s = &*r;\n  take(r);\n  *s\n}\n",
    );
    fixture.expect("E5009", "cannot pass `r` as an exclusive argument");
}

#[test]
fn a_reborrow_argument_is_authorized_by_its_parent_loan() {
    check_clean(
        "func take(p: *mut i32) {}\n\n\
         func main() -> i32 {\n  let mut x = 1;\n  let r = &mut x;\n  take(&mut *r);\n  *r\n}\n",
    );
}

#[test]
fn field_arguments_of_one_pointee_stay_distinct() {
    check_clean(
        "struct Point {\n  x: i32,\n  y: i32,\n}\n\n\
         func take(a: *mut i32, b: *mut i32) {}\n\n\
         func main() -> i32 {\n  let mut point = Point { x: 1, y: 2 };\n  let p = &mut point;\n  take(&mut (*p).x, &mut (*p).y);\n  0\n}\n",
    );
}

#[test]
fn a_proven_callee_may_be_marked_noalias() {
    let fixture = Fixture::new(
        "func bump(count: *mut i32) {\n  *count = *count + 1;\n}\n\n\
         func main() -> i32 {\n  let mut value = 1;\n  bump(&mut value);\n  value\n}\n",
    );
    let (hir, mir, facts) = fixture.check_facts();
    let owner = exclusive_owner(&hir, &mir);
    assert!(
        facts.noalias_functions().contains(&owner),
        "the proof closes for `bump`"
    );
}

#[test]
fn a_callee_that_follows_an_unknown_pointer_is_not_marked_noalias() {
    let fixture = Fixture::new(
        "struct Node {\n  next: *mut i32,\n}\n\n\
         func follow(node: *mut Node) -> i32 {\n  let next = (*node).next;\n  *next\n}\n\n\
         func main() -> i32 {\n  let mut value = 7;\n  let mut node = Node { next: &mut value };\n  follow(&mut node)\n}\n",
    );
    let (hir, mir, facts) = fixture.check_facts();
    let owner = exclusive_owner(&hir, &mir);
    assert!(
        !facts.noalias_functions().contains(&owner),
        "`follow` reaches its pointee through an unknown pointer"
    );
}

#[test]
fn a_call_that_cannot_prove_its_argument_withholds_noalias() {
    let fixture = Fixture::new(
        "struct Node {\n  next: *mut i32,\n}\n\n\
         func bump(count: *mut i32) {\n  *count = *count + 1;\n}\n\n\
         func main() -> i32 {\n  let mut value = 7;\n  let mut node = Node { next: &mut value };\n  let r = &node;\n  bump((*r).next);\n  0\n}\n",
    );
    let (hir, mir, facts) = fixture.check_facts();
    let owner = exclusive_owner(&hir, &mir);
    assert!(
        !facts.noalias_functions().contains(&owner),
        "the call site cannot prove the pointer's storage"
    );
}
