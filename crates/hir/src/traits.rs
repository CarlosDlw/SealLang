//! Trait implementations and static method lookup.
//!
//! Seal resolves trait calls statically: an `impl Trait for Type` block binds
//! one concrete function per trait method, and a call through a value of that
//! type lowers to a direct call. There is no vtable, and a type may implement
//! several traits as long as no call becomes ambiguous.

use seal_def::DefId;
use seal_hash::FxHashMap;
use seal_symbol::Symbol;

/// The outcome of looking up a trait method for one nominal type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TraitMethodLookup {
    /// No trait implemented by the type declares the name.
    Missing,
    /// Exactly one trait implemented by the type declares the name.
    Found {
        /// The trait declaration that requires the method.
        trait_def: DefId,
        /// The function that implements the method for the type.
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
    trait_def: DefId,
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
        trait_def: DefId,
        type_def: DefId,
        functions: FxHashMap<Symbol, DefId>,
    ) {
        self.entries.push(TraitImpl {
            trait_def,
            type_def,
            functions,
        });
    }

    /// Returns the function implementing `name` for `type_def`.
    pub(crate) fn lookup(&self, type_def: DefId, name: Symbol) -> TraitMethodLookup {
        let mut matches = self.entries.iter().filter_map(|entry| {
            if entry.type_def != type_def {
                return None;
            }
            entry
                .functions
                .get(&name)
                .map(|function| (entry.trait_def, *function))
        });
        let Some((trait_def, function)) = matches.next() else {
            return TraitMethodLookup::Missing;
        };
        let mut traits = vec![trait_def];
        traits.extend(matches.map(|(trait_def, _)| trait_def));
        if traits.len() == 1 {
            TraitMethodLookup::Found {
                trait_def,
                function,
            }
        } else {
            TraitMethodLookup::Ambiguous(traits)
        }
    }
}
