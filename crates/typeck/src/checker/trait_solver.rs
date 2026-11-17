//! Trait-bound proof search.
//!
//! The solver works on the semantic types recorded by HIR: a goal is a
//! [`TraitRef`], and candidates are the trait implementations declared in the
//! current file. Matching is structural and deliberately non-inferential: by
//! the time a bound is checked, every inference variable has been resolved, so
//! an unresolved type is a user error (`E3038`), not a solver decision.
//!
//! Proofs are cached per body and keyed by the resolved goal. Recursion is
//! bounded and guarded by an active-goal stack, so a conditional impl that
//! depends on itself yields `E3042` instead of overflowing the stack.

use seal_def::DefId;
use seal_hash::{FxHashMap, FxHashSet};
use seal_hir::Hir;
use seal_ty::{Bound, GenericParamDef, ParamEnv, ParamTyId, TraitRef, TyId, TyKind};

/// The maximum number of nested proof steps for one goal.
pub(super) const MAX_PROOF_DEPTH: usize = 32;

/// The result of proving one trait goal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Proof {
    /// Exactly one implementation satisfies the goal.
    Proven,
    /// No implementation and no bound satisfies the goal.
    Missing,
    /// More than one implementation satisfies the goal.
    Ambiguous,
    /// The proof depends on itself or exceeded the depth limit.
    Recursive,
}

/// Shared state for one top-level proof.
pub(super) struct Solver<'a> {
    pub(super) hir: &'a Hir,
    pub(super) env: &'a ParamEnv,
    pub(super) cache: &'a mut FxHashMap<TraitRef, Proof>,
    pub(super) active: FxHashSet<TraitRef>,
}

impl<'a> Solver<'a> {
    /// Creates a solver over `hir` and the caller's parameter environment.
    pub(super) fn new(
        hir: &'a Hir,
        env: &'a ParamEnv,
        cache: &'a mut FxHashMap<TraitRef, Proof>,
    ) -> Self {
        Self {
            hir,
            env,
            cache,
            active: FxHashSet::default(),
        }
    }

    /// Proves `goal`, using and updating the cache.
    pub(super) fn prove(&mut self, goal: TraitRef, depth: usize) -> Proof {
        if let Some(proof) = self.cache.get(&goal) {
            return *proof;
        }
        if depth >= MAX_PROOF_DEPTH || !self.active.insert(goal) {
            return Proof::Recursive;
        }
        let proof = self.search(goal, depth);
        self.active.remove(&goal);
        self.cache.insert(goal, proof);
        proof
    }

    fn search(&mut self, goal: TraitRef, depth: usize) -> Proof {
        match self.hir.ty_ctxt().kind(goal.self_ty) {
            TyKind::Error => Proof::Proven,
            TyKind::Param(param) => self.prove_from_env(goal, param),
            TyKind::Dyn { .. } => self.prove_from_dyn(goal),
            _ => self.prove_from_impls(goal, depth),
        }
    }

    /// Proves a goal for a rigid parameter from its visible bounds.
    fn prove_from_env(&mut self, goal: TraitRef, param: ParamTyId) -> Proof {
        let mut found = Vec::new();
        for bound in self.env.bounds(param) {
            let Bound::Trait(candidate) = bound else {
                continue;
            };
            self.collect_supertrait_goals(candidate, goal, &mut found);
        }
        // A bound in the environment is itself a proof obligation already
        // assumed for this body, and so is every supertrait it guarantees.
        match found.len() {
            0 => Proof::Missing,
            1 => Proof::Proven,
            _ => Proof::Ambiguous,
        }
    }

    /// Adds `candidate` and its supertraits that structurally match `goal`.
    fn collect_supertrait_goals(
        &mut self,
        candidate: &TraitRef,
        goal: TraitRef,
        found: &mut Vec<TraitRef>,
    ) {
        if candidate.trait_def == goal.trait_def && self.trait_refs_equal(*candidate, goal) {
            if !found.contains(candidate) {
                found.push(*candidate);
            }
            return;
        }
        let Some(definition) = self.hir.trait_definition(candidate.trait_def) else {
            return;
        };
        for supertrait in &definition.supertraits {
            let Some(instantiated) = self
                .hir
                .ty_ctxt()
                .instantiate_supertrait(*supertrait, *candidate)
            else {
                continue;
            };
            if instantiated.trait_def == goal.trait_def && self.trait_refs_equal(instantiated, goal)
            {
                if !found.contains(candidate) {
                    found.push(*candidate);
                }
                return;
            }
            self.collect_supertrait_goals(&instantiated, goal, found);
        }
    }

    /// Proves a goal from the principal trait and supertraits of a trait object.
    ///
    /// [`TyCtxt::dyn_trait_refs`](seal_ty::TyCtxt::dyn_trait_refs) already
    /// flattens supertraits depth-first and removes duplicate paths, so a
    /// diamond does not turn one proof into an ambiguity. Distinct arguments of
    /// the same supertrait remain distinct references and only the exact goal
    /// matches.
    fn prove_from_dyn(&self, goal: TraitRef) -> Proof {
        let refs = self.hir.ty_ctxt().dyn_trait_refs(goal.self_ty);
        if refs.iter().any(|candidate| {
            candidate.trait_def == goal.trait_def && self.trait_refs_equal(*candidate, goal)
        }) {
            Proof::Proven
        } else {
            Proof::Missing
        }
    }

    fn trait_refs_equal(&self, left: TraitRef, right: TraitRef) -> bool {
        left.self_ty == right.self_ty && left.args == right.args
    }

    /// Searches declared impls, checking their conditional bounds.
    fn prove_from_impls(&mut self, goal: TraitRef, depth: usize) -> Proof {
        let mut proven = 0usize;
        let mut ambiguous = false;
        for implementation in self
            .hir
            .ty_ctxt()
            .trait_impls_for(goal.trait_def, goal.self_ty)
        {
            let Some(mapping) = self.match_impl(&implementation.trait_ref, goal) else {
                continue;
            };
            match self.check_impl_bounds(implementation.def, &mapping, depth) {
                Proof::Proven => proven += 1,
                Proof::Recursive => return Proof::Recursive,
                Proof::Ambiguous => ambiguous = true,
                Proof::Missing => {}
            }
        }
        if ambiguous || proven > 1 {
            Proof::Ambiguous
        } else if proven == 1 {
            Proof::Proven
        } else {
            Proof::Missing
        }
    }

    /// Matches an impl header against a goal, returning its parameter mapping.
    fn match_impl(&self, header: &TraitRef, goal: TraitRef) -> Option<FxHashMap<ParamTyId, TyId>> {
        let mut mapping = FxHashMap::default();
        if !match_type(self.hir, header.self_ty, goal.self_ty, &mut mapping) {
            return None;
        }
        let header_args = self.hir.ty_ctxt().substs_args(header.args);
        let goal_args = self.hir.ty_ctxt().substs_args(goal.args);
        if header_args.len() != goal_args.len() {
            return None;
        }
        for (pattern, actual) in header_args.into_iter().zip(goal_args) {
            if !match_type(self.hir, pattern, actual, &mut mapping) {
                return None;
            }
        }
        Some(mapping)
    }

    fn check_impl_bounds(
        &mut self,
        impl_def: DefId,
        mapping: &FxHashMap<ParamTyId, TyId>,
        depth: usize,
    ) -> Proof {
        let params = self.hir.flattened_generics(impl_def);
        let mut arguments = Vec::with_capacity(params.len());
        for parameter in &params {
            match mapping.get(&parameter.param).copied() {
                Some(argument) => arguments.push(argument),
                None => return Proof::Missing,
            }
        }
        let substs = self.hir.ty_ctxt().substs(&arguments);
        let mut result = Proof::Proven;
        for parameter in &params {
            let Some(argument) = mapping.get(&parameter.param).copied() else {
                return Proof::Missing;
            };
            match self.check_bounds(parameter, argument, substs, depth) {
                Proof::Proven => {}
                Proof::Recursive => return Proof::Recursive,
                Proof::Ambiguous => result = Proof::Ambiguous,
                Proof::Missing => return Proof::Missing,
            }
        }
        result
    }

    fn check_bounds(
        &mut self,
        parameter: &GenericParamDef,
        argument: TyId,
        substs: seal_ty::SubstsId,
        depth: usize,
    ) -> Proof {
        let mut result = Proof::Proven;
        for bound in &parameter.bounds {
            match bound {
                Bound::Copy => {
                    if !self.hir.ty_ctxt().is_copy_with(argument, self.env) {
                        return Proof::Missing;
                    }
                }
                Bound::Trait(trait_ref) => {
                    let self_ty = self.hir.ty_ctxt().substitute(trait_ref.self_ty, substs);
                    let args: Vec<TyId> = self
                        .hir
                        .ty_ctxt()
                        .substs_args(trait_ref.args)
                        .into_iter()
                        .map(|argument| self.hir.ty_ctxt().substitute(argument, substs))
                        .collect();
                    let goal = TraitRef::new(
                        trait_ref.trait_def,
                        self_ty,
                        self.hir.ty_ctxt().substs(&args),
                    );
                    match self.prove(goal, depth + 1) {
                        Proof::Proven => {}
                        Proof::Recursive => return Proof::Recursive,
                        Proof::Ambiguous => result = Proof::Ambiguous,
                        Proof::Missing => return Proof::Missing,
                    }
                }
            }
        }
        result
    }
}

/// Matches a type template against a concrete type, binding template params.
///
/// The algorithm is shared with HIR impl selection and coherence, so the
/// definition lives in [`seal_ty::match_template`].
pub(super) fn match_type(
    hir: &Hir,
    pattern: TyId,
    actual: TyId,
    mapping: &mut FxHashMap<ParamTyId, TyId>,
) -> bool {
    seal_ty::match_template(hir.ty_ctxt(), pattern, actual, mapping)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use seal_diagnostics::{DiagCtxt, SilentEmitter};
    use seal_hir::Hir;
    use seal_span::{FileName, SourceMap};
    use seal_symbol::Interner;
    use seal_ty::{IntTy, ParamEnv, TraitRef, TyId};

    use super::{Proof, Solver};

    fn lower(source: &str) -> (Hir, Interner) {
        let map = Arc::new(SourceMap::new());
        let id = map
            .add_file(FileName::virtual_name("test.sl"), source.to_owned())
            .expect("test source registers");
        let file = map.file(id).expect("registered file exists");
        let dcx = DiagCtxt::new(Box::new(SilentEmitter::new()), Arc::clone(&map));
        let interner = Interner::new();
        let lexed = seal_lexer::lex(&file, &dcx, &interner);
        let ast = seal_parser::parse(&lexed, &dcx, &interner);
        let hir = Hir::lower(&ast, &dcx, &interner);
        (hir, interner)
    }

    fn object(hir: &Hir, interner: &Interner, name: &str, args: &[TyId]) -> TyId {
        let trait_def = hir
            .trait_def(interner.intern(name))
            .unwrap_or_else(|| panic!("trait `{name}` was not lowered"));
        hir.ty_ctxt().dyn_ty(trait_def, hir.ty_ctxt().substs(args))
    }

    fn goal(hir: &Hir, interner: &Interner, name: &str, object: TyId, args: &[TyId]) -> TraitRef {
        let trait_def = hir
            .trait_def(interner.intern(name))
            .unwrap_or_else(|| panic!("trait `{name}` was not lowered"));
        TraitRef::new(trait_def, object, hir.ty_ctxt().substs(args))
    }

    fn prove(hir: &Hir, goal: TraitRef) -> Proof {
        let env = ParamEnv::new();
        let mut cache = seal_hash::FxHashMap::default();
        Solver::new(hir, &env, &mut cache).prove(goal, 0)
    }

    #[test]
    fn dyn_object_proves_principal_and_super_traits() {
        let (hir, interner) = lower("trait Base {} trait Child: Base {} trait Missing {}");
        let object = object(&hir, &interner, "Child", &[]);

        assert_eq!(
            prove(&hir, goal(&hir, &interner, "Child", object, &[])),
            Proof::Proven
        );
        assert_eq!(
            prove(&hir, goal(&hir, &interner, "Base", object, &[])),
            Proof::Proven
        );
        assert_eq!(
            prove(&hir, goal(&hir, &interner, "Missing", object, &[])),
            Proof::Missing
        );
    }

    #[test]
    fn dyn_diamond_supertraits_are_not_ambiguous() {
        let (hir, interner) = lower(
            "trait Root {} trait Left: Root {} trait Right: Root {} trait Leaf: Left + Right {}",
        );
        let object = object(&hir, &interner, "Leaf", &[]);
        assert_eq!(
            prove(&hir, goal(&hir, &interner, "Root", object, &[])),
            Proof::Proven
        );
    }

    #[test]
    fn dyn_goals_keep_distinct_trait_arguments() {
        let (hir, interner) =
            lower("trait Convert<T> {} trait Both: Convert<i32> + Convert<bool> {}");
        let object = object(&hir, &interner, "Both", &[]);
        let tcx = hir.ty_ctxt();
        let int = tcx.int_ty(IntTy::I32);
        let bool_ty = tcx.bool_ty();
        let byte = tcx.uint_ty(seal_ty::UintTy::U8);

        assert_eq!(
            prove(&hir, goal(&hir, &interner, "Convert", object, &[int])),
            Proof::Proven
        );
        assert_eq!(
            prove(&hir, goal(&hir, &interner, "Convert", object, &[bool_ty])),
            Proof::Proven
        );
        assert_eq!(
            prove(&hir, goal(&hir, &interner, "Convert", object, &[byte])),
            Proof::Missing
        );
    }
}
