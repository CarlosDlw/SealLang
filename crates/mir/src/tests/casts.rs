//! Numeric cast lowering and structural validation.

use seal_ty::{CastKind, InferCtxt, NumericCastKind};

use crate::body::Body;
use crate::operand::Operand;
use crate::rvalue::Rvalue;
use crate::statement::StatementKind;
use crate::validate::ValidationError;

use super::support::{assignment, function, lower_clean, ty_text};

fn cast_rvalue(body: &Body) -> &Rvalue {
    body.blocks
        .iter()
        .flat_map(|block| block.statements.iter())
        .find_map(|statement| {
            let (_, rvalue) = assignment(statement);
            matches!(rvalue, Rvalue::Cast { .. }).then_some(rvalue)
        })
        .expect("the body contains a cast")
}

#[test]
fn an_integer_cast_records_its_kind_and_endpoints() {
    let (fixture, hir, mir) = lower_clean("func main(value: i32) -> i64 { value as i64 }");
    let body = function(&fixture, &hir, &mir, "main");
    let Rvalue::Cast {
        kind,
        operand,
        from,
        to,
    } = cast_rvalue(body)
    else {
        panic!("expected a cast");
    };
    assert_eq!(*kind, CastKind::Numeric(NumericCastKind::IntToInt));
    assert_eq!(ty_text(&hir, *from), "i32");
    assert_eq!(ty_text(&hir, *to), "i64");
    assert!(matches!(operand, Operand::Copy(_)));
}

#[test]
fn an_integer_to_float_cast_records_its_kind() {
    let (fixture, hir, mir) = lower_clean("func main(value: i32) -> f64 { value as f64 }");
    let body = function(&fixture, &hir, &mir, "main");
    let Rvalue::Cast { kind, from, to, .. } = cast_rvalue(body) else {
        panic!("expected a cast");
    };
    assert_eq!(*kind, CastKind::Numeric(NumericCastKind::IntToFloat));
    assert_eq!(ty_text(&hir, *from), "i32");
    assert_eq!(ty_text(&hir, *to), "f64");
}

#[test]
fn a_narrowing_cast_records_its_kind() {
    let (fixture, hir, mir) = lower_clean("func main(value: i32) -> u8 { value as u8 }");
    let body = function(&fixture, &hir, &mir, "main");
    let Rvalue::Cast { kind, to, .. } = cast_rvalue(body) else {
        panic!("expected a cast");
    };
    assert_eq!(*kind, CastKind::Numeric(NumericCastKind::IntToInt));
    assert_eq!(ty_text(&hir, *to), "u8");
}

#[test]
fn a_cast_kind_must_match_its_endpoint_types() {
    let source = "func main(value: i32) -> i64 { value as i64 }";
    let (fixture, hir, mir) = lower_clean(source);
    let mut body = function(&fixture, &hir, &mir, "main").clone();
    let mut found = false;
    for statement in body
        .blocks
        .iter_mut()
        .flat_map(|block| block.statements.iter_mut())
    {
        if let StatementKind::Assign {
            rvalue: Rvalue::Cast { kind, .. },
            ..
        } = &mut statement.kind
        {
            *kind = CastKind::Numeric(NumericCastKind::IntToFloat);
            found = true;
        }
    }
    assert!(found, "the body contains a cast");
    assert!(matches!(
        body.validate_with(&hir),
        Err(ValidationError::CastKindMismatch { .. })
    ));
}

#[test]
fn a_cast_operand_type_must_match_its_source() {
    let source = "func main(value: i32) -> i64 { value as i64 }";
    let (fixture, hir, mir) = lower_clean(source);
    let mut body = function(&fixture, &hir, &mir, "main").clone();
    let u8 = hir.ty_ctxt().uint_ty(seal_ty::UintTy::U8);
    for statement in body
        .blocks
        .iter_mut()
        .flat_map(|block| block.statements.iter_mut())
    {
        if let StatementKind::Assign {
            rvalue: Rvalue::Cast { from, .. },
            ..
        } = &mut statement.kind
        {
            *from = u8;
        }
    }
    assert!(matches!(
        body.validate_with(&hir),
        Err(ValidationError::CastOperandTypeMismatch { .. })
    ));
}

#[test]
fn a_cast_destination_must_match_its_target() {
    let source = "func main(value: i32) -> i64 { value as i64 }";
    let (fixture, hir, mir) = lower_clean(source);
    let mut body = function(&fixture, &hir, &mir, "main").clone();
    let f64 = hir.ty_ctxt().float_ty(seal_ty::FloatTy::F64);
    for statement in body
        .blocks
        .iter_mut()
        .flat_map(|block| block.statements.iter_mut())
    {
        if let StatementKind::Assign {
            rvalue: Rvalue::Cast { to, .. },
            ..
        } = &mut statement.kind
        {
            *to = f64;
        }
    }
    assert!(matches!(
        body.validate_with(&hir),
        Err(ValidationError::CastDestinationTypeMismatch { .. })
    ));
}

#[test]
fn a_cast_endpoint_must_be_concrete() {
    let source = "func main(value: i32) -> i64 { value as i64 }";
    let (fixture, hir, mir) = lower_clean(source);
    let mut body = function(&fixture, &hir, &mir, "main").clone();
    let mut infer = InferCtxt::new();
    let infer_ty = infer.fresh_ty_var(hir.ty_ctxt());
    for statement in body
        .blocks
        .iter_mut()
        .flat_map(|block| block.statements.iter_mut())
    {
        if let StatementKind::Assign {
            rvalue: Rvalue::Cast { from, .. },
            ..
        } = &mut statement.kind
        {
            *from = infer_ty;
        }
    }
    assert!(matches!(
        body.validate_with(&hir),
        Err(ValidationError::CastTypeNotConcrete { .. })
    ));
}

#[test]
fn a_cast_reads_its_operand_and_writes_one_destination() {
    let (fixture, hir, mir) =
        lower_clean("func main(value: i32) -> i64 { let converted = value as i64; converted }");
    let body = function(&fixture, &hir, &mir, "main");
    let mut casts = 0;
    for (place, rvalue) in body
        .blocks
        .iter()
        .flat_map(|block| block.statements.iter())
        .map(assignment)
    {
        if let Rvalue::Cast { operand, .. } = rvalue {
            casts += 1;
            assert!(matches!(operand, Operand::Copy(_)));
            assert_ne!(place.expect_local(), Body::return_local());
        }
    }
    assert_eq!(casts, 1);
}
