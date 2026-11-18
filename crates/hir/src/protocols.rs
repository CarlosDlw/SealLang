//! Compiler-provided iteration and indexing protocols.
//!
//! The language reserves four trait names for operators that need a stable
//! contract across every file: `Iterator`, `IntoIterator`, `Index`, and
//! `IndexMut`. This module installs their definitions before user items are
//! collected, so ordinary trait lookup, impl validation, method dispatch, and
//! object-safety checks can use them without a prelude or a second type system.

use seal_def::{DefData, DefId, DefKind};
use seal_span::Span;
use seal_symbol::{Interner, Symbol};
use seal_ty::{
    GenericParamDef, GenericSignature, Generics, RefKind, TraitDef, TraitMethodDef,
    TraitMethodReceiver, TyCtxt, TyId, TyKind,
};

use crate::Hir;

/// One compiler-provided protocol.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Protocol {
    /// `Iterator<T>`: a cursor with `has_next` and `next`.
    Iterator,
    /// `IntoIterator<I, T>`: a source that produces an iterator.
    IntoIterator,
    /// `Index<I, T>`: read-only indexed access.
    Index,
    /// `IndexMut<I, T>`: writable indexed access.
    IndexMut,
}

impl Protocol {
    /// Every protocol, in installation order.
    pub const ALL: [Self; 4] = [
        Self::Iterator,
        Self::IntoIterator,
        Self::Index,
        Self::IndexMut,
    ];

    /// Returns the source-level trait name.
    pub fn name(self) -> &'static str {
        match self {
            Self::Iterator => "Iterator",
            Self::IntoIterator => "IntoIterator",
            Self::Index => "Index",
            Self::IndexMut => "IndexMut",
        }
    }

    /// Looks up a protocol by its exact trait name.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|protocol| protocol.name() == name)
    }
}

/// One method declared by a compiler-provided protocol.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ProtocolMethod {
    /// `Iterator::has_next`.
    HasNext,
    /// `Iterator::next`.
    Next,
    /// `IntoIterator::iter`.
    Iter,
    /// `Index::index`.
    Index,
    /// `IndexMut::index_mut`.
    IndexMut,
}

impl ProtocolMethod {
    /// Returns the source-level method name.
    pub fn name(self) -> &'static str {
        match self {
            Self::HasNext => "has_next",
            Self::Next => "next",
            Self::Iter => "iter",
            Self::Index => "index",
            Self::IndexMut => "index_mut",
        }
    }
}

/// The stable definition IDs of the compiler-provided protocols.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProtocolDefs {
    iterator: DefId,
    into_iterator: DefId,
    index: DefId,
    index_mut: DefId,
    has_next: DefId,
    next: DefId,
    iter: DefId,
    index_fn: DefId,
    index_mut_fn: DefId,
}

impl ProtocolDefs {
    /// Returns the trait definition for `protocol`.
    pub fn trait_def(self, protocol: Protocol) -> DefId {
        match protocol {
            Protocol::Iterator => self.iterator,
            Protocol::IntoIterator => self.into_iterator,
            Protocol::Index => self.index,
            Protocol::IndexMut => self.index_mut,
        }
    }

    /// Returns the method definition for `method` in `protocol`.
    pub fn method(self, protocol: Protocol, method: ProtocolMethod) -> DefId {
        match (protocol, method) {
            (Protocol::Iterator, ProtocolMethod::HasNext) => self.has_next,
            (Protocol::Iterator, ProtocolMethod::Next) => self.next,
            (Protocol::IntoIterator, ProtocolMethod::Iter) => self.iter,
            (Protocol::Index, ProtocolMethod::Index) => self.index_fn,
            (Protocol::IndexMut, ProtocolMethod::IndexMut) => self.index_mut_fn,
            (_, method) => panic!(
                "protocol `{}` has no method `{}`",
                protocol.name(),
                method.name()
            ),
        }
    }

    /// Returns the protocol that owns `def`, when `def` is one of its traits.
    pub fn protocol_of_trait(self, def: DefId) -> Option<Protocol> {
        Protocol::ALL
            .into_iter()
            .find(|protocol| self.trait_def(*protocol) == def)
    }

    /// Returns the protocol and method named by `def`, when `def` is one of
    /// their method declarations.
    pub fn protocol_method_of(self, def: DefId) -> Option<(Protocol, ProtocolMethod)> {
        [
            (Protocol::Iterator, ProtocolMethod::HasNext),
            (Protocol::Iterator, ProtocolMethod::Next),
            (Protocol::IntoIterator, ProtocolMethod::Iter),
            (Protocol::Index, ProtocolMethod::Index),
            (Protocol::IndexMut, ProtocolMethod::IndexMut),
        ]
        .into_iter()
        .find(|(protocol, method)| self.method(*protocol, *method) == def)
    }
}

/// Returns `true` when `ty` is an array, a slice, or a reference to either.
///
/// Those targets have compiler-owned iteration and indexing behavior, so a
/// protocol implementation for them would never be selected.
pub(crate) fn is_builtin_sequence(ty_ctxt: &TyCtxt, ty: TyId) -> bool {
    match ty_ctxt.kind(ty) {
        TyKind::Array { .. } | TyKind::Slice { .. } => true,
        TyKind::Ref { pointee, .. } => is_builtin_sequence(ty_ctxt, pointee),
        _ => false,
    }
}

/// Installs the four protocol traits into an empty HIR.
pub(crate) fn install(hir: &mut Hir, interner: &Interner) -> ProtocolDefs {
    let iterator = install_iterator(hir, interner);
    let into_iterator = install_into_iterator(hir, interner);
    let index = install_index(hir, interner);
    let index_mut = install_index_mut(hir, interner);

    ProtocolDefs {
        iterator: iterator.trait_def,
        into_iterator: into_iterator.trait_def,
        index: index.trait_def,
        index_mut: index_mut.trait_def,
        has_next: iterator.has_next,
        next: iterator.next,
        iter: into_iterator.iter,
        index_fn: index.index,
        index_mut_fn: index_mut.index_mut,
    }
}

struct IteratorDefs {
    trait_def: DefId,
    has_next: DefId,
    next: DefId,
}

struct IntoIteratorDefs {
    trait_def: DefId,
    iter: DefId,
}

struct IndexDefs {
    trait_def: DefId,
    index: DefId,
}

struct IndexMutDefs {
    trait_def: DefId,
    index_mut: DefId,
}

fn install_iterator(hir: &mut Hir, interner: &Interner) -> IteratorDefs {
    let (trait_def, generics) = create_trait(hir, interner, Protocol::Iterator, &["T"]);
    let bool_ty = hir.ty_ctxt().bool_ty();
    let (has_next, has_next_def) = build_method(
        hir,
        interner,
        trait_def,
        "has_next",
        TraitMethodReceiver {
            reference: Some(RefKind::Shared),
        },
        Vec::new(),
        bool_ty,
    );
    let (next, next_def) = build_method(
        hir,
        interner,
        trait_def,
        "next",
        TraitMethodReceiver {
            reference: Some(RefKind::Exclusive),
        },
        Vec::new(),
        param_ty(hir, trait_def, 1),
    );
    install_trait_definition(
        hir,
        interner,
        Protocol::Iterator,
        trait_def,
        generics,
        vec![has_next_def, next_def],
    );

    IteratorDefs {
        trait_def,
        has_next,
        next,
    }
}

fn install_into_iterator(hir: &mut Hir, interner: &Interner) -> IntoIteratorDefs {
    let (trait_def, generics) = create_trait(hir, interner, Protocol::IntoIterator, &["I", "T"]);
    let (iter, iter_def) = build_method(
        hir,
        interner,
        trait_def,
        "iter",
        TraitMethodReceiver { reference: None },
        Vec::new(),
        param_ty(hir, trait_def, 1),
    );
    install_trait_definition(
        hir,
        interner,
        Protocol::IntoIterator,
        trait_def,
        generics,
        vec![iter_def],
    );

    IntoIteratorDefs { trait_def, iter }
}

fn install_index(hir: &mut Hir, interner: &Interner) -> IndexDefs {
    let (trait_def, generics) = create_trait(hir, interner, Protocol::Index, &["I", "T"]);
    let return_ty = hir
        .ty_ctxt()
        .ref_ty_with(RefKind::Shared, param_ty(hir, trait_def, 2));
    let (index, index_def) = build_method(
        hir,
        interner,
        trait_def,
        "index",
        TraitMethodReceiver {
            reference: Some(RefKind::Shared),
        },
        vec![param_ty(hir, trait_def, 1)],
        return_ty,
    );
    install_trait_definition(
        hir,
        interner,
        Protocol::Index,
        trait_def,
        generics,
        vec![index_def],
    );

    IndexDefs { trait_def, index }
}

fn install_index_mut(hir: &mut Hir, interner: &Interner) -> IndexMutDefs {
    let (trait_def, generics) = create_trait(hir, interner, Protocol::IndexMut, &["I", "T"]);
    let return_ty = hir
        .ty_ctxt()
        .ref_ty_with(RefKind::Exclusive, param_ty(hir, trait_def, 2));
    let (index_mut, index_mut_def) = build_method(
        hir,
        interner,
        trait_def,
        "index_mut",
        TraitMethodReceiver {
            reference: Some(RefKind::Exclusive),
        },
        vec![param_ty(hir, trait_def, 1)],
        return_ty,
    );
    install_trait_definition(
        hir,
        interner,
        Protocol::IndexMut,
        trait_def,
        generics,
        vec![index_mut_def],
    );

    IndexMutDefs {
        trait_def,
        index_mut,
    }
}

/// Creates a protocol trait, its implicit `Self`, and its declared parameters.
fn create_trait(
    hir: &mut Hir,
    interner: &Interner,
    protocol: Protocol,
    arguments: &[&str],
) -> (DefId, Generics) {
    let name = interner.intern(protocol.name());
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
fn build_method(
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
fn install_trait_definition(
    hir: &mut Hir,
    interner: &Interner,
    protocol: Protocol,
    trait_def: DefId,
    generics: Generics,
    methods: Vec<TraitMethodDef>,
) {
    let name = interner.intern(protocol.name());
    hir.ty_ctxt_mut().insert_trait_def(TraitDef::new(
        trait_def,
        name,
        generics,
        Vec::new(),
        methods,
    ));
}

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

fn param_ty(hir: &Hir, trait_def: DefId, index: usize) -> TyId {
    let generics = hir
        .ty_ctxt()
        .generics(trait_def)
        .expect("protocol trait has generics");
    hir.ty_ctxt().param_ty(generics.params[index].param)
}
