//! Compiler-provided traits installed into every HIR.
//!
//! The language reserves a handful of trait names for behavior the compiler
//! itself relies on: the iteration and indexing protocols, `Drop`, and the
//! `Copy` marker. Installing them before user items are collected means
//! ordinary trait lookup, implementation validation, method dispatch, and
//! object safety treat them like any other declaration, without a prelude or a
//! second type system.

pub(crate) mod copy;
pub(crate) mod drop;
pub(crate) mod fn_traits;
pub(crate) mod protocols;

use seal_def::{DefData, DefId, DefKind};
use seal_span::Span;
use seal_symbol::{Interner, Symbol};
use seal_ty::{
    GenericParamDef, GenericSignature, Generics, TraitDef, TraitMethodDef, TraitMethodReceiver,
    TraitRef, TyId,
};

use crate::Hir;

/// The stable definition IDs of every compiler-provided trait.
#[derive(Debug)]
pub(crate) struct BuiltinTraits {
    /// The iteration and indexing protocols.
    pub(crate) protocols: protocols::ProtocolDefs,
    /// The `Drop` trait.
    pub(crate) drop: drop::DropDefs,
    /// The `Copy` marker trait.
    pub(crate) copy: copy::CopyDefs,
    /// The function traits.
    pub(crate) fn_traits: fn_traits::FnTraits,
    /// The built-in opaque `c_void` type.
    pub(crate) c_void: DefId,
}

/// Installs every compiler-provided trait into an empty HIR.
pub(crate) fn install(hir: &mut Hir, interner: &Interner) -> BuiltinTraits {
    let protocols = protocols::install(hir, interner);
    let drop = drop::install(hir, interner);
    let copy = copy::install(hir, interner);
    let fn_traits = fn_traits::install(hir, interner);
    let c_void = install_c_void(hir, interner);
    BuiltinTraits {
        protocols,
        drop,
        copy,
        fn_traits,
        c_void,
    }
}

/// Installs the built-in `c_void` opaque type.
///
/// C uses `void` as the pointee of an untyped pointer. The type has no size and
/// only appears behind a raw pointer, exactly like a user `extern type`.
fn install_c_void(hir: &mut Hir, interner: &Interner) -> DefId {
    let name = interner.intern("c_void");
    hir.define(DefData::new(
        DefKind::OpaqueType,
        name,
        Span::default(),
        Span::default(),
        None,
        false,
    ))
}

/// Creates a reserved trait, its implicit `Self`, and its declared parameters.
pub(super) fn create_trait(
    hir: &mut Hir,
    interner: &Interner,
    trait_name: &str,
    arguments: &[&str],
) -> (DefId, Generics) {
    let name = interner.intern(trait_name);
    let trait_def = hir.define(DefData::new(
        DefKind::Trait,
        name,
        Span::default(),
        Span::default(),
        None,
        false,
    ));

    let mut params = Vec::with_capacity(arguments.len() + 1);
    params.push(define_param(hir, trait_def, interner.intern("Self"), 0).as_self());
    for (index, argument) in arguments.iter().enumerate() {
        params.push(define_param(
            hir,
            trait_def,
            interner.intern(argument),
            (index + 1) as u32,
        ));
    }
    let generics = Generics::new(trait_def, None, params);
    hir.ty_ctxt_mut().insert_generics(generics.clone());
    hir.insert_trait_name(name, trait_def);
    (trait_def, generics)
}

/// Creates one trait method definition, including its own empty generics.
pub(super) fn build_method(
    hir: &mut Hir,
    interner: &Interner,
    trait_def: DefId,
    name: &str,
    receiver: TraitMethodReceiver,
    params: Vec<TyId>,
    ret: TyId,
) -> (DefId, TraitMethodDef) {
    let name = interner.intern(name);
    let method_def = hir.define(DefData::new(
        DefKind::TraitMethod,
        name,
        Span::default(),
        Span::default(),
        Some(trait_def),
        false,
    ));
    let generics = Generics::new(method_def, Some(trait_def), Vec::new());
    hir.ty_ctxt_mut().insert_generics(generics.clone());
    let definition = TraitMethodDef::new(
        method_def,
        name,
        GenericSignature::new(params, ret),
        generics,
        Some(receiver),
    )
    .with_body(false);
    (method_def, definition)
}

/// Stores the completed semantic trait declaration.
pub(super) fn install_trait_definition(
    hir: &mut Hir,
    interner: &Interner,
    trait_name: &str,
    trait_def: DefId,
    generics: Generics,
    supertraits: Vec<TraitRef>,
    methods: Vec<TraitMethodDef>,
) {
    let name = interner.intern(trait_name);
    hir.ty_ctxt_mut().insert_trait_def(TraitDef::new(
        trait_def,
        name,
        generics,
        supertraits,
        methods,
    ));
}

/// Declares one generic parameter of a compiler-provided trait.
fn define_param(hir: &mut Hir, owner: DefId, name: Symbol, index: u32) -> GenericParamDef {
    let definition = hir.define(DefData::new(
        DefKind::GenericParam,
        name,
        Span::default(),
        Span::default(),
        Some(owner),
        false,
    ));
    let param = hir.ty_ctxt_mut().push_param(definition, owner, index, name);
    GenericParamDef::new(definition, param, name, index)
}

/// Returns the lowered type of a compiler-provided trait parameter.
pub(super) fn param_ty(hir: &Hir, trait_def: DefId, index: usize) -> TyId {
    let generics = hir
        .ty_ctxt()
        .generics(trait_def)
        .expect("builtin trait has generics");
    hir.ty_ctxt().param_ty(generics.params[index].param)
}
