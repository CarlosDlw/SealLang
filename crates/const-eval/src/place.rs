//! Navigating compiler-owned values as if they were memory.
//!
//! References store a byte offset instead of a path, so every dereference locates
//! the sub-value again from the allocation root and the type layout. This keeps
//! the public value representation identical to what code generation consumes.

use seal_layout::LayoutCtxt;
use seal_ty::{TyCtxt, TyId, TyKind, VariantIdx};

use crate::value::{AllocId, ConstValue};

/// One step into a value tree.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathStep {
    /// A struct or tuple-struct field, by declaration index.
    Field(usize),
    /// A tuple element, by position.
    TupleField(usize),
    /// An enum variant field, by declaration index inside the variant.
    VariantField(usize),
    /// An array element, by position.
    Index(usize),
}

/// A place resolved to an allocation and a byte offset.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedPlace {
    /// The allocation that owns the storage.
    pub alloc: AllocId,
    /// Byte offset from the allocation's start.
    pub offset: u64,
    /// The type stored at the place.
    pub ty: TyId,
    /// The steps from the allocation's root value.
    pub path: Vec<PathStep>,
    /// Set when the place views a slice and `ty` is the unsized slice type.
    pub slice: Option<TyId>,
    /// The slice length, present exactly when `slice` is set.
    pub slice_len: Option<u64>,
}

impl ResolvedPlace {
    /// Returns the resolved place with an empty path at `alloc`.
    pub fn at(alloc: AllocId, ty: TyId) -> Self {
        Self {
            alloc,
            offset: 0,
            ty,
            path: Vec::new(),
            slice: None,
            slice_len: None,
        }
    }
}

/// Returns the value at `path`, if the tree has that shape.
pub(crate) fn read_at<'a>(root: &'a ConstValue, path: &[PathStep]) -> Option<&'a ConstValue> {
    let mut value = root;
    for step in path {
        value = match (step, value) {
            (PathStep::Field(index), ConstValue::Struct { fields, .. })
            | (PathStep::VariantField(index), ConstValue::Variant { fields, .. }) => {
                &fields.get(*index)?.1
            }
            (PathStep::TupleField(index), ConstValue::Tuple(elements)) => elements.get(*index)?,
            (PathStep::Index(index), ConstValue::Array(elements)) => elements.get(*index)?,
            _ => return None,
        };
    }
    Some(value)
}

/// Returns the mutable value at `path`.
pub(crate) fn slot_at_mut<'a>(
    root: &'a mut ConstValue,
    path: &[PathStep],
) -> Option<&'a mut ConstValue> {
    let mut value = root;
    for step in path {
        value = match (step, value) {
            (PathStep::Field(index), ConstValue::Struct { fields, .. })
            | (PathStep::VariantField(index), ConstValue::Variant { fields, .. }) => {
                &mut fields.get_mut(*index)?.1
            }
            (PathStep::TupleField(index), ConstValue::Tuple(elements)) => {
                elements.get_mut(*index)?
            }
            (PathStep::Index(index), ConstValue::Array(elements)) => elements.get_mut(*index)?,
            _ => return None,
        };
    }
    Some(value)
}

/// Writes `new` at `path`, returning `false` if the tree has another shape.
pub(crate) fn write_at(root: &mut ConstValue, path: &[PathStep], new: ConstValue) -> bool {
    match slot_at_mut(root, path) {
        Some(slot) => {
            *slot = new;
            true
        }
        None => false,
    }
}

/// Returns the path of the value that starts at byte `offset`.
pub(crate) fn locate(
    tcx: &TyCtxt,
    layout: &LayoutCtxt<'_>,
    root: &ConstValue,
    ty: TyId,
    offset: u64,
) -> Option<Vec<PathStep>> {
    if offset == 0 {
        return Some(Vec::new());
    }
    locate_member(tcx, layout, root, ty, offset)
}

/// Returns the path of the member of `root` that starts at byte `offset`.
///
/// Unlike [`locate`], offset zero addresses the first member of the container
/// rather than the container itself. A view that starts at the beginning of an
/// aggregate needs the member: the first byte of an array holds its first
/// element, not the array.
pub(crate) fn locate_member(
    tcx: &TyCtxt,
    layout: &LayoutCtxt<'_>,
    root: &ConstValue,
    ty: TyId,
    offset: u64,
) -> Option<Vec<PathStep>> {
    match (tcx.kind(ty), root) {
        (TyKind::Adt { adt, args }, ConstValue::Struct { fields, .. }) => {
            let layouts = layout.field_layouts_with(adt, args).ok()?;
            let adt = tcx.adt(adt);
            for (index, field) in layouts.iter().enumerate() {
                let field_value = &fields.get(index)?.1;
                if offset == field.offset {
                    return Some(vec![PathStep::Field(index)]);
                }
                let size = field.layout.size();
                if offset > field.offset && offset < field.offset.saturating_add(size) {
                    let mut path = vec![PathStep::Field(index)];
                    path.extend(locate(
                        tcx,
                        layout,
                        field_value,
                        adt.fields.get(index)?.ty,
                        offset - field.offset,
                    )?);
                    return Some(path);
                }
            }
            None
        }
        (
            TyKind::Adt { adt, args },
            ConstValue::Variant {
                variant, fields, ..
            },
        ) => {
            let enum_layout = layout.enum_layout_with(adt, args).ok()?;
            let variant_layout = enum_layout.variants.get(variant.index() as usize)?;
            let declaration = tcx.adt(adt).variant(*variant)?.fields.to_vec();
            for (index, field) in variant_layout.fields.iter().enumerate() {
                let field_value = &fields.get(index)?.1;
                if offset == field.offset {
                    return Some(vec![PathStep::VariantField(index)]);
                }
                let size = field.layout.size();
                if offset > field.offset && offset < field.offset.saturating_add(size) {
                    let mut path = vec![PathStep::VariantField(index)];
                    path.extend(locate(
                        tcx,
                        layout,
                        field_value,
                        declaration.get(index)?.ty,
                        offset - field.offset,
                    )?);
                    return Some(path);
                }
            }
            None
        }
        (TyKind::Tuple(tuple), ConstValue::Tuple(elements)) => {
            let layouts = layout.tuple_field_layouts(ty).ok()?;
            let tys = tcx.tuple_elements(tuple);
            for (index, field) in layouts.iter().enumerate() {
                let element = elements.get(index)?;
                if offset == field.offset {
                    return Some(vec![PathStep::TupleField(index)]);
                }
                let size = field.layout.size();
                if offset > field.offset && offset < field.offset.saturating_add(size) {
                    let mut path = vec![PathStep::TupleField(index)];
                    path.extend(locate(
                        tcx,
                        layout,
                        element,
                        *tys.get(index)?,
                        offset - field.offset,
                    )?);
                    return Some(path);
                }
            }
            None
        }
        (TyKind::Array { element, length }, ConstValue::Array(elements)) => {
            let element_layout = layout.layout_of(element).ok()?;
            let size = element_layout.size();
            if size == 0 {
                return (offset == 0).then_some(vec![PathStep::Index(0)]);
            }
            let index = offset / size;
            if index >= length {
                return None;
            }
            let mut path = vec![PathStep::Index(index as usize)];
            path.extend(locate(
                tcx,
                layout,
                elements.get(index as usize)?,
                element,
                offset % size,
            )?);
            Some(path)
        }
        _ => None,
    }
}

/// Returns the declaration index of `field` inside an enum variant.
pub(crate) fn variant_field_index(
    tcx: &TyCtxt,
    adt: seal_ty::AdtId,
    variant: VariantIdx,
    field: seal_def::DefId,
) -> Option<usize> {
    tcx.adt(adt)
        .variant(variant)?
        .fields
        .iter()
        .position(|candidate| candidate.def == field)
}
