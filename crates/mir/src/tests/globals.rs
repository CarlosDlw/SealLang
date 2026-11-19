//! MIR lowering for constants, statics, and their bodies.

use crate::{BodyKind, Operand, PlaceRoot, Rvalue, Statement, StatementKind};
use seal_def::DefKind;

use super::support::{definition, function, lower_clean};

fn statements(body: &crate::Body) -> Vec<&Statement> {
    body.blocks
        .iter()
        .flat_map(|block| block.statements.iter())
        .collect()
}

fn places(body: &crate::Body) -> Vec<PlaceRoot> {
    statements(body)
        .into_iter()
        .flat_map(|statement| match &statement.kind {
            StatementKind::Assign { place, rvalue } => {
                let mut roots = vec![place.root];
                roots.extend(match rvalue {
                    Rvalue::Use(Operand::Copy(place) | Operand::Move(place))
                    | Rvalue::Ref { place, .. }
                    | Rvalue::SliceRef { base: place, .. } => vec![place.root],
                    _ => Vec::new(),
                });
                roots
            }
            other => panic!("expected an assignment, found {other:?}"),
        })
        .collect()
}

#[test]
fn const_function_bodies_carry_the_const_flag() {
    let (fixture, hir, mir) = lower_clean(
        "const func twice(value: i32) -> i32 { value * 2 } \
         const RESULT: i32 = twice(21);",
    );
    let function = definition(&fixture, &hir, DefKind::Function, "twice");
    let body = mir.body_of(function).expect("twice has a body");
    assert!(body.is_const);
    assert_eq!(body.kind, BodyKind::Function);

    let value = definition(&fixture, &hir, DefKind::Const, "RESULT");
    let body = mir.body_of(value).expect("RESULT has a body");
    assert!(!body.is_const);
    assert_eq!(body.kind, BodyKind::Const);
}

#[test]
fn a_static_read_uses_a_static_rooted_place() {
    let (fixture, hir, mir) = lower_clean("static LIMIT: i32 = 3; func main() -> i32 { LIMIT }");
    let limit = definition(&fixture, &hir, DefKind::Static, "LIMIT");
    let body = function(&fixture, &hir, &mir, "main");
    assert!(places(body).contains(&PlaceRoot::Static(limit)));
}

#[test]
fn a_static_write_target_keeps_the_static_root() {
    let source = "static mut COUNTER: i32 = 0; \
                  func main() -> i32 { unsafe { COUNTER = 1; COUNTER } }";
    let (fixture, hir, mir) = lower_clean(source);
    let counter = definition(&fixture, &hir, DefKind::Static, "COUNTER");
    let body = function(&fixture, &hir, &mir, "main");
    let destinations: Vec<PlaceRoot> = statements(body)
        .into_iter()
        .map(|statement| {
            let StatementKind::Assign { place, .. } = &statement.kind else {
                panic!("expected an assignment, found {:?}", statement.kind);
            };
            place.root
        })
        .collect();
    assert!(destinations.contains(&PlaceRoot::Static(counter)));
}

#[test]
fn a_static_field_projection_stays_on_the_static() {
    let source = "struct Config { limit: i32 } \
                  static CONFIG: Config = Config { limit: 4 }; \
                  func main() -> i32 { CONFIG.limit }";
    let (fixture, hir, mir) = lower_clean(source);
    let config = definition(&fixture, &hir, DefKind::Static, "CONFIG");
    let body = function(&fixture, &hir, &mir, "main");
    let projected = statements(body).into_iter().any(|statement| {
        let StatementKind::Assign { rvalue, .. } = &statement.kind else {
            panic!("expected an assignment, found {:?}", statement.kind);
        };
        matches!(
            rvalue,
            Rvalue::Use(Operand::Copy(place) | Operand::Move(place))
                if place.root == PlaceRoot::Static(config) && place.projections.len() == 1
        )
    });
    assert!(projected, "the field read projects from the static");
}

#[test]
fn borrowing_a_static_keeps_the_static_root() {
    let source = "struct Config { limit: i32 } \
                  static CONFIG: Config = Config { limit: 4 }; \
                  func main() -> *Config { &CONFIG }";
    let (fixture, hir, mir) = lower_clean(source);
    let config = definition(&fixture, &hir, DefKind::Static, "CONFIG");
    let body = function(&fixture, &hir, &mir, "main");
    let borrowed = statements(body).into_iter().any(|statement| {
        let StatementKind::Assign { rvalue, .. } = &statement.kind else {
            panic!("expected an assignment, found {:?}", statement.kind);
        };
        matches!(
            rvalue,
            Rvalue::Ref { place, .. } if place.root == PlaceRoot::Static(config)
        )
    });
    assert!(borrowed, "the borrow names the static allocation");
}

#[test]
fn slicing_a_static_keeps_the_static_root() {
    let source = "static VALUES: [i32; 2] = [1, 2]; \
                  func main() -> *[i32] { &VALUES[..] }";
    let (fixture, hir, mir) = lower_clean(source);
    let values = definition(&fixture, &hir, DefKind::Static, "VALUES");
    let body = function(&fixture, &hir, &mir, "main");
    let sliced = statements(body).into_iter().any(|statement| {
        let StatementKind::Assign { rvalue, .. } = &statement.kind else {
            panic!("expected an assignment, found {:?}", statement.kind);
        };
        matches!(
            rvalue,
            Rvalue::SliceRef { base, .. } if base.root == PlaceRoot::Static(values)
        )
    });
    assert!(sliced, "the slice names the static allocation");
}

#[test]
fn a_static_argument_is_passed_from_the_static() {
    let source = "static LIMIT: i32 = 3; \
                  func identity(value: i32) -> i32 { value } \
                  func main() -> i32 { identity(LIMIT) }";
    let (fixture, hir, mir) = lower_clean(source);
    let limit = definition(&fixture, &hir, DefKind::Static, "LIMIT");
    let body = function(&fixture, &hir, &mir, "main");
    let mut static_temp = None;
    for statement in statements(body) {
        let StatementKind::Assign { place, rvalue } = &statement.kind else {
            panic!("expected an assignment, found {:?}", statement.kind);
        };
        if let Rvalue::Use(Operand::Copy(source)) = rvalue
            && source.root == PlaceRoot::Static(limit)
        {
            static_temp = place.local_id();
        }
    }
    let static_temp = static_temp.expect("the static is read into a temporary");
    let passed = statements(body).into_iter().any(|statement| {
        let StatementKind::Assign { rvalue, .. } = &statement.kind else {
            panic!("expected an assignment, found {:?}", statement.kind);
        };
        matches!(
            rvalue,
            Rvalue::Call { args, .. }
                if args.iter().any(|operand| {
                    operand
                        .place()
                        .is_some_and(|place| place.local_id() == Some(static_temp))
                })
        )
    });
    assert!(passed, "the argument reads the temporary fed by the static");
}

#[test]
fn a_const_read_is_a_const_operand() {
    let (fixture, hir, mir) = lower_clean("const ANSWER: i32 = 42; func main() -> i32 { ANSWER }");
    let answer = definition(&fixture, &hir, DefKind::Const, "ANSWER");
    let body = function(&fixture, &hir, &mir, "main");
    let read = statements(body).into_iter().any(|statement| {
        let StatementKind::Assign { rvalue, .. } = &statement.kind else {
            panic!("expected an assignment, found {:?}", statement.kind);
        };
        matches!(rvalue, Rvalue::Use(Operand::Const(def)) if *def == answer)
    });
    assert!(read, "a const path becomes a const operand");
}

#[test]
fn a_const_field_is_materialized_into_a_temporary() {
    let source = "struct Point { x: i32, y: i32 } \
                  const ORIGIN: Point = Point { x: 0, y: 0 }; \
                  func main() -> i32 { ORIGIN.x }";
    let (fixture, hir, mir) = lower_clean(source);
    let origin = definition(&fixture, &hir, DefKind::Const, "ORIGIN");
    let body = function(&fixture, &hir, &mir, "main");

    let mut materialized = None;
    for statement in statements(body) {
        let StatementKind::Assign { place, rvalue } = &statement.kind else {
            panic!("expected an assignment, found {:?}", statement.kind);
        };
        if let Rvalue::Use(Operand::Const(def)) = rvalue
            && *def == origin
        {
            materialized = Some(place.root);
        }
    }
    let root = materialized.expect("the constant is materialized into a temporary");
    assert!(
        matches!(root, PlaceRoot::Local(_)),
        "materialization targets a local temporary"
    );
    assert!(
        !places(body).contains(&PlaceRoot::Const(origin)),
        "a const root never survives as a MIR place"
    );
}

#[test]
fn borrowing_a_const_materializes_it_first() {
    let (fixture, hir, mir) =
        lower_clean("const ANSWER: i32 = 42; func main() -> *i32 { &ANSWER }");
    let answer = definition(&fixture, &hir, DefKind::Const, "ANSWER");
    let body = function(&fixture, &hir, &mir, "main");
    let has_operand = statements(body).into_iter().any(|statement| {
        let StatementKind::Assign { rvalue, .. } = &statement.kind else {
            panic!("expected an assignment, found {:?}", statement.kind);
        };
        matches!(rvalue, Rvalue::Use(Operand::Const(def)) if *def == answer)
    });
    let borrowed_local = statements(body).into_iter().any(|statement| {
        let StatementKind::Assign { rvalue, .. } = &statement.kind else {
            panic!("expected an assignment, found {:?}", statement.kind);
        };
        matches!(rvalue, Rvalue::Ref { place, .. } if place.root != PlaceRoot::Const(answer))
    });
    assert!(
        has_operand && borrowed_local,
        "the borrow targets the temporary"
    );
}

#[test]
fn value_bodies_record_their_kind_and_have_no_arguments() {
    let source = "const ANSWER: i32 = 42; static mut COUNTER: i32 = 0; \
                  func main() -> i32 { ANSWER }";
    let (fixture, hir, mir) = lower_clean(source);
    let answer = definition(&fixture, &hir, DefKind::Const, "ANSWER");
    let counter = definition(&fixture, &hir, DefKind::Static, "COUNTER");

    let constant = mir.body_of(answer).expect("const body");
    assert_eq!(constant.kind, BodyKind::Const);
    assert_eq!(constant.arg_count, 0);
    let static_body = mir.body_of(counter).expect("static body");
    assert_eq!(static_body.kind, BodyKind::Static { mutable: true });
    assert_eq!(static_body.arg_count, 0);
    assert_eq!(
        function(&fixture, &hir, &mir, "main").kind,
        BodyKind::Function
    );
}

#[test]
fn a_value_body_with_arguments_fails_validation() {
    let source = "func identity(value: i32) -> i32 { value }";
    let (fixture, hir, mir) = lower_clean(source);
    let mut body = function(&fixture, &hir, &mir, "identity").clone();
    body.kind = BodyKind::Const;
    assert!(matches!(
        body.validate_with(&hir),
        Err(crate::ValidationError::ValueBodyHasArguments { arg_count: 1 })
    ));
}

#[test]
fn a_place_root_of_the_wrong_kind_fails_validation() {
    let source = "static LIMIT: i32 = 3; func identity(value: i32) -> i32 { value }";
    let (fixture, hir, mir) = lower_clean(source);
    let identity = definition(&fixture, &hir, DefKind::Function, "identity");
    let mut body = function(&fixture, &hir, &mir, "identity").clone();
    let block = body.entry;
    let statement = body.blocks[block]
        .statements
        .first_mut()
        .expect("the body assigns its argument");
    let StatementKind::Assign { place, .. } = &mut statement.kind else {
        panic!("expected an assignment, found {:?}", statement.kind);
    };
    place.root = PlaceRoot::Static(identity);
    assert!(matches!(
        body.validate_with(&hir),
        Err(crate::ValidationError::PlaceRootKindMismatch { .. })
    ));
}
