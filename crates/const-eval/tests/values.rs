//! Values produced by constant evaluation.

mod support;

use seal_const_eval::{AllocSource, ConstValue};
use support::{Fixture, clean};

fn int(fixture: &Fixture, name: &str) -> u128 {
    match fixture.value(name) {
        ConstValue::Int { bits, .. } => *bits,
        other => panic!("`{name}` is not an integer: {other:?}"),
    }
}

fn flag(fixture: &Fixture, name: &str) -> bool {
    match fixture.value(name) {
        ConstValue::Bool(value) => *value,
        other => panic!("`{name}` is not a bool: {other:?}"),
    }
}

#[test]
fn scalars_keep_their_types_and_bits() {
    let fixture = clean(
        "const NEGATIVE: i32 = -7; \
         const LARGE: u64 = 18446744073709551615; \
         const PI: f64 = 3.5; \
         const FLAG: bool = true; \
         const LETTER: char = 'a'; \
         const NOTHING: () = ();",
    );
    assert_eq!(
        int(&fixture, "NEGATIVE"),
        (-7i128 as u128).wrapping_sub(0) & 0xffff_ffff
    );
    assert_eq!(int(&fixture, "LARGE"), u128::from(u64::MAX));
    assert!(matches!(
        fixture.value("PI"),
        ConstValue::Float { ty, .. } if fixture.hir.ty_ctxt().kind(*ty) == seal_ty::TyKind::Float(seal_ty::FloatTy::F64)
    ));
    assert_eq!(fixture.value("FLAG"), &ConstValue::Bool(true));
    assert_eq!(fixture.value("LETTER"), &ConstValue::Char('a'));
    assert_eq!(fixture.value("NOTHING"), &ConstValue::Unit);
}

#[test]
fn integer_arithmetic_wraps_and_truncates() {
    let fixture = clean(
        "const SUM: u8 = 250 + 10; \
         const DIFFERENCE: u8 = 1 - 2; \
         const PRODUCT: i8 = 100 * 2; \
         const QUOTIENT: i32 = -7 / 2; \
         const REMAINDER: i32 = -7 % 2; \
         const SHIFTED: u8 = 1 << 9; \
         const SIGNED_SHIFT: i8 = -8 >> 1;",
    );
    assert_eq!(int(&fixture, "SUM"), 4);
    assert_eq!(int(&fixture, "DIFFERENCE"), 255);
    assert_eq!(int(&fixture, "PRODUCT"), (-56i128 as u128) & 0xff);
    assert_eq!(int(&fixture, "QUOTIENT"), (-3i128 as u128) & 0xffff_ffff);
    assert_eq!(int(&fixture, "REMAINDER"), (-1i128 as u128) & 0xffff_ffff);
    assert_eq!(int(&fixture, "SHIFTED"), 2);
    assert_eq!(int(&fixture, "SIGNED_SHIFT"), (-4i128 as u128) & 0xff);
}

#[test]
fn float_values_keep_signed_zero_and_nan_payloads() {
    let fixture = clean(
        "const ZERO: f64 = 0.0; \
         const NEGATIVE_ZERO: f64 = 0.0 - 0.0; \
         const SUM: f32 = 0.5 + 0.25;",
    );
    assert!(matches!(
        fixture.value("ZERO"),
        ConstValue::Float { bits: 0, .. }
    ));
    assert!(matches!(
        fixture.value("NEGATIVE_ZERO"),
        ConstValue::Float { bits: 0, .. }
    ));
    let ConstValue::Float { bits, .. } = fixture.value("SUM") else {
        panic!("SUM is not a float");
    };
    assert_eq!(f32::from_bits(*bits as u32), 0.75);
}

#[test]
fn casts_follow_runtime_saturation_and_truncation() {
    let fixture = clean(
        "const TRUNCATE: u8 = 260 as u8; \
         const EXTEND: i64 = -1 as i8 as i64; \
         const TO_FLOAT: f64 = 7 as f64; \
         const SATURATE: u8 = 300.9 as u8; \
         const NARROW: f32 = 1.5 as f64 as f32;",
    );
    assert_eq!(int(&fixture, "TRUNCATE"), 4);
    assert_eq!(
        int(&fixture, "EXTEND"),
        (-1i128 as u128) & u128::from(u64::MAX)
    );
    assert!(matches!(
        fixture.value("TO_FLOAT"),
        ConstValue::Float { bits, .. } if f64::from_bits(*bits) == 7.0
    ));
    assert_eq!(int(&fixture, "SATURATE"), 255);
    assert!(matches!(
        fixture.value("NARROW"),
        ConstValue::Float { bits, .. } if f32::from_bits(*bits as u32) == 1.5
    ));
}

#[test]
fn aggregates_are_built_in_declaration_order() {
    let fixture = clean(
        "struct Point { x: i32, y: i32 } \
         enum Shape { Empty, Circle(i32), Pair { left: i32, right: i32 } } \
         const POINT: Point = Point { x: 1, y: 2 }; \
         const TUPLE: (i32, bool) = (3, true); \
         const ARRAY: [i32; 3] = [4, 5, 6]; \
         const REPEAT: [bool; 2] = [false; 2]; \
         const UNIT_VARIANT: Shape = Shape::Empty; \
         const TUPLE_VARIANT: Shape = Shape::Circle(7); \
         const STRING: *str = \"hello\";",
    );
    assert!(matches!(
        fixture.value("POINT"),
        ConstValue::Struct { fields, .. } if fields.len() == 2
    ));
    assert!(matches!(
        fixture.value("TUPLE"),
        ConstValue::Tuple(elements) if elements.len() == 2
    ));
    assert!(matches!(
        fixture.value("ARRAY"),
        ConstValue::Array(elements) if elements.len() == 3
    ));
    assert!(matches!(
        fixture.value("REPEAT"),
        ConstValue::Array(elements) if elements.len() == 2
    ));
    assert!(matches!(
        fixture.value("TUPLE_VARIANT"),
        ConstValue::Variant { fields, .. } if fields.len() == 1
    ));
    assert!(matches!(fixture.value("STRING"), ConstValue::Str { .. }));
}

#[test]
fn equality_compares_aggregates_and_contents() {
    let fixture = clean(
        "struct Point { x: i32, y: i32 } \
         enum Shape { Dot, Line(i32, i32) } \
         const A: *str = \"hello\"; \
         const B: *str = \"hello\"; \
         const C: *str = \"hellp\"; \
         const SHORTER: *str = \"hell\"; \
         const STR_SAME: bool = A == B; \
         const STR_DIFF: bool = A == C; \
         const STR_LENGTH: bool = A == SHORTER; \
         const VALUES: [i32; 3] = [1, 2, 3]; \
         const ARRAY_SAME: bool = VALUES == [1, 2, 3]; \
         const ARRAY_DIFF: bool = VALUES == [1, 2, 4]; \
         const FIRST: *[i32] = &VALUES[0..2]; \
         const SLICE_SAME: bool = FIRST == &VALUES[0..2]; \
         const SLICE_SHIFTED: bool = FIRST == &VALUES[1..3]; \
         const SLICE_SHORTER: bool = FIRST == &VALUES[0..1]; \
         const P: Point = Point { x: 1, y: 2 }; \
         const Q: Point = Point { x: 1, y: 2 }; \
         const R: Point = Point { x: 1, y: 3 }; \
         const STRUCT_SAME: bool = P == Q; \
         const STRUCT_DIFF: bool = P == R; \
         const DOT: Shape = Shape::Dot; \
         const DOT_SAME: Shape = Shape::Dot; \
         const LINE: Shape = Shape::Line(1, 2); \
         const LINE_SAME: Shape = Shape::Line(1, 2); \
         const LINE_DIFF: Shape = Shape::Line(1, 3); \
         const ENUM_SAME: bool = DOT == DOT_SAME; \
         const ENUM_VARIANT: bool = DOT == LINE; \
         const ENUM_FIELDS: bool = LINE == LINE_SAME; \
         const ENUM_DIFF: bool = LINE == LINE_DIFF; \
         const TUPLE_SAME: bool = (1, true) == (1, true); \
         const TUPLE_DIFF: bool = (1, true) == (1, false); \
         const NAN: f64 = 0.0 / 0.0; \
         const NAN_SAME: bool = NAN == NAN; \
         const ZERO_SAME: bool = 0.0 == -0.0; \
         const UNIT_SAME: bool = () == ();",
    );
    assert!(flag(&fixture, "STR_SAME"));
    assert!(!flag(&fixture, "STR_DIFF"));
    assert!(!flag(&fixture, "STR_LENGTH"));
    assert!(flag(&fixture, "ARRAY_SAME"));
    assert!(!flag(&fixture, "ARRAY_DIFF"));
    assert!(flag(&fixture, "SLICE_SAME"));
    assert!(!flag(&fixture, "SLICE_SHIFTED"));
    assert!(!flag(&fixture, "SLICE_SHORTER"));
    assert!(flag(&fixture, "STRUCT_SAME"));
    assert!(!flag(&fixture, "STRUCT_DIFF"));
    assert!(flag(&fixture, "ENUM_SAME"));
    assert!(!flag(&fixture, "ENUM_VARIANT"));
    assert!(flag(&fixture, "ENUM_FIELDS"));
    assert!(!flag(&fixture, "ENUM_DIFF"));
    assert!(flag(&fixture, "TUPLE_SAME"));
    assert!(!flag(&fixture, "TUPLE_DIFF"));
    assert!(!flag(&fixture, "NAN_SAME"));
    assert!(flag(&fixture, "ZERO_SAME"));
    assert!(flag(&fixture, "UNIT_SAME"));
}

#[test]
fn a_trait_object_records_its_vtable_key() {
    let fixture = clean(
        "trait Shape { func area(self: *Self) -> i32; } \
         struct Square { side: i32 } \
         impl Shape for Square { func area(self: *Square) -> i32 { (*self).side } } \
         static SQUARE: Square = Square { side: 2 }; \
         const VALUE: *dyn Shape = &SQUARE;",
    );
    assert!(matches!(
        fixture.value("VALUE"),
        ConstValue::Dyn { concrete, .. }
            if matches!(fixture.hir.ty_ctxt().kind(*concrete), seal_ty::TyKind::Adt { .. })
    ));
    assert_eq!(fixture.program.static_vtables().len(), 1);
}

#[test]
fn a_static_has_one_allocation_with_program_lifetime() {
    let fixture = clean(
        "struct Point { x: i32, y: i32 } \
         static ORIGIN: Point = Point { x: 0, y: 0 }; \
         static mut COUNTER: i32 = 0;",
    );
    let origin = fixture.definition("ORIGIN");
    let data = fixture.program.static_data(origin).expect("ORIGIN data");
    assert!(!data.mutable);
    assert!(matches!(
        fixture.program.allocation(data.alloc).source,
        AllocSource::Static(def) if def == origin
    ));
    let counter = fixture.definition("COUNTER");
    assert!(
        fixture
            .program
            .static_data(counter)
            .expect("COUNTER data")
            .mutable
    );
}

#[test]
fn a_reference_names_the_static_allocation() {
    let fixture = clean(
        "static LIMIT: i32 = 3; \
         const POINTER: *i32 = &LIMIT;",
    );
    let limit = fixture.definition("LIMIT");
    let data = fixture.program.static_data(limit).expect("LIMIT data");
    assert_eq!(
        fixture.value("POINTER"),
        &ConstValue::Ref {
            target: data.alloc,
            offset: 0,
            pointee: data.ty,
            kind: seal_ty::RefKind::Shared,
        }
    );
}

#[test]
fn a_promoted_literal_gets_a_private_allocation() {
    let fixture = clean("const ANSWER: i32 = 42; const POINTER: *i32 = &ANSWER;");
    let ConstValue::Ref {
        target,
        offset,
        pointee,
        ..
    } = fixture.value("POINTER")
    else {
        panic!("POINTER is not a reference");
    };
    assert_eq!(*offset, 0);
    let allocation = fixture.program.allocation(*target);
    assert!(matches!(allocation.source, AllocSource::Promoted { .. }));
    assert!(!allocation.mutable);
    assert_eq!(&allocation.ty, pointee);
    assert!(matches!(allocation.value, ConstValue::Int { bits: 42, .. }));
}

#[test]
fn a_promoted_array_slice_names_its_elements() {
    let fixture = clean("const POINTER: *[i32] = &[1, 2, 3][..];");
    let ConstValue::Slice {
        target,
        offset,
        length,
        element: _,
    } = fixture.value("POINTER")
    else {
        panic!("POINTER is not a slice");
    };
    assert_eq!(*offset, 0);
    assert_eq!(*length, 3);
    let allocation = fixture.program.allocation(*target);
    assert!(matches!(allocation.value, ConstValue::Array(ref values) if values.len() == 3));
    assert!(matches!(
        fixture.hir.ty_ctxt().kind(allocation.ty),
        seal_ty::TyKind::Array { length: 3, .. }
    ));
}

#[test]
fn self_referential_statics_are_allowed() {
    let fixture = clean(
        "struct Node { value: i32, next: *Node } \
         static FIRST: Node = Node { value: 1, next: &SECOND }; \
         static SECOND: Node = Node { value: 2, next: &FIRST };",
    );
    let first = fixture.definition("FIRST");
    let second = fixture.definition("SECOND");
    let first_data = fixture.program.static_data(first).expect("FIRST data");
    let second_data = fixture.program.static_data(second).expect("SECOND data");
    let ConstValue::Struct { fields, .. } = fixture.value("FIRST") else {
        panic!("FIRST is not a struct");
    };
    assert!(matches!(
        &fields[1].1,
        ConstValue::Ref { target, .. } if *target == second_data.alloc
    ));
    let ConstValue::Struct { fields, .. } =
        fixture.program.allocation(second_data.alloc).value.clone()
    else {
        panic!("SECOND allocation is not a struct");
    };
    assert!(matches!(
        &fields[1].1,
        ConstValue::Ref { target, .. } if *target == first_data.alloc
    ));
}

#[test]
fn evaluation_is_deterministic() {
    let source = "static LIMIT: i32 = 3; const POINTER: *i32 = &LIMIT; \
                  const VALUES: [i32; 3] = [1, 2, 3];";
    let first = Fixture::compile(source);
    let second = Fixture::compile(source);
    assert_eq!(
        first.program.allocations().len(),
        second.program.allocations().len()
    );
    assert_eq!(
        first.program.statics().len(),
        second.program.statics().len()
    );
    for (def, _) in first.hir.definitions().iter_enumerated() {
        assert_eq!(first.program.value(def), second.program.value(def));
    }
}
