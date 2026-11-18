//! Constants, statics, and their initializer bodies.

use seal_def::{DefId, DefKind};

use seal_ty::IntTy;

use crate::{BodyKind, ExprKind, Hir, Res, StmtKind};

use super::support::{Fixture, assert_invariants, function, lower_clean, path_res, tail};

fn definition(fixture: &Fixture, hir: &Hir, kind: DefKind, name: &str) -> DefId {
    hir.definitions()
        .iter_enumerated()
        .find(|(_, data)| data.kind == kind && fixture.symbol(data.name) == name)
        .map(|(id, _)| id)
        .unwrap_or_else(|| panic!("no {kind:?} named `{name}`"))
}

fn declaration_body(hir: &Hir, statement: crate::HirStmtId) -> (DefId, &crate::Body) {
    let def = match hir.stmt(statement).kind.clone() {
        StmtKind::Const { def } | StmtKind::Static { def } => def,
        other => panic!("statement is not a value declaration: {other:?}"),
    };
    let body = hir
        .body_of(def)
        .expect("a value declaration owns its initializer body");
    (def, body)
}

#[test]
fn file_scope_values_resolve_from_later_items() {
    let source = "const ANSWER: i32 = 42;\n\
                  static GREETING: *str = \"hello\";\n\
                  func main() -> i32 { ANSWER }";
    let (fixture, hir) = lower_clean(source);

    let answer = definition(&fixture, &hir, DefKind::Const, "ANSWER");
    let body = hir.body_of(answer).expect("constant body");
    assert_eq!(body.kind, BodyKind::Const);
    assert!(body.params.is_empty(), "a constant has no parameters");
    assert_eq!(body.ret, Some(hir.ty_ctxt().int_ty(IntTy::I32)));
    assert!(matches!(hir.expr(tail(&hir, body)).kind, ExprKind::Lit(_)));

    let greeting = definition(&fixture, &hir, DefKind::Static, "GREETING");
    assert!(!hir.definitions().get(greeting).mutable);
    let body = hir.body_of(greeting).expect("static body");
    assert_eq!(body.kind, BodyKind::Static { mutable: false });
    assert!(body.params.is_empty(), "a static has no parameters");

    let (_, main) = function(&fixture, &hir, "main");
    assert!(matches!(
        path_res(&hir, tail(&hir, main)),
        Res::Const(def) if def == answer
    ));
}

#[test]
fn a_constant_can_name_a_static_declared_later() {
    let (fixture, hir) = lower_clean("const A: i32 = B;\nstatic B: i32 = 1;");
    let a = definition(&fixture, &hir, DefKind::Const, "A");
    let b = definition(&fixture, &hir, DefKind::Static, "B");
    let body = hir.body_of(a).expect("constant body");
    assert!(matches!(
        path_res(&hir, tail(&hir, body)),
        Res::Static(def) if def == b
    ));
}

#[test]
fn static_mut_records_mutability_in_every_place() {
    let (fixture, hir) = lower_clean("static mut COUNTER: i32 = 0;");
    let counter = definition(&fixture, &hir, DefKind::Static, "COUNTER");
    let data = hir.definitions().get(counter);
    assert!(data.mutable, "the definition records `mut`");
    assert!(data.parent.is_none(), "a file-scope static has no owner");
    let body = hir.body_of(counter).expect("static body");
    assert_eq!(body.kind, BodyKind::Static { mutable: true });
}

#[test]
fn functions_and_values_share_one_value_namespace() {
    let fixture = Fixture::new("func X() {}\nconst X: i32 = 1;");
    fixture.lower();
    fixture.expect("E2068", "the value `X` is defined twice");
    assert_eq!(fixture.codes(), ["E2068"]);
}

#[test]
fn const_and_static_names_cannot_collide() {
    let fixture = Fixture::new("const X: i32 = 1;\nstatic X: i32 = 2;");
    fixture.lower();
    fixture.expect("E2068", "the value `X` is defined twice");
}

#[test]
fn a_value_may_share_a_name_with_a_type() {
    let (fixture, hir) = lower_clean("struct LIMIT { n: i32 }\nconst LIMIT: i32 = 1;");
    assert!(
        hir.adt(definition(&fixture, &hir, DefKind::Struct, "LIMIT"))
            .is_some()
    );
    assert!(
        hir.body_of(definition(&fixture, &hir, DefKind::Const, "LIMIT"))
            .is_some()
    );
}

#[test]
fn a_local_constant_owns_its_initializer_body() {
    let source = "func main() -> i32 { const SCALE: i32 = 3; SCALE }";
    let (fixture, hir) = lower_clean(source);
    let (main, body) = function(&fixture, &hir, "main");
    let statement = body.root;
    let declaration = hir.block(statement).statements[0];
    let (def, initializer) = declaration_body(&hir, declaration);

    let data = hir.definitions().get(def);
    assert_eq!(data.kind, DefKind::Const);
    assert_eq!(data.parent, Some(main));
    assert!(!data.mutable);
    assert_eq!(initializer.kind, BodyKind::Const);
    assert!(initializer.params.is_empty());
    assert_eq!(initializer.ret, Some(hir.ty_ctxt().int_ty(IntTy::I32)));

    assert!(matches!(
        path_res(&hir, tail(&hir, body)),
        Res::Const(resolved) if resolved == def
    ));
}

#[test]
fn a_local_declaration_is_not_visible_in_its_own_initializer() {
    let source = "const X: i32 = 1;\n\
                  func main() -> i32 {\n\
                      const X: i32 = X;\n\
                      X\n\
                  }";
    let (fixture, hir) = lower_clean(source);
    let global = definition(&fixture, &hir, DefKind::Const, "X");
    let (_, body) = function(&fixture, &hir, "main");
    let declaration = hir.block(body.root).statements[0];
    let (local, initializer) = declaration_body(&hir, declaration);

    assert!(matches!(
        path_res(&hir, tail(&hir, initializer)),
        Res::Const(resolved) if resolved == global
    ));
    assert!(matches!(
        path_res(&hir, tail(&hir, body)),
        Res::Const(resolved) if resolved == local
    ));
    assert_ne!(local, global);
}

#[test]
fn a_local_constant_may_shadow_an_outer_value() {
    let source = "const X: i32 = 1;\nfunc main() -> i32 { const X: i32 = 2; X }";
    let (fixture, hir) = lower_clean(source);
    let global = definition(&fixture, &hir, DefKind::Const, "X");
    let (_, body) = function(&fixture, &hir, "main");
    let declaration = hir.block(body.root).statements[0];
    let (local, _) = declaration_body(&hir, declaration);

    assert!(matches!(
        path_res(&hir, tail(&hir, body)),
        Res::Const(resolved) if resolved == local
    ));
    assert_ne!(local, global);
}

#[test]
fn a_duplicate_local_declaration_is_rejected() {
    let fixture = Fixture::new("func main() { const X: i32 = 1; const X: i32 = 2; }");
    fixture.lower();
    fixture.expect("E2068", "the value `X` is defined twice");
    assert_eq!(fixture.codes(), ["E2068"]);
}

#[test]
fn a_local_static_owns_one_initializer_body() {
    let source = "func main() -> i32 { static mut CALLS: i32 = 0; 0 }";
    let (fixture, hir) = lower_clean(source);
    let (main, body) = function(&fixture, &hir, "main");
    let declaration = hir.block(body.root).statements[0];
    let (def, initializer) = declaration_body(&hir, declaration);

    let data = hir.definitions().get(def);
    assert_eq!(data.kind, DefKind::Static);
    assert!(data.mutable);
    assert_eq!(data.parent, Some(main));
    assert_eq!(initializer.kind, BodyKind::Static { mutable: true });
    assert!(initializer.params.is_empty());
}

#[test]
fn a_local_static_cannot_capture_a_parameter() {
    let fixture = Fixture::new("func f(x: i32) -> i32 { static S: i32 = x; S }");
    fixture.lower();
    fixture.expect("E3060", "cannot capture `x`");
    assert_eq!(fixture.codes(), ["E3060"]);
}

#[test]
fn a_local_static_cannot_capture_a_binding() {
    let fixture = Fixture::new("func f() -> i32 { let x = 1; static S: i32 = x; S }");
    fixture.lower();
    fixture.expect("E3060", "cannot capture `x`");
    assert_eq!(fixture.codes(), ["E3060"]);
}

#[test]
fn a_local_static_cannot_capture_a_local_constant() {
    let fixture = Fixture::new("func f() -> i32 { const X: i32 = 1; static S: i32 = X; S }");
    fixture.lower();
    fixture.expect("E3060", "cannot capture `X`");
}

#[test]
fn a_local_static_cannot_mention_a_generic_parameter() {
    let fixture = Fixture::new("func f<T>(value: T) -> i32 { static S: T = value; 0 }");
    fixture.lower();
    fixture.expect("E3060", "generic parameter `T`");
    assert!(
        fixture.codes().iter().all(|code| code == "E3060"),
        "all capture diagnostics use E3060: {:?}",
        fixture.codes()
    );
}

#[test]
fn a_local_static_cannot_mention_self() {
    let source = "struct Point { x: i32 }\n\
                  impl Point {\n\
                      func f() -> i32 {\n\
                          static S: Self = Point { x: 1 };\n\
                          0\n\
                      }\n\
                  }";
    let fixture = Fixture::new(source);
    fixture.lower();
    fixture.expect("E3060", "cannot mention `Self`");
}

#[test]
fn a_const_function_body_records_its_flag() {
    let (fixture, hir) = lower_clean("const func twice(value: i32) -> i32 { value + value }");
    let (_, body) = function(&fixture, &hir, "twice");
    assert!(body.is_const, "a `const func` body records the flag");

    let (fixture, hir) = lower_clean("func plain(value: i32) -> i32 { value }");
    let (_, body) = function(&fixture, &hir, "plain");
    assert!(!body.is_const);
}

#[test]
fn an_unsafe_expression_keeps_its_block() {
    let (fixture, hir) = lower_clean("func main() -> i32 { unsafe { 1 } }");
    let (_, body) = function(&fixture, &hir, "main");
    let ExprKind::Unsafe { block } = hir.expr(tail(&hir, body)).kind else {
        panic!("the unsafe expression keeps its own kind");
    };
    assert!(hir.block(block).tail.is_some(), "the block keeps its tail");
}

#[test]
fn a_local_constant_cannot_capture_a_parameter() {
    let fixture = Fixture::new("func f(x: i32) -> i32 { const X: i32 = x; X }");
    fixture.lower();
    fixture.expect("E3060", "cannot capture `x`");
}

#[test]
fn value_bodies_keep_hir_invariants() {
    let sources = [
        "const A: i32 = 1;",
        "static mut B: i32 = 0;",
        "const A: i32 = 1; static B: i32 = A;",
        "func main() -> i32 { const X: i32 = 1; X }",
        "func main() -> i32 { static mut S: i32 = 0; 0 }",
        "func f(x: i32) -> i32 { static S: i32 = x; S }",
        "func f<T>(value: T) -> i32 { static S: T = value; 0 }",
        "func f() -> i32 { const func g() -> i32 { 1 } 2 }",
    ];
    for source in sources {
        let fixture = Fixture::new(source);
        let hir = fixture.lower();
        assert_invariants(&fixture, &hir);
    }
}

#[test]
fn a_local_static_may_read_a_file_scope_constant() {
    let source = "const BASE: i32 = 2;\n\
                  func f() -> i32 { static S: i32 = BASE; S }";
    let (fixture, hir) = lower_clean(source);
    let base = definition(&fixture, &hir, DefKind::Const, "BASE");
    let (_, body) = function(&fixture, &hir, "f");
    let declaration = hir.block(body.root).statements[0];
    let (_, initializer) = declaration_body(&hir, declaration);
    assert!(matches!(
        path_res(&hir, tail(&hir, initializer)),
        Res::Const(resolved) if resolved == base
    ));
}
