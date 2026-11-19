//! Compiler-provided traits installed into every HIR.
//!
//! The language reserves a handful of trait names for behavior the compiler
//! itself relies on: the iteration and indexing protocols, and `Drop`.
//! Installing them before user items are collected means ordinary trait
//! lookup, implementation validation, method dispatch, and object safety treat
//! them like any other declaration, without a prelude or a second type system.

pub(crate) mod drop;
pub(crate) mod protocols;

use seal_def::{DefData, DefId, DefKind};
use seal_span::Span;
use seal_symbol::{Interner, Symbol};
use seal_ty::{
    GenericParamDef, GenericSignature, Generics, TraitDef, TraitMethodDef, TraitMethodReceiver,
    TyId,
};

use crate::Hir;

/// The stable definition IDs of every compiler-provided trait.
#[derive(Debug)]
pub(crate) struct BuiltinTraits {
    /// The iteration and indexing protocols.
    pub(crate) protocols: protocols::ProtocolDefs,
    /// The `Drop` trait.
    pub(crate) drop: drop::DropDefs,
}

/// Installs every compiler-provided trait into an empty HIR.
pub(crate) fn install(hir: &mut Hir, interner: &Interner) -> BuiltinTraits {
    let protocols = protocols::install(hir, interner);
    let drop = drop::install(hir, interner);
    BuiltinTraits { protocols, drop }
}

/// Creates a reserved trait, its implicit `Self`, and its declared parameters.
pub(super) fn create_trait(
    hir: &mut Hir,
    interner: &Interner,
    trait_name: &str,
    arguments: &[&str],
) -> (DefId, Generics) {
    let name = interner.intern(trait_name);
    let trait_def = hir.define(DefData {
        kind: DefKind::Trait,
        name,
        name_span: Span::default(),
        span: Span::default(),
        parent: None,
        mutable: false,
    });

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
    let method_def = hir.define(DefData {
        kind: DefKind::TraitMethod,
        name,
        name_span: Span::default(),
        span: Span::default(),
        parent: Some(trait_def),
        mutable: false,
    });
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
    methods: Vec<TraitMethodDef>,
) {
    let name = interner.intern(trait_name);
    hir.ty_ctxt_mut().insert_trait_def(TraitDef::new(
        trait_def,
        name,
        generics,
        Vec::new(),
        methods,
    ));
}

/// Declares one generic parameter of a compiler-provided trait.
fn define_param(hir: &mut Hir, owner: DefId, name: Symbol, index: u32) -> GenericParamDef {
    let definition = hir.define(DefData {
        kind: DefKind::GenericParam,
        name,
        name_span: Span::default(),
        span: Span::default(),
        parent: Some(owner),
        mutable: false,
    });
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
