//! Rendering public API snapshots as documentation.
//!
//! The renderer consumes only [`seal_api::ApiSnapshot`], so the HTML output and
//! the JSON document always describe the same public surface the registry
//! checks. Private items never reach this crate because they are absent from the
//! snapshot.

#![warn(missing_docs)]

mod error;
mod html;
mod json;
mod server;

pub use error::{DocError, DocResult};
pub use html::{render as render_html, write as write_html};
pub use json::{render as render_json, write as write_json};
pub use server::{Served, serve};

use seal_api::ApiSnapshot;
use serde::Serialize;

/// One documented module.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ModuleDoc {
    /// The module path, with `::` separators.
    pub path: String,
    /// Whether dependents can import the module.
    pub exposed: bool,
    /// The module documentation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub docs: Option<String>,
    /// The public items declared in this module.
    pub items: Vec<ItemDoc>,
}

/// One documented public item.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ItemDoc {
    /// The item name.
    pub name: String,
    /// The item kind.
    pub kind: String,
    /// The item documentation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub docs: Option<String>,
    /// The rendered signature.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
    /// The `cfg` condition, when present.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cfg: Option<String>,
    /// Deprecation metadata, when the item is deprecated.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deprecated: Option<DeprecationDoc>,
    /// Struct fields.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<FieldDoc>,
    /// Enum variants.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub variants: Vec<VariantDoc>,
    /// Size and alignment of an aggregate.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub layout: Option<LayoutDoc>,
    /// Attached implementations.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub implementations: Vec<ImplementationDoc>,
}

/// One documented field.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct FieldDoc {
    /// The field name; `None` for a positional field.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The field type.
    pub ty: String,
    /// The field visibility.
    pub visibility: String,
    /// The field documentation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub docs: Option<String>,
    /// Deprecation metadata, when the field is deprecated.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deprecated: Option<DeprecationDoc>,
}

/// One documented enum variant.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct VariantDoc {
    /// The variant name.
    pub name: String,
    /// The variant documentation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub docs: Option<String>,
    /// Deprecation metadata, when the variant is deprecated.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deprecated: Option<DeprecationDoc>,
    /// The variant fields.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<FieldDoc>,
}

/// Deprecation metadata attached to an item, field, or variant.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct DeprecationDoc {
    /// The release that deprecated the declaration.
    pub since: String,
    /// Migration guidance.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// The replacement the declaration points to.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub replacement: Option<String>,
}

/// One documented aggregate layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct LayoutDoc {
    /// Size in bytes.
    pub size: u64,
    /// Alignment in bytes.
    pub align: u64,
}

/// One documented implementation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ImplementationDoc {
    /// The implemented trait, if any.
    #[serde(rename = "trait", skip_serializing_if = "Option::is_none")]
    pub trait_name: Option<String>,
    /// The method names.
    pub methods: Vec<String>,
}

/// The complete documentation document.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Documentation {
    /// The document schema version.
    pub version: u32,
    /// The package name.
    pub package: String,
    /// The package version.
    pub package_version: String,
    /// The target triple.
    pub target: String,
    /// Public modules, sorted by path.
    pub modules: Vec<ModuleDoc>,
}

impl Documentation {
    /// Builds a documentation document from a public API snapshot.
    pub fn from_snapshot(snapshot: &ApiSnapshot) -> Self {
        let mut modules: Vec<ModuleDoc> = snapshot
            .modules
            .iter()
            .map(|module| ModuleDoc {
                path: module.path.clone(),
                exposed: module.exposed,
                docs: module.docs.clone(),
                items: Vec::new(),
            })
            .collect();
        for item in &snapshot.items {
            let module = modules.iter_mut().find(|module| module.path == item.module);
            let Some(module) = module else {
                continue;
            };
            module.items.push(item_doc(item));
        }
        for module in &mut modules {
            module.items.sort_by(|left, right| {
                left.name
                    .cmp(&right.name)
                    .then_with(|| left.kind.cmp(&right.kind))
            });
        }
        modules.sort_by(|left, right| left.path.cmp(&right.path));
        Self {
            version: 1,
            package: snapshot.package.clone(),
            package_version: snapshot.package_version.clone(),
            target: snapshot.target.clone(),
            modules,
        }
    }
}

fn deprecation_doc(deprecation: &seal_api::ApiDeprecation) -> DeprecationDoc {
    DeprecationDoc {
        since: deprecation.since.clone(),
        note: deprecation.note.clone(),
        replacement: deprecation.replacement.clone(),
    }
}

fn item_doc(item: &seal_api::ApiItem) -> ItemDoc {
    ItemDoc {
        name: item.name.clone(),
        kind: item.kind.clone(),
        docs: item.docs.clone(),
        signature: item.signature.clone(),
        cfg: item.cfg.clone(),
        deprecated: item.deprecated.as_ref().map(deprecation_doc),
        fields: item
            .fields
            .iter()
            .map(|field| FieldDoc {
                name: field.name.clone(),
                ty: field.ty.clone(),
                visibility: field.visibility.clone(),
                docs: field.docs.clone(),
                deprecated: field.deprecated.as_ref().map(deprecation_doc),
            })
            .collect(),
        variants: item
            .variants
            .iter()
            .map(|variant| VariantDoc {
                name: variant.name.clone(),
                docs: variant.docs.clone(),
                deprecated: variant.deprecated.as_ref().map(deprecation_doc),
                fields: variant
                    .fields
                    .iter()
                    .map(|field| FieldDoc {
                        name: field.name.clone(),
                        ty: field.ty.clone(),
                        visibility: field.visibility.clone(),
                        docs: field.docs.clone(),
                        deprecated: field.deprecated.as_ref().map(deprecation_doc),
                    })
                    .collect(),
            })
            .collect(),
        layout: item.layout.map(|layout| LayoutDoc {
            size: layout.size,
            align: layout.align,
        }),
        implementations: item
            .implementations
            .iter()
            .map(|implementation| ImplementationDoc {
                trait_name: implementation.trait_name.clone(),
                methods: implementation.methods.clone(),
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use seal_api::{ApiDeprecation, ApiField, ApiItem, ApiModule, ApiSnapshot};

    fn snapshot() -> ApiSnapshot {
        ApiSnapshot {
            version: 1,
            package: "demo".to_owned(),
            package_version: "1.0.0".to_owned(),
            target: "x86_64-unknown-linux-gnu".to_owned(),
            modules: vec![
                ApiModule {
                    path: "root".to_owned(),
                    exposed: true,
                    package: 0,
                    docs: Some("The root module.".to_owned()),
                },
                ApiModule {
                    path: "app".to_owned(),
                    exposed: true,
                    package: 0,
                    docs: None,
                },
            ],
            items: vec![
                ApiItem {
                    name: "Point".to_owned(),
                    kind: "struct".to_owned(),
                    module: "root".to_owned(),
                    canonical: None,
                    visibility: "public".to_owned(),
                    cfg: None,
                    signature: None,
                    fields: vec![ApiField {
                        name: Some("x".to_owned()),
                        ty: "i32".to_owned(),
                        visibility: "public".to_owned(),
                        docs: Some("The x coordinate.".to_owned()),
                        deprecated: None,
                    }],
                    variants: Vec::new(),
                    docs: Some("A point.".to_owned()),
                    deprecated: None,
                    layout: Some(seal_api::ApiLayout { size: 4, align: 4 }),
                    implementations: Vec::new(),
                },
                ApiItem {
                    name: "value".to_owned(),
                    kind: "function".to_owned(),
                    module: "app".to_owned(),
                    canonical: None,
                    visibility: "public".to_owned(),
                    cfg: Some("target.os == \"linux\"".to_owned()),
                    signature: Some("func value() -> i32".to_owned()),
                    fields: Vec::new(),
                    variants: Vec::new(),
                    docs: Some("Returns a value.".to_owned()),
                    deprecated: None,
                    layout: None,
                    implementations: Vec::new(),
                },
            ],
        }
    }

    #[test]
    fn private_items_are_absent_and_modules_are_sorted() {
        let docs = Documentation::from_snapshot(&snapshot());
        assert_eq!(docs.modules.len(), 2);
        assert_eq!(docs.modules[0].path, "app");
        assert_eq!(
            docs.modules[0].items[0].cfg.as_deref(),
            Some("target.os == \"linux\"")
        );
    }

    #[test]
    fn deprecation_metadata_reaches_json_and_escaped_html() {
        let mut snapshot = snapshot();
        snapshot.items[0].deprecated = Some(ApiDeprecation {
            since: "0.2.0".to_owned(),
            note: Some("<script>alert(1)</script>".to_owned()),
            replacement: Some("NewPoint".to_owned()),
        });
        let documentation = Documentation::from_snapshot(&snapshot);
        let item = documentation
            .modules
            .iter()
            .flat_map(|module| &module.items)
            .find(|item| item.name == "Point")
            .expect("the Point item is documented");
        assert_eq!(
            item.deprecated.as_ref().expect("deprecated item").since,
            "0.2.0"
        );
        let json = render_json(&documentation).expect("JSON");
        assert!(json.contains("\"since\": \"0.2.0\""));
        assert!(json.contains("NewPoint"));
        let directory =
            std::env::temp_dir().join(format!("seal-doc-deprecation-{}", std::process::id()));
        let paths = write_html(&documentation, &directory).expect("HTML files");
        let root = paths
            .iter()
            .find(|path| path.ends_with("root.html"))
            .expect("the root module page is written");
        let html = std::fs::read_to_string(root).expect("the root page is readable");
        assert!(html.contains("Deprecated since <code>0.2.0</code>"));
        assert!(html.contains("&lt;script&gt;alert(1)&lt;/script&gt;"));
        assert!(!html.contains("<script>alert(1)</script>"));
        assert!(html.contains("Use <code>NewPoint</code> instead"));
        std::fs::remove_dir_all(&directory).expect("the temporary documentation is removed");
    }

    #[test]
    fn a_type_alias_keeps_its_kind_and_target_in_json_and_html() {
        let mut snapshot = snapshot();
        snapshot.items[1].name = "Id".to_owned();
        snapshot.items[1].kind = "type-alias".to_owned();
        snapshot.items[1].signature = Some("= i32".to_owned());
        snapshot.items[1].cfg = None;
        let documentation = Documentation::from_snapshot(&snapshot);
        let item = documentation
            .modules
            .iter()
            .flat_map(|module| &module.items)
            .find(|item| item.name == "Id")
            .expect("the alias is documented");
        assert_eq!(item.kind, "type-alias");
        assert_eq!(item.signature.as_deref(), Some("= i32"));
        assert!(item.fields.is_empty());
        assert!(item.variants.is_empty());
        assert!(item.implementations.is_empty());

        let json = render_json(&documentation).expect("JSON");
        assert!(json.contains("\"kind\": \"type-alias\""));
        assert!(json.contains("\"signature\": \"= i32\""));
        let index = render_html(&documentation);
        assert!(index.contains("<span class=\"kind\">type-alias</span>"));
        let directory = std::env::temp_dir().join(format!("seal-doc-alias-{}", std::process::id()));
        let paths = write_html(&documentation, &directory).expect("HTML files");
        let app = paths
            .iter()
            .find(|path| path.ends_with("app.html"))
            .expect("the app module page is written");
        let html = std::fs::read_to_string(app).expect("the app page is readable");
        assert!(html.contains("<span class=\"kind\">type-alias</span>"));
        assert!(html.contains("<pre class=\"signature\"><code>= i32</code></pre>"));
        std::fs::remove_dir_all(&directory).expect("the temporary documentation is removed");
    }

    #[test]
    fn html_contains_docs_cfg_and_links() {
        let html = render_html(&Documentation::from_snapshot(&snapshot()));
        assert!(html.contains("A point."));
        assert!(html.contains("The x coordinate."));
        assert!(html.contains("target.os"));
        assert!(html.contains("href=\"app.html\""));
        assert!(!html.contains("private"));
    }
}
