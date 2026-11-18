//! Trait implementations and method lookup.
//!
//! An `impl Trait for Type` block binds one concrete function per trait method,
//! so a call through a value of that type lowers to a direct call. Object-safe
//! traits can also be used behind `dyn`, where dispatch is dynamic. A type may
//! implement several traits as long as no static call or trait-object method
//! becomes ambiguous.

use seal_def::DefId;
use seal_hash::FxHashMap;
use seal_symbol::Symbol;
use seal_ty::TraitRef;

/// The outcome of looking up a trait method for one nominal type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TraitMethodLookup {
    /// No trait implemented by the type declares the name.
    Missing,
    /// Exactly one trait implemented by the type declares the name.
    Found {
        /// The trait declaration that requires the method.
        trait_def: DefId,
        /// The implemented trait reference, in template form.
        trait_ref: TraitRef,
        /// The function that implements the method for the type.
        function: DefId,
    },
    /// The implementation omits the name and the trait supplies a default body.
    Default {
        /// The implemented trait reference, in template form.
        trait_ref: TraitRef,
        /// The trait function declaration carrying the default body.
        function: DefId,
    },
    /// Several implemented traits declare the name, so the call is ambiguous.
    ///
    /// The trait definitions are listed in implementation order so the caller
    /// can report the same set every run.
    Ambiguous(Vec<DefId>),
}

/// Every trait implementation in one file, in declaration order.
#[derive(Debug, Default)]
pub(crate) struct TraitImpls {
    entries: Vec<TraitImpl>,
}

/// One `impl Trait for Type` block.
#[derive(Debug)]
struct TraitImpl {
    trait_ref: TraitRef,
    type_def: DefId,
    functions: FxHashMap<Symbol, DefId>,
}

impl TraitImpls {
    /// Records an implementation.
    ///
    /// The caller rules out a duplicate first. Several impls may exist for one
    /// nominal type with disjoint arguments, so entries are kept in order and
    /// every one is consulted by [`TraitImpls::lookup`].
    pub(crate) fn insert(
        &mut self,
        trait_ref: TraitRef,
        type_def: DefId,
        functions: FxHashMap<Symbol, DefId>,
    ) {
        self.entries.push(TraitImpl {
            trait_ref,
            type_def,
            functions,
        });
    }

    /// Returns the function implementing `name` for `type_def`.
    ///
    /// `default` receives an implemented trait reference and returns the trait
    /// declaration carrying a default body for `name`, when one exists. The
    /// callback keeps the registry independent from the trait definition table.
    pub(crate) fn lookup(
        &self,
        type_def: DefId,
        name: Symbol,
        default: impl Fn(TraitRef, Symbol) -> Option<DefId>,
    ) -> TraitMethodLookup {
        let mut matches = self.entries.iter().filter_map(|entry| {
            if entry.type_def != type_def {
                return None;
            }
            if let Some(function) = entry.functions.get(&name).copied() {
                return Some((entry.trait_ref, function, false));
            }
            default(entry.trait_ref, name).map(|function| (entry.trait_ref, function, true))
        });
        let Some((trait_ref, function, is_default)) = matches.next() else {
            return TraitMethodLookup::Missing;
        };
        let mut traits = vec![trait_ref.trait_def];
        traits.extend(matches.map(|(trait_ref, _, _)| trait_ref.trait_def));
        if traits.len() != 1 {
            return TraitMethodLookup::Ambiguous(traits);
        }
        if is_default {
            TraitMethodLookup::Default {
                trait_ref,
                function,
            }
        } else {
            TraitMethodLookup::Found {
                trait_def: trait_ref.trait_def,
                trait_ref,
                function,
            }
        }
    }
}
