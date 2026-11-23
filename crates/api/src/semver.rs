//! Public-API compatibility classification for releases.
//!
//! The rules follow semantic versioning from a consumer's point of view. Removing
//! or changing anything a consumer can name is breaking; adding a new name is
//! minor. A new enum variant is breaking because Seal has no `non_exhaustive`
//! escape hatch, so every `match` over the enum would stop compiling.

use std::collections::BTreeSet;

use semver::Version;
use serde::Serialize;

use crate::error::{ApiError, ApiResult};
use crate::snapshot::{ApiField, ApiImplementation, ApiItem, ApiSnapshot, ApiVariant};

/// The smallest version bump a change requires.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Bump {
    /// Documentation or implementation only.
    Patch,
    /// Backwards-compatible additions.
    Minor,
    /// Incompatible removals or changes.
    Major,
}

impl Bump {
    /// Returns the conventional name of the bump.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Patch => "patch",
            Self::Minor => "minor",
            Self::Major => "major",
        }
    }
}

/// One classified API change.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Change {
    /// A stable change kind, such as `item-removed`.
    pub kind: &'static str,
    /// The item or module that changed.
    pub target: String,
    /// A human-readable explanation.
    pub detail: String,
}

impl Change {
    fn new(kind: &'static str, target: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            kind,
            target: target.into(),
            detail: detail.into(),
        }
    }
}

/// The complete difference between two snapshots.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct ApiDiff {
    /// Incompatible changes.
    pub breaking: Vec<Change>,
    /// Compatible additions.
    pub minor: Vec<Change>,
    /// Reserved for future documentation-only changes.
    pub patch: Vec<Change>,
}

impl ApiDiff {
    /// Returns whether the two snapshots expose the same API.
    pub fn is_empty(&self) -> bool {
        self.breaking.is_empty() && self.minor.is_empty() && self.patch.is_empty()
    }

    /// Returns the smallest bump this difference requires.
    pub fn required_bump(&self) -> Bump {
        if !self.breaking.is_empty() {
            Bump::Major
        } else if !self.minor.is_empty() {
            Bump::Minor
        } else {
            Bump::Patch
        }
    }

    /// Returns whether `next` satisfies the bump required from `previous`.
    ///
    /// For `0.x` releases, the minor component is the compatibility line, so a
    /// breaking change requires a new minor version rather than a new major.
    pub fn satisfied_by(&self, previous: &Version, next: &Version) -> bool {
        if next <= previous {
            return false;
        }
        match self.required_bump() {
            Bump::Patch => true,
            Bump::Minor => {
                if previous.major == 0 {
                    next.minor > previous.minor
                } else {
                    next.minor > previous.minor || next.major > previous.major
                }
            }
            Bump::Major => {
                if previous.major == 0 {
                    next.minor > previous.minor || next.major > previous.major
                } else {
                    next.major > previous.major
                }
            }
        }
    }
}

/// Compares two snapshots.
pub fn check(previous: &ApiSnapshot, next: &ApiSnapshot) -> ApiResult<ApiDiff> {
    previous.validate()?;
    next.validate()?;
    if previous.package != next.package {
        return Err(ApiError::snapshot(format!(
            "cannot compare the package `{}` with `{}`",
            previous.package, next.package
        )));
    }
    if previous.target != next.target {
        return Err(ApiError::snapshot(format!(
            "cannot compare snapshots for `{}` and `{}`",
            previous.target, next.target
        )));
    }
    let mut diff = ApiDiff::default();
    let previous_modules = previous.module_map();
    let next_modules = next.module_map();
    for path in previous_modules.keys() {
        if !next_modules.contains_key(path) {
            diff.breaking.push(Change::new(
                "module-removed",
                *path,
                format!("the public module `{path}` was removed"),
            ));
        }
    }
    for path in next_modules.keys() {
        if !previous_modules.contains_key(path) {
            diff.minor.push(Change::new(
                "module-added",
                *path,
                format!("the public module `{path}` was added"),
            ));
        }
    }

    let previous_items = previous.item_map();
    let next_items = next.item_map();
    for (key, item) in &previous_items {
        match next_items.get(key) {
            Some(next_item) => compare_item(item, next_item, &mut diff),
            None => diff.breaking.push(Change::new(
                "item-removed",
                key.clone(),
                format!("the public item `{key}` was removed"),
            )),
        }
    }
    for key in next_items.keys() {
        if !previous_items.contains_key(key) {
            diff.minor.push(Change::new(
                "item-added",
                key.clone(),
                format!("the public item `{key}` was added"),
            ));
        }
    }
    Ok(diff)
}

fn compare_item(previous: &ApiItem, next: &ApiItem, diff: &mut ApiDiff) {
    let key = previous.key();
    // A re-export can keep its public name while pointing at a different item.
    // Snapshots from older tools do not record a target, so only compare known
    // targets; a missing one cannot be proven to have changed.
    if let (Some(previous_target), Some(next_target)) = (&previous.canonical, &next.canonical) {
        if previous_target != next_target {
            diff.breaking.push(Change::new(
                "target-changed",
                key.clone(),
                format!("`{key}` now refers to `{next_target}` instead of `{previous_target}`"),
            ));
        }
    }
    if previous.signature != next.signature {
        diff.breaking.push(Change::new(
            "signature-changed",
            key.clone(),
            format!(
                "the signature of `{key}` changed from `{}` to `{}`",
                previous.signature.as_deref().unwrap_or("none"),
                next.signature.as_deref().unwrap_or("none")
            ),
        ));
    }
    if previous.cfg != next.cfg {
        diff.breaking.push(Change::new(
            "cfg-changed",
            key.clone(),
            format!(
                "the availability of `{key}` changed from `{}` to `{}`",
                previous.cfg.as_deref().unwrap_or("always"),
                next.cfg.as_deref().unwrap_or("always")
            ),
        ));
    }
    // A snapshot from an older tool may not carry layout information. An
    // unknown layout cannot be proven to have changed, so only compare two
    // known layouts; otherwise every aggregate would become breaking once the
    // field was introduced.
    if let (Some(previous_layout), Some(next_layout)) = (&previous.layout, &next.layout) {
        if previous_layout != next_layout {
            diff.breaking.push(Change::new(
                "layout-changed",
                key.clone(),
                format!(
                    "the layout of `{key}` changed from {} to {}",
                    layout_text(Some(previous_layout)),
                    layout_text(Some(next_layout))
                ),
            ));
        }
    }
    // Only public struct fields are part of the callable surface. Private
    // fields are represented through the layout comparison instead, and older
    // snapshots that recorded them must not turn their removal into breaking.
    let previous_fields = public_fields(&previous.fields);
    let next_fields = public_fields(&next.fields);
    for (name, field) in &previous_fields {
        match next_fields.get(name) {
            Some(next_field) => compare_field(&key, name, field, next_field, diff, false),
            None => diff.breaking.push(Change::new(
                "field-removed",
                format!("{key}.{name}"),
                format!("the public field `{name}` of `{key}` was removed"),
            )),
        }
    }
    for name in next_fields.keys() {
        if !previous_fields.contains_key(name) {
            diff.minor.push(Change::new(
                "field-added",
                format!("{key}.{name}"),
                format!("the public field `{name}` was added to `{key}`"),
            ));
        }
    }

    let previous_variants: std::collections::BTreeMap<&str, &ApiVariant> = previous
        .variants
        .iter()
        .map(|variant| (variant.name.as_str(), variant))
        .collect();
    let next_variants: std::collections::BTreeMap<&str, &ApiVariant> = next
        .variants
        .iter()
        .map(|variant| (variant.name.as_str(), variant))
        .collect();
    for (name, variant) in &previous_variants {
        match next_variants.get(name) {
            Some(next_variant) => compare_variant(&key, variant, next_variant, diff),
            None => diff.breaking.push(Change::new(
                "variant-removed",
                format!("{key}::{name}"),
                format!("the public variant `{name}` of `{key}` was removed"),
            )),
        }
    }
    for name in next_variants.keys() {
        if !previous_variants.contains_key(name) {
            diff.breaking.push(Change::new(
                "variant-added",
                format!("{key}::{name}"),
                format!(
                    "the public variant `{name}` was added to `{key}`; exhaustive matches over the enum would stop compiling"
                ),
            ));
        }
    }
    compare_implementations(&key, previous, next, diff);
}

fn compare_implementations(key: &str, previous: &ApiItem, next: &ApiItem, diff: &mut ApiDiff) {
    let previous_impls = implementation_map(&previous.implementations);
    let next_impls = implementation_map(&next.implementations);
    for (name, methods) in &previous_impls {
        match next_impls.get(name) {
            Some(next_methods) => {
                for method in methods {
                    if !next_methods.contains(method) {
                        diff.breaking.push(Change::new(
                            "method-removed",
                            format!("{key}::{name}::{method}"),
                            format!("the method `{method}` of `{key}` was removed from `{name}`"),
                        ));
                    }
                }
                for method in next_methods {
                    if !methods.contains(method) {
                        diff.minor.push(Change::new(
                            "method-added",
                            format!("{key}::{name}::{method}"),
                            format!("the method `{method}` was added to `{key}`"),
                        ));
                    }
                }
            }
            None => diff.breaking.push(Change::new(
                "implementation-removed",
                format!("{key}::{name}"),
                format!("the implementation `{name}` of `{key}` was removed"),
            )),
        }
    }
    for name in next_impls.keys() {
        if !previous_impls.contains_key(name) {
            diff.minor.push(Change::new(
                "implementation-added",
                format!("{key}::{name}"),
                format!("the implementation `{name}` was added to `{key}`"),
            ));
        }
    }
}

fn implementation_map(
    implementations: &[ApiImplementation],
) -> std::collections::BTreeMap<String, std::collections::BTreeSet<&str>> {
    let mut map: std::collections::BTreeMap<String, std::collections::BTreeSet<&str>> =
        std::collections::BTreeMap::new();
    for implementation in implementations {
        let name = implementation
            .trait_name
            .clone()
            .unwrap_or_else(|| "inherent".to_owned());
        map.entry(name)
            .or_default()
            .extend(implementation.methods.iter().map(String::as_str));
    }
    map
}

fn compare_variant(key: &str, previous: &ApiVariant, next: &ApiVariant, diff: &mut ApiDiff) {
    let target = format!("{key}::{}", previous.name);
    let previous_fields: std::collections::BTreeMap<String, &ApiField> = previous
        .fields
        .iter()
        .map(|field| (field_key(field), field))
        .collect();
    let next_fields: std::collections::BTreeMap<String, &ApiField> = next
        .fields
        .iter()
        .map(|field| (field_key(field), field))
        .collect();
    for (name, field) in &previous_fields {
        match next_fields.get(name) {
            Some(next_field) => compare_field(&target, name, field, next_field, diff, true),
            None => diff.breaking.push(Change::new(
                "variant-field-removed",
                format!("{target}.{name}"),
                format!("the field `{name}` of `{target}` was removed"),
            )),
        }
    }
    for name in next_fields.keys() {
        if !previous_fields.contains_key(name) {
            diff.breaking.push(Change::new(
                "variant-field-added",
                format!("{target}.{name}"),
                format!(
                    "the field `{name}` of `{target}` was added; constructing the variant would need another value"
                ),
            ));
        }
    }
}

fn compare_field(
    target: &str,
    name: &str,
    previous: &ApiField,
    next: &ApiField,
    diff: &mut ApiDiff,
    variant: bool,
) {
    if previous.ty != next.ty {
        diff.breaking.push(Change::new(
            if variant {
                "variant-field-type-changed"
            } else {
                "field-type-changed"
            },
            format!("{target}.{name}"),
            format!(
                "the type of `{name}` changed from `{}` to `{}`",
                previous.ty, next.ty
            ),
        ));
    }
    if previous.visibility != next.visibility {
        if next.visibility == "public" {
            diff.minor.push(Change::new(
                "field-made-public",
                format!("{target}.{name}"),
                format!("the field `{name}` became public"),
            ));
        } else {
            diff.breaking.push(Change::new(
                "field-made-private",
                format!("{target}.{name}"),
                format!("the public field `{name}` became {}", next.visibility),
            ));
        }
    }
}

fn layout_text(layout: Option<&crate::ApiLayout>) -> String {
    match layout {
        Some(layout) => format!("{} byte(s), align {}", layout.size, layout.align),
        None => "unknown".to_owned(),
    }
}

fn public_fields(fields: &[ApiField]) -> std::collections::BTreeMap<String, &ApiField> {
    fields
        .iter()
        .filter(|field| field.visibility == "public")
        .map(|field| (field_key(field), field))
        .collect()
}

fn field_key(field: &ApiField) -> String {
    field.name.clone().unwrap_or_else(|| "#".to_owned())
}

/// Returns the bumps required by the whole document.
pub fn required_bump(previous: &ApiSnapshot, next: &ApiSnapshot) -> ApiResult<Bump> {
    Ok(check(previous, next)?.required_bump())
}

/// Rejects a release whose version does not carry a breaking change.
pub fn enforce(
    previous: &ApiSnapshot,
    next: &ApiSnapshot,
    version: &Version,
) -> ApiResult<ApiDiff> {
    let diff = check(previous, next)?;
    let previous_version = Version::parse(&previous.package_version).map_err(|error| {
        ApiError::snapshot(format!(
            "the previous snapshot has the invalid package version `{}`: {error}",
            previous.package_version
        ))
    })?;
    if !diff.satisfied_by(&previous_version, version) {
        return Err(ApiError::version(format!(
            "the API change requires a {} bump from `{previous_version}`, but the release is `{version}`",
            diff.required_bump().as_str()
        )));
    }
    Ok(diff)
}

/// Returns a sorted list of the public item keys in a snapshot.
pub fn public_keys(snapshot: &ApiSnapshot) -> BTreeSet<String> {
    snapshot.items.iter().map(ApiItem::key).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::{ApiImplementation, ApiModule, ApiVariant};

    fn snapshot(version: &str) -> ApiSnapshot {
        ApiSnapshot {
            version: 1,
            package: "demo".to_owned(),
            package_version: version.to_owned(),
            target: "x86_64-unknown-linux-gnu".to_owned(),
            modules: vec![ApiModule {
                path: "root".to_owned(),
                exposed: true,
                package: 0,
                docs: None,
            }],
            items: vec![ApiItem {
                name: "Mode".to_owned(),
                kind: "enum".to_owned(),
                module: "root".to_owned(),
                canonical: None,
                visibility: "public".to_owned(),
                cfg: None,
                signature: None,
                fields: Vec::new(),
                variants: vec![ApiVariant {
                    name: "On".to_owned(),
                    fields: Vec::new(),
                    docs: None,
                    deprecated: None,
                }],
                docs: None,
                deprecated: None,
                layout: None,
                implementations: Vec::new(),
            }],
        }
    }

    #[test]
    fn identical_snapshots_need_only_a_patch() {
        let diff = check(&snapshot("1.0.0"), &snapshot("1.0.1")).unwrap();
        assert!(diff.is_empty());
        assert_eq!(diff.required_bump(), Bump::Patch);
    }

    #[test]
    fn removing_an_item_is_breaking() {
        let mut next = snapshot("1.1.0");
        next.items.clear();
        let diff = check(&snapshot("1.0.0"), &next).unwrap();
        assert_eq!(diff.required_bump(), Bump::Major);
    }

    #[test]
    fn adding_a_public_enum_variant_is_breaking() {
        let mut next = snapshot("1.1.0");
        next.items[0].variants.push(ApiVariant {
            name: "Off".to_owned(),
            fields: Vec::new(),
            docs: None,
            deprecated: None,
        });
        let diff = check(&snapshot("1.0.0"), &next).unwrap();
        assert_eq!(diff.breaking[0].kind, "variant-added");
    }

    #[test]
    fn removing_a_public_field_is_breaking() {
        let mut previous = snapshot("1.0.0");
        previous.items[0].fields.push(ApiField {
            name: Some("x".to_owned()),
            ty: "i32".to_owned(),
            visibility: "public".to_owned(),
            docs: None,
            deprecated: None,
        });
        let next = snapshot("2.0.0");
        let diff = check(&previous, &next).unwrap();
        assert_eq!(diff.breaking[0].kind, "field-removed");
    }

    #[test]
    fn changing_a_signature_is_breaking() {
        let mut previous = snapshot("1.0.0");
        previous.items[0].signature = Some("func() -> i32".to_owned());
        let mut next = snapshot("1.1.0");
        next.items[0].signature = Some("func(x: i32) -> i32".to_owned());
        let diff = check(&previous, &next).unwrap();
        assert_eq!(diff.breaking[0].kind, "signature-changed");
    }

    #[test]
    fn an_unknown_previous_layout_is_not_breaking_but_a_known_change_is() {
        let mut next = snapshot("1.1.0");
        next.items[0].layout = Some(crate::ApiLayout { size: 4, align: 4 });
        let diff = check(&snapshot("1.0.0"), &next).unwrap();
        assert!(diff.is_empty());

        let mut later = next.clone();
        later.package_version = "2.0.0".to_owned();
        later.items[0].layout = Some(crate::ApiLayout { size: 8, align: 8 });
        let diff = check(&next, &later).unwrap();
        assert_eq!(diff.breaking[0].kind, "layout-changed");
    }

    #[test]
    fn private_struct_fields_are_not_part_of_the_public_surface() {
        let mut previous = snapshot("1.0.0");
        previous.items[0].fields.push(ApiField {
            name: Some("hidden".to_owned()),
            ty: "i32".to_owned(),
            visibility: "private".to_owned(),
            docs: None,
            deprecated: None,
        });
        let diff = check(&previous, &snapshot("1.1.0")).unwrap();
        assert!(diff.is_empty());
    }

    #[test]
    fn method_additions_are_minor_and_removals_are_breaking() {
        let mut with_method = snapshot("1.1.0");
        with_method.items[0].implementations = vec![ApiImplementation {
            trait_name: None,
            methods: vec!["len".to_owned()],
        }];
        let diff = check(&snapshot("1.0.0"), &with_method).unwrap();
        assert_eq!(diff.required_bump(), Bump::Minor);

        let mut without_method = with_method.clone();
        without_method.package_version = "2.0.0".to_owned();
        without_method.items[0].implementations[0].methods.clear();
        let diff = check(&with_method, &without_method).unwrap();
        assert_eq!(diff.breaking[0].kind, "method-removed");
    }

    #[test]
    fn changing_a_reexport_target_is_breaking() {
        let previous = snapshot("1.0.0");
        let mut next = snapshot("2.0.0");
        next.items[0].canonical = Some("canonical::old".to_owned());
        let previous = {
            let mut previous = previous;
            previous.items[0].canonical = Some("canonical::old".to_owned());
            previous
        };
        let diff = check(&previous, &next).unwrap();
        assert!(diff.is_empty());

        let mut changed = next.clone();
        changed.items[0].canonical = Some("canonical::new".to_owned());
        let diff = check(&next, &changed).unwrap();
        assert_eq!(diff.breaking[0].kind, "target-changed");
    }

    #[test]
    fn additive_changes_are_minor() {
        let mut next = snapshot("1.1.0");
        next.items.push(ApiItem {
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
                docs: None,
                deprecated: None,
            }],
            variants: Vec::new(),
            docs: None,
            deprecated: None,
            layout: None,
            implementations: Vec::new(),
        });
        let diff = check(&snapshot("1.0.0"), &next).unwrap();
        assert_eq!(diff.required_bump(), Bump::Minor);
    }

    #[test]
    fn deprecation_metadata_does_not_change_the_bump() {
        let mut next = snapshot("1.0.1");
        next.items[0].deprecated = Some(crate::ApiDeprecation {
            since: "0.2.0".to_owned(),
            note: None,
            replacement: None,
        });
        let diff = check(&snapshot("1.0.0"), &next).unwrap();
        assert!(diff.is_empty());
        assert_eq!(diff.required_bump(), Bump::Patch);
    }

    #[test]
    fn a_breaking_change_without_a_major_bump_is_rejected() {
        let mut next = snapshot("1.1.0");
        next.items.clear();
        let error =
            enforce(&snapshot("1.0.0"), &next, &Version::parse("1.1.0").unwrap()).unwrap_err();
        assert_eq!(error.code(), "E8092");
        assert!(enforce(&snapshot("1.0.0"), &next, &Version::parse("2.0.0").unwrap()).is_ok());
    }

    #[test]
    fn zero_major_lines_require_a_minor_bump_for_breaking_changes() {
        let mut next = snapshot("0.2.0");
        next.items.clear();
        assert!(enforce(&snapshot("0.1.0"), &next, &Version::parse("0.2.0").unwrap()).is_ok());
        assert!(enforce(&snapshot("0.1.0"), &next, &Version::parse("0.1.5").unwrap()).is_err());
    }
}
