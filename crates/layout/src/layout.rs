//! Size, alignment, and field offsets.

use std::fmt;

use seal_ty::{AdtId, TyCtxt, TyId, TyKind};

/// The size and alignment of a type, in bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    size: u64,
    align: u64,
}

impl Layout {
    /// Creates a layout.
    pub const fn new(size: u64, align: u64) -> Self {
        Self { size, align }
    }

    /// Returns the size in bytes, padding included.
    #[inline]
    pub const fn size(self) -> u64 {
        self.size
    }

    /// Returns the alignment in bytes. Always a power of two.
    #[inline]
    pub const fn align(self) -> u64 {
        self.align
    }
}

/// Where one field lives inside its aggregate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FieldLayout {
    /// Byte offset from the start of the aggregate.
    pub offset: u64,
    /// The field's own layout.
    pub layout: Layout,
}

/// A layout that cannot be computed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LayoutError {
    /// An aggregate contains itself by value, so its size is infinite.
    Recursive(AdtId),
    /// The type has no compile-time size.
    Unsized(TyId),
    /// The type is still an inference variable.
    Unknown(TyId),
    /// The field index does not belong to the aggregate.
    UnknownField(AdtId, usize),
    /// The type's size does not fit in `u64`.
    TooLarge(TyId),
}

impl fmt::Display for LayoutError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Recursive(adt) => write!(f, "aggregate {adt:?} is recursive by value"),
            Self::Unsized(ty) => write!(f, "type {ty:?} has no static size"),
            Self::Unknown(ty) => write!(f, "type {ty:?} was not inferred"),
            Self::UnknownField(adt, index) => {
                write!(f, "aggregate {adt:?} has no field {index}")
            }
            Self::TooLarge(ty) => write!(f, "type {ty:?} is too large to lay out"),
        }
    }
}

impl std::error::Error for LayoutError {}

/// Computes layouts for one type context and target.
#[derive(Clone, Copy, Debug)]
pub struct LayoutCtxt<'a> {
    tcx: &'a TyCtxt,
    pointer_width: u8,
}

impl<'a> LayoutCtxt<'a> {
    /// Creates a context for a target whose pointers are `pointer_width` bits
    /// wide.
    pub fn new(tcx: &'a TyCtxt, pointer_width: u8) -> Self {
        Self { tcx, pointer_width }
    }

    /// Returns the layout of `ty`.
    pub fn layout_of(&self, ty: TyId) -> Result<Layout, LayoutError> {
        self.layout_inner(ty, &mut Vec::new())
    }

    /// Returns the layouts of every field of `adt`, in declaration order.
    pub fn field_layouts(&self, adt: AdtId) -> Result<Vec<FieldLayout>, LayoutError> {
        self.aggregate_fields(adt, &mut vec![adt])
    }

    /// Returns the layout of one field of `adt`.
    pub fn field_layout(&self, adt: AdtId, index: usize) -> Result<FieldLayout, LayoutError> {
        self.field_layouts(adt)?
            .get(index)
            .copied()
            .ok_or(LayoutError::UnknownField(adt, index))
    }

    fn layout_inner(&self, ty: TyId, stack: &mut Vec<AdtId>) -> Result<Layout, LayoutError> {
        let pointer = u64::from(self.pointer_width) / 8;
        let layout = match self.tcx.kind(ty) {
            TyKind::Bool => Layout::new(1, 1),
            TyKind::Char => Layout::new(4, 4),
            TyKind::Int(int) => {
                scalar(u64::from(int.bits().unwrap_or(self.pointer_width_u16())) / 8)
            }
            TyKind::Uint(uint) => {
                scalar(u64::from(uint.bits().unwrap_or(self.pointer_width_u16())) / 8)
            }
            TyKind::Float(float) => scalar(u64::from(float.bits()) / 8),
            TyKind::Ref { pointee, .. } => match self.tcx.kind(pointee) {
                // `*str` and `*[T]` are fat pointers: data plus a
                // pointer-sized length.
                TyKind::Str | TyKind::Slice { .. } => Layout::new(pointer * 2, pointer),
                _ => Layout::new(pointer, pointer),
            },
            TyKind::Unit | TyKind::Never | TyKind::Error => Layout::new(0, 1),
            TyKind::Array { element, length } => {
                let element_layout = self.layout_inner(element, stack)?;
                let size = element_layout
                    .size
                    .checked_mul(length)
                    .ok_or(LayoutError::TooLarge(ty))?;
                Layout::new(size, element_layout.align)
            }
            TyKind::Str | TyKind::Slice { .. } => return Err(LayoutError::Unsized(ty)),
            TyKind::Infer(_) => return Err(LayoutError::Unknown(ty)),
            TyKind::Adt(adt) => {
                if stack.contains(&adt) {
                    return Err(LayoutError::Recursive(adt));
                }
                stack.push(adt);
                let mut offset = 0;
                let mut align = 1;
                for field in self.tcx.adt(adt).fields {
                    let field_layout = self.layout_inner(field.ty, stack)?;
                    offset = round_up(offset, field_layout.align);
                    offset += field_layout.size;
                    align = align.max(field_layout.align);
                }
                stack.pop();
                Layout::new(round_up(offset, align), align)
            }
        };
        Ok(layout)
    }

    fn aggregate_fields(
        &self,
        adt: AdtId,
        stack: &mut Vec<AdtId>,
    ) -> Result<Vec<FieldLayout>, LayoutError> {
        let mut offset = 0;
        let mut fields = Vec::new();
        for field in self.tcx.adt(adt).fields {
            let layout = self.layout_inner(field.ty, stack)?;
            offset = round_up(offset, layout.align);
            fields.push(FieldLayout { offset, layout });
            offset += layout.size;
        }
        Ok(fields)
    }

    fn pointer_width_u16(&self) -> u16 {
        u16::from(self.pointer_width)
    }
}

fn scalar(size: u64) -> Layout {
    Layout::new(size, size.max(1))
}

fn round_up(value: u64, align: u64) -> u64 {
    debug_assert!(align.is_power_of_two());
    (value + align - 1) & !(align - 1)
}

#[cfg(test)]
mod tests {
    use seal_def::DefId;
    use seal_symbol::Interner;

    use super::{Layout, LayoutCtxt, LayoutError};
    use seal_ty::{AdtId, FieldDef, IntTy, TyCtxt, TyKind};

    fn def(index: u32) -> DefId {
        DefId::from_u32(index).expect("test definitions are small")
    }

    fn point(ctxt: &TyCtxt, interner: &Interner) -> AdtId {
        let adt = ctxt.push_adt(def(0), interner.intern("Point"));
        let int = ctxt.int_ty(IntTy::I32);
        ctxt.fill_adt(
            adt,
            vec![
                FieldDef {
                    def: def(1),
                    name: interner.intern("x"),
                    ty: int,
                },
                FieldDef {
                    def: def(2),
                    name: interner.intern("y"),
                    ty: int,
                },
            ],
        );
        adt
    }

    #[test]
    fn struct_fields_follow_declaration_order() {
        let ctxt = TyCtxt::new();
        let interner = Interner::new();
        let adt = point(&ctxt, &interner);
        let layouts = LayoutCtxt::new(&ctxt, 64)
            .field_layouts(adt)
            .expect("sized");
        assert_eq!(layouts[0].offset, 0);
        assert_eq!(layouts[1].offset, 4);
        assert_eq!(layouts[0].layout, Layout::new(4, 4));
        assert_eq!(
            LayoutCtxt::new(&ctxt, 64).layout_of(ctxt.adt_ty(adt)),
            Ok(Layout::new(8, 4))
        );
    }

    #[test]
    fn padding_and_alignment_are_deterministic() {
        let ctxt = TyCtxt::new();
        let interner = Interner::new();
        let adt = ctxt.push_adt(def(0), interner.intern("Mixed"));
        ctxt.fill_adt(
            adt,
            vec![
                FieldDef {
                    def: def(1),
                    name: interner.intern("flag"),
                    ty: ctxt.bool_ty(),
                },
                FieldDef {
                    def: def(2),
                    name: interner.intern("value"),
                    ty: ctxt.int_ty(IntTy::I32),
                },
                FieldDef {
                    def: def(3),
                    name: interner.intern("small"),
                    ty: ctxt.int_ty(IntTy::I8),
                },
            ],
        );
        let layouts = LayoutCtxt::new(&ctxt, 64)
            .field_layouts(adt)
            .expect("sized");
        assert_eq!(layouts[0].offset, 0);
        assert_eq!(layouts[1].offset, 4);
        assert_eq!(layouts[2].offset, 8);
        assert_eq!(
            LayoutCtxt::new(&ctxt, 64).layout_of(ctxt.adt_ty(adt)),
            Ok(Layout::new(12, 4))
        );
    }

    #[test]
    fn empty_structs_are_zero_sized() {
        let ctxt = TyCtxt::new();
        let interner = Interner::new();
        let adt = ctxt.push_adt(def(0), interner.intern("Empty"));
        ctxt.fill_adt(adt, Vec::new());
        assert_eq!(
            LayoutCtxt::new(&ctxt, 64).layout_of(ctxt.adt_ty(adt)),
            Ok(Layout::new(0, 1))
        );
        assert!(
            LayoutCtxt::new(&ctxt, 64)
                .field_layouts(adt)
                .expect("empty")
                .is_empty()
        );
    }

    #[test]
    fn a_string_reference_is_a_fat_pointer() {
        let ctxt = TyCtxt::new();
        let reference = ctxt.ref_ty(ctxt.str_ty());
        assert_eq!(
            LayoutCtxt::new(&ctxt, 64).layout_of(reference),
            Ok(Layout::new(16, 8))
        );
        assert_eq!(
            LayoutCtxt::new(&ctxt, 32).layout_of(reference),
            Ok(Layout::new(8, 4))
        );
    }

    #[test]
    fn a_slice_reference_is_a_fat_pointer() {
        let ctxt = TyCtxt::new();
        let int = ctxt.int_ty(IntTy::I32);
        let reference = ctxt.ref_ty(ctxt.slice_ty(int));
        assert_eq!(
            LayoutCtxt::new(&ctxt, 64).layout_of(reference),
            Ok(Layout::new(16, 8))
        );
        assert_eq!(
            LayoutCtxt::new(&ctxt, 32).layout_of(reference),
            Ok(Layout::new(8, 4))
        );
        assert_eq!(
            LayoutCtxt::new(&ctxt, 64).layout_of(ctxt.slice_ty(int)),
            Err(LayoutError::Unsized(ctxt.slice_ty(int)))
        );
    }

    #[test]
    fn references_follow_the_pointer_width() {
        let ctxt = TyCtxt::new();
        let int = ctxt.int_ty(IntTy::I32);
        let reference = ctxt.ref_ty(int);
        assert_eq!(
            LayoutCtxt::new(&ctxt, 64).layout_of(reference),
            Ok(Layout::new(8, 8))
        );
        assert_eq!(
            LayoutCtxt::new(&ctxt, 32).layout_of(reference),
            Ok(Layout::new(4, 4))
        );
    }

    #[test]
    fn scalars_use_natural_alignment() {
        let ctxt = TyCtxt::new();
        let layout = LayoutCtxt::new(&ctxt, 64);
        assert_eq!(
            layout.layout_of(ctxt.int_ty(IntTy::I128)),
            Ok(Layout::new(16, 16))
        );
        assert_eq!(layout.layout_of(ctxt.bool_ty()), Ok(Layout::new(1, 1)));
        assert_eq!(layout.layout_of(ctxt.char_ty()), Ok(Layout::new(4, 4)));
        assert_eq!(layout.layout_of(ctxt.unit_ty()), Ok(Layout::new(0, 1)));
        assert_eq!(
            layout.layout_of(ctxt.str_ty()),
            Err(LayoutError::Unsized(ctxt.str_ty()))
        );
    }

    #[test]
    fn recursive_aggregates_are_rejected() {
        let ctxt = TyCtxt::new();
        let interner = Interner::new();
        let adt = ctxt.push_adt(def(0), interner.intern("Bad"));
        let own = ctxt.adt_ty(adt);
        ctxt.fill_adt(
            adt,
            vec![FieldDef {
                def: def(1),
                name: interner.intern("next"),
                ty: own,
            }],
        );
        assert_eq!(
            LayoutCtxt::new(&ctxt, 64).layout_of(own),
            Err(LayoutError::Recursive(adt))
        );
    }

    #[test]
    fn untouched_context_has_no_aggregates() {
        let ctxt = TyCtxt::new();
        assert_eq!(ctxt.adt_len(), 0);
        assert_eq!(ctxt.kind(ctxt.int_ty(IntTy::I32)), TyKind::Int(IntTy::I32));
    }

    #[test]
    fn arrays_repeat_the_element_size_and_alignment() {
        let ctxt = TyCtxt::new();
        let int = ctxt.int_ty(IntTy::I32);
        let array = ctxt.array_ty(int, 3);
        assert_eq!(
            LayoutCtxt::new(&ctxt, 64).layout_of(array),
            Ok(Layout::new(12, 4))
        );
        let empty = ctxt.array_ty(int, 0);
        assert_eq!(
            LayoutCtxt::new(&ctxt, 64).layout_of(empty),
            Ok(Layout::new(0, 4))
        );
        let nested = ctxt.array_ty(array, 2);
        assert_eq!(
            LayoutCtxt::new(&ctxt, 64).layout_of(nested),
            Ok(Layout::new(24, 4))
        );
    }

    #[test]
    fn an_array_that_overflows_the_size_is_rejected() {
        let ctxt = TyCtxt::new();
        let u16_ty = ctxt.uint_ty(seal_ty::UintTy::U16);
        let array = ctxt.array_ty(u16_ty, u64::MAX);
        assert_eq!(
            LayoutCtxt::new(&ctxt, 64).layout_of(array),
            Err(LayoutError::TooLarge(array))
        );
    }
}
