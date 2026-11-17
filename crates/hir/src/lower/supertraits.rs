//! Supertrait validation.
//!
//! Supertraits form a requirement graph: `T: Sub` implies every trait `Sub`
//! requires. A cycle in that graph can never be satisfied by any type, and the
//! solver would keep proving the same goal forever, so lowering rejects it
//! before type checking. Each cycle is reported once, at the trait that closes
//! it.

use seal_def::DefId;
use seal_error_codes as codes;
use seal_hash::FxHashSet;

use super::Lowering;

impl Lowering<'_> {
    /// Rejects cycles in the supertrait graph.
    pub(super) fn validate_supertraits(&mut self) {
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
