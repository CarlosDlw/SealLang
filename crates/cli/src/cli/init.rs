//! `seal init`: add a project to the current directory.

use std::process::ExitCode;

use crate::cli::{Cli, InitArgs};
use crate::ui;

pub(crate) fn run(cli: &Cli, args: &InitArgs) -> ExitCode {
    let root = std::env::current_dir().unwrap_or_else(|_| ".".into());
    if root.join("seal.toml").exists() {
        return ui::error(
            "E8011",
            &format!("`{}` already contains a seal.toml", root.display()),
        );
    }
    let name = root.file_name().map_or_else(
        || "app".to_owned(),
        |name| name.to_string_lossy().into_owned(),
    );
    let name = name
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_owned();
    let name = if name.is_empty() {
        "app".to_owned()
    } else {
        name
    };
    if let Err(error) = crate::cli::new::scaffold(&root, &name, args.template) {
        return ui::error("E8011", &error);
    }
    ui::status(
        cli,
        &format!("Initialized project `{name}` in {}", root.display()),
    );
    ExitCode::SUCCESS
}
