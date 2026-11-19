//! C scalar type names resolve against the target C ABI.

use seal_target::TargetTriple;
use seal_ty::{CAbiProperties, IntTy, TyId, TyKind, UintTy};

use super::support::{Fixture, function};
use crate::{Hir, StmtKind};

fn c_abi(triple: &str) -> CAbiProperties {
    let spec = TargetTriple::parse(triple)
        .expect("the test triple is valid")
        .spec();
    CAbiProperties::for_target(&spec)
}

fn annotation(fixture: &Fixture, hir: &Hir, name: &str) -> TyId {
    let (_, body) = function(fixture, hir, "main");
    let statements = hir.block(body.root).statements.clone();
    for statement in statements {
        let kind = hir.stmt(statement).kind.clone();
        let StmtKind::Let {
            local,
            ty: Some(ty),
            ..
        } = kind
        else {
            continue;
        };
        if fixture.symbol(hir.definitions().get(local).name) == name {
            return ty;
        }
    }
    panic!("no annotated let named `{name}`");
}

fn kind(hir: &Hir, ty: TyId) -> TyKind {
    hir.ty_ctxt().kind(ty)
}

#[test]
fn c_scalars_resolve_to_the_lp64_unix_types() {
    let fixture = Fixture::new(
        "func main() {
            let a: c_int = 1;
            let b: c_uint = 1;
            let c: c_long = 1;
            let d: c_char = 1;
            let e: c_wchar = 1;
            let f: size_t = 1;
            let g: ssize_t = 1;
        }",
    );
    let hir = fixture.lower_with_c_abi(c_abi("x86_64-unknown-linux-gnu"));
    fixture.assert_clean();

    assert_eq!(
        kind(&hir, annotation(&fixture, &hir, "a")),
        TyKind::Int(IntTy::I32)
    );
    assert_eq!(
        kind(&hir, annotation(&fixture, &hir, "b")),
        TyKind::Uint(UintTy::U32)
    );
    assert_eq!(
        kind(&hir, annotation(&fixture, &hir, "c")),
        TyKind::Int(IntTy::I64)
    );
    assert_eq!(
        kind(&hir, annotation(&fixture, &hir, "d")),
        TyKind::Int(IntTy::I8)
    );
    assert_eq!(
        kind(&hir, annotation(&fixture, &hir, "e")),
        TyKind::Int(IntTy::I32)
    );
    assert_eq!(
        kind(&hir, annotation(&fixture, &hir, "f")),
        TyKind::Uint(UintTy::Usize)
    );
    assert_eq!(
        kind(&hir, annotation(&fixture, &hir, "g")),
        TyKind::Int(IntTy::Isize)
    );
}

#[test]
fn windows_is_llp64_and_wchar_is_unsigned() {
    let fixture = Fixture::new(
        "func main() {
            let long: c_long = 1;
            let wchar: c_wchar = 1;
            let boolean: c_bool = 1;
        }",
    );
    let hir = fixture.lower_with_c_abi(c_abi("x86_64-pc-windows-msvc"));
    fixture.assert_clean();

    assert_eq!(
        kind(&hir, annotation(&fixture, &hir, "long")),
        TyKind::Int(IntTy::I32)
    );
    assert_eq!(
        kind(&hir, annotation(&fixture, &hir, "wchar")),
        TyKind::Uint(UintTy::U16)
    );
    assert_eq!(
        kind(&hir, annotation(&fixture, &hir, "boolean")),
        TyKind::Uint(UintTy::U8)
    );
}

#[test]
fn aarch64_linux_uses_an_unsigned_c_char() {
    let fixture = Fixture::new("func main() { let value: c_char = 1; }");
    let hir = fixture.lower_with_c_abi(c_abi("aarch64-unknown-linux-gnu"));
    fixture.assert_clean();

    assert_eq!(
        kind(&hir, annotation(&fixture, &hir, "value")),
        TyKind::Uint(UintTy::U8)
    );
}
