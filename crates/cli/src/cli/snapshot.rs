//! Generating API snapshots through the compiler driver.

use std::process::ExitCode;
use std::sync::atomic::{AtomicU64, Ordering};

use seal_api::ApiSnapshot;
use seal_plan::BuildMode;
use seal_project::{PlanOptions, Project};

use crate::cli::{Cli, MessageFormat};
use crate::ui;

static NEXT: AtomicU64 = AtomicU64::new(0);

/// Generates the public API snapshot of one package.
pub(crate) fn generate(
    cli: &Cli,
    project: &Project,
    package: Option<&str>,
) -> Result<ApiSnapshot, ExitCode> {
    let index = match package {
        Some(name) => project
            .package_by_name(name)
            .map(|(index, _)| index)
            .ok_or_else(|| {
                ui::error(
                    "E8090",
                    &format!("the project has no package named `{name}`"),
                )
            })?,
        None => project
            .default_package()
            .map_err(|error| ui::error(error.code(), error.message()))?,
    };
    let package = project.package(index);
    let target = package.library().ok_or_else(|| {
        ui::error(
            "E8090",
            &format!(
                "the package `{}` has no `lib` target to snapshot",
                package.name
            ),
        )
    })?;
    let sequence = NEXT.fetch_add(1, Ordering::Relaxed);
    let base = std::env::temp_dir().join(format!("seal-api-{}-{sequence}", std::process::id()));
    let plan_path = base.with_extension("plan.json");
    let api_path = base.with_extension("api.json");
    let options = PlanOptions {
        target: seal_target::host_triple().as_str().to_owned(),
        profile: "dev".to_owned(),
        profile_explicit: false,
        mode: BuildMode::Check,
        output: Some(api_path.clone()),
        toolchain: seal_toolchain::EMBEDDED_VERSION.to_owned(),
        ..PlanOptions::default()
    };
    let plan = project
        .compile_plan(index, &target.name, &options)
        .map_err(|error| ui::error(error.code(), error.message()))?;
    plan.write(&plan_path)
        .map_err(|error| ui::error(error.code(), error.message()))?;
    let mut arguments = vec![
        "sealc".to_owned(),
        "build".to_owned(),
        "--plan-file".to_owned(),
        plan_path.display().to_string(),
        "--emit=api".to_owned(),
        "-o".to_owned(),
        api_path.display().to_string(),
    ];
    if cli.message_format == MessageFormat::Json {
        arguments.push("--error-format=json".to_owned());
    }
    arguments.push(format!(
        "--color={}",
        match cli.color {
            crate::cli::ColorChoice::Auto => "auto",
            crate::cli::ColorChoice::Always => "always",
            crate::cli::ColorChoice::Never => "never",
        }
    ));
    let status = seal_driver::run_from(arguments);
    let result = if status == ExitCode::SUCCESS {
        ApiSnapshot::read(&api_path).map_err(|error| ui::error(error.code(), error.message()))
    } else {
        Err(status)
    };
    let _ = std::fs::remove_file(&plan_path);
    let _ = std::fs::remove_file(&api_path);
    result
}
