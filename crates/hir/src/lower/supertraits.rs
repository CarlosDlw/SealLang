//! Supertrait validation.
//!
//! Supertraits form a requirement graph: `T: Sub` implies every trait `Sub`
//! requires. A cycle in that graph can never be satisfied by any type, and the
//! solver would keep proving the same goal forever, so lowering rejects it
//! before type checking. Each cycle is reported once, at the trait that closes
//! it.

use seal_ast::ItemKind;
use seal_def::DefId;
use seal_error_codes as codes;
use seal_hash::FxHashSet;

use super::Lowering;

impl Lowering<'_> {
    /// Rejects cycles in the supertrait graph and `Copy` as a supertrait.
    pub(super) fn validate_supertraits(&mut self) {
        self.reject_copy_supertraits();
        let roots: Vec<DefId> = self
            .trait_items
            .iter()
            .map(|(_, definition)| *definition)
            .collect();
        let mut finished = FxHashSet::default();
        let mut reported = FxHashSet::default();
        for root in roots {
            let mut path = Vec::new();
            self.walk_supertraits(root, &mut path, &mut finished, &mut reported);
        }
    }

    fn walk_supertraits(
        &mut self,
        current: DefId,
        path: &mut Vec<DefId>,
        finished: &mut FxHashSet<DefId>,
        reported: &mut FxHashSet<DefId>,
    ) {
        if let Some(position) = path.iter().position(|candidate| *candidate == current) {
            let cycle = path[position..].to_vec();
            if !cycle.iter().any(|member| reported.contains(member)) {
                self.report_supertrait_cycle(&cycle);
                reported.extend(cycle);
            }
            return;
        }
        if finished.contains(&current) {
            return;
        }
        path.push(current);
        let supertraits = self
            .hir
            .trait_definition(current)
            .map(|declaration| declaration.supertraits)
            .unwrap_or_default();
        for supertrait in supertraits {
            self.walk_supertraits(supertrait.trait_def, path, finished, reported);
        }
        path.pop();
        finished.insert(current);
    }

    /// Rejects `trait X: Copy` and removes the requirement from the semantic
    /// declaration, so a bound on `X` never silently implies `Copy`.
    fn reject_copy_supertraits(&mut self) {
        let copy = self.hir.copy_trait();
        let traits = self.trait_items.clone();
        for (item_id, trait_def) in traits {
            let supertraits = match &self.ast.item(item_id).kind {
                ItemKind::Trait(declaration) => declaration.supertraits.clone(),
                _ => continue,
            };
            let mut reported = false;
            for path in &supertraits {
                if self.traits.get(&path.name).copied() != Some(copy) {
                    continue;
                }
                self.error(
                    codes::E3078,
                    path.span,
                    "the built-in `Copy` marker cannot be a supertrait",
                )
                .help("remove `Copy` from the supertrait list; use it as a generic bound instead")
                .emit();
                reported = true;
            }

            let Some(mut definition) = self.hir.trait_definition(trait_def) else {
                continue;
            };
            let before = definition.supertraits.len();
            definition
                .supertraits
                .retain(|supertrait| supertrait.trait_def != copy);
            if definition.supertraits.len() == before {
                continue;
            }
            if !reported {
                let name = self.hir.definitions().name(trait_def);
                let span = self.hir.definitions().get(trait_def).name_span;
                self.error(
                    codes::E3078,
                    span,
                    format!(
                        "the trait `{}` cannot require `Copy` as a supertrait",
                        self.interner.resolve(name)
                    ),
                )
                .help("remove `Copy` from the supertrait list; use it as a generic bound instead")
                .emit();
            }
            self.hir.ty_ctxt_mut().insert_trait_def(definition);
        }
    }

    fn report_supertrait_cycle(&mut self, cycle: &[DefId]) {
        let names: Vec<String> = cycle
            .iter()
            .map(|definition| {
                self.interner
                    .resolve(self.hir.definitions().name(*definition))
                    .to_owned()
            })
            .collect();
        let chain = format!("{} -> {}", names.join(" -> "), names[0]);
        let head_span = self.hir.definitions().get(cycle[0]).name_span;
        let mut error = self.error(
            codes::E2064,
            head_span,
            format!(
                "the supertrait relation for `{}` is cyclic: `{chain}`",
                names[0]
            ),
        );
        for definition in &cycle[1..] {
            let name_span = self.hir.definitions().get(*definition).name_span;
            error = error.secondary(name_span, "part of the same cycle");
        }
        error
            .help("remove one of the supertrait requirements that closes the cycle")
            .emit();
    }
}
