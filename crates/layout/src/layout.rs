//! Size, alignment, and field offsets.

use std::fmt;

use seal_ty::{AdtId, AdtKind, TyCtxt, TyId, TyKind};

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

/// Where the fields of one enum variant live inside the enum value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VariantLayout {
    /// Fields in declaration order, with offsets from the start of the enum.
    pub fields: Vec<FieldLayout>,
}

/// The layout of an enum value.
///
/// Every enum value starts with a discriminant (the tag) whose width is the
/// smallest unsigned integer that can address every variant. The payload starts
/// at the next multiple of the payload alignment, so each variant's fields keep
/// their natural alignment; padding inside the payload and at the end is
/// deterministic. Variant field offsets are absolute, measured from the start of
/// the enum value, which is what code generation needs to store them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EnumLayout {
    /// The layout of the whole enum value.
    pub layout: Layout,
    /// The discriminant's own layout; its size equals its alignment.
    pub tag: Layout,
    /// Byte offset of the payload from the start of the enum value.
    pub payload_offset: u64,
    /// Size of the largest variant payload, in bytes.
    pub payload_size: u64,
    /// Alignment of the largest variant payload field, in bytes.
    pub payload_align: u64,
    /// Per-variant field layouts, in discriminant order.
    pub variants: Vec<VariantLayout>,
}

impl EnumLayout {
    /// Returns the layout of the payload region.
    #[inline]
    pub fn payload(&self) -> Layout {
        Layout::new(self.payload_size, self.payload_align)
    }
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
    /// The aggregate is an enum, but a single struct field list was requested.
    NotAStruct(AdtId),
    /// The aggregate is a struct, but an enum layout was requested.
    NotAnEnum(AdtId),
    /// A tuple field list was requested for a non-tuple type.
    NotATuple(TyId),
    /// The enum declares more variants than a `u32` discriminant can address.
    TooManyVariants(AdtId),
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
            Self::NotAStruct(adt) => write!(f, "aggregate {adt:?} is not a struct"),
            Self::NotAnEnum(adt) => write!(f, "aggregate {adt:?} is not an enum"),
            Self::NotATuple(ty) => write!(f, "type {ty:?} is not a tuple"),
            Self::TooManyVariants(adt) => {
                write!(f, "enum {adt:?} has more variants than a `u32` can address")
            }
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

    /// Returns the layouts of every field of the struct `adt`, in declaration
    /// order.
    pub fn field_layouts(&self, adt: AdtId) -> Result<Vec<FieldLayout>, LayoutError> {
        if !self.tcx.adt(adt).is_struct() {
            return Err(LayoutError::NotAStruct(adt));
        }
        self.aggregate_fields(adt, &mut vec![adt])
    }

    /// Returns the tag and payload layout of the enum `adt`.
    pub fn enum_layout(&self, adt: AdtId) -> Result<EnumLayout, LayoutError> {
        self.enum_layout_inner(adt, &mut Vec::new())
    }

    /// Returns the layout of one field of `adt`.
    pub fn field_layout(&self, adt: AdtId, index: usize) -> Result<FieldLayout, LayoutError> {
        self.field_layouts(adt)?
            .get(index)
            .copied()
            .ok_or(LayoutError::UnknownField(adt, index))
    }

    /// Returns the layouts of every element of the tuple `ty`, in order.
    pub fn tuple_field_layouts(&self, ty: TyId) -> Result<Vec<FieldLayout>, LayoutError> {
        let TyKind::Tuple(tuple) = self.tcx.kind(ty) else {
            return Err(LayoutError::NotATuple(ty));
        };
        let elements = self.tcx.tuple_elements(tuple);
        let mut offset = 0;
        let mut fields = Vec::with_capacity(elements.len());
        for element in elements {
            let layout = self.layout_inner(element, &mut Vec::new())?;
            offset = round_up(offset, layout.align);
            fields.push(FieldLayout { offset, layout });
            offset = offset
                .checked_add(layout.size)
                .ok_or(LayoutError::TooLarge(ty))?;
        }
        Ok(fields)
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
            TyKind::Tuple(tuple) => {
                let mut offset = 0;
                let mut align = 1;
                for element in self.tcx.tuple_elements(tuple) {
                    let element_layout = self.layout_inner(element, stack)?;
                    offset = round_up(offset, element_layout.align);
                    offset = offset
                        .checked_add(element_layout.size)
                        .ok_or(LayoutError::TooLarge(ty))?;
                    align = align.max(element_layout.align);
                }
                Layout::new(round_up(offset, align), align)
            }
            TyKind::Str | TyKind::Slice { .. } => return Err(LayoutError::Unsized(ty)),
            TyKind::Infer(_) => return Err(LayoutError::Unknown(ty)),
            TyKind::Adt(adt) => {
                let definition = self.tcx.adt(adt);
                match definition.kind {
                    AdtKind::Struct => {
                        if stack.contains(&adt) {
                            return Err(LayoutError::Recursive(adt));
                        }
                        stack.push(adt);
                        let mut offset = 0;
                        let mut align = 1;
                        for field in &definition.fields {
                            let field_layout = self.layout_inner(field.ty, stack)?;
                            offset = round_up(offset, field_layout.align);
                            offset = offset
                                .checked_add(field_layout.size)
                                .ok_or(LayoutError::TooLarge(ty))?;
                            align = align.max(field_layout.align);
                        }
                        stack.pop();
                        Layout::new(round_up(offset, align), align)
                    }
                    AdtKind::Enum => self.enum_layout_inner(adt, stack)?.layout,
                }
            }
        };
        Ok(layout)
    }

    fn enum_layout_inner(
        &self,
        adt: AdtId,
        stack: &mut Vec<AdtId>,
    ) -> Result<EnumLayout, LayoutError> {
        let definition = self.tcx.adt(adt);
        if !definition.is_enum() {
            return Err(LayoutError::NotAnEnum(adt));
        }
        if u32::try_from(definition.variants.len()).is_err() {
            return Err(LayoutError::TooManyVariants(adt));
        }
        if stack.contains(&adt) {
            return Err(LayoutError::Recursive(adt));
        }

        let ty = self.tcx.adt_ty(adt);
        let tag = tag_layout(definition.variants.len());
        let mut payload_align = 1;
        let mut payload_size = 0;
        let mut raw = Vec::with_capacity(definition.variants.len());

        stack.push(adt);
        for variant in &definition.variants {
            let mut offset = 0;
            let mut fields = Vec::with_capacity(variant.fields.len());
            for field in &variant.fields {
                let layout = self.layout_inner(field.ty, stack)?;
                offset = round_up(offset, layout.align);
                fields.push(FieldLayout { offset, layout });
                offset = offset
                    .checked_add(layout.size)
                    .ok_or(LayoutError::TooLarge(ty))?;
                payload_align = payload_align.max(layout.align);
            }
            payload_size = payload_size.max(offset);
            raw.push(fields);
        }
        stack.pop();

        let payload_offset = round_up(tag.size, payload_align);
        let variants = raw
            .into_iter()
            .map(|fields| VariantLayout {
                fields: fields
                    .into_iter()
                    .map(|field| FieldLayout {
                        offset: field.offset + payload_offset,
                        layout: field.layout,
                    })
                    .collect(),
            })
            .collect();
        let align = tag.align.max(payload_align);
        let end = payload_offset
            .checked_add(payload_size)
            .ok_or(LayoutError::TooLarge(ty))?;

        Ok(EnumLayout {
            layout: Layout::new(round_up(end, align), align),
            tag,
            payload_offset,
            payload_size,
            payload_align,
            variants,
        })
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

/// Returns the discriminant layout for an enum with `variant_count` variants.
///
/// The tag is the smallest unsigned integer that can address every variant:
/// one byte up to 256 variants, two bytes up to 65536, and four bytes above that.
/// A zero-variant enum still reserves one byte, so its layout stays well-formed
/// even though no value of it can be constructed.
pub fn tag_layout(variant_count: usize) -> Layout {
    let size = if variant_count <= 1usize << 8 {
        1
    } else if variant_count <= 1usize << 16 {
        2
    } else {
        4
    };
    Layout::new(size, size)
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

    use super::{Layout, LayoutCtxt, LayoutError, tag_layout};
    use seal_ty::{AdtId, AdtKind, FieldDef, IntTy, TyCtxt, TyKind, VariantDef};

    fn def(index: u32) -> DefId {
        DefId::from_u32(index).expect("test definitions are small")
    }

    fn point(ctxt: &TyCtxt, interner: &Interner) -> AdtId {
        let adt = ctxt.push_adt(def(0), interner.intern("Point"), AdtKind::Struct);
        let int = ctxt.int_ty(IntTy::I32);
        ctxt.fill_adt(
            adt,
            vec![
                FieldDef {
                    def: def(1),
                    name: Some(interner.intern("x")),
                    ty: int,
                },
                FieldDef {
                    def: def(2),
                    name: Some(interner.intern("y")),
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
        let adt = ctxt.push_adt(def(0), interner.intern("Mixed"), AdtKind::Struct);
        ctxt.fill_adt(
            adt,
            vec![
                FieldDef {
                    def: def(1),
                    name: Some(interner.intern("flag")),
                    ty: ctxt.bool_ty(),
                },
                FieldDef {
                    def: def(2),
                    name: Some(interner.intern("value")),
                    ty: ctxt.int_ty(IntTy::I32),
                },
                FieldDef {
                    def: def(3),
                    name: Some(interner.intern("small")),
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
        let adt = ctxt.push_adt(def(0), interner.intern("Empty"), AdtKind::Struct);
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
        let adt = ctxt.push_adt(def(0), interner.intern("Bad"), AdtKind::Struct);
        let own = ctxt.adt_ty(adt);
        ctxt.fill_adt(
            adt,
            vec![FieldDef {
                def: def(1),
                name: Some(interner.intern("next")),
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
    fn tuple_fields_follow_declaration_order_without_reordering() {
        let ctxt = TyCtxt::new();
        let tuple = ctxt.tuple_ty(&[
            ctxt.bool_ty(),
            ctxt.int_ty(IntTy::I32),
            ctxt.int_ty(IntTy::I8),
        ]);
        let layout = LayoutCtxt::new(&ctxt, 64);
        let fields = layout.tuple_field_layouts(tuple).expect("tuple is sized");
        assert_eq!(fields[0].offset, 0);
        assert_eq!(fields[1].offset, 4);
        assert_eq!(fields[2].offset, 8);
        assert_eq!(layout.layout_of(tuple), Ok(Layout::new(12, 4)));
        assert_eq!(
            layout.tuple_field_layouts(ctxt.int_ty(IntTy::I32)),
            Err(LayoutError::NotATuple(ctxt.int_ty(IntTy::I32)))
        );
    }

    #[test]
    fn the_unit_tuple_is_zero_sized() {
        let ctxt = TyCtxt::new();
        let tuple = ctxt.tuple_ty(&[]);
        assert_eq!(
            LayoutCtxt::new(&ctxt, 64).layout_of(tuple),
            Ok(Layout::new(0, 1))
        );
        assert_eq!(tuple, ctxt.unit_ty());
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

    fn variant(
        interner: &Interner,
        base: u32,
        name: &str,
        fields: Vec<(&str, seal_ty::TyId)>,
    ) -> VariantDef {
        VariantDef {
            def: def(base),
            name: interner.intern(name),
            fields: fields
                .into_iter()
                .enumerate()
                .map(|(index, (name, ty))| FieldDef {
                    def: def(base + index as u32 + 1),
                    name: Some(interner.intern(name)),
                    ty,
                })
                .collect(),
        }
    }

    fn enum_with(
        ctxt: &TyCtxt,
        interner: &Interner,
        name: &str,
        variants: Vec<VariantDef>,
    ) -> AdtId {
        let adt = ctxt.push_adt(def(0), interner.intern(name), AdtKind::Enum);
        ctxt.fill_variants(adt, variants);
        adt
    }

    #[test]
    fn unit_only_enums_reserve_one_tag_byte() {
        let ctxt = TyCtxt::new();
        let interner = Interner::new();
        let adt = enum_with(
            &ctxt,
            &interner,
            "Status",
            vec![
                variant(&interner, 10, "Ready", Vec::new()),
                variant(&interner, 20, "Busy", Vec::new()),
            ],
        );

        let layout = LayoutCtxt::new(&ctxt, 64)
            .enum_layout(adt)
            .expect("the enum is sized");
        assert_eq!(layout.tag, Layout::new(1, 1));
        assert_eq!(layout.payload_offset, 1);
        assert_eq!(layout.payload_size, 0);
        assert_eq!(layout.payload_align, 1);
        assert_eq!(layout.layout, Layout::new(1, 1));
        assert!(
            layout
                .variants
                .iter()
                .all(|variant| variant.fields.is_empty())
        );
        assert_eq!(
            LayoutCtxt::new(&ctxt, 64).layout_of(ctxt.adt_ty(adt)),
            Ok(Layout::new(1, 1))
        );
    }

    #[test]
    fn an_enum_payload_starts_at_the_payload_alignment() {
        let ctxt = TyCtxt::new();
        let interner = Interner::new();
        let adt = enum_with(
            &ctxt,
            &interner,
            "Number",
            vec![
                variant(
                    &interner,
                    10,
                    "Small",
                    vec![("value", ctxt.uint_ty(seal_ty::UintTy::U8))],
                ),
                variant(
                    &interner,
                    20,
                    "Large",
                    vec![("value", ctxt.uint_ty(seal_ty::UintTy::U64))],
                ),
            ],
        );

        let layout = LayoutCtxt::new(&ctxt, 64)
            .enum_layout(adt)
            .expect("the enum is sized");
        assert_eq!(layout.tag, Layout::new(1, 1));
        assert_eq!(layout.payload_offset, 8);
        assert_eq!(layout.payload_size, 8);
        assert_eq!(layout.payload_align, 8);
        assert_eq!(layout.layout, Layout::new(16, 8));
        assert_eq!(layout.variants[0].fields[0].offset, 8);
        assert_eq!(layout.variants[0].fields[0].layout, Layout::new(1, 1));
        assert_eq!(layout.variants[1].fields[0].offset, 8);
        assert_eq!(layout.variants[1].fields[0].layout, Layout::new(8, 8));
    }

    #[test]
    fn a_small_payload_keeps_the_enum_tight() {
        let ctxt = TyCtxt::new();
        let interner = Interner::new();
        let adt = enum_with(
            &ctxt,
            &interner,
            "Cell",
            vec![variant(
                &interner,
                10,
                "Int",
                vec![("value", ctxt.int_ty(IntTy::I32))],
            )],
        );

        let layout = LayoutCtxt::new(&ctxt, 64)
            .enum_layout(adt)
            .expect("the enum is sized");
        assert_eq!(layout.payload_offset, 4);
        assert_eq!(layout.payload_size, 4);
        assert_eq!(layout.layout, Layout::new(8, 4));
        assert_eq!(layout.variants[0].fields[0].offset, 4);
    }

    #[test]
    fn an_enum_payload_can_repeat_fields_and_name_collisions_are_per_variant() {
        let ctxt = TyCtxt::new();
        let interner = Interner::new();
        let adt = enum_with(
            &ctxt,
            &interner,
            "Shape",
            vec![
                variant(
                    &interner,
                    10,
                    "Circle",
                    vec![("radius", ctxt.int_ty(IntTy::I32))],
                ),
                variant(
                    &interner,
                    20,
                    "Ring",
                    vec![
                        ("radius", ctxt.int_ty(IntTy::I32)),
                        ("width", ctxt.int_ty(IntTy::I32)),
                    ],
                ),
            ],
        );

        let layout = LayoutCtxt::new(&ctxt, 64)
            .enum_layout(adt)
            .expect("the enum is sized");
        assert_eq!(layout.variants[0].fields.len(), 1);
        assert_eq!(layout.variants[1].fields.len(), 2);
        assert_eq!(layout.variants[1].fields[0].offset, 4);
        assert_eq!(layout.variants[1].fields[1].offset, 8);
        assert_eq!(layout.layout, Layout::new(12, 4));
    }

    #[test]
    fn an_enum_that_contains_itself_by_value_is_rejected() {
        let ctxt = TyCtxt::new();
        let interner = Interner::new();
        let adt = ctxt.push_adt(def(0), interner.intern("Tree"), AdtKind::Enum);
        let own = ctxt.adt_ty(adt);
        ctxt.fill_variants(
            adt,
            vec![variant(&interner, 10, "Leaf", vec![("value", own)])],
        );
        assert_eq!(
            LayoutCtxt::new(&ctxt, 64).layout_of(own),
            Err(LayoutError::Recursive(adt))
        );
        assert_eq!(
            LayoutCtxt::new(&ctxt, 64).enum_layout(adt),
            Err(LayoutError::Recursive(adt))
        );
    }

    #[test]
    fn struct_only_and_enum_only_layout_queries_reject_the_other_kind() {
        let ctxt = TyCtxt::new();
        let interner = Interner::new();
        let structure = ctxt.push_adt(def(0), interner.intern("Point"), AdtKind::Struct);
        ctxt.fill_adt(structure, Vec::new());
        let enumeration = enum_with(
            &ctxt,
            &interner,
            "Status",
            vec![variant(&interner, 10, "Ready", Vec::new())],
        );

        let layout = LayoutCtxt::new(&ctxt, 64);
        assert_eq!(
            layout.field_layouts(enumeration),
            Err(LayoutError::NotAStruct(enumeration))
        );
        assert_eq!(
            layout.enum_layout(structure),
            Err(LayoutError::NotAnEnum(structure))
        );
        assert!(
            layout
                .field_layouts(structure)
                .expect("an empty struct")
                .is_empty()
        );
    }

    #[test]
    fn the_tag_is_the_smallest_unsigned_integer_that_fits_every_variant() {
        assert_eq!(tag_layout(0), Layout::new(1, 1));
        assert_eq!(tag_layout(1), Layout::new(1, 1));
        assert_eq!(tag_layout(256), Layout::new(1, 1));
        assert_eq!(tag_layout(257), Layout::new(2, 2));
        assert_eq!(tag_layout(65_536), Layout::new(2, 2));
        assert_eq!(tag_layout(65_537), Layout::new(4, 4));
    }

    #[test]
    fn positional_variant_fields_use_declaration_order_offsets() {
        let ctxt = TyCtxt::new();
        let interner = Interner::new();
        let adt = ctxt.push_adt(def(0), interner.intern("Pair"), AdtKind::Enum);
        ctxt.fill_variants(
            adt,
            vec![VariantDef {
                def: def(1),
                name: interner.intern("Both"),
                fields: vec![
                    FieldDef {
                        def: def(2),
                        name: None,
                        ty: ctxt.int_ty(IntTy::I32),
                    },
                    FieldDef {
                        def: def(3),
                        name: None,
                        ty: ctxt.uint_ty(seal_ty::UintTy::U8),
                    },
                ],
            }],
        );

        let layout = LayoutCtxt::new(&ctxt, 64)
            .enum_layout(adt)
            .expect("positional fields lay out");
        assert_eq!(layout.payload_offset, 4);
        assert_eq!(layout.variants[0].fields[0].offset, 4);
        assert_eq!(layout.variants[0].fields[1].offset, 8);
        assert_eq!(layout.payload_size, 5);
        assert_eq!(layout.layout, Layout::new(12, 4));
    }
}
