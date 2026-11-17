//! Generic instance mapping tests.
//!
//! These tests inspect the LLVM types and declared functions directly, so they
//! cover the per-instance layout and symbols without running a program.

use inkwell::context::Context;
use seal_ty::{IntTy, UintTy};

use super::support::{Compiled, adt, front_end, module_ir};
use crate::types::TypeMapper;

/// Runs `check` with a mapper for the host pointer width.
fn with_mapper(compiled: &Compiled, check: impl FnOnce(&TypeMapper<'_>)) {
    with_width(compiled, compiled.session.target().pointer_width(), check);
}

/// Runs `check` with a mapper for `pointer_width`.
fn with_width(compiled: &Compiled, pointer_width: u8, check: impl FnOnce(&TypeMapper<'_>)) {
    let context = Context::create();
    let mapper = TypeMapper::new(
        &context,
        compiled.hir.ty_ctxt(),
        compiled.session.interner(),
        pointer_width,
    )
    .expect("types map to LLVM");
    check(&mapper);
}

#[test]
fn generic_aggregates_get_one_llvm_struct_per_instantiation() {
    let compiled = front_end(
        "struct Pair<T, U> {\n  first: T,\n  second: U,\n}\n\n\
         func main() -> i32 { 0 }\n",
    );
    let pair = adt(&compiled, "Pair");
    let tcx = compiled.hir.ty_ctxt();
    let int_bool = tcx.adt_ty_with(pair, tcx.substs(&[tcx.int_ty(IntTy::I32), tcx.bool_ty()]));
    let u8_i32 = tcx.adt_ty_with(
        pair,
        tcx.substs(&[tcx.uint_ty(UintTy::U8), tcx.int_ty(IntTy::I32)]),
    );

    with_mapper(&compiled, |mapper| {
        let first = mapper
            .basic(int_bool)
            .expect("a concrete aggregate is sized")
            .into_struct_type();
        let second = mapper
            .basic(u8_i32)
            .expect("a concrete aggregate is sized")
            .into_struct_type();
        assert_ne!(
            first.print_to_string(),
            second.print_to_string(),
            "instantiations must not share one LLVM struct"
        );
        assert_eq!(first.get_field_types().len(), 2);

        let layout = mapper.layout(int_bool).expect("layout exists");
        assert_eq!((layout.size(), layout.align()), (8, 4));
        let fields = mapper
            .field_layouts(pair, tcx.substs(&[tcx.int_ty(IntTy::I32), tcx.bool_ty()]))
            .expect("fields have layouts");
        assert_eq!(fields[0].offset, 0);
        assert_eq!(fields[1].offset, 4);

        let layout = mapper.layout(u8_i32).expect("layout exists");
        assert_eq!((layout.size(), layout.align()), (8, 4));
        let fields = mapper
            .field_layouts(
                pair,
                tcx.substs(&[tcx.uint_ty(UintTy::U8), tcx.int_ty(IntTy::I32)]),
            )
            .expect("fields have layouts");
        assert_eq!(fields[0].offset, 0);
        assert_eq!(fields[1].offset, 4);
    });
}

#[test]
fn nested_generic_aggregates_nest_their_layouts() {
    let compiled = front_end(
        "struct Pair<T, U> {\n  first: T,\n  second: U,\n}\n\n\
         func main() -> i32 { 0 }\n",
    );
    let pair = adt(&compiled, "Pair");
    let tcx = compiled.hir.ty_ctxt();
    let inner = tcx.adt_ty_with(pair, tcx.substs(&[tcx.int_ty(IntTy::I32), tcx.bool_ty()]));
    let outer = tcx.adt_ty_with(pair, tcx.substs(&[inner, tcx.uint_ty(UintTy::U8)]));

    with_mapper(&compiled, |mapper| {
        let layout = mapper.layout(outer).expect("layout exists");
        assert_eq!((layout.size(), layout.align()), (12, 4));
        let outer_type = mapper
            .basic(outer)
            .expect("a concrete aggregate is sized")
            .into_struct_type();
        let outer_fields = outer_type.get_field_types();
        assert_eq!(outer_fields.len(), 2);
        assert_eq!(
            outer_fields[0].into_struct_type().get_field_types().len(),
            2,
            "the nested instantiation keeps its own struct type"
        );
    });
}

#[test]
fn generic_enums_get_one_tag_and_payload_layout_per_instantiation() {
    let compiled = front_end(
        "struct Pair<T, U> {\n  first: T,\n  second: U,\n}\n\n\
         enum Option<T> {\n  Some(T),\n  None,\n}\n\n\
         func main() -> i32 { 0 }\n",
    );
    let pair = adt(&compiled, "Pair");
    let option = adt(&compiled, "Option");
    let tcx = compiled.hir.ty_ctxt();
    let int = tcx.int_ty(IntTy::I32);
    let pair_int_bool = tcx.adt_ty_with(pair, tcx.substs(&[int, tcx.bool_ty()]));
    let option_int = tcx.adt_ty_with(option, tcx.substs(&[int]));
    let option_pair = tcx.adt_ty_with(option, tcx.substs(&[pair_int_bool]));

    with_mapper(&compiled, |mapper| {
        let int_args = tcx.substs(&[int]);
        let int_layout = mapper
            .enum_layout(option, int_args)
            .expect("enum layout exists");
        assert_eq!(int_layout.tag.size(), 1);
        assert_eq!(int_layout.payload_size, 4);
        assert_eq!(int_layout.payload_offset, 4);

        let pair_args = tcx.substs(&[pair_int_bool]);
        let pair_layout = mapper
            .enum_layout(option, pair_args)
            .expect("enum layout exists");
        assert_eq!(pair_layout.tag.size(), 1);
        assert_eq!(pair_layout.payload_size, 8);
        assert_eq!(pair_layout.payload_offset, 4);

        let int_type = mapper
            .basic(option_int)
            .expect("a concrete enum is sized")
            .into_struct_type();
        let pair_type = mapper
            .basic(option_pair)
            .expect("a concrete enum is sized")
            .into_struct_type();
        assert_ne!(int_type.print_to_string(), pair_type.print_to_string());
        assert_eq!(int_type.get_field_types().len(), 3);
    });
}

#[test]
fn generic_functions_get_one_symbol_per_instantiation() {
    let source = "func identity<T>(value: T) -> T {\n  value\n}\n\n\
                  func main() -> i32 {\n  let a = identity::<i32>(1);\n  \
                  let b = identity::<bool>(true);\n  if b { a } else { 0 }\n}\n";
    let compiled = front_end(source);
    let ir = module_ir(&compiled);
    let definitions: Vec<&str> = ir
        .lines()
        .filter(|line| line.starts_with("declare") && line.contains("seal.identity."))
        .collect();
    assert_eq!(definitions.len(), 2, "expected two instances in:\n{ir}");
    assert_ne!(definitions[0], definitions[1]);
    assert_eq!(
        ir,
        module_ir(&front_end(source)),
        "symbols must be stable across compilations"
    );
}

#[test]
fn fat_pointers_inside_generic_aggregates_follow_the_pointer_width() {
    let compiled = front_end(
        "struct Holder<T> {\n  label: *str,\n  data: T,\n}\n\n\
         func main() -> i32 { 0 }\n",
    );
    let holder = adt(&compiled, "Holder");
    let tcx = compiled.hir.ty_ctxt();
    let int = tcx.int_ty(IntTy::I32);
    let slice = tcx.ref_ty(tcx.slice_ty(int));

    for pointer_width in [32, 64] {
        with_width(&compiled, pointer_width, |mapper| {
            let holder_int = tcx.adt_ty_with(holder, tcx.substs(&[int]));
            let holder_slice = tcx.adt_ty_with(holder, tcx.substs(&[slice]));
            for ty in [holder_int, holder_slice] {
                let fields = mapper
                    .basic(ty)
                    .expect("a concrete aggregate is sized")
                    .into_struct_type()
                    .get_field_types();
                assert_eq!(fields.len(), 2);
                let label = fields[0].into_struct_type().get_field_types();
                assert_eq!(label.len(), 2, "`*str` is a fat pointer");
                assert_eq!(
                    label[1].into_int_type().get_bit_width(),
                    u32::from(pointer_width),
                    "the length follows the target pointer width"
                );
            }
            let slice_holder = tcx.adt_ty_with(holder, tcx.substs(&[slice]));
            let fields = mapper
                .basic(slice_holder)
                .expect("a concrete aggregate is sized")
                .into_struct_type()
                .get_field_types();
            assert_eq!(
                fields[1].into_struct_type().get_field_types().len(),
                2,
                "a slice reference stays a fat pointer"
            );
        });
    }
}

#[test]
fn generic_noalias_is_withheld_when_the_proof_cannot_close() {
    let compiled = front_end(
        "func store<T>(slot: *mut T, value: T) {\n  *slot = value;\n}\n\n\
         func main() -> i32 {\n  let mut value = 1;\n  store(&mut value, 2);\n  value\n}\n",
    );
    let ir = module_ir(&compiled);
    assert!(
        !ir.contains("noalias"),
        "an opaque parameter cannot prove aliasing: {ir}"
    );
}
