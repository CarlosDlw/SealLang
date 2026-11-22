//! JSON API snapshots for `--emit=api`.
//!
//! The snapshot is intentionally data, not a stable ABI: it lists the package's
//! modules, public items, signatures, struct fields, and enum variants so an
//! IDE, a project tool, or the registry can inspect a library without loading
//! the compiler again. The document is the same one `seal-api` hashes and
//! compares for compatibility.

use std::path::Path;

use seal_api::{
    ApiField, ApiImplementation, ApiItem, ApiLayout, ApiModule, ApiSnapshot, ApiVariant,
};
use seal_def::{DefId, DefKind, Visibility};
use seal_hir::Hir;
use seal_session::Session;
use seal_ty::{AdtId, AdtKind, TyId, TyKind};

use crate::cli::plan::path_string;

/// Writes the API snapshot for `hir` to `output`.
pub(crate) fn write(
    session: &Session,
    hir: &Hir,
    package_version: &str,
    output: &Path,
) -> Result<(), String> {
    let package = hir
        .packages()
        .get(hir.primary_package())
        .map(|package| session.interner().resolve(package.name).to_owned())
        .unwrap_or_else(|| "package".to_owned());
    let mut modules: Vec<ApiModule> = hir
        .modules()
        .iter()
        .filter(|module| module.package == hir.primary_package())
        .map(|module| ApiModule {
            path: path_string(&module.path, session.interner()),
            exposed: module.exposed,
            // A snapshot describes one package; using a project-wide index
            // would make the same package hash differently in a workspace and
            // when built alone.
            package: 0,
            docs: crate::doc_text::render(session.source_map(), module.docs, session.interner()),
        })
        .collect();
    for alias in hir.api().module_aliases() {
        let path = path_string(&alias.path, session.interner());
        if modules.iter().any(|module| module.path == path) {
            continue;
        }
        let target = hir.module(alias.module);
        modules.push(ApiModule {
            path,
            exposed: true,
            package: 0,
            docs: crate::doc_text::render(session.source_map(), target.docs, session.interner()),
        });
    }
    let mut items = Vec::new();
    for def in hir.public_items() {
        if hir.definitions().get(*def).package != hir.primary_package() {
            continue;
        }
        if let Some(item) = item(session, hir, *def) {
            items.push(item);
        }
    }
    for alias in hir.api().aliases() {
        let Some(mut aliased) = item(session, hir, alias.def) else {
            continue;
        };
        let canonical = Some(format!("{}::{}", aliased.module, aliased.name));
        aliased.module = path_string(&alias.path, session.interner());
        aliased.name = session.interner().resolve(alias.name).to_owned();
        aliased.canonical = canonical;
        let duplicate = items
            .iter()
            .any(|item| item.module == aliased.module && item.name == aliased.name);
        if !duplicate {
            items.push(aliased);
        }
    }
    let mut snapshot = ApiSnapshot {
        version: seal_api::FORMAT,
        package,
        package_version: package_version.to_owned(),
        target: session.target().triple().as_str().to_owned(),
        modules,
        items,
    };
    snapshot.normalize();
    snapshot
        .validate()
        .map_err(|error| format!("the compiler produced an invalid API snapshot: {error}"))?;
    let text = serde_json::to_string_pretty(&snapshot)
        .map_err(|error| format!("cannot serialize the API snapshot: {error}"))?;
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            format!(
                "cannot create the API snapshot directory `{}`: {error}",
                parent.display()
            )
        })?;
    }
    let temporary = output.with_extension(format!("json.{}.tmp", std::process::id()));
    std::fs::write(&temporary, text)
        .map_err(|error| format!("cannot write `{}`: {error}", temporary.display()))?;
    std::fs::rename(&temporary, output)
        .map_err(|error| format!("cannot replace `{}`: {error}", output.display()))?;
    Ok(())
}

fn item(session: &Session, hir: &Hir, def: DefId) -> Option<ApiItem> {
    let data = hir.definitions().get(def);
    let kind = kind_name(data.kind)?;
    let module = path_string(&hir.module(data.module).path, session.interner());
    let name = session.interner().resolve(data.name).to_owned();
    let visibility = format!("{:?}", data.visibility).to_lowercase();
    let cfg = data
        .cfg
        .map(|cfg| session.interner().resolve(cfg).to_owned());
    let mut item = ApiItem {
        name,
        kind: kind.to_owned(),
        module,
        canonical: None,
        visibility,
        cfg,
        signature: None,
        fields: Vec::new(),
        variants: Vec::new(),
        docs: crate::doc_text::render(session.source_map(), data.doc_span, session.interner()),
        layout: None,
        implementations: Vec::new(),
    };
    match data.kind {
        DefKind::Function | DefKind::ExternFunction => {
            item.signature = Some(signature(session, hir, def));
        }
        DefKind::Const | DefKind::Static | DefKind::ExternStatic => {
            item.signature = hir
                .body_of(def)
                .and_then(|body| body.ret)
                .map(|ty| format!(": {}", display(session, hir, ty)));
        }
        DefKind::Struct | DefKind::Enum => {
            let adt = hir.adt(def)?;
            let definition = hir.ty_ctxt().adt(adt);
            match definition.kind {
                AdtKind::Struct => {
                    item.fields = fields(session, hir, &definition.fields, true);
                }
                AdtKind::Enum => {
                    item.variants = definition
                        .variants
                        .iter()
                        .map(|variant| ApiVariant {
                            name: session.interner().resolve(variant.name).to_owned(),
                            // Enum variant fields are part of the variant's
                            // public constructor even without a `public`
                            // modifier in the source.
                            fields: fields(session, hir, &variant.fields, false),
                            docs: crate::doc_text::render(
                                session.source_map(),
                                hir.definitions().get(variant.def).doc_span,
                                session.interner(),
                            ),
                        })
                        .collect();
                }
                AdtKind::Closure => return None,
            }
            item.layout = layout(session, hir, adt);
            item.implementations = implementations(session, hir, def);
        }
        _ => {}
    }
    Some(item)
}

fn fields(
    session: &Session,
    hir: &Hir,
    fields: &[seal_ty::FieldDef],
    public_only: bool,
) -> Vec<ApiField> {
    fields
        .iter()
        .filter_map(|field| {
            let data = hir.definitions().get(field.def);
            if public_only && data.visibility != Visibility::Public {
                return None;
            }
            Some(ApiField {
                name: field
                    .name
                    .map(|name| session.interner().resolve(name).to_owned()),
                ty: display(session, hir, field.ty),
                visibility: format!("{:?}", data.visibility).to_lowercase(),
                docs: crate::doc_text::render(
                    session.source_map(),
                    data.doc_span,
                    session.interner(),
                ),
            })
        })
        .collect()
}

fn layout(session: &Session, hir: &Hir, adt: AdtId) -> Option<ApiLayout> {
    let ty = hir.ty_ctxt().adt_ty(adt);
    let layout = seal_layout::LayoutCtxt::new(hir.ty_ctxt(), session.target().pointer_width())
        .layout_of(ty)
        .ok()?;
    Some(ApiLayout {
        size: layout.size(),
        align: layout.align(),
    })
}

fn implementations(session: &Session, hir: &Hir, def: DefId) -> Vec<ApiImplementation> {
    let mut implementations = Vec::new();
    let matches = |ty: TyId| match hir.ty_ctxt().kind(ty) {
        TyKind::Adt { adt, .. } => hir.ty_ctxt().adt(adt).def == def,
        _ => false,
    };
    for implementation in hir.inherent_impls() {
        if !matches(implementation.self_ty) {
            continue;
        }
        let methods: Vec<String> = implementation
            .methods
            .iter()
            .filter(|(_, method)| hir.definitions().get(*method).visibility == Visibility::Public)
            .map(|(name, _)| session.interner().resolve(*name).to_owned())
            .collect();
        if methods.is_empty() {
            continue;
        }
        implementations.push(ApiImplementation {
            trait_name: None,
            methods,
        });
    }
    for implementation in hir.trait_impls() {
        if !matches(implementation.trait_ref.self_ty) {
            continue;
        }
        let trait_name = session
            .interner()
            .resolve(
                hir.definitions()
                    .get(implementation.trait_ref.trait_def)
                    .name,
            )
            .to_owned();
        implementations.push(ApiImplementation {
            trait_name: Some(trait_name),
            methods: implementation
                .methods
                .iter()
                .map(|(name, _)| session.interner().resolve(*name).to_owned())
                .collect(),
        });
    }
    implementations
}

fn signature(session: &Session, hir: &Hir, def: DefId) -> String {
    let data = hir.definitions().get(def);
    let Some(body) = hir.body_of(def) else {
        return format!("func {}", session.interner().resolve(data.name));
    };
    let mut text = String::new();
    if body.is_const {
        text.push_str("const ");
    }
    if body.is_unsafe {
        text.push_str("unsafe ");
    }
    text.push_str("func ");
    text.push_str(session.interner().resolve(data.name));
    text.push('(');
    for (index, param) in body.params.iter().enumerate() {
        if index > 0 {
            text.push_str(", ");
        }
        let name = session
            .interner()
            .resolve(hir.definitions().get(param.def).name);
        text.push_str(name);
        text.push_str(": ");
        text.push_str(&display(session, hir, param.ty));
    }
    text.push(')');
    if let Some(ret) = body.ret {
        text.push_str(" -> ");
        text.push_str(&display(session, hir, ret));
    }
    text
}

fn display(session: &Session, hir: &Hir, ty: TyId) -> String {
    hir.ty_ctxt()
        .display_with(ty, session.interner())
        .to_string()
}

fn kind_name(kind: DefKind) -> Option<&'static str> {
    match kind {
        DefKind::Function => Some("function"),
        DefKind::Struct => Some("struct"),
        DefKind::Enum => Some("enum"),
        DefKind::Trait => Some("trait"),
        DefKind::Const => Some("const"),
        DefKind::Static => Some("static"),
        DefKind::ExternFunction => Some("extern-function"),
        DefKind::ExternStatic => Some("extern-static"),
        DefKind::OpaqueType => Some("opaque-type"),
        _ => None,
    }
}
