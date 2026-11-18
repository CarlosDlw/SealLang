//! The memory image of one constant value.
//!
//! An image is a byte buffer plus the symbolic pointer slots the value contains.
//! It is the single source of truth for a global: the LLVM type and the LLVM
//! initializer both come from the same split of the buffer, so the layout of the
//! emitted global is exactly the encoded layout.
//!
//! The LLVM type is a packed struct of byte runs and pointer fields. Packed
//! fields make the LLVM layout equal to the encoded offsets regardless of the
//! natural alignment of the members; the global carries the language alignment,
//! and the opaque pointer ABI lets every load and store use the language type.

use inkwell::context::Context;
use inkwell::types::BasicTypeEnum;
use inkwell::values::BasicValueEnum;
use seal_target::Endianness;

use crate::error::CodegenError;
use crate::relocation::{PointerTarget, Pointers};

/// The memory image of one constant value.
pub(crate) struct Image {
    bytes: Vec<u8>,
    slots: Vec<Slot>,
    /// Which bytes a scalar write already claimed. Overlapping writes mean two
    /// values were encoded into one slot, which is a compiler bug.
    written: Vec<bool>,
    endianness: Endianness,
    pointer_size: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Slot {
    offset: u64,
    target: PointerTarget,
}

/// One field of the packed LLVM representation.
#[derive(Clone, Copy, Debug)]
enum Field {
    /// A run of literal bytes.
    Bytes { start: usize, end: usize },
    /// One pointer slot.
    Slot(usize),
}

impl Image {
    /// Creates a zero-filled image of `size` bytes.
    pub(crate) fn new(size: u64, endianness: Endianness, pointer_size: u64) -> Self {
        Self {
            bytes: vec![0; size as usize],
            slots: Vec::new(),
            written: vec![false; size as usize],
            endianness,
            pointer_size,
        }
    }

    /// Returns `true` when the value occupies no bytes.
    pub(crate) fn is_zero_sized(&self) -> bool {
        self.bytes.is_empty()
    }

    /// Writes the low `size` bytes of `bits` at `offset` in target order.
    pub(crate) fn write(&mut self, offset: u64, size: u64, bits: u128) -> Result<(), CodegenError> {
        if size > 16 {
            return Err(CodegenError::internal(format!(
                "a constant scalar of {size} bytes does not fit a 128-bit word"
            )));
        }
        let start = self.checked_start(offset, size)?;
        for slot in &self.slots {
            let end = slot.offset + self.pointer_size;
            if offset < end && slot.offset < offset + size {
                return Err(CodegenError::internal(
                    "a constant scalar overlaps a pointer slot",
                ));
            }
        }
        for (index, written) in self.written[start..start + size as usize]
            .iter()
            .enumerate()
        {
            if *written {
                return Err(CodegenError::internal(format!(
                    "constant byte {} is written twice",
                    offset + index as u64
                )));
            }
        }
        for index in 0..size as usize {
            let shift = match self.endianness {
                Endianness::Little => index * 8,
                Endianness::Big => (size as usize - 1 - index) * 8,
            };
            self.bytes[start + index] = (bits >> shift) as u8;
            self.written[start + index] = true;
        }
        Ok(())
    }

    /// Records a pointer slot at `offset`, keeping slots in offset order.
    pub(crate) fn slot(&mut self, offset: u64, target: PointerTarget) -> Result<(), CodegenError> {
        let start = self.checked_start(offset, self.pointer_size)?;
        for (index, written) in self.written[start..start + self.pointer_size as usize]
            .iter()
            .enumerate()
        {
            if *written {
                return Err(CodegenError::internal(format!(
                    "pointer slot at byte {} overlaps a constant scalar",
                    offset + index as u64
                )));
            }
        }
        for other in &self.slots {
            let end = other.offset + self.pointer_size;
            if offset < end && other.offset < offset + self.pointer_size {
                return Err(CodegenError::internal(
                    "two pointer slots of one constant overlap",
                ));
            }
        }
        let position = self
            .slots
            .iter()
            .position(|slot| slot.offset > offset)
            .unwrap_or(self.slots.len());
        self.slots.insert(position, Slot { offset, target });
        Ok(())
    }

    /// Returns the LLVM representation type of this image.
    ///
    /// A zero-sized value still receives one byte, so its global has a unique
    /// address even though no language-level byte is stored.
    pub(crate) fn llvm_type<'ctx>(
        &self,
        context: &'ctx Context,
    ) -> Result<BasicTypeEnum<'ctx>, CodegenError> {
        if self.is_zero_sized() {
            return Ok(context.i8_type().into());
        }
        let mut types = Vec::new();
        for field in self.fields()? {
            types.push(self.field_type(context, field)?);
        }
        Ok(context.struct_type(&types, true).into())
    }

    /// Returns the LLVM constant for this image, resolving every pointer slot.
    pub(crate) fn llvm_value<'ctx>(
        &self,
        context: &'ctx Context,
        pointers: &Pointers<'_, 'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        if self.is_zero_sized() {
            return Ok(context.i8_type().const_zero().into());
        }
        let mut values = Vec::new();
        let mut types: Vec<BasicTypeEnum<'ctx>> = Vec::new();
        for field in self.fields()? {
            let value = match field {
                Field::Bytes { start, end } => {
                    let bytes = context.const_string(&self.bytes[start..end], false);
                    BasicValueEnum::from(bytes)
                }
                Field::Slot(index) => {
                    BasicValueEnum::from(pointers.resolve(self.slots[index].target)?)
                }
            };
            types.push(value.get_type());
            values.push(value);
        }
        Ok(context
            .struct_type(&types, true)
            .const_named_struct(&values)
            .into())
    }

    fn field_type<'ctx>(
        &self,
        context: &'ctx Context,
        field: Field,
    ) -> Result<BasicTypeEnum<'ctx>, CodegenError> {
        Ok(match field {
            Field::Bytes { start, end } => {
                let length = u32::try_from(end - start).map_err(|_| {
                    CodegenError::internal("a constant byte run does not fit an LLVM array")
                })?;
                context.i8_type().array_type(length).into()
            }
            Field::Slot(_) => context.ptr_type(inkwell::AddressSpace::default()).into(),
        })
    }

    /// Returns the start index of `offset..offset + size`, rejecting overflow
    /// and reads outside the image.
    fn checked_start(&self, offset: u64, size: u64) -> Result<usize, CodegenError> {
        let end = offset.checked_add(size).ok_or_else(|| {
            CodegenError::internal("a constant field offset overflows the address space")
        })?;
        if end > self.bytes.len() as u64 {
            return Err(CodegenError::internal(format!(
                "a constant field at {offset}..{end} is outside its {} byte value",
                self.bytes.len()
            )));
        }
        Ok(offset as usize)
    }

    /// Splits the image into packed-struct fields, in offset order.
    fn fields(&self) -> Result<Vec<Field>, CodegenError> {
        let pointer_size = self.pointer_size;
        let mut fields = Vec::new();
        let mut position = 0u64;
        for (index, slot) in self.slots.iter().enumerate() {
            if slot.offset < position {
                return Err(CodegenError::internal(
                    "two pointer slots of one constant overlap",
                ));
            }
            if slot.offset > position {
                fields.push(Field::Bytes {
                    start: position as usize,
                    end: slot.offset as usize,
                });
            }
            self.checked_start(slot.offset, pointer_size)?;
            fields.push(Field::Slot(index));
            position = slot.offset + pointer_size;
        }
        if position < self.bytes.len() as u64 {
            fields.push(Field::Bytes {
                start: position as usize,
                end: self.bytes.len(),
            });
        }
        Ok(fields)
    }
}
