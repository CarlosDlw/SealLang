//! The compiler-provided function traits.
//!
//! `Fn`, `FnMut`, and `FnOnce` describe the callable protocol of closures and
//! function pointers. They are installed before user items, so a bound such as
//! `F: Fn(i32) -> i32` resolves without a prelude and the ordinary trait solver
//! sees the real supertrait chain.
//!
//! The traits cannot be declared or implemented by users: their only
//! implementations are the closures the compiler types and safe function
//! pointers. That is why the call methods are declarations without bodies and
//! no synthetic implementation is ever created.

use seal_def::DefId;
use seal_symbol::Interner;
use seal_ty::{ClosureKind, RefKind, TraitMethodReceiver, TraitRef, TyId};

use super::{build_method, create_trait, install_trait_definition, param_ty};
use crate::Hir;

/// The stable definition IDs of the compiler-provided function traits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FnTraits {
    /// The `Fn` trait declaration.
    pub(crate) fn_def: DefId,
    /// The `FnMut` trait declaration.
    pub(crate) fn_mut_def: DefId,
    /// The `FnOnce` trait declaration.
    pub(crate) fn_once_def: DefId,
    /// The `Fn::call` method declaration.
    pub(crate) call: DefId,
    /// The `FnMut::call_mut` method declaration.
    pub(crate) call_mut: DefId,
    /// The `FnOnce::call_once` method declaration.
    pub(crate) call_once: DefId,
}

impl FnTraits {
    /// Returns the trait declaration for `kind`.
    pub(crate) fn trait_def(self, kind: ClosureKind) -> DefId {
        match kind {
            ClosureKind::Fn => self.fn_def,
            ClosureKind::FnMut => self.fn_mut_def,
            ClosureKind::FnOnce => self.fn_once_def,
        }
    }

    /// Returns the kind named by `trait_def`, if it is one of these traits.
    pub(crate) fn kind(self, trait_def: DefId) -> Option<ClosureKind> {
        ClosureKind::ALL
            .into_iter()
            .find(|kind| self.trait_def(*kind) == trait_def)
    }

    /// Returns the call method declared by `kind`.
    pub(crate) fn method(self, kind: ClosureKind) -> DefId {
        match kind {
            ClosureKind::Fn => self.call,
            ClosureKind::FnMut => self.call_mut,
            ClosureKind::FnOnce => self.call_once,
        }
    }
}

/// Installs the three function traits into an empty HIR.
///
/// `FnOnce` is installed first because each stronger trait names the weaker one
/// as a supertrait: `Fn: FnMut` and `FnMut: FnOnce`.
pub(crate) fn install(hir: &mut Hir, interner: &Interner) -> FnTraits {
    let fn_once = install_fn_once(hir, interner);
    let fn_mut = install_fn_mut(hir, interner, fn_once.trait_def);
    let fn_ = install_fn(hir, interner, fn_mut.trait_def);
    FnTraits {
        fn_def: fn_.trait_def,
        fn_mut_def: fn_mut.trait_def,
        fn_once_def: fn_once.trait_def,
        call: fn_.call,
        call_mut: fn_mut.call_mut,
        call_once: fn_once.call_once,
    }
}

struct FnOnceDefs {
    trait_def: DefId,
    call_once: DefId,
}

struct FnMutDefs {
    trait_def: DefId,
    call_mut: DefId,
}

struct FnDefs {
    trait_def: DefId,
    call: DefId,
}

fn install_fn_once(hir: &mut Hir, interner: &Interner) -> FnOnceDefs {
    let (trait_def, generics) = create_trait(hir, interner, "FnOnce", &["Args", "Output"]);
    let (call_once, definition) = build_method(
        hir,
        interner,
        trait_def,
        "call_once",
        TraitMethodReceiver { reference: None },
        vec![param_ty(hir, trait_def, 1)],
        param_ty(hir, trait_def, 2),
    );
    install_trait_definition(
        hir,
        interner,
        "FnOnce",
        trait_def,
        generics,
        Vec::new(),
        vec![definition],
    );
    FnOnceDefs {
        trait_def,
        call_once,
    }
}

fn install_fn_mut(hir: &mut Hir, interner: &Interner, fn_once_def: DefId) -> FnMutDefs {
    let (trait_def, generics) = create_trait(hir, interner, "FnMut", &["Args", "Output"]);
    let (call_mut, definition) = build_method(
        hir,
        interner,
        trait_def,
        "call_mut",
        TraitMethodReceiver {
            reference: Some(RefKind::Exclusive),
        },
        vec![param_ty(hir, trait_def, 1)],
        param_ty(hir, trait_def, 2),
    );
    let supertrait = instantiated(
        hir,
        fn_once_def,
        param_ty(hir, trait_def, 0),
        param_ty(hir, trait_def, 1),
        param_ty(hir, trait_def, 2),
    );
    install_trait_definition(
        hir,
        interner,
        "FnMut",
        trait_def,
        generics,
        vec![supertrait],
        vec![definition],
    );
    FnMutDefs {
        trait_def,
        call_mut,
    }
}

fn install_fn(hir: &mut Hir, interner: &Interner, fn_mut_def: DefId) -> FnDefs {
    let (trait_def, generics) = create_trait(hir, interner, "Fn", &["Args", "Output"]);
    let (call, definition) = build_method(
        hir,
        interner,
        trait_def,
        "call",
        TraitMethodReceiver {
            reference: Some(RefKind::Shared),
        },
        vec![param_ty(hir, trait_def, 1)],
        param_ty(hir, trait_def, 2),
    );
    let supertrait = instantiated(
        hir,
        fn_mut_def,
        param_ty(hir, trait_def, 0),
        param_ty(hir, trait_def, 1),
        param_ty(hir, trait_def, 2),
    );
    install_trait_definition(
        hir,
        interner,
        "Fn",
        trait_def,
        generics,
        vec![supertrait],
        vec![definition],
    );
    FnDefs { trait_def, call }
}

/// Builds `Trait<Args, Output>` for the declaring trait's own parameters.
fn instantiated(hir: &Hir, trait_def: DefId, self_ty: TyId, args: TyId, output: TyId) -> TraitRef {
    let args = hir.ty_ctxt().substs(&[args, output]);
    TraitRef::new(trait_def, self_ty, args)
}
