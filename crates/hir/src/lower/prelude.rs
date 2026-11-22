//! Prelude resolution.
//!
//! A package may name one module as its prelude in the plan. The public items
//! of that module become available unqualified while the package is lowered.
//! The prelude is data from the plan: no standard-library name is hardcoded in
//! this pass.
//!
//! Resolution runs after re-exports settle, so a prelude may publish an item
//! through a public alias. It runs before plain imports and bodies, which keeps
//! the precedence rule simple: a local scope wins over a module item, a module
//! item wins over an explicit import, and an explicit import wins over the
//! prelude. A prelude binding is not an import and never produces
//! `unused_import`.

use seal_def::{ModuleId, PackageId, Visibility};
use seal_diagnostics::DiagCtxt;
use seal_error_codes as codes;
use seal_hash::{FxHashMap, FxHashSet};
use seal_index::Idx;
use seal_span::Span;
use seal_symbol::Interner;

use crate::module::{
    ExportTarget, ModuleGraph, Namespace, PackagePlan, PreludeBinding, PreludeSource, PreludeTable,
};

/// Resolves every package prelude named by `plan`.
pub(crate) fn resolve_preludes(
    plan: &PackagePlan,
    graph: &ModuleGraph,
    dcx: &DiagCtxt,
    interner: &Interner,
) -> FxHashMap<PackageId, PreludeTable> {
    let mut tables = FxHashMap::default();
    for (index, entry) in plan.packages.iter().enumerate() {
        let Some(source) = &entry.prelude else {
            continue;
        };
        // `core` and `std` are the source of the prelude, not consumers of it.
        // The plan leaves their field absent; ignoring a stray value keeps the
        // bootstrap from recursively preluding the prelude itself.
        if matches!(interner.resolve(entry.name), "core" | "std") {
            continue;
        }
        let package = PackageId::from_usize(index);
        if let Some(first) = cycle_first(plan, index) {
            if first == index {
                dcx.error("the prelude packages form a cycle")
                    .code(codes::E2103.diag_code())
                    .primary(Span::default(), "this prelude is part of the cycle")
                    .help("point each prelude at a package outside the cycle")
                    .emit();
            }
            continue;
        }
        let source_index = source.package.index() as usize;
        if source_index >= plan.packages.len() || source_index >= graph.packages.len() {
            dcx.error(format!(
                "the prelude module `{}` names package {} outside the plan",
                source.module.display(interner),
                source_index
            ))
            .code(codes::E2100.diag_code())
            .primary(Span::default(), "the prelude source is missing")
            .help("point the package prelude at a package in the plan")
            .emit();
            continue;
        }
        let Some(module) = source_module(graph, source) else {
            dcx.error(format!(
                "the prelude module `{}` does not exist in package `{}`",
                source.module.display(interner),
                interner.resolve(plan.packages[source_index].name)
            ))
            .code(codes::E2100.diag_code())
            .primary(Span::default(), "the prelude source is missing")
            .help("declare the module, or point the package prelude at another module")
            .emit();
            continue;
        };
        let mut table = PreludeTable::new(source.clone());
        for (&(item_module, name), info) in &graph.items {
            if item_module != module || info.visibility != Visibility::Public {
                continue;
            }
            add(
                &mut table,
                name,
                info.namespace,
                PreludeBinding {
                    module: item_module,
                    name,
                },
                info.span,
                dcx,
                interner,
            );
        }
        for (&(owner, name), binding) in &graph.exports {
            if owner != module || binding.visibility != Visibility::Public {
                continue;
            }
            match binding.target {
                ExportTarget::Module { .. } => {
                    dcx.error(format!(
                        "the prelude name `{}` is a module",
                        interner.resolve(name)
                    ))
                    .code(codes::E2101.diag_code())
                    .primary(binding.span, "this prelude name is not an item")
                    .help("re-export an item, or reach the module through its name")
                    .emit();
                }
                ExportTarget::Item {
                    module,
                    name: target,
                    namespace,
                } => {
                    add(
                        &mut table,
                        name,
                        namespace,
                        PreludeBinding {
                            module,
                            name: target,
                        },
                        binding.span,
                        dcx,
                        interner,
                    );
                }
            }
        }
        tables.insert(package, table);
    }
    tables
}

/// Adds one prelude binding, diagnosing a same-namespace collision.
fn add(
    table: &mut PreludeTable,
    name: seal_symbol::Symbol,
    namespace: Namespace,
    binding: PreludeBinding,
    span: Span,
    dcx: &DiagCtxt,
    interner: &Interner,
) {
    if table.insert(name, namespace, binding).is_some() {
        // A colliding name must not resolve arbitrarily. Remove the first
        // binding too, so the name is absent until the source is corrected.
        table.remove(name, namespace);
        dcx.error(format!(
            "the prelude binds `{}` twice in the same namespace",
            interner.resolve(name)
        ))
        .code(codes::E2102.diag_code())
        .primary(span, "this name collides with another prelude name")
        .help("rename one re-export with `as`, or remove the duplicate")
        .emit();
    }
}

/// Finds the module named by `source` inside the source package.
fn source_module(graph: &ModuleGraph, source: &PreludeSource) -> Option<ModuleId> {
    let mut current = graph.packages[source.package].root;
    for segment in source.module.segments() {
        let lookup = graph.module_lookup(current, *segment)?;
        if lookup
            .export
            .is_some_and(|export| export.visibility != Visibility::Public)
        {
            return None;
        }
        current = lookup.module;
    }
    Some(current)
}

/// Returns the smallest package index in a prelude cycle reachable from
/// `start`, if any. A self-edge is not a cycle: a package may use its own
/// module as its prelude.
fn cycle_first(plan: &PackagePlan, start: usize) -> Option<usize> {
    let mut seen = FxHashSet::default();
    let mut order = Vec::new();
    let mut current = start;
    loop {
        if current >= plan.packages.len() {
            return None;
        }
        if !seen.insert(current) {
            let position = order.iter().position(|package| *package == current)?;
            return order[position..].iter().copied().min();
        }
        order.push(current);
        let next = plan.packages[current].prelude.as_ref()?;
        let next = next.package.index() as usize;
        if next == current {
            return None;
        }
        current = next;
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use seal_diagnostics::SilentEmitter;
    use seal_index::Idx;
    use seal_span::{FileName, SourceMap, Span};

    use super::*;
    use crate::module::{ModulePath, PreludeTable};

    #[test]
    fn a_same_namespace_collision_removes_both_bindings() {
        let map = SourceMap::new();
        map.add_file(FileName::virtual_name("test.sl"), String::new())
            .expect("the empty source registers");
        let dcx = DiagCtxt::new(Box::new(SilentEmitter::new()), Arc::new(map));
        let interner = Interner::new();
        let name = interner.intern("Option");
        let mut table = PreludeTable::new(PreludeSource {
            package: PackageId::from_usize(0),
            module: ModulePath::root(),
        });
        let binding = PreludeBinding {
            module: ModuleId::from_usize(0),
            name,
        };
        add(
            &mut table,
            name,
            Namespace::Type,
            binding,
            Span::default(),
            &dcx,
            &interner,
        );
        add(
            &mut table,
            name,
            Namespace::Type,
            binding,
            Span::default(),
            &dcx,
            &interner,
        );
        assert!(table.is_empty());
        assert_eq!(dcx.error_count(), 1);
    }
}
