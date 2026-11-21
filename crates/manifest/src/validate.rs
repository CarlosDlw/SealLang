//! Parsing with locations and semantic validation.

use std::collections::BTreeMap;
use std::path::{Component, Path};

use semver::Version;
use toml_edit::{DocumentMut, Item, Table, Value};

use crate::error::{ManifestError, ManifestResult, ManifestWarning};
use crate::{DependencySpec, Location, Manifest, TargetKind};

/// A parsed manifest plus non-fatal diagnostics.
#[derive(Clone, Debug)]
pub struct LoadedManifest {
    /// The typed manifest.
    pub manifest: Manifest,
    /// Unknown keys and other warnings.
    pub warnings: Vec<ManifestWarning>,
}

/// Reads a manifest from disk.
pub fn load(path: &Path) -> ManifestResult<LoadedManifest> {
    let source = std::fs::read_to_string(path)
        .map_err(|error| ManifestError::io(format!("cannot read `{}`: {error}", path.display())))?;
    load_str(&source)
}

/// Parses a manifest from text.
pub fn load_str(source: &str) -> ManifestResult<LoadedManifest> {
    let document: DocumentMut = source.parse().map_err(|error: toml_edit::TomlError| {
        let span = error.span();
        let location = span
            .as_ref()
            .map(|range| Location::from_offset(source, range.start));
        ManifestError::invalid(format!("invalid TOML: {error}")).at(location)
    })?;

    let manifest: Manifest = toml::from_str(source).map_err(|error: toml::de::Error| {
        let location = error
            .span()
            .map(|range| Location::from_offset(source, range.start));
        ManifestError::invalid(format!("invalid manifest: {error}")).at(location)
    })?;

    let index = KeyIndex::build(&document, source);
    let warnings = unknown_keys(&document);
    validate(&manifest, &index)?;
    Ok(LoadedManifest { manifest, warnings })
}

struct KeyIndex {
    locations: BTreeMap<String, Location>,
    source: String,
}

impl KeyIndex {
    fn build(document: &DocumentMut, source: &str) -> Self {
        let mut index = Self {
            locations: BTreeMap::new(),
            source: source.to_owned(),
        };
        index.walk_table(document.as_table(), "");
        index
    }

    fn walk_table(&mut self, table: &Table, prefix: &str) {
        for (name, item) in table.iter() {
            let path = join(prefix, name);
            if let Some(key) = table.key(name) {
                if let Some(span) = key.span() {
                    self.locations.insert(
                        path.clone(),
                        Location::from_offset(&self.source, span.start),
                    );
                }
            }
            match item {
                Item::Table(table) => self.walk_table(table, &path),
                Item::ArrayOfTables(tables) => {
                    for (position, table) in tables.iter().enumerate() {
                        self.walk_table(table, &format!("{path}.{position}"));
                    }
                }
                Item::Value(Value::InlineTable(table)) => {
                    for (key, value) in table.iter() {
                        let child = join(&path, key);
                        if let Some(key) = table.key(key) {
                            if let Some(span) = key.span() {
                                self.locations.insert(
                                    child.clone(),
                                    Location::from_offset(&self.source, span.start),
                                );
                            }
                        }
                        if let Value::InlineTable(nested) = value {
                            self.walk_inline(nested, &child);
                        }
                    }
                }
                Item::Value(_) => {}
                Item::None => {}
            }
        }
    }

    fn walk_inline(&mut self, table: &toml_edit::InlineTable, prefix: &str) {
        for (key, value) in table.iter() {
            let path = join(prefix, key);
            if let Some(key) = table.key(key) {
                if let Some(span) = key.span() {
                    self.locations.insert(
                        path.clone(),
                        Location::from_offset(&self.source, span.start),
                    );
                }
            }
            if let Value::InlineTable(nested) = value {
                self.walk_inline(nested, &path);
            }
        }
    }

    fn location(&self, path: &str) -> Option<Location> {
        self.locations.get(path).copied()
    }

    fn fail(&self, path: &str, message: impl Into<String>) -> ManifestError {
        ManifestError::invalid(message).at(self.location(path))
    }
}

fn join(prefix: &str, key: &str) -> String {
    if prefix.is_empty() {
        key.to_owned()
    } else {
        format!("{prefix}.{key}")
    }
}

fn validate(manifest: &Manifest, index: &KeyIndex) -> ManifestResult<()> {
    if manifest.project.is_none() && manifest.package.is_none() {
        return Err(ManifestError::invalid(
            "a seal.toml needs a `[project]` table, a `[package]` table, or both",
        ));
    }

    if let Some(project) = &manifest.project {
        validate_name("project", &project.name, index, "project.name")?;
        if let Some(language) = &project.language {
            if !matches!(language.as_str(), "0.1" | "0.2") {
                return Err(index.fail(
                    "project.language",
                    format!("the language version `{language}` is not supported; expected `0.1` or `0.2`"),
                ));
            }
        }
        for member in &project.members {
            validate_glob(member, index, "project.members")?;
        }
        for member in &project.default_members {
            validate_glob(member, index, "project.default-members")?;
        }
    }

    if let Some(package) = &manifest.package {
        validate_name("package", &package.name, index, "package.name")?;
        Version::parse(&package.version).map_err(|error| {
            index.fail(
                "package.version",
                format!(
                    "the package version `{}` is not semantic: {error}",
                    package.version
                ),
            )
        })?;
        for source in &package.sources {
            validate_glob(source, index, "package.sources")?;
        }
        for excluded in &package.exclude {
            validate_glob(excluded, index, "package.exclude")?;
        }
        if let Some(api) = &package.api {
            let mut seen = std::collections::BTreeSet::new();
            for exposed in &api.expose {
                validate_entry_path(exposed, index, "package.api.expose")?;
                if !seen.insert(exposed.clone()) {
                    return Err(index.fail(
                        "package.api.expose",
                        format!("the module `{exposed}` is exposed twice"),
                    ));
                }
            }
        }
        for (name, feature) in package.features.values() {
            validate_identifier(name, index, "package.features")?;
            for value in feature {
                validate_feature_request(value, index, "package.features")?;
            }
        }
        for (name, conflicts) in package.features.conflict_map() {
            validate_identifier(name, index, "package.features.conflicts")?;
            for conflict in conflicts {
                validate_identifier(conflict, index, "package.features.conflicts")?;
            }
        }
    }

    let target_paths = manifest
        .targets
        .keys()
        .map(|name| format!("targets.{name}"))
        .collect::<Vec<_>>();
    let mut libraries = 0;
    for (name, target) in &manifest.targets {
        let path = format!("targets.{name}");
        validate_target_name(name, index, &path)?;
        if target.kind == TargetKind::Lib {
            libraries += 1;
        }
        if let Some(entry) = &target.entry {
            validate_entry_path(entry, index, &format!("{path}.entry"))?;
        }
        for source in target.sources.iter().chain(&target.include) {
            validate_glob(source, index, &format!("{path}.sources"))?;
        }
        if let Some(reused) = &target.reuse {
            if !manifest.targets.contains_key(reused) {
                return Err(index.fail(
                    &format!("{path}.reuse"),
                    format!("the `{name}` target reuses the unknown target `{reused}`"),
                ));
            }
        }
        for key in target.env.keys() {
            validate_identifier(key, index, &format!("{path}.env"))?;
        }
        if let Some(runtime) = &target.runtime {
            if !matches!(runtime.as_str(), "default" | "none") {
                return Err(index.fail(
                    &format!("{path}.runtime"),
                    format!("the runtime policy `{runtime}` is not `default` or `none`"),
                ));
            }
        }
        if target.kind == TargetKind::Generator {
            if target.outputs.is_empty() {
                return Err(index.fail(
                    &format!("{path}.outputs"),
                    "a generator target must declare the outputs it produces",
                ));
            }
            for output in &target.outputs {
                validate_glob(output, index, &format!("{path}.outputs"))?;
                if !output.contains('/') {
                    return Err(index.fail(
                        &format!("{path}.outputs"),
                        format!(
                            "the generator output `{output}` must live in a directory so undeclared files can be detected"
                        ),
                    ));
                }
            }
            for input in &target.inputs {
                validate_glob(input, index, &format!("{path}.inputs"))?;
            }
        } else if !target.inputs.is_empty() || !target.outputs.is_empty() {
            return Err(index.fail(
                &path,
                format!(
                    "only a generator target can declare `inputs` and `outputs`; `{name}` is `{}`",
                    target.kind.as_str()
                ),
            ));
        }
        for generated in &target.generated {
            match manifest.targets.get(generated) {
                Some(declaration) if declaration.kind == TargetKind::Generator => {}
                Some(_) => {
                    return Err(index.fail(
                        &format!("{path}.generated"),
                        format!("`{generated}` is not a generator target"),
                    ));
                }
                None => {
                    return Err(index.fail(
                        &format!("{path}.generated"),
                        format!("the `{name}` target names the unknown generator `{generated}`"),
                    ));
                }
            }
            if target.kind == TargetKind::Generator {
                return Err(index.fail(
                    &format!("{path}.generated"),
                    "a generator target cannot consume another generator's outputs",
                ));
            }
        }
        for alias in &target.native {
            if !manifest.native.contains_key(alias) {
                return Err(index.fail(
                    &format!("{path}.native"),
                    format!("the `{name}` target names the unknown native dependency `{alias}`"),
                ));
            }
        }
    }

    for (alias, native) in &manifest.native {
        let path = format!("native.{alias}");
        validate_identifier(alias, index, &path)?;
        if native.is_empty() {
            return Err(index.fail(
                &path,
                format!("the native dependency `{alias}` declares no pkg-config package, library, or search path"),
            ));
        }
        if let Some(package) = &native.pkg_config {
            validate_program_text(
                package,
                index,
                &format!("{path}.pkg-config"),
                "pkg-config package",
            )?;
        }
        for library in &native.libs {
            validate_library(library, index, &format!("{path}.libs"))?;
        }
        for search in &native.search_paths {
            validate_native_path(search, index, &format!("{path}.search-paths"))?;
        }
    }

    for (name, task) in &manifest.tasks {
        let path = format!("tasks.{name}");
        validate_identifier(name, index, &path)?;
        if task.steps.is_empty() {
            return Err(index.fail(&path, format!("the task `{name}` has no steps")));
        }
        for (position, step) in task.steps.iter().enumerate() {
            let step_path = format!("{path}.steps.{position}");
            validate_program_text(
                &step.program,
                index,
                &format!("{step_path}.program"),
                "program",
            )?;
            for argument in &step.args {
                if argument.contains('\0') {
                    return Err(index.fail(
                        &format!("{step_path}.args"),
                        "an argument cannot contain a NUL byte",
                    ));
                }
            }
            for key in step.env.keys() {
                validate_identifier(key, index, &format!("{step_path}.env"))?;
            }
            if let Some(directory) = &step.cwd {
                validate_relative_path(directory, index, &format!("{step_path}.cwd"))?;
            }
            if step.timeout == Some(0) {
                return Err(index.fail(
                    &format!("{step_path}.timeout"),
                    "a step timeout must be at least one second",
                ));
            }
        }
    }
    if libraries > 1 {
        let library_path = target_paths
            .iter()
            .find(|path| {
                path.rsplit('.')
                    .next()
                    .and_then(|name| manifest.targets.get(name))
                    .is_some_and(|target| target.kind == TargetKind::Lib)
            })
            .cloned()
            .unwrap_or_else(|| "targets".to_owned());
        return Err(index.fail(
            &library_path,
            "a package can declare at most one `lib` target",
        ));
    }

    for (section, dependencies) in [
        ("dependencies", &manifest.dependencies),
        ("dev-dependencies", &manifest.dev_dependencies),
        ("build-dependencies", &manifest.build_dependencies),
    ] {
        for (alias, dependency) in dependencies {
            validate_identifier(alias, index, section)?;
            let path = format!("{section}.{alias}");
            if let DependencySpec::Version(version) = dependency {
                semver::VersionReq::parse(version).map_err(|_| {
                    index.fail(
                        &path,
                        format!("the dependency version `{version}` is not valid"),
                    )
                })?;
            }
            if let DependencySpec::Detailed(detail) = dependency {
                if detail.path.is_none() && detail.version.is_none() && detail.git.is_none() {
                    return Err(index.fail(
                        &path,
                        format!("the dependency `{alias}` needs a `path`, `version`, or `git`"),
                    ));
                }
                if let Some(path_value) = &detail.path {
                    if path_value.as_os_str().is_empty() {
                        return Err(
                            index.fail(&format!("{path}.path"), "the dependency path is empty")
                        );
                    }
                }
            }
        }
    }

    for (name, value) in &manifest.env {
        validate_identifier(name, index, "env")?;
        if value.value().contains('\0') {
            return Err(index.fail(
                &format!("env.{name}"),
                format!("the environment value `{name}` contains a NUL byte"),
            ));
        }
    }

    fn profile_known(manifest: &Manifest, name: &str) -> bool {
        matches!(name, "dev" | "release" | "test" | "bench") || manifest.profiles.contains_key(name)
    }

    for (name, profile) in &manifest.profiles {
        validate_identifier(name, index, "profiles")?;
        let path = format!("profiles.{name}");
        if let Some(inherited) = &profile.inherits {
            if !profile_known(manifest, inherited) {
                return Err(index.fail(
                    &format!("{path}.inherits"),
                    format!("the profile `{name}` inherits the unknown profile `{inherited}`"),
                ));
            }
        }
        if let Some(level) = profile.opt_level {
            if level > 3 {
                return Err(index.fail(
                    &format!("{path}.opt-level"),
                    format!("the optimization level {level} is outside 0 through 3"),
                ));
            }
        }
        if profile.codegen_units == Some(0) {
            return Err(index.fail(
                &format!("{path}.codegen-units"),
                "the profile requests zero code generation units",
            ));
        }
        if let Some(lto) = &profile.lto {
            if !matches!(lto.as_str(), "off" | "thin" | "full") {
                return Err(index.fail(
                    &format!("{path}.lto"),
                    format!("the LTO mode `{lto}` is not `off`, `thin`, or `full`"),
                ));
            }
        }
    }

    Ok(())
}

fn validate_name(kind: &str, name: &str, index: &KeyIndex, path: &str) -> ManifestResult<()> {
    let valid = if let Some(rest) = name.strip_prefix('@') {
        match rest.split_once('/') {
            Some((scope, package)) => valid_simple_name(scope) && valid_simple_name(package),
            None => false,
        }
    } else {
        valid_simple_name(name)
    };
    if valid {
        Ok(())
    } else {
        Err(index.fail(
            path,
            format!(
                "`{name}` is not a valid {kind} name; use lowercase letters, digits, `-`, `_`, and at most one `@scope/` prefix"
            ),
        ))
    }
}

fn valid_simple_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-' || byte == b'_'
        })
        && name.as_bytes()[0].is_ascii_lowercase()
}

fn validate_target_name(name: &str, index: &KeyIndex, path: &str) -> ManifestResult<()> {
    if valid_simple_name(name) {
        Ok(())
    } else {
        Err(index.fail(
            path,
            format!(
                "`{name}` is not a valid target name; use lowercase letters, digits, `-`, and `_`"
            ),
        ))
    }
}

fn validate_feature_request(value: &str, index: &KeyIndex, path: &str) -> ManifestResult<()> {
    if let Some(dependency) = value.strip_prefix("dep:") {
        return validate_identifier(dependency, index, path);
    }
    if let Some((dependency, feature)) = value.split_once('/') {
        let dependency = dependency.strip_suffix('?').unwrap_or(dependency);
        if dependency.is_empty() || feature.is_empty() || feature.contains('/') {
            return Err(index.fail(
                path,
                format!("`{value}` is not `dependency/feature` or `dependency?/feature`"),
            ));
        }
        validate_identifier(dependency, index, path)?;
        return validate_identifier(feature, index, path);
    }
    validate_identifier(value, index, path)
}

fn validate_identifier(name: &str, index: &KeyIndex, path: &str) -> ManifestResult<()> {
    let mut characters = name.chars();
    let valid = characters
        .next()
        .is_some_and(|character| character == '_' || character.is_ascii_alphabetic())
        && characters.all(|character| character == '_' || character.is_ascii_alphanumeric());
    if valid {
        Ok(())
    } else {
        Err(index.fail(path, format!("`{name}` is not a valid identifier")))
    }
}

fn validate_entry_path(entry: &str, index: &KeyIndex, path: &str) -> ManifestResult<()> {
    if entry.is_empty() || entry.split("::").any(str::is_empty) {
        return Err(index.fail(
            path,
            format!("`{entry}` is not an entry path; write a module path such as `root::main`"),
        ));
    }
    Ok(())
}

fn validate_library(name: &str, index: &KeyIndex, path: &str) -> ManifestResult<()> {
    if name.is_empty()
        || name.contains(['/', '\\'])
        || name
            .chars()
            .any(|character| character.is_whitespace() || character.is_control())
    {
        return Err(index.fail(path, format!("`{name}` is not a valid library name")));
    }
    Ok(())
}

fn validate_program_text(
    text: &str,
    index: &KeyIndex,
    path: &str,
    kind: &str,
) -> ManifestResult<()> {
    if text.trim().is_empty() {
        return Err(index.fail(path, format!("a {kind} cannot be empty")));
    }
    if text.chars().any(char::is_control) {
        return Err(index.fail(path, format!("a {kind} cannot contain control characters")));
    }
    Ok(())
}

fn validate_native_path(path_value: &Path, index: &KeyIndex, path: &str) -> ManifestResult<()> {
    if path_value.as_os_str().is_empty() {
        return Err(index.fail(path, "a search path cannot be empty"));
    }
    if path_value.is_absolute() {
        return Ok(());
    }
    validate_relative_path(path_value, index, path)
}

fn validate_relative_path(path_value: &Path, index: &KeyIndex, path: &str) -> ManifestResult<()> {
    if path_value.as_os_str().is_empty() {
        return Err(index.fail(path, "a path cannot be empty"));
    }
    if path_value.is_absolute()
        || path_value
            .components()
            .any(|component| matches!(component, Component::ParentDir | Component::Prefix(_)))
    {
        return Err(index.fail(
            path,
            format!(
                "the path `{}` must stay inside the package",
                path_value.display()
            ),
        ));
    }
    Ok(())
}

fn validate_glob(glob: &str, index: &KeyIndex, path: &str) -> ManifestResult<()> {
    if glob.is_empty() {
        return Err(index.fail(path, "a source glob cannot be empty"));
    }
    let candidate = Path::new(glob);
    if candidate.is_absolute() {
        return Err(index.fail(path, format!("the glob `{glob}` must be relative")));
    }
    if candidate
        .components()
        .any(|component| matches!(component, Component::ParentDir | Component::Prefix(_)))
    {
        return Err(index.fail(
            path,
            format!("the glob `{glob}` cannot leave the project directory"),
        ));
    }
    Ok(())
}

// Unknown-key detection. Every known table is checked independently so a typo
// points at the exact key and suggests the intended one.

fn unknown_keys(document: &DocumentMut) -> Vec<ManifestWarning> {
    let mut warnings = Vec::new();
    let root = [
        "project",
        "package",
        "targets",
        "dependencies",
        "dev-dependencies",
        "build-dependencies",
        "env",
        "profiles",
        "toolchain",
        "registry",
        "tasks",
        "native",
    ];
    check_static(document.as_table(), &root, "", &mut warnings);

    if let Some(project) = table(document.as_table(), "project") {
        check_static(
            project,
            &["name", "language", "members", "default-members", "metadata"],
            "project",
            &mut warnings,
        );
    }
    if let Some(package) = table(document.as_table(), "package") {
        check_static(
            package,
            &[
                "name",
                "version",
                "description",
                "license",
                "repository",
                "readme",
                "sources",
                "exclude",
                "api",
                "features",
                "metadata",
            ],
            "package",
            &mut warnings,
        );
        if let Some(api) = table(package, "api") {
            check_static(
                api,
                &["expose", "all-features"],
                "package.api",
                &mut warnings,
            );
        }
    }
    if let Some(targets) = table(document.as_table(), "targets") {
        for (name, item) in targets.iter() {
            if let Some(target) = item.as_table_like() {
                check_table_like(
                    target,
                    &[
                        "kind",
                        "entry",
                        "sources",
                        "include",
                        "reuse",
                        "required-features",
                        "when",
                        "env",
                        "profile",
                        "runtime",
                        "panic",
                        "linker",
                        "sysroot",
                        "linker-script",
                        "link-args",
                        "output",
                        "inputs",
                        "outputs",
                        "generated",
                        "native",
                    ],
                    &format!("targets.{name}"),
                    &mut warnings,
                );
            }
        }
    }
    for section in ["dependencies", "dev-dependencies", "build-dependencies"] {
        if let Some(dependencies) = table(document.as_table(), section) {
            for (name, item) in dependencies.iter() {
                if let Some(detail) = item.as_table_like() {
                    check_table_like(
                        detail,
                        &[
                            "version",
                            "path",
                            "git",
                            "features",
                            "default-features",
                            "optional",
                            "when",
                            "registry",
                        ],
                        &format!("{section}.{name}"),
                        &mut warnings,
                    );
                }
            }
        }
    }
    if let Some(env) = table(document.as_table(), "env") {
        for (name, item) in env.iter() {
            if let Some(detail) = item.as_table_like() {
                check_table_like(
                    detail,
                    &["value", "tracked", "secret"],
                    &format!("env.{name}"),
                    &mut warnings,
                );
            }
        }
    }
    if let Some(native) = table(document.as_table(), "native") {
        for (name, item) in native.iter() {
            if let Some(detail) = item.as_table_like() {
                check_table_like(
                    detail,
                    &["pkg-config", "libs", "search-paths", "static"],
                    &format!("native.{name}"),
                    &mut warnings,
                );
            }
        }
    }
    if let Some(tasks) = table(document.as_table(), "tasks") {
        for (name, item) in tasks.iter() {
            if let Some(task) = item.as_table_like() {
                check_table_like(
                    task,
                    &["description", "steps"],
                    &format!("tasks.{name}"),
                    &mut warnings,
                );
                if let Some(steps) = task.get("steps").and_then(|item| item.as_array_of_tables()) {
                    for (position, step) in steps.iter().enumerate() {
                        check_table_like(
                            step,
                            &["program", "args", "env", "cwd", "timeout"],
                            &format!("tasks.{name}.steps.{position}"),
                            &mut warnings,
                        );
                    }
                }
            }
        }
    }
    if let Some(profiles) = table(document.as_table(), "profiles") {
        for (name, item) in profiles.iter() {
            if let Some(profile) = item.as_table_like() {
                check_table_like(
                    profile,
                    &[
                        "inherits",
                        "opt-level",
                        "debug",
                        "debug-assertions",
                        "lto",
                        "codegen-units",
                        "strip",
                        "incremental",
                    ],
                    &format!("profiles.{name}"),
                    &mut warnings,
                );
            }
        }
    }
    warnings
}

fn table<'a>(table: &'a Table, key: &str) -> Option<&'a Table> {
    table.get(key).and_then(Item::as_table)
}

fn check_static(
    table: &Table,
    allowed: &[&str],
    prefix: &str,
    warnings: &mut Vec<ManifestWarning>,
) {
    for (key, item) in table.iter() {
        if item.is_table() || item.is_array_of_tables() || item.is_inline_table() {
            continue;
        }
        if !allowed.contains(&key) {
            warnings.push(unknown_warning(key, allowed, table, prefix));
        }
    }
}

fn check_table_like(
    table: &dyn toml_edit::TableLike,
    allowed: &[&str],
    prefix: &str,
    warnings: &mut Vec<ManifestWarning>,
) {
    for (key, _) in table.iter() {
        if !allowed.contains(&key) {
            warnings.push(ManifestWarning {
                message: format!("unknown key `{key}` in `{prefix}`"),
                location: None,
                suggestion: nearest(key, allowed),
            });
        }
    }
}

fn unknown_warning(key: &str, allowed: &[&str], _table: &Table, prefix: &str) -> ManifestWarning {
    ManifestWarning {
        message: if prefix.is_empty() {
            format!("unknown key `{key}`")
        } else {
            format!("unknown key `{key}` in `{prefix}`")
        },
        location: None,
        suggestion: nearest(key, allowed),
    }
}

fn nearest(key: &str, candidates: &[&str]) -> Option<String> {
    candidates
        .iter()
        .map(|candidate| (distance(key, candidate), *candidate))
        .filter(|(distance, _)| *distance <= 3)
        .min_by_key(|(distance, _)| *distance)
        .map(|(_, candidate)| candidate.to_owned())
}

fn distance(left: &str, right: &str) -> usize {
    let right: Vec<char> = right.chars().collect();
    let mut previous: Vec<usize> = (0..=right.len()).collect();
    let mut current = vec![0; right.len() + 1];
    for (row, left_character) in left.chars().enumerate() {
        current[0] = row + 1;
        for (column, right_character) in right.iter().enumerate() {
            let substitution = previous[column] + usize::from(left_character != *right_character);
            current[column + 1] = (previous[column + 1] + 1)
                .min(current[column] + 1)
                .min(substitution);
        }
        std::mem::swap(&mut previous, &mut current);
    }
    previous[right.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn load(text: &str) -> LoadedManifest {
        load_str(text).expect("the manifest is valid")
    }

    #[test]
    fn a_minimal_package_manifest_is_valid() {
        let manifest = load("[package]\nname = \"demo\"\nversion = \"0.1.0\"\n");
        assert_eq!(manifest.manifest.package.unwrap().name, "demo");
    }

    #[test]
    fn unknown_keys_are_warnings_with_a_suggestion() {
        let manifest = load("[package]\nname = \"demo\"\nversion = \"0.1.0\"\nversoin = \"1\"\n");
        assert_eq!(manifest.warnings.len(), 1);
        assert_eq!(manifest.warnings[0].suggestion.as_deref(), Some("version"));
    }

    #[test]
    fn invalid_toml_reports_a_location() {
        let error = load_str("[package\nname = \"demo\"").unwrap_err();
        assert_eq!(error.code(), "E8005");
        assert!(error.location().is_some());
    }

    #[test]
    fn a_package_name_must_be_a_lowercase_identifier() {
        let error = load_str("[package]\nname = \"Demo!\"\nversion = \"0.1.0\"\n").unwrap_err();
        assert!(error.message().contains("not a valid package name"));
    }

    #[test]
    fn an_entry_less_bin_is_left_for_the_project_diagnostic() {
        let manifest = load(
            "[package]\nname = \"demo\"\nversion = \"0.1.0\"\n[targets.main]\nkind = \"bin\"\n",
        );
        assert!(manifest.manifest.targets["main"].entry.is_none());
    }

    #[test]
    fn a_glob_cannot_leave_the_project() {
        let error = load_str(
            "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nsources = [\"../src/*.sl\"]\n",
        )
        .unwrap_err();
        assert!(error.message().contains("cannot leave"));
    }

    #[test]
    fn a_profile_with_an_invalid_optimization_level_is_rejected() {
        let error = load_str(
            "[package]\nname = \"demo\"\nversion = \"0.1.0\"\n[profiles.fast]\nopt-level = 9\n",
        )
        .unwrap_err();
        assert!(error.message().contains("outside 0 through 3"));
    }

    #[test]
    fn a_package_cannot_declare_two_library_targets() {
        let error = load_str(
            "[package]\nname = \"demo\"\nversion = \"0.1.0\"\n\n\
             [targets.one]\nkind = \"lib\"\n\n[targets.two]\nkind = \"lib\"\n",
        )
        .unwrap_err();
        assert!(error.message().contains("at most one"));
    }

    #[test]
    fn generator_native_and_task_tables_are_validated_together() {
        let manifest = load(
            r#"
[package]
name = "demo"
version = "0.1.0"

[native.math]
libs = ["m"]
search-paths = ["/usr/lib"]

[targets.gen]
kind = "generator"
entry = "root::main"
sources = ["src/gen.sl"]
inputs = ["data/*.txt"]
outputs = ["generated/*.sl"]

[targets.app]
kind = "bin"
entry = "root::main"
generated = ["gen"]
native = ["math"]

[tasks.ci]
steps = [
  { program = "echo", args = ["ok"], cwd = "." },
]
"#,
        );
        assert_eq!(manifest.manifest.native.len(), 1);
        assert_eq!(manifest.manifest.tasks.len(), 1);
        assert_eq!(
            manifest.manifest.targets["gen"].outputs,
            vec!["generated/*.sl".to_owned()]
        );
    }

    #[test]
    fn a_generator_without_outputs_is_rejected() {
        let error = load_str(
            "[package]\nname = \"demo\"\nversion = \"0.1.0\"\n\n\
             [targets.gen]\nkind = \"generator\"\nentry = \"root::main\"\n",
        )
        .unwrap_err();
        assert!(error.message().contains("must declare the outputs"));
    }

    #[test]
    fn only_a_generator_may_declare_inputs_and_outputs() {
        let error = load_str(
            "[package]\nname = \"demo\"\nversion = \"0.1.0\"\n\n\
             [targets.app]\nkind = \"bin\"\nentry = \"root::main\"\noutputs = [\"generated/*.sl\"]\n",
        )
        .unwrap_err();
        assert!(error.message().contains("only a generator"));
    }

    #[test]
    fn generated_and_native_references_must_exist() {
        let generated = load_str(
            "[package]\nname = \"demo\"\nversion = \"0.1.0\"\n\n\
             [targets.app]\nkind = \"bin\"\nentry = \"root::main\"\ngenerated = [\"missing\"]\n",
        )
        .unwrap_err();
        assert!(generated.message().contains("unknown generator"));

        let native = load_str(
            "[package]\nname = \"demo\"\nversion = \"0.1.0\"\n\n\
             [targets.app]\nkind = \"bin\"\nentry = \"root::main\"\nnative = [\"missing\"]\n",
        )
        .unwrap_err();
        assert!(native.message().contains("unknown native dependency"));
    }

    #[test]
    fn an_empty_native_declaration_or_task_is_rejected() {
        let native =
            load_str("[package]\nname = \"demo\"\nversion = \"0.1.0\"\n\n[native.empty]\n")
                .unwrap_err();
        assert!(native.message().contains("declares no"));

        let task =
            load_str("[package]\nname = \"demo\"\nversion = \"0.1.0\"\n\n[tasks.ci]\nsteps = []\n")
                .unwrap_err();
        assert!(task.message().contains("has no steps"));
    }

    #[test]
    fn a_native_library_name_cannot_smuggle_a_path() {
        let error = load_str(
            "[package]\nname = \"demo\"\nversion = \"0.1.0\"\n\n[native.bad]\nlibs = [\"../lib\"]\n",
        )
        .unwrap_err();
        assert!(error.message().contains("not a valid library name"));
    }

    #[test]
    fn an_unknown_target_profile_is_resolved_at_plan_time() {
        let manifest = load_str(
            "[package]\nname = \"demo\"\nversion = \"0.1.0\"\n[targets.main]\nkind = \"bin\"\nentry = \"root::main\"\nprofile = \"missing\"\n",
        )
        .expect("the manifest itself is valid");
        assert!(manifest.manifest.targets.contains_key("main"));
    }
}
