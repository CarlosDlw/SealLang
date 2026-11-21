//! `seal info`: every published version and dependency of one package.

use std::path::Path;
use std::process::ExitCode;

use crate::cli::{Cli, InfoArgs};
use crate::ui;

pub(crate) fn run(cli: &Cli, root: &Path, args: &InfoArgs) -> ExitCode {
    let session = match crate::cli::registry::open(cli, root) {
        Ok(session) => session,
        Err(code) => return code,
    };
    let package = match session.info(&args.name) {
        Ok(package) => package,
        Err(error) => return ui::error(error.code(), error.message()),
    };
    if args.json {
        match serde_json::to_string_pretty(&package) {
            Ok(text) => println!("{text}"),
            Err(error) => return ui::error("E8032", &format!("cannot serialize info: {error}")),
        }
        return ExitCode::SUCCESS;
    }
    println!("{}", package.name);
    for (version, entry) in package.parsed_versions().into_iter().rev() {
        let yanked = if entry.yanked { " (yanked)" } else { "" };
        println!("  {version}{yanked}");
        if let Some(license) = &entry.license {
            println!("    license: {license}");
        }
        if let Some(description) = &entry.description {
            println!("    {description}");
        }
        for dependency in &entry.dependencies {
            println!("    depends on {} {}", dependency.name, dependency.req);
        }
    }
    ExitCode::SUCCESS
}
