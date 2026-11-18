use seal_def::{DefId, DefKind};
use seal_hash::{FxHashMap, FxHashSet};
use seal_hir::Hir;

pub(super) fn trait_candidates(hir: &Hir) -> FxHashMap<DefId, Vec<DefId>> {
    let impls = hir.trait_impls();
    let mut table = FxHashMap::default();
    for (def, data) in hir.definitions().iter_enumerated() {
        if data.kind != DefKind::Trait {
            continue;
        }
        let Some(trait_def) = hir.trait_definition(def) else {
            continue;
        };
        for method in &trait_def.methods {
            let mut candidates = Vec::new();
            for implementation in &impls {
                if implementation.trait_ref.trait_def != def {
                    continue;
                }
                if let Some(function) = implementation.method(method.name)
                    && !candidates.contains(&function)
                {
                    candidates.push(function);
                }
            }
            if method.has_body && !candidates.contains(&method.def) {
                candidates.push(method.def);
            }
            candidates.sort_by_key(|candidate| candidate.index());
            table.insert(method.def, candidates);
        }
    }
    table
}

/// Builds the candidates of every method reachable through a vtable.
pub(super) fn dynamic_candidates(hir: &Hir) -> FxHashMap<DefId, Vec<DefId>> {
    let impls = hir.trait_impls();
    let mut table = FxHashMap::default();
    for (def, data) in hir.definitions().iter_enumerated() {
        if data.kind != DefKind::Trait {
            continue;
        }
        let Some(trait_def) = hir.trait_definition(def) else {
            continue;
        };
        let chain = trait_chain(hir, def);
        for method in &trait_def.methods {
            let mut candidates = Vec::new();
            for implementation in &impls {
                if !chain.contains(&implementation.trait_ref.trait_def) {
                    continue;
                }
                if let Some(function) = implementation.method(method.name)
                    && !candidates.contains(&function)
                {
                    candidates.push(function);
                }
            }
            if method.has_body && !candidates.contains(&method.def) {
                candidates.push(method.def);
            }
            candidates.sort_by_key(|candidate| candidate.index());
            table.insert(method.def, candidates);
        }
    }
    table
}

/// Returns `trait_def` and its transitive supertraits.
fn trait_chain(hir: &Hir, trait_def: DefId) -> FxHashSet<DefId> {
    let mut chain = FxHashSet::default();
    let mut stack = vec![trait_def];
    while let Some(def) = stack.pop() {
        if !chain.insert(def) {
            continue;
        }
        if let Some(definition) = hir.trait_definition(def) {
            for supertrait in &definition.supertraits {
                stack.push(supertrait.trait_def);
            }
        }
    }
    chain
}
