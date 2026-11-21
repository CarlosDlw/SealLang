//! Target discovery and source selection.

use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};

use seal_manifest::{Manifest, TargetKind, TargetManifest};

use crate::Target;
use crate::error::{ProjectError, ProjectResult};
use crate::source;

/// Builds the targets of one package.
pub(crate) fn build(
    root: &Path,
    manifest: &Manifest,
    files: &[PathBuf],
) -> ProjectResult<Vec<Target>> {
    let package_name = manifest
        .package
        .as_ref()
        .map(|package| package.name.clone())
        .ok_or_else(|| ProjectError::manifest("a package manifest has no `[package]` table"))?;

    if manifest.targets.is_empty() {
        return infer(root, &package_name, files);
    }

    let exclude = manifest
        .package
        .as_ref()
        .map_or(&[][..], |package| package.exclude.as_slice());
    let mut targets = Vec::with_capacity(manifest.targets.len());
    for (name, declaration) in &manifest.targets {
        targets.push(build_one(root, name, declaration, files, exclude)?);
    }
    reject_overlapping_target_sources(&targets, manifest)?;
    Ok(targets)
}

fn reject_overlapping_target_sources(targets: &[Target], manifest: &Manifest) -> ProjectResult<()> {
    let explicit: Vec<(&Target, BTreeSet<PathBuf>)> = targets
        .iter()
        .filter_map(|target| {
            let declaration = manifest.targets.get(&target.name)?;
            if declaration.sources.is_empty()
                || matches!(
                    target.kind,
                    TargetKind::Test | TargetKind::Bench | TargetKind::Example
                )
            {
                return None;
            }
            Some((target, target.files.iter().cloned().collect()))
        })
        .collect();
    for (index, (left, left_files)) in explicit.iter().enumerate() {
        for (right, right_files) in explicit.iter().skip(index + 1) {
            if let Some(file) = left_files.intersection(right_files).next() {
                return Err(ProjectError::target(format!(
                    "the targets `{}` and `{}` both claim `{}`; share files through `include` on a test, bench, or example",
                    left.name,
                    right.name,
                    file.display()
                )));
            }
        }
    }
    Ok(())
}

fn infer(root: &Path, package_name: &str, files: &[PathBuf]) -> ProjectResult<Vec<Target>> {
    let main = root.join("src/main.sl");
    if main.is_file() {
        let primary = source::inside(root, &main)?;
        return Ok(vec![Target {
            name: package_name.to_owned(),
            kind: TargetKind::Bin,
            entry: Some("root::main".to_owned()),
            files: ordered(files, &primary),
            primary,
            output: None,
            profile: None,
            runtime: None,
            panic: None,
            linker: None,
            sysroot: None,
            linker_script: None,
            link_args: Vec::new(),
            required_features: Vec::new(),
            when: None,
            generator_inputs: Vec::new(),
            generator_outputs: Vec::new(),
            generated: Vec::new(),
            native: Vec::new(),
        }]);
    }
    let lib = root.join("src/lib.sl");
    if lib.is_file() {
        let primary = source::inside(root, &lib)?;
        return Ok(vec![Target {
            name: package_name.to_owned(),
            kind: TargetKind::Lib,
            entry: None,
            files: ordered(files, &primary),
            primary,
            output: None,
            profile: None,
            runtime: None,
            panic: None,
            linker: None,
            sysroot: None,
            linker_script: None,
            link_args: Vec::new(),
            required_features: Vec::new(),
            when: None,
            generator_inputs: Vec::new(),
            generator_outputs: Vec::new(),
            generated: Vec::new(),
            native: Vec::new(),
        }]);
    }
    Err(ProjectError::target(format!(
        "the package `{package_name}` has no target: create `src/main.sl` for a program or `src/lib.sl` for a library"
    )))
}

fn build_one(
    root: &Path,
    name: &str,
    declaration: &TargetManifest,
    base: &[PathBuf],
    exclude: &[String],
) -> ProjectResult<Target> {
    if matches!(
        declaration.kind,
        TargetKind::Lib | TargetKind::Staticlib | TargetKind::Cdylib
    ) && declaration.entry.is_some()
    {
        return Err(ProjectError::target(format!(
            "the library target `{name}` cannot name an entry function"
        )));
    }
    if declaration.kind.needs_entry() && declaration.entry.is_none() {
        return Err(ProjectError::entry(format!(
            "the `{name}` target needs an `entry` function, such as `root::main`"
        )));
    }
    if !declaration.include.is_empty()
        && !matches!(
            declaration.kind,
            TargetKind::Test | TargetKind::Bench | TargetKind::Example
        )
    {
        return Err(ProjectError::target(format!(
            "the target `{name}` can only use `include` when it is a test, bench, or example"
        )));
    }

    let mut selected: BTreeSet<PathBuf> = if declaration.sources.is_empty() {
        base.iter().cloned().collect()
    } else {
        source::target_files(root, &declaration.sources, exclude)?
            .into_iter()
            .collect()
    };
    if !declaration.include.is_empty() {
        selected.extend(source::target_files(root, &declaration.include, exclude)?);
    }
    if selected.is_empty() {
        return Err(ProjectError::source(format!(
            "the target `{name}` has no source files"
        )));
    }

    let primary = choose_primary(root, name, declaration.kind, &selected)?;
    selected.insert(primary.clone());
    let files = ordered(&selected.into_iter().collect::<Vec<_>>(), &primary);

    let output = match &declaration.output {
        Some(path) => {
            if path.is_absolute()
                || path.components().any(|component| {
                    matches!(component, Component::ParentDir | Component::Prefix(_))
                })
            {
                return Err(ProjectError::target(format!(
                    "the output path of `{name}` must stay inside the project"
                )));
            }
            Some(path.clone())
        }
        None => None,
    };
    let linker_script = declaration
        .linker_script
        .as_ref()
        .map(|script| root.join(script));
    let generator_inputs = if declaration.kind == TargetKind::Generator {
        source::input_files(root, &declaration.inputs, exclude)?
    } else {
        Vec::new()
    };
    if declaration
        .runtime
        .as_deref()
        .is_some_and(|runtime| !matches!(runtime, "default" | "none"))
    {
        return Err(ProjectError::target(format!(
            "the target `{name}` declares the unknown runtime policy `{}`",
            declaration.runtime.as_deref().unwrap_or_default()
        )));
    }
    if declaration
        .panic
        .as_deref()
        .is_some_and(|panic| !matches!(panic, "abort" | "trap"))
    {
        return Err(ProjectError::target(format!(
            "the target `{name}` declares the unknown panic policy `{}`",
            declaration.panic.as_deref().unwrap_or_default()
        )));
    }
    if declaration.runtime.as_deref() == Some("none")
        && declaration.panic.as_deref() == Some("abort")
    {
        return Err(ProjectError::target(format!(
            "the freestanding target `{name}` must use `panic = \"trap\"`"
        )));
    }
    if declaration
        .link_args
        .iter()
        .any(|argument| argument.is_empty() || argument.contains('\0'))
    {
        return Err(ProjectError::target(format!(
            "the target `{name}` declares an empty or NUL-containing link argument"
        )));
    }

    Ok(Target {
        name: name.to_owned(),
        kind: declaration.kind,
        entry: declaration.entry.clone(),
        files,
        primary,
        output,
        profile: declaration.profile.clone(),
        runtime: declaration.runtime.clone(),
        panic: declaration.panic.clone(),
        linker: declaration.linker.clone(),
        sysroot: declaration
            .sysroot
            .as_ref()
            .map(|sysroot| root.join(sysroot)),
        linker_script,
        link_args: declaration.link_args.clone(),
        required_features: declaration.required_features.clone(),
        when: declaration.when.clone(),
        generator_inputs,
        generator_outputs: declaration.outputs.clone(),
        generated: declaration.generated.clone(),
        native: declaration.native.clone(),
    })
}

fn choose_primary(
    root: &Path,
    name: &str,
    kind: TargetKind,
    selected: &BTreeSet<PathBuf>,
) -> ProjectResult<PathBuf> {
    let candidates: Vec<PathBuf> = match kind {
        TargetKind::Lib | TargetKind::Staticlib | TargetKind::Cdylib => {
            vec![root.join("src/lib.sl")]
        }
        TargetKind::Bin => {
            vec![
                root.join("src/main.sl"),
                root.join("src").join(format!("{name}.sl")),
            ]
        }
        TargetKind::Test => vec![
            root.join("tests").join(format!("{name}.sl")),
            root.join("src/main.sl"),
        ],
        TargetKind::Bench => vec![
            root.join("benches").join(format!("{name}.sl")),
            root.join("src/main.sl"),
        ],
        TargetKind::Example => vec![
            root.join("examples").join(format!("{name}.sl")),
            root.join("src/main.sl"),
        ],
        TargetKind::Generator => vec![root.join("src/main.sl")],
    };
    for candidate in candidates {
        if candidate.is_file() {
            let canonical = source::inside(root, &candidate)?;
            if selected.contains(&canonical) {
                return Ok(canonical);
            }
        }
    }
    selected
        .iter()
        .next()
        .cloned()
        .ok_or_else(|| ProjectError::target(format!("the target `{name}` has no primary source")))
}

fn ordered(files: &[PathBuf], primary: &Path) -> Vec<PathBuf> {
    let mut ordered = Vec::with_capacity(files.len());
    ordered.push(primary.to_path_buf());
    for file in files {
        if file != primary && !ordered.contains(file) {
            ordered.push(file.clone());
        }
    }
    ordered
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_primary_file_comes_first() {
        let files = vec![
            PathBuf::from("/a.sl"),
            PathBuf::from("/b.sl"),
            PathBuf::from("/c.sl"),
        ];
        assert_eq!(
            ordered(&files, Path::new("/b.sl")),
            vec![
                PathBuf::from("/b.sl"),
                PathBuf::from("/a.sl"),
                PathBuf::from("/c.sl")
            ]
        );
    }
}
