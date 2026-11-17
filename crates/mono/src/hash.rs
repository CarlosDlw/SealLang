//! Stable names and fingerprints for instances.
//!
//! Code generation names one symbol per instance. The name must be identical
//! across runs and builds of the same source, so the fingerprint walks the type
//! structure instead of hashing arena handles or pointer-sized values.

use seal_hash::{Fingerprint, StableHasher};
use seal_hir::Hir;
use seal_symbol::Interner;
use seal_ty::{Instance, TyCtxt, TyId, TyKind};

/// Returns the stable fingerprint of `instance`.
pub fn instance_hash(hir: &Hir, instance: Instance) -> Fingerprint {
    let tcx = hir.ty_ctxt();
    let mut hasher = StableHasher::new();
    hasher.write_usize(instance.def.index() as usize);
    let arguments = tcx.substs_args(instance.args);
    hasher.write_length(arguments.len());
    for argument in arguments {
        hash_type(tcx, argument, &mut hasher);
    }
    hasher.finish()
}

/// Returns a readable name such as `identity<i32>` for diagnostics.
pub fn instance_name(hir: &Hir, interner: &Interner, instance: Instance) -> String {
    let base = interner.resolve(hir.definitions().name(instance.def));
    let arguments = hir.ty_ctxt().substs_args(instance.args);
    if arguments.is_empty() {
        return base.to_owned();
    }
    let rendered: Vec<String> = arguments
        .iter()
        .map(|argument| hir.ty_ctxt().display_with(*argument, interner).to_string())
        .collect();
    format!("{base}<{}>", rendered.join(", "))
}

fn hash_type(tcx: &TyCtxt, ty: TyId, hasher: &mut StableHasher) {
    match tcx.kind(ty) {
        TyKind::Bool => hasher.write_u8(0),
        TyKind::Char => hasher.write_u8(1),
        TyKind::Str => hasher.write_u8(2),
        TyKind::Unit => hasher.write_u8(3),
        TyKind::Never => hasher.write_u8(4),
        TyKind::Error => hasher.write_u8(5),
        TyKind::Int(int) => {
            hasher.write_u8(6);
            hasher.write_u8(int as u8);
        }
        TyKind::Uint(uint) => {
            hasher.write_u8(7);
            hasher.write_u8(uint as u8);
        }
        TyKind::Float(float) => {
            hasher.write_u8(8);
            hasher.write_u8(float as u8);
        }
        TyKind::Ref { kind, pointee } => {
            hasher.write_u8(9);
            hasher.write_u8(u8::from(kind.is_exclusive()));
            hash_type(tcx, pointee, hasher);
        }
        TyKind::Slice { element } => {
            hasher.write_u8(10);
            hash_type(tcx, element, hasher);
        }
        TyKind::Array { element, length } => {
            hasher.write_u8(11);
            hasher.write_u64(length);
            hash_type(tcx, element, hasher);
        }
        TyKind::Tuple(tuple) => {
            hasher.write_u8(12);
            let elements = tcx.tuple_elements(tuple);
            hasher.write_length(elements.len());
            for element in elements {
                hash_type(tcx, element, hasher);
            }
        }
        TyKind::Adt { adt, args } => {
            hasher.write_u8(13);
            hasher.write_usize(adt.index() as usize);
            let arguments = tcx.substs_args(args);
            hasher.write_length(arguments.len());
            for argument in arguments {
                hash_type(tcx, argument, hasher);
            }
        }
        TyKind::Param(param) => {
            hasher.write_u8(14);
            hasher.write_usize(param.index() as usize);
        }
        TyKind::Infer(variable) => {
            hasher.write_u8(15);
            hasher.write_usize(variable.index() as usize);
        }
    }
}
