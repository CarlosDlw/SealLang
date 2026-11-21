//! `seal why`: explains why a package, feature, file, or target is selected.

use std::path::Path;
use std::process::ExitCode;

use seal_plan::CompilePlan;
use serde::Serialize;

use crate::cli::{Cli, QueryArgs, WhyArgs};
use crate::ui;

#[derive(Serialize)]
struct WhyReport {
    version: u32,
    project: String,
    entries: Vec<WhyEntry>,
}

#[derive(Serialize)]
struct WhyEntry {
    kind: &'static str,
    name: String,
    package: String,
    version: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    path: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    detail: Option<String>,
}

pub(crate) fn why(cli: &Cli, root: &Path, args: &WhyArgs) -> ExitCode {
    let project = match crate::cli::load(cli, root) {
        Ok(project) => project,
        Err(code) => return code,
    };
    let query = QueryArgs {
        package: args.package.clone(),
        target_triple: args.target_triple.clone(),
        features: args.features.clone(),
        all_features: args.all_features,
        no_default_features: args.no_default_features,
        cfgs: args.cfgs.clone(),
        duplicates: false,
    };
    // A package that appears in the default selection is explained in that
    // plan, so every instance and dependency path is visible. A package
    // outside the default selection becomes the primary for this query.
    let plan = if let Some(name) = args.package.as_deref() {
        let mut default_query = query.clone();
        default_query.package = None;
        let default_plan = match super::tree::compile(&project, &default_query) {
            Ok(plan) => plan,
            Err(error) => return ui::error(error.code(), error.message()),
        };
        if default_plan
            .packages
            .iter()
            .any(|package| package.name == name)
        {
            default_plan
        } else {
            match super::tree::compile(&project, &query) {
                Ok(plan) => plan,
                Err(error) => return ui::error(error.code(), error.message()),
            }
        }
    } else {
        match super::tree::compile(&project, &query) {
            Ok(plan) => plan,
            Err(error) => return ui::error(error.code(), error.message()),
        }
    };
    if let Some(target) = &args.target {
        if !project
            .package(plan.primary)
            .targets
            .iter()
            .any(|candidate| candidate.name == *target)
        {
            return ui::error(
                "E8014",
                &format!(
                    "the package `{}` has no target named `{target}`",
                    project.package(plan.primary).name
                ),
            );
        }
    }
    let report = build_report(&project.name, &plan, args);
    if args.json {
        match serde_json::to_string_pretty(&report) {
            Ok(text) => println!("{text}"),
            Err(error) => {
                return ui::error("E8014", &format!("cannot serialize the report: {error}"));
            }
        }
        return ExitCode::SUCCESS;
    }
    if report.entries.is_empty() {
        println!("no matching selection");
        return ExitCode::SUCCESS;
    }
    for entry in &report.entries {
        let path = if entry.path.is_empty() {
            String::new()
        } else {
            format!(" via {}", entry.path.join(" -> "))
        };
        println!(
            "{} {} {} {}{}{}",
            entry.kind,
            entry.package,
            entry.version,
            entry.name,
            path,
            entry
                .detail
                .as_ref()
                .map(|detail| format!(" ({detail})"))
                .unwrap_or_default()
        );
    }
    ExitCode::SUCCESS
}

fn build_report(project: &str, plan: &CompilePlan, args: &WhyArgs) -> WhyReport {
    let mut entries = Vec::new();
    if let Some(feature) = &args.feature {
        for (index, package) in plan.packages.iter().enumerate() {
            if package.cfg.features.iter().any(|item| item == feature) {
                entries.push(WhyEntry {
                    kind: "feature",
                    name: feature.clone(),
                    package: package.name.clone(),
                    version: package.version.clone(),
                    path: dependency_path(plan, index),
                    detail: Some("enabled for this instance".to_owned()),
                });
            }
        }
    } else if let Some(file) = &args.file {
        for (index, package) in plan.packages.iter().enumerate() {
            for source in &package.files {
                let path = source.path.to_string_lossy();
                if path.ends_with(file) || path == Path::new(file).to_string_lossy() {
                    entries.push(WhyEntry {
                        kind: "file",
                        name: file.clone(),
                        package: package.name.clone(),
                        version: package.version.clone(),
                        path: dependency_path(plan, index),
                        detail: Some(format!("compiled as {}", source.path.display())),
                    });
                    break;
                }
            }
        }
    } else if let Some(target) = &args.target {
        for (index, package) in plan.packages.iter().enumerate() {
            if index == plan.primary {
                entries.push(WhyEntry {
                    kind: "target",
                    name: target.clone(),
                    package: package.name.clone(),
                    version: package.version.clone(),
                    path: dependency_path(plan, index),
                    detail: Some("selected by the command".to_owned()),
                });
            }
        }
    } else {
        for (index, package) in plan.packages.iter().enumerate() {
            if args
                .package
                .as_deref()
                .is_some_and(|name| name != package.name)
            {
                continue;
            }
            entries.push(WhyEntry {
                kind: "package",
                name: package.name.clone(),
                package: package.name.clone(),
                version: package.version.clone(),
                path: dependency_path(plan, index),
                detail: Some(if index == plan.primary {
                    "selected as the primary package".to_owned()
                } else {
                    "required by a dependency edge".to_owned()
                }),
            });
        }
    }
    WhyReport {
        version: 1,
        project: project.to_owned(),
        entries,
    }
}

/// Returns the dependency path from the primary package to `target`.
fn dependency_path(plan: &CompilePlan, target: usize) -> Vec<String> {
    if target == plan.primary {
        return vec![plan.packages[target].name.clone()];
    }
    let mut path = Vec::new();
    if find_path(plan, plan.primary, target, &mut Vec::new(), &mut path) {
        path.reverse();
        path
    } else {
        Vec::new()
    }
}

fn find_path(
    plan: &CompilePlan,
    current: usize,
    target: usize,
    visiting: &mut Vec<usize>,
    path: &mut Vec<String>,
) -> bool {
    if current == target {
        path.push(plan.packages[current].name.clone());
        return true;
    }
    if visiting.contains(&current) {
        return false;
    }
    visiting.push(current);
    for dependency in &plan.packages[current].dependencies {
        if find_path(plan, dependency.package, target, visiting, path) {
            path.push(plan.packages[current].name.clone());
            return true;
        }
    }
    visiting.pop();
    false
}
