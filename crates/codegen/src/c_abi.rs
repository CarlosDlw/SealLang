//! C ABI classification for aggregates passed or returned by value.
//!
//! LLVM lowers a raw struct type by its own rules, not by the C ABI of the
//! target. A C compiler coerces a small struct into integer or floating-point
//! pieces, and passes a large one indirectly. Code generation must use the same
//! shape at every external call, or the C callee reads the wrong registers.
//!
//! This module classifies one C-layout struct for the supported targets. The
//! classification depends only on size, field offsets, and scalar classes, so
//! the same answer is used for declarations and call sites.

use inkwell::types::{BasicTypeEnum, FunctionType};
use seal_layout::{LayoutCtxt, LayoutError};
use seal_target::{Arch, Os, TargetSpec};
use seal_ty::{AdtKind, FloatTy, TyCtxt, TyId, TyKind};

use crate::error::CodegenError;

/// The C ABI family used by a target.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum CAbi {
    /// The System V AMD64 ABI used on Linux, macOS, and the BSDs.
    SysVX86_64,
    /// The Microsoft x64 ABI.
    MsvcX64,
    /// A target whose aggregate ABI is not implemented yet.
    Unsupported(String),
}

impl CAbi {
    /// Selects the aggregate ABI for `target`.
    pub(crate) fn for_target(target: &TargetSpec) -> Self {
        match (target.triple().arch(), target.triple().os()) {
            (Arch::X86_64, Os::Windows) => Self::MsvcX64,
            (Arch::X86_64, _) => Self::SysVX86_64,
            (arch, _) => Self::Unsupported(format!(
                "{} ({} C aggregates by value)",
                target.triple(),
                arch.as_str()
            )),
        }
    }

    /// Classifies `ty` at a C boundary.
    ///
    /// A non-aggregate type is direct. A C-layout struct is classified by the
    /// target ABI; a Seal-layout aggregate cannot reach here because FFI
    /// checking rejected it.
    pub(crate) fn classify(
        &self,
        tcx: &TyCtxt,
        layouts: &LayoutCtxt<'_>,
        ty: TyId,
    ) -> Result<CAggregate, CodegenError> {
        let TyKind::Adt { adt, args } = tcx.kind(ty) else {
            return Ok(CAggregate::Direct);
        };
        let definition = tcx.adt(adt);
        if definition.kind != AdtKind::Struct || !definition.is_c_repr() {
            return Ok(CAggregate::Direct);
        }
        let layout = layouts.layout_of(ty).map_err(layout_error)?;
        let size = layout.size();
        match self {
            Self::SysVX86_64 if size > 16 => Ok(CAggregate::Memory { byval: true }),
            Self::SysVX86_64 => {
                let parts = sysv_parts(tcx, layouts, adt, args, size)?;
                Ok(CAggregate::Small(parts))
            }
            Self::MsvcX64 if matches!(size, 1 | 2 | 4 | 8) => {
                Ok(CAggregate::Small(vec![CoercedPart::Integer(
                    (size as u16) * 8,
                )]))
            }
            Self::MsvcX64 => Ok(CAggregate::Memory { byval: false }),
            Self::Unsupported(name) => Err(CodegenError::unsupported_target(
                name,
                "C aggregates by value are not supported on this target",
            )),
        }
    }
}

/// How one aggregate crosses a C boundary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum CAggregate {
    /// A scalar type with its ordinary LLVM representation.
    Direct,
    /// A small struct represented by one or more coerced pieces.
    Small(Vec<CoercedPart>),
    /// A struct passed or returned through memory.
    ///
    /// `byval` selects the System V form, where LLVM copies the pointed-to
    /// bytes to the stack as part of the call. Other ABIs pass an ordinary
    /// pointer to a caller-owned copy.
    Memory {
        /// Whether the pointer parameter carries the `byval` attribute.
        byval: bool,
    },
}

/// One LLVM piece of a coerced C aggregate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CoercedPart {
    /// An integer of the given width, holding raw field bits.
    Integer(u16),
    /// A single floating-point value.
    Float(FloatTy),
    /// A vector of floating-point values packed in one SSE register.
    Vector(FloatTy, u32),
}

/// The LLVM signature of one external function.
///
/// It carries both the LLVM types and what each position means, so a call site
/// can coerce arguments and results without re-classifying the declaration.
pub(crate) struct CSignature<'ctx> {
    /// The LLVM function type used by the declaration.
    pub function_type: FunctionType<'ctx>,
    /// Parameters in LLVM order, including a leading `sret` pointer.
    pub params: Vec<CParam<'ctx>>,
    /// How the result is returned.
    pub ret: CReturn<'ctx>,
}

/// One external function parameter.
pub(crate) struct CParam<'ctx> {
    /// How the source argument maps to its LLVM parameter.
    pub kind: CParamKind<'ctx>,
}

/// The mapping from a source argument to its LLVM parameter.
pub(crate) enum CParamKind<'ctx> {
    /// The scalar value is passed as-is.
    Direct,
    /// The struct is reinterpreted as its coerced pieces.
    Coerced(BasicTypeEnum<'ctx>),
    /// The struct is copied and a pointer is passed.
    ByPointer {
        /// Whether the pointer carries the System V `byval` attribute.
        byval: bool,
        /// The original struct type, used by `byval`.
        aggregate: BasicTypeEnum<'ctx>,
    },
}

/// How an external function returns its result.
pub(crate) enum CReturn<'ctx> {
    /// The function returns `void`.
    Void,
    /// The scalar result is returned as-is.
    Direct,
    /// The struct result is returned as coerced pieces.
    Coerced(BasicTypeEnum<'ctx>),
    /// The struct result is written through a leading pointer parameter.
    Sret(BasicTypeEnum<'ctx>),
}

/// One eightbyte's System V class.
#[derive(Clone, Default)]
struct EightByte {
    integer: bool,
    sse: bool,
    floats: Vec<FloatTy>,
}

fn sysv_parts(
    tcx: &TyCtxt,
    layouts: &LayoutCtxt<'_>,
    adt: seal_ty::AdtId,
    args: seal_ty::SubstsId,
    size: u64,
) -> Result<Vec<CoercedPart>, CodegenError> {
    let count = size.div_ceil(8).max(1) as usize;
    let mut eightbytes = vec![EightByte::default(); count];
    mark_struct(tcx, layouts, adt, args, 0, &mut eightbytes)?;

    let mut parts = Vec::with_capacity(count);
    for (index, eightbyte) in eightbytes.iter().enumerate() {
        let bits = ((size - index as u64 * 8).min(8) * 8) as u16;
        if eightbyte.integer || !eightbyte.sse {
            parts.push(CoercedPart::Integer(bits));
        } else {
            parts.push(sse_part(eightbyte, bits));
        }
    }
    Ok(parts)
}

fn sse_part(eightbyte: &EightByte, bits: u16) -> CoercedPart {
    let f32_count = eightbyte
        .floats
        .iter()
        .filter(|float| **float == FloatTy::F32)
        .count();
    let f64_count = eightbyte.floats.len() - f32_count;
    match (f32_count, f64_count) {
        (0, 1) => CoercedPart::Float(FloatTy::F64),
        (1, 0) => CoercedPart::Float(FloatTy::F32),
        (count, 0) if count > 1 => CoercedPart::Vector(FloatTy::F32, count as u32),
        // A mixed floating eightbyte (or one the classifier cannot describe)
        // keeps its raw bits in an integer. It only occurs for exotic layouts
        // that this phase does not declare.
        _ => CoercedPart::Integer(bits),
    }
}

/// Marks every byte occupied by one field of the struct.
fn mark_struct(
    tcx: &TyCtxt,
    layouts: &LayoutCtxt<'_>,
    adt: seal_ty::AdtId,
    args: seal_ty::SubstsId,
    base: u64,
    eightbytes: &mut [EightByte],
) -> Result<(), CodegenError> {
    let definition = tcx.adt(adt);
    let fields = layouts
        .field_layouts_with(adt, args)
        .map_err(layout_error)?;
    for (field, field_layout) in definition.fields.iter().zip(fields) {
        let ty = tcx.substitute(field.ty, args);
        mark_type(tcx, layouts, ty, base + field_layout.offset, eightbytes)?;
    }
    Ok(())
}

fn mark_type(
    tcx: &TyCtxt,
    layouts: &LayoutCtxt<'_>,
    ty: TyId,
    offset: u64,
    eightbytes: &mut [EightByte],
) -> Result<(), CodegenError> {
    match tcx.kind(ty) {
        TyKind::Float(float) => {
            let start = offset / 8;
            let slot = &mut eightbytes[start as usize];
            slot.sse = true;
            slot.floats.push(float);
        }
        TyKind::Adt { adt, args } => {
            let definition = tcx.adt(adt);
            if definition.is_struct() {
                mark_struct(tcx, layouts, adt, args, offset, eightbytes)?;
            }
        }
        TyKind::Array { element, length } => {
            let element_layout = layouts.layout_of(element).map_err(layout_error)?;
            for index in 0..length {
                mark_type(
                    tcx,
                    layouts,
                    element,
                    offset + index * element_layout.size(),
                    eightbytes,
                )?;
            }
        }
        _ => {
            let layout = layouts.layout_of(ty).map_err(layout_error)?;
            let start = offset / 8;
            let end = (offset + layout.size().saturating_sub(1)) / 8;
            for slot in &mut eightbytes[start as usize..=end as usize] {
                slot.integer = true;
            }
        }
    }
    Ok(())
}

fn layout_error(error: LayoutError) -> CodegenError {
    CodegenError::internal(format!("C aggregate has no usable layout: {error}"))
}
