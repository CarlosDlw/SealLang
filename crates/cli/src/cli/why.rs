//! `seal why`: explains why a package instance is in the plan.

use std::process::ExitCode;

use seal_plan::CompilePlan;

use crate::cli::{Cli, QueryArgs};
use crate::ui;

pub(crate) fn why(cli: &Cli, root: &std::path::Path, args: &QueryArgs) -> ExitCode {
    let project = match crate::cli::load(root) {
        Ok(project) => project,
        Err(code) => return code,
    };
    let plan = match super::tree::compile_graph(&project, args) {
        Ok(plan) => plan,
        Err(error) => return ui::error(error.code(), error.message()),
    };
    let _ = cli;
    report(&plan, args.package.as_deref());
    ExitCode::SUCCESS
}

fn report(plan: &CompilePlan, package: Option<&str>) {
    for (index, instance) in plan.packages.iter().enumerate() {
        if package.is_some_and(|name| name != instance.name) {
            continue;
        }
        let features = if instance.cfg.features.is_empty() {
            "no features".to_owned()
        } else {
            format!("features {}", instance.cfg.features.join(", "))
        };
        println!(
            "{} {} (instance {:08x}, {features})",
            instance.name, instance.version, instance.instance
        );
        if index == plan.primary {
            println!("  selected as the primary package");
        }
        for parent in &plan.packages {
            for dependency in &parent.dependencies {
                if dependency.package == index {
                    println!(
                        "  required by {} as `{}`{}",
                        parent.name,
                        dependency.alias,
                        features_suffix(&parent.cfg.features)
                    );
                }
            }
        }
        if package.is_some() {
            println!(
                "  source root: {}",
                instance.root.join("seal.toml").display()
            );
        }
    }
}

fn features_suffix(features: &[String]) -> String {
    if features.is_empty() {
        String::new()
    } else {
        format!(" [{}]", features.join(","))
    }
}
