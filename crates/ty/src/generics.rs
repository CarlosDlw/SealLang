//! Generic parameters, substitutions, trait references, and instances.
//!
//! These are semantic values, not syntax. The parser and HIR own source spans
//! and declaration sites; this module owns the identities and relationships that
//! type checking, layout, and monomorphization compare.
//!
//! A [`ParamTy`] is a declared type parameter. It is rigid while a generic body
//! is checked and is replaced by [`TyCtxt::substitute`](crate::TyCtxt::substitute)
//! when the declaration is instantiated. A [`SubstsId`] is an interned ordered
//! list of type arguments; a [`TraitRef`] records the implementing type and the
//! trait's own arguments.

use seal_def::DefId;
use seal_symbol::Symbol;

use crate::id::{ParamTyId, SubstsId, TyId};
use crate::kind::RefKind;

/// The identity and display data of one declared type parameter.
///
/// Two parameters with the same name in different owners are different
/// parameters. Equality is therefore by declaration, not by spelling.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ParamTy {
    /// The parameter's declaration.
    pub def: DefId,
    /// The owner that declares the parameter, for diagnostics and scope.
    pub owner: DefId,
    /// The parameter's flattened position in its owner's generic list.
    pub index: u32,
    /// The source-level name, used for display.
    pub name: Symbol,
}

/// One declared generic parameter and its bounds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GenericParamDef {
    /// The definition created for this parameter.
    pub def: DefId,
    /// The interned parameter type standing for this declaration.
    pub param: ParamTyId,
    /// The source-level name.
    pub name: Symbol,
    /// The flattened position, outermost first.
    pub index: u32,
    /// Bounds that every instantiation must satisfy.
    pub bounds: Vec<Bound>,
    /// Whether this is a trait's implicit `Self` parameter.
    pub is_self: bool,
}

impl GenericParamDef {
    /// Creates a parameter definition with no bounds.
    pub fn new(def: DefId, param: ParamTyId, name: Symbol, index: u32) -> Self {
        Self {
            def,
            param,
            name,
            index,
            bounds: Vec::new(),
            is_self: false,
        }
    }

    /// Marks this parameter as the implicit `Self` of a trait.
    pub fn as_self(mut self) -> Self {
        self.is_self = true;
        self
    }

    /// Adds a bound to this parameter.
    pub fn with_bound(mut self, bound: Bound) -> Self {
        self.bounds.push(bound);
        self
    }
}

/// The generic parameters declared by one owner.
///
/// A function inside an impl has the impl as [`Generics::parent`]; its flattened
/// parameter indices start after the parent's parameters. The `params` field
/// contains only the owner's own declarations.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Generics {
    /// The definition that owns these parameters.
    pub owner: DefId,
    /// The enclosing generic owner, if any.
    pub parent: Option<DefId>,
    /// The owner's own parameters, in declaration order.
    pub params: Vec<GenericParamDef>,
}

impl Generics {
    /// Creates a generic-parameter list for `owner`.
    pub fn new(owner: DefId, parent: Option<DefId>, params: Vec<GenericParamDef>) -> Self {
        Self {
            owner,
            parent,
            params,
        }
    }

    /// Returns the number of parameters declared by this owner.
    pub fn len(&self) -> usize {
        self.params.len()
    }

    /// Returns `true` if this owner declares no parameters of its own.
    pub fn is_empty(&self) -> bool {
        self.params.is_empty()
    }

    /// Returns the parameter with the given interned type, if it belongs here.
    pub fn param(&self, param: ParamTyId) -> Option<&GenericParamDef> {
        self.params.iter().find(|data| data.param == param)
    }

    /// Returns the parameter with the given name, if it belongs here.
    pub fn param_by_name(&self, name: Symbol) -> Option<&GenericParamDef> {
        self.params.iter().find(|data| data.name == name)
    }
}

/// A requirement placed on a generic parameter.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Bound {
    /// The argument must be a type that can be copied bitwise.
    Copy,
    /// The argument must implement the referenced trait.
    Trait(TraitRef),
}

#[cfg(test)]
mod tests;

/// A trait applied to a self type with the trait's own type arguments.
///
/// `Self` is stored separately in [`TraitRef::self_ty`] and is not part of
/// `args`. This keeps `impl<T> Trait<T> for Pair<T>` unambiguous: the trait has
/// one own argument, while `Self` is `Pair<T>`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TraitRef {
    /// The trait declaration.
    pub trait_def: DefId,
    /// The type implementing the trait.
    pub self_ty: TyId,
    /// The trait's own type arguments, excluding `Self`.
    pub args: SubstsId,
}

impl TraitRef {
    /// Creates a trait reference.
    pub fn new(trait_def: DefId, self_ty: TyId, args: SubstsId) -> Self {
        Self {
            trait_def,
            self_ty,
            args,
        }
    }
}

/// A function signature expressed with declared parameters and type arguments.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GenericSignature {
    /// Parameter types in declaration order.
    pub params: Vec<TyId>,
    /// The return type; `()` when no return type is written.
    pub ret: TyId,
}

impl GenericSignature {
    /// Creates a generic signature.
    pub fn new(params: Vec<TyId>, ret: TyId) -> Self {
        Self { params, ret }
    }
}

/// A semantic trait declaration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TraitDef {
    /// The trait's definition.
    pub def: DefId,
    /// The trait's source name.
    pub name: Symbol,
    /// The implicit `Self` plus the trait's own parameters.
    pub generics: Generics,
    /// Traits that every implementor must also implement.
    pub supertraits: Vec<TraitRef>,
    /// Required methods in declaration order.
    pub methods: Vec<TraitMethodDef>,
}

impl TraitDef {
    /// Creates a trait declaration.
    pub fn new(
        def: DefId,
        name: Symbol,
        generics: Generics,
        supertraits: Vec<TraitRef>,
        methods: Vec<TraitMethodDef>,
    ) -> Self {
        Self {
            def,
            name,
            generics,
            supertraits,
            methods,
        }
    }

    /// Returns the method with the given name, if the trait declares it.
    pub fn method(&self, name: Symbol) -> Option<&TraitMethodDef> {
        self.methods.iter().find(|method| method.name == name)
    }
}

/// One method declared by a trait.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TraitMethodDef {
    /// The method's definition.
    pub def: DefId,
    /// The method's source name.
    pub name: Symbol,
    /// The declared signature, with `Self` left as a parameter.
    pub signature: GenericSignature,
    /// The method's own generic parameters, with the trait as parent.
    pub generics: Generics,
    /// How the method takes its receiver, or `None` for an associated function.
    pub receiver: Option<TraitMethodReceiver>,
}

/// The receiver declared by a trait method.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TraitMethodReceiver {
    /// The reference kind used around `Self`, or `None` for a by-value receiver.
    pub reference: Option<RefKind>,
}

impl TraitMethodDef {
    /// Creates a trait method declaration.
    pub fn new(
        def: DefId,
        name: Symbol,
        signature: GenericSignature,
        generics: Generics,
        receiver: Option<TraitMethodReceiver>,
    ) -> Self {
        Self {
            def,
            name,
            signature,
            generics,
            receiver,
        }
    }
}

/// One concrete implementation of a trait.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TraitImplDef {
    /// The impl block's definition.
    pub def: DefId,
    /// The trait and self type this impl provides.
    pub trait_ref: TraitRef,
    /// The impl's own generic parameters.
    pub generics: Generics,
    /// Method names bound to their implementation functions.
    pub methods: Vec<(Symbol, DefId)>,
}

impl TraitImplDef {
    /// Creates a trait implementation record.
    pub fn new(
        def: DefId,
        trait_ref: TraitRef,
        generics: Generics,
        methods: Vec<(Symbol, DefId)>,
    ) -> Self {
        Self {
            def,
            trait_ref,
            generics,
            methods,
        }
    }

    /// Returns the implementation function for `name`, if present.
    pub fn method(&self, name: Symbol) -> Option<DefId> {
        self.methods
            .iter()
            .find_map(|(candidate, def)| (*candidate == name).then_some(*def))
    }
}

/// A complete monomorphization instance: a definition plus all its type args.
///
/// `args` are flattened outermost first. For a method inside an impl, they
/// include the impl's parameters followed by the method's own. This is the key
/// used to deduplicate instances, never an index into a hash map.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Instance {
    /// The function or method being instantiated.
    pub def: DefId,
    /// The complete ordered type arguments.
    pub args: SubstsId,
}

impl Instance {
    /// Creates an instance key.
    pub fn new(def: DefId, args: SubstsId) -> Self {
        Self { def, args }
    }
}

/// The bounds visible while checking one generic body.
///
/// Entries are kept in insertion order so diagnostics and solver iteration are
/// deterministic. A parameter may have several bounds; [`ParamEnv::has_copy`]
/// looks only for the built-in `Copy` bound.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ParamEnv {
    entries: Vec<(ParamTyId, Vec<Bound>)>,
}

impl ParamEnv {
    /// Creates an empty environment.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds bounds for `param`, preserving previously recorded bounds.
    pub fn insert(&mut self, param: ParamTyId, bounds: Vec<Bound>) {
        if let Some((_, existing)) = self
            .entries
            .iter_mut()
            .find(|(candidate, _)| *candidate == param)
        {
            existing.extend(bounds);
        } else {
            self.entries.push((param, bounds));
        }
    }

    /// Returns the bounds recorded for `param`, or an empty slice.
    pub fn bounds(&self, param: ParamTyId) -> &[Bound] {
        self.entries
            .iter()
            .find_map(|(candidate, bounds)| (*candidate == param).then_some(bounds.as_slice()))
            .unwrap_or(&[])
    }

    /// Returns `true` if `param` carries the built-in `Copy` bound.
    pub fn has_copy(&self, param: ParamTyId) -> bool {
        self.bounds(param)
            .iter()
            .any(|bound| matches!(bound, Bound::Copy))
    }

    /// Returns the number of parameters with recorded bounds.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns `true` if no parameter has bounds.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}
