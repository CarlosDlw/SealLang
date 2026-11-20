//! Import expansion and resolution.
//!
//! `use` paths are absolute: they start at the package root (`root`), at a
//! top-level module of the current package, or at a dependency alias. Every
//! import is expanded into one binding per imported name. The bindings live in
//! the file's [`ImportTable`], so two fragments of one module never share the
//! names they imported.

use seal_ast::{ItemKind, UsePath, UseRoot, UseSegment};
use seal_def::{ModuleId, PackageId, Visibility};
use seal_diagnostics::DiagCtxt;
use seal_error_codes as codes;
use seal_span::{FileId, Span};
use seal_symbol::{Interner, Symbol};

use crate::module::{ImportBinding, ImportTable, ImportTarget, ModuleGraph};

/// One fully expanded import.
#[derive(Clone)]
struct Leaf {
    root: UseRoot,
    root_span: Span,
    names: Vec<(Symbol, Span)>,
    alias: Option<(Symbol, Span)>,
    span: Span,
}

/// Resolves every `use` declaration of every file.
pub(crate) fn resolve_imports(
    graph: &ModuleGraph,
    asts: &[(FileId, &seal_ast::Ast)],
    dcx: &DiagCtxt,
    interner: &Interner,
) -> Vec<ImportTable> {
    let mut tables = vec![ImportTable::default(); asts.len()];
    for (index, (file, ast)) in asts.iter().enumerate() {
        let Some(&package) = graph.file_package.get(file) else {
            continue;
        };
        let Some(&module) = graph.file_module.get(file) else {
            continue;
        };
        for &item_id in ast.program().items() {
            let ItemKind::Use(declaration) = &ast.item(item_id).kind else {
                continue;
            };
            resolve_declaration(
                graph,
                &mut tables[index],
                package,
                module,
                &declaration.path,
                dcx,
                interner,
            );
        }
    }
    tables
}

fn resolve_declaration(
    graph: &ModuleGraph,
    table: &mut ImportTable,
    package: PackageId,
    module: ModuleId,
    path: &UsePath,
    dcx: &DiagCtxt,
    interner: &Interner,
) {
    let leaves = expand(path);
    for leaf in leaves {
        resolve_leaf(graph, table, package, module, &leaf, dcx, interner);
    }
}

/// Expands groups into one leaf per imported name.
fn expand(path: &UsePath) -> Vec<Leaf> {
    let mut leaves = vec![Leaf {
        root: path.root,
        root_span: path.root_span,
        names: Vec::new(),
        alias: path
            .alias
            .zip(path.alias_span)
            .or_else(|| path.alias.map(|alias| (alias, path.span))),
        span: path.span,
    }];

    for segment in &path.segments {
        match segment {
            UseSegment::Name { name, span } => {
                for leaf in &mut leaves {
                    leaf.names.push((*name, *span));
                }
            }
            UseSegment::Group { items, .. } => {
                let mut next = Vec::new();
                for leaf in &leaves {
                    for item in items {
                        let mut expanded = leaf.clone();
                        expanded.names.push((item.name, item.name_span));
                        if let Some(alias) = item.alias {
                            expanded.alias = Some((alias, item.alias_span.unwrap_or(item.span)));
                        }
                        next.push(expanded);
                    }
                }
                leaves = next;
            }
        }
    }
    leaves
}

fn resolve_leaf(
    graph: &ModuleGraph,
    table: &mut ImportTable,
    package: PackageId,
    module: ModuleId,
    leaf: &Leaf,
    dcx: &DiagCtxt,
    interner: &Interner,
) {
    let mut current = match resolve_root(graph, package, leaf, dcx, interner) {
        Some(module) => module,
        None => return,
    };

    let (last, prefix) = match leaf.names.split_last() {
        Some((last, prefix)) => (Some(*last), prefix),
        None => (None, &[][..]),
    };

    for (name, span) in prefix {
        match graph.children.get(&(current, *name)).copied() {
            Some(child) => current = child,
            None => {
                unresolved(dcx, interner, *span, *name);
                return;
            }
        }
    }

    let Some((name, name_span)) = last else {
        // `use a;` imports the module itself.
        let local = leaf.alias.map_or_else(
            || leaf.root.name().unwrap_or_else(|| interner.intern("root")),
            |(alias, _)| alias,
        );
        bind_module(
            graph, table, package, module, current, local, leaf, dcx, interner,
        );
        return;
    };

    let child = graph.children.get(&(current, name)).copied();
    let item = graph.items.get(&(current, name)).copied();
    match (child, item) {
        (Some(_), Some(_)) => {
            dcx.error(format!(
                "the name `{}` denotes both a module and an item",
                interner.resolve(name)
            ))
            .code(codes::E2074.diag_code())
            .primary(name_span, "this name is ambiguous")
            .help("import the module and the item with different aliases")
            .emit();
        }
        (Some(child), None) => {
            let local = leaf.alias.map_or(name, |(alias, _)| alias);
            bind_module(
                graph, table, package, module, child, local, leaf, dcx, interner,
            );
        }
        (None, Some(info)) => {
            let accessible = visible(
                graph,
                package,
                module,
                current,
                info.visibility,
                info.span,
                dcx,
                interner,
            );
            if !accessible {
                return;
            }
            let local = leaf.alias.map_or(name, |(alias, _)| alias);
            let local_span = leaf.alias.map_or(name_span, |(_, span)| span);
            insert(
                graph,
                table,
                module,
                local,
                local_span,
                ImportTarget::Item {
                    module: current,
                    name,
                    namespace: info.namespace,
                },
                info.visibility,
                leaf.span,
                dcx,
                interner,
            );
        }
        (None, None) => unresolved(dcx, interner, name_span, name),
    }
}

fn resolve_root(
    graph: &ModuleGraph,
    package: PackageId,
    leaf: &Leaf,
    dcx: &DiagCtxt,
    interner: &Interner,
) -> Option<ModuleId> {
    let root = graph.packages[package].root;
    match leaf.root {
        UseRoot::Root => Some(root),
        UseRoot::Name(name) => {
            if let Some(module) = graph.children.get(&(root, name)).copied() {
                return Some(module);
            }
            if let Some(dependency) = graph.packages[package]
                .dependencies
                .iter()
                .find(|dependency| dependency.alias == name)
            {
                return Some(graph.packages[dependency.package].root);
            }
            dcx.error(format!(
                "cannot find the module or dependency `{}`",
                interner.resolve(name)
            ))
            .code(codes::E2073.diag_code())
            .primary(leaf.root_span, "this import does not resolve")
            .help("name a top-level module of this package, a dependency alias, or `root`")
            .emit();
            None
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn bind_module(
    graph: &ModuleGraph,
    table: &mut ImportTable,
    package: PackageId,
    module: ModuleId,
    target: ModuleId,
    local: Symbol,
    leaf: &Leaf,
    dcx: &DiagCtxt,
    interner: &Interner,
) {
    let target_package = graph.modules[target].package;
    if target_package != package && !graph.modules[target].exposed {
        dcx.error(format!(
            "the module `{}` is not part of the public API",
            graph.modules[target].path.display(interner)
        ))
        .code(codes::E2080.diag_code())
        .primary(leaf.span, "this import reaches a private module")
        .help("expose the module in `[package.api]`, or keep the import inside its package")
        .emit();
        return;
    }
    let visibility = if target_package == package {
        Visibility::Internal
    } else {
        Visibility::Public
    };
    insert(
        graph,
        table,
        module,
        local,
        leaf.root_span,
        ImportTarget::Module { module: target },
        visibility,
        leaf.span,
        dcx,
        interner,
    );
}

#[allow(clippy::too_many_arguments)]
fn visible(
    graph: &ModuleGraph,
    package: PackageId,
    from: ModuleId,
    target_module: ModuleId,
    visibility: Visibility,
    item_span: Span,
    dcx: &DiagCtxt,
    interner: &Interner,
) -> bool {
    let target_package = graph.modules[target_module].package;
    if target_package != package {
        if visibility != Visibility::Public {
            dcx.error(format!(
                "the item is `{}` and is not visible outside its package",
                visibility.as_str()
            ))
            .code(codes::E2072.diag_code())
            .primary(item_span, "declared here")
            .help("make the item `public`, or keep the import inside its package")
            .emit();
            return false;
        }
        if !graph.modules[target_module].exposed {
            dcx.error(format!(
                "the item is public, but the module `{}` is not exposed",
                graph.modules[target_module].path.display(interner)
            ))
            .code(codes::E2080.diag_code())
            .primary(item_span, "declared here")
            .help("expose the module in `[package.api]`")
            .emit();
            return false;
        }
        return true;
    }
    if visibility != Visibility::Private {
        return true;
    }
    let from_path = &graph.modules[from].path;
    let target_path = &graph.modules[target_module].path;
    if from_path.starts_with(target_path) {
        return true;
    }
    dcx.error("the item is private and is not visible from this module")
        .code(codes::E2072.diag_code())
        .primary(item_span, "declared here")
        .help("make the item `internal`, or import it from a module inside the declaring one")
        .emit();
    false
}

#[allow(clippy::too_many_arguments)]
fn insert(
    graph: &ModuleGraph,
    table: &mut ImportTable,
    module: ModuleId,
    local: Symbol,
    name_span: Span,
    target: ImportTarget,
    visibility: Visibility,
    span: Span,
    dcx: &DiagCtxt,
    interner: &Interner,
) {
    if table.get(local).is_some() {
        dcx.error(format!(
            "the import name `{}` is already used by another import",
            interner.resolve(local)
        ))
        .code(codes::E2078.diag_code())
        .primary(name_span, "duplicate import name")
        .help("rename one of the imports with `as`")
        .emit();
        return;
    }
    if graph.items.contains_key(&(module, local)) {
        dcx.error(format!(
            "the import name `{}` conflicts with an item of this module",
            interner.resolve(local)
        ))
        .code(codes::E2077.diag_code())
        .primary(name_span, "this import name is already declared")
        .help("import the name with an alias")
        .emit();
        return;
    }
    if graph.children.contains_key(&(module, local)) {
        dcx.error(format!(
            "the import name `{}` conflicts with a submodule",
            interner.resolve(local)
        ))
        .code(codes::E2077.diag_code())
        .primary(name_span, "this import name is already declared")
        .help("import the name with an alias")
        .emit();
        return;
    }
    table.insert(
        local,
        ImportBinding {
            target,
            span,
            name_span,
            visibility,
        },
    );
}

fn unresolved(dcx: &DiagCtxt, interner: &Interner, span: Span, name: Symbol) {
    dcx.error(format!(
        "cannot find `{}` in the imported module",
        interner.resolve(name)
    ))
    .code(codes::E2073.diag_code())
    .primary(span, "this name does not resolve")
    .help("check the spelling, or declare the item in the module")
    .emit();
}
