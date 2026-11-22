//! Re-export collection and resolution.
//!
//! `public use` and `internal use` are aliases, not definitions. Resolution is
//! a small fixed point because one re-export may name another. The graph stores
//! only canonical targets, so import and path resolution has one identity per
//! item or module.

use seal_ast::{ItemKind, UseRoot};
use seal_def::{ModuleId, PackageId, Visibility};
use seal_diagnostics::DiagCtxt;
use seal_error_codes as codes;
use seal_hash::FxHashSet;
use seal_span::{FileId, Span};
use seal_symbol::{Interner, Symbol};

use super::imports::{Leaf, expand};
use crate::module::{ExportBinding, ExportTarget, ModuleGraph, visibility_from};

/// One leaf of one re-export declaration.
#[derive(Clone)]
struct Pending {
    package: PackageId,
    module: ModuleId,
    file: FileId,
    visibility: Visibility,
    leaf: Leaf,
    span: Span,
}

/// What can stop a re-export target from resolving.
enum Failure {
    MissingModule { span: Span, name: Symbol },
    MissingItem { span: Span, name: Symbol },
    Ambiguous { span: Span, name: Symbol },
    Cycle { span: Span, name: Symbol },
    Inaccessible { span: Span, name: Symbol },
}

/// Resolves every re-export before imports are resolved.
pub(crate) fn resolve_reexports(
    graph: &mut ModuleGraph,
    asts: &[(FileId, &seal_ast::Ast)],
    dcx: &DiagCtxt,
    interner: &Interner,
) {
    let pending = collect(graph, asts, interner);
    let declared: FxHashSet<(ModuleId, Symbol)> = pending
        .iter()
        .filter_map(|entry| local_name(&entry.leaf).map(|(name, _)| (entry.module, name)))
        .collect();

    let mut remaining = pending;
    loop {
        let mut next = Vec::new();
        let mut progress = false;
        for entry in remaining {
            match resolve_target(graph, &entry, &declared) {
                Ok(target) => {
                    if insert(graph, &entry, target, dcx, interner) {
                        progress = true;
                    }
                }
                Err(_) => next.push(entry),
            }
        }
        remaining = next;
        if !progress {
            break;
        }
    }

    for entry in remaining {
        match resolve_target(graph, &entry, &declared) {
            Ok(target) => {
                let _ = insert(graph, &entry, target, dcx, interner);
            }
            Err(Failure::Cycle { span, name }) => {
                dcx.error(format!(
                    "the re-export of `{}` is part of a cycle",
                    interner.resolve(name)
                ))
                .code(codes::E2089.diag_code())
                .primary(span, "this re-export never reaches an item or module")
                .help("remove one edge of the cycle so every re-export has a concrete target")
                .emit();
            }
            Err(Failure::Inaccessible { span, name }) => {
                dcx.error(format!(
                    "the re-export path through `{}` is not visible here",
                    interner.resolve(name)
                ))
                .code(codes::E2093.diag_code())
                .primary(span, "this re-export is internal to its package")
                .help("re-export the canonical item directly, or use a public re-export")
                .emit();
            }
            Err(failure) => report_failure(dcx, interner, failure),
        }
    }
}

fn collect(
    graph: &ModuleGraph,
    asts: &[(FileId, &seal_ast::Ast)],
    interner: &Interner,
) -> Vec<Pending> {
    let mut pending = Vec::new();
    for (file, ast) in asts {
        let Some(&package) = graph.file_package.get(file) else {
            continue;
        };
        let Some(&module) = graph.file_module.get(file) else {
            continue;
        };
        let cfg = &graph.packages[package].cfg;
        if !crate::cfg::module_enabled(ast, cfg, interner) {
            continue;
        }
        for &item_id in ast.program().items() {
            let item = ast.item(item_id);
            let ItemKind::Use(declaration) = &item.kind else {
                continue;
            };
            let Some(visibility) = declaration.visibility else {
                continue;
            };
            if item
                .modifiers
                .cfg
                .is_some_and(|condition| !cfg.evaluate(ast, condition, interner))
            {
                continue;
            }
            let visibility = visibility_from(visibility);
            for leaf in expand(&declaration.path) {
                pending.push(Pending {
                    package,
                    module,
                    file: *file,
                    visibility,
                    span: leaf.span,
                    leaf,
                });
            }
        }
    }
    pending
}

/// Resolves one re-export leaf to its canonical target.
fn resolve_target(
    graph: &ModuleGraph,
    entry: &Pending,
    declared: &FxHashSet<(ModuleId, Symbol)>,
) -> Result<ExportTarget, Failure> {
    let mut current = resolve_root(graph, entry.package, &entry.leaf)?;

    let (last, prefix) = match entry.leaf.names.split_last() {
        Some((last, prefix)) => (Some(*last), prefix),
        None => (None, &[][..]),
    };

    for (name, span) in prefix {
        current = child_module(graph, entry.package, current, *name, *span, declared)?;
    }

    let Some((name, name_span)) = last else {
        return Ok(ExportTarget::Module { module: current });
    };

    let child = graph.module_lookup(current, name);
    let item = graph.item_lookup(current, name);
    match (child, item) {
        (Some(_), Some(_)) => Err(Failure::Ambiguous {
            span: name_span,
            name,
        }),
        (Some(child), None) => {
            accessible_export(graph, entry.package, child.export, name_span, name)?;
            Ok(ExportTarget::Module {
                module: child.module,
            })
        }
        (None, Some(item)) => {
            accessible_export(graph, entry.package, item.export, name_span, name)?;
            Ok(ExportTarget::Item {
                module: item.module,
                name: item.name,
                namespace: item.info.namespace,
            })
        }
        (None, None) if declared.contains(&(current, name)) => Err(Failure::Cycle {
            span: name_span,
            name,
        }),
        (None, None) => Err(Failure::MissingItem {
            span: name_span,
            name,
        }),
    }
}

fn child_module(
    graph: &ModuleGraph,
    package: PackageId,
    parent: ModuleId,
    name: Symbol,
    span: Span,
    declared: &FxHashSet<(ModuleId, Symbol)>,
) -> Result<ModuleId, Failure> {
    match graph.module_lookup(parent, name) {
        Some(lookup) => {
            accessible_export(graph, package, lookup.export, span, name)?;
            Ok(lookup.module)
        }
        None if declared.contains(&(parent, name)) => Err(Failure::Cycle { span, name }),
        None => Err(Failure::MissingModule { span, name }),
    }
}

fn accessible_export(
    graph: &ModuleGraph,
    package: PackageId,
    export: Option<ExportBinding>,
    span: Span,
    name: Symbol,
) -> Result<(), Failure> {
    let Some(export) = export else {
        return Ok(());
    };
    let owner = export.module;
    let owner_package = graph.modules[owner].package;
    if owner_package != package
        && (export.visibility == Visibility::Internal || !graph.modules[owner].exposed)
    {
        return Err(Failure::Inaccessible { span, name });
    }
    Ok(())
}

fn resolve_root(graph: &ModuleGraph, package: PackageId, leaf: &Leaf) -> Result<ModuleId, Failure> {
    let root = graph.packages[package].root;
    match leaf.root {
        UseRoot::Root => Ok(root),
        UseRoot::Name(name) => {
            if let Some(lookup) = graph.module_lookup(root, name) {
                accessible_export(graph, package, lookup.export, leaf.root_span, name)?;
                return Ok(lookup.module);
            }
            graph.packages[package]
                .dependencies
                .iter()
                .find(|dependency| dependency.alias == name)
                .map(|dependency| graph.packages[dependency.package].root)
                .ok_or(Failure::MissingModule {
                    span: leaf.root_span,
                    name,
                })
        }
    }
}

/// Inserts one validated re-export. Returns whether it was inserted.
fn insert(
    graph: &mut ModuleGraph,
    entry: &Pending,
    target: ExportTarget,
    dcx: &DiagCtxt,
    interner: &Interner,
) -> bool {
    let Some((name, name_span)) = local_name(&entry.leaf) else {
        dcx.error("a re-export needs a name")
            .code(codes::E2091.diag_code())
            .primary(entry.span, "this path does not name an item or module")
            .help("write `public use path::to::name;`")
            .emit();
        return false;
    };

    if graph.exports.contains_key(&(entry.module, name)) {
        dcx.error(format!(
            "the re-export name `{}` is declared twice",
            interner.resolve(name)
        ))
        .code(codes::E2090.diag_code())
        .primary(name_span, "this name is already re-exported")
        .help("rename one of the re-exports with `as`")
        .emit();
        return false;
    }
    if graph.items.contains_key(&(entry.module, name)) {
        dcx.error(format!(
            "the re-export name `{}` conflicts with an item of this module",
            interner.resolve(name)
        ))
        .code(codes::E2094.diag_code())
        .primary(name_span, "this name is already declared")
        .help("rename the re-export with `as`")
        .emit();
        return false;
    }
    if graph.children.contains_key(&(entry.module, name)) {
        dcx.error(format!(
            "the re-export name `{}` conflicts with a submodule",
            interner.resolve(name)
        ))
        .code(codes::E2094.diag_code())
        .primary(name_span, "this name is already declared")
        .help("rename the re-export with `as`")
        .emit();
        return false;
    }

    let target_module = match target {
        ExportTarget::Module { module } => module,
        ExportTarget::Item { module, .. } => module,
    };
    let target_module_data = &graph.modules[target_module];
    let target_package = target_module_data.package;
    let target_item = match target {
        ExportTarget::Item { module, name, .. } => graph.items.get(&(module, name)).copied(),
        ExportTarget::Module { .. } => None,
    };

    if entry.visibility == Visibility::Public {
        if !graph.modules[entry.module].exposed {
            dcx.error(format!(
                "the re-export `{}` is public, but its module is not exposed",
                interner.resolve(name)
            ))
            .code(codes::E2088.diag_code())
            .primary(entry.span, "this module is not part of the package API")
            .help("expose the declaring module in `[package.api]`, or use `internal use`")
            .emit();
            return false;
        }
        if !graph.modules[target_module].exposed {
            dcx.error(format!(
                "the re-export `{}` reaches a module that is not exposed",
                interner.resolve(name)
            ))
            .code(codes::E2093.diag_code())
            .primary(
                target_module_data.declaration_span,
                "the target module is here",
            )
            .help("expose the target module, or re-export an item from an exposed module")
            .emit();
            return false;
        }
        if target_item.is_some_and(|info| info.visibility != Visibility::Public) {
            let info = target_item.expect("the item was just checked");
            dcx.error(format!(
                "the re-export `{}` targets an item that is not public",
                interner.resolve(name)
            ))
            .code(codes::E2092.diag_code())
            .primary(info.span, "the target is not public")
            .help("make the target public, or use `internal use` inside the package")
            .emit();
            return false;
        }
    } else {
        if target_package != entry.package && !graph.modules[target_module].exposed {
            dcx.error(format!(
                "the re-export `{}` reaches a module that is not exposed",
                interner.resolve(name)
            ))
            .code(codes::E2093.diag_code())
            .primary(
                target_module_data.declaration_span,
                "the target module is here",
            )
            .help("expose the target module in its package")
            .emit();
            return false;
        }
        if target_package != entry.package
            && target_item.is_some_and(|info| info.visibility != Visibility::Public)
        {
            let info = target_item.expect("the item was just checked");
            dcx.error(format!(
                "the re-export `{}` targets an item that is not public",
                interner.resolve(name)
            ))
            .code(codes::E2092.diag_code())
            .primary(info.span, "the target is not public outside its package")
            .help("use a public item from an exposed module")
            .emit();
            return false;
        }
        if target_item.is_some_and(|info| info.visibility == Visibility::Private) {
            let info = target_item.expect("the item was just checked");
            dcx.error(format!(
                "the re-export `{}` targets a private item",
                interner.resolve(name)
            ))
            .code(codes::E2092.diag_code())
            .primary(info.span, "the target is private")
            .help("make the target `internal` or `public`")
            .emit();
            return false;
        }
    }

    graph.insert_export(
        entry.module,
        name,
        ExportBinding {
            target,
            visibility: entry.visibility,
            span: entry.span,
            source_file: entry.file,
            module: entry.module,
        },
    );
    true
}

fn local_name(leaf: &Leaf) -> Option<(Symbol, Span)> {
    if let Some((name, span)) = leaf.alias {
        return Some((name, span));
    }
    if let Some((name, span)) = leaf.names.last() {
        return Some((*name, *span));
    }
    leaf.root.name().map(|name| (name, leaf.root_span))
}

fn report_failure(dcx: &DiagCtxt, interner: &Interner, failure: Failure) {
    let (span, name, module) = match failure {
        Failure::MissingModule { span, name } => (span, name, true),
        Failure::MissingItem { span, name } => (span, name, false),
        Failure::Ambiguous { span, name } => {
            dcx.error(format!(
                "the name `{}` denotes both a module and an item",
                interner.resolve(name)
            ))
            .code(codes::E2074.diag_code())
            .primary(span, "this name is ambiguous")
            .help("re-export the module and the item with different aliases")
            .emit();
            return;
        }
        Failure::Cycle { span, name } => {
            dcx.error(format!(
                "the re-export of `{}` is part of a cycle",
                interner.resolve(name)
            ))
            .code(codes::E2089.diag_code())
            .primary(span, "this re-export never reaches an item or module")
            .help("remove one edge of the cycle so every re-export has a concrete target")
            .emit();
            return;
        }
        Failure::Inaccessible { span, name } => {
            dcx.error(format!(
                "the re-export path through `{}` is not visible here",
                interner.resolve(name)
            ))
            .code(codes::E2093.diag_code())
            .primary(span, "this re-export is internal to its package")
            .help("re-export the canonical item directly, or use a public re-export")
            .emit();
            return;
        }
    };
    dcx.error(format!(
        "cannot find `{}` in the re-export path",
        interner.resolve(name)
    ))
    .code(codes::E2073.diag_code())
    .primary(
        span,
        if module {
            "this module does not resolve"
        } else {
            "this name does not resolve"
        },
    )
    .help("check the path, or declare the target before re-exporting it")
    .emit();
}
