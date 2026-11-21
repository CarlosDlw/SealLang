//! `seal doc`: generate browsable documentation from the public API.

use std::path::Path;
use std::process::ExitCode;

use seal_doc::Documentation;

use crate::cli::{Cli, DocArgs, DocFormat};
use crate::ui;

pub(crate) fn run(cli: &Cli, root: &Path, args: &DocArgs) -> ExitCode {
    let project = match crate::cli::load(cli, root) {
        Ok(project) => project,
        Err(code) => return code,
    };
    let snapshot = match crate::cli::snapshot::generate(cli, &project, args.package.as_deref()) {
        Ok(snapshot) => snapshot,
        Err(code) => return code,
    };
    let documentation = Documentation::from_snapshot(&snapshot);
    let output = if args.output.is_absolute() {
        args.output.clone()
    } else {
        project.root.join(&args.output)
    };
    if args.json {
        return match seal_doc::render_json(&documentation) {
            Ok(text) => {
                print!("{text}");
                ExitCode::SUCCESS
            }
            Err(error) => ui::error(error.code(), &error.to_string()),
        };
    }
    if args.format == DocFormat::Json {
        let path = output.join("index.json");
        if let Err(error) = seal_doc::write_json(&documentation, &path) {
            return ui::error(error.code(), &error.to_string());
        }
        ui::status(cli, &format!("Wrote {}", path.display()));
        return ExitCode::SUCCESS;
    }
    if args.format == DocFormat::Html {
        let written = match seal_doc::write_html(&documentation, &output) {
            Ok(written) => written,
            Err(error) => return ui::error(error.code(), &error.to_string()),
        };
        ui::status(
            cli,
            &format!(
                "Documented {} {} ({} page(s)) in {}",
                documentation.package,
                documentation.package_version,
                written.len(),
                output.display()
            ),
        );
        if args.open {
            if let Err(error) = open_browser(&output.join("index.html")) {
                return ui::error("E8095", &error);
            }
        }
        if args.serve {
            println!("Serving {} at http://{}/", output.display(), args.address);
            if let Err(error) = seal_doc::serve(&output, &args.address) {
                return ui::error("E8095", &error.to_string());
            }
        }
    }
    ExitCode::SUCCESS
}

/// Opens `path` in the platform's default browser.
fn open_browser(path: &Path) -> Result<(), String> {
    let (program, arguments): (&str, Vec<&str>) = if cfg!(target_os = "macos") {
        ("open", vec![])
    } else if cfg!(windows) {
        ("cmd", vec!["/C", "start", ""])
    } else {
        ("xdg-open", vec![])
    };
    let path = path
        .canonicalize()
        .map_err(|error| format!("cannot resolve `{}`: {error}", path.display()))?;
    let status = std::process::Command::new(program)
        .args(arguments)
        .arg(&path)
        .status()
        .map_err(|error| format!("cannot run `{program}`: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("`{program}` exited with {status}"))
    }
}
