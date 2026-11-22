//! `seal new`: project scaffolding.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use crate::cli::{Cli, NewArgs, Template, Vcs};
use crate::ui;

pub(crate) fn run(cli: &Cli, args: &NewArgs) -> ExitCode {
    let path = if args.path.as_os_str() == "." {
        std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
    } else {
        args.path.clone()
    };
    if path.exists() {
        let empty = path
            .read_dir()
            .map(|mut entries| entries.next().is_none())
            .unwrap_or(false);
        if !empty {
            return ui::error(
                "E8011",
                &format!("`{}` already exists and is not empty", path.display()),
            );
        }
    } else if let Err(error) = std::fs::create_dir_all(&path) {
        return ui::error(
            "E8011",
            &format!("cannot create `{}`: {error}", path.display()),
        );
    }
    let name = project_name(&path);
    if let Err(error) = scaffold(&path, &name, args.template) {
        return ui::error("E8011", &error);
    }
    if args.vcs == Vcs::Git {
        let status = std::process::Command::new("git")
            .arg("init")
            .arg("--quiet")
            .current_dir(&path)
            .status();
        if let Ok(status) = status {
            if !status.success() && !cli.quiet {
                ui::warning("git init failed; the project files were still created");
            }
        }
    }
    ui::status(
        cli,
        &format!(
            "Created {} project `{name}` in {}",
            template_name(args.template),
            path.display()
        ),
    );
    ExitCode::SUCCESS
}

fn project_name(path: &Path) -> String {
    let raw = path.file_name().map_or_else(
        || "app".to_owned(),
        |name| name.to_string_lossy().into_owned(),
    );
    let mut name: String = raw
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    while name.contains("--") {
        name = name.replace("--", "-");
    }
    name = name.trim_matches('-').to_owned();
    if name.is_empty() || !name.as_bytes()[0].is_ascii_lowercase() {
        name.insert(0, 'a');
    }
    name
}

fn template_name(template: Template) -> &'static str {
    match template {
        Template::Bin => "binary",
        Template::Lib => "library",
        Template::Workspace => "workspace",
        Template::Kernel => "freestanding kernel",
        Template::Compiler => "compiler",
    }
}

pub(crate) fn scaffold(root: &Path, name: &str, template: Template) -> Result<(), String> {
    let src = root.join("src");
    let tests = root.join("tests");
    std::fs::create_dir_all(&src).map_err(|error| error.to_string())?;
    std::fs::create_dir_all(&tests).map_err(|error| error.to_string())?;
    write(
        root.join(".gitignore"),
        "/seal/target/\n/seal/toolchains/\n/target/\n",
    )?;
    match template {
        Template::Bin => {
            write(
                root.join("seal.toml"),
                &format!(
                    "[project]\nname = \"{name}\"\nlanguage = \"0.1\"\n\n\
                     [package]\nname = \"{name}\"\nversion = \"0.1.0\"\n"
                ),
            )?;
            write(
                src.join("main.sl"),
                "use app::util::value;\n\nfunc main() -> i32 {\n  value()\n}\n",
            )?;
            write(
                src.join("util.sl"),
                "module app::util;\n\ninternal func value() -> i32 {\n  0\n}\n",
            )?;
        }
        Template::Lib => {
            write(
                root.join("seal.toml"),
                &format!(
                    "[project]\nname = \"{name}\"\nlanguage = \"0.1\"\n\n\
                     [package]\nname = \"{name}\"\nversion = \"0.1.0\"\n\n\
                     [package.api]\nexpose = [\"root\"]\n"
                ),
            )?;
            write(
                src.join("lib.sl"),
                "public func answer() -> i32 {\n  42\n}\n",
            )?;
        }
        Template::Workspace => return scaffold_workspace(root, name),
        Template::Compiler => {
            write(
                root.join("seal.toml"),
                &format!(
                    "[project]\nname = \"{name}\"\nlanguage = \"0.1\"\n\n\
                     [package]\nname = \"{name}\"\nversion = \"0.1.0\"\n"
                ),
            )?;
            write(
                src.join("main.sl"),
                "use app::ast::Program;\nuse app::eval::evaluate;\n\
                 use app::parser::parse;\n\n\
                 func main() -> i32 {\n  let program = parse(40, 2);\n  evaluate(&program)\n}\n",
            )?;
            write(
                src.join("ast.sl"),
                "module app::ast;\n\n\
                 internal struct Program {\n    internal left: i32,\n    internal right: i32,\n}\n",
            )?;
            write(
                src.join("parser.sl"),
                "module app::parser;\n\nuse app::ast::Program;\n\n\
                 internal func parse(left: i32, right: i32) -> Program {\n  \
                 Program { left: left, right: right }\n}\n",
            )?;
            write(
                src.join("eval.sl"),
                "module app::eval;\n\nuse app::ast::Program;\n\n\
                 internal func evaluate(program: *Program) -> i32 {\n  \
                 (*program).left + (*program).right\n}\n",
            )?;
        }
        Template::Kernel => {
            write(
                root.join("seal.toml"),
                &format!(
                    "[project]\nname = \"{name}\"\nlanguage = \"0.1\"\n\n\
                     [package]\nname = \"{name}\"\nversion = \"0.1.0\"\n\n\
                     [targets.kernel]\nkind = \"bin\"\nentry = \"root::_start\"\n\
                     runtime = \"none\"\npanic = \"trap\"\nlinker = \"cc\"\n\
                     linker-script = \"kernel.ld\"\nlink-args = [\"-no-pie\"]\n"
                ),
            )?;
            write(
                src.join("main.sl"),
                "extern \"c\" {\n  func exit(code: c_int) -> ();\n}\n\n\
                 func _start() {\n  unsafe {\n    exit(42);\n  }\n}\n",
            )?;
            write(
                root.join("kernel.ld"),
                "ENTRY(_start)\n\nSECTIONS {\n  . = 0x10000;\n\
                 .text : { *(.text*) }\n  .rodata : { *(.rodata*) }\n\
                 .data : { *(.data*) }\n  .bss : { *(.bss*) *(COMMON) }\n}\n",
            )?;
        }
    }
    write_lockfile(root)
}

fn scaffold_workspace(root: &Path, name: &str) -> Result<(), String> {
    write(
        root.join("seal.toml"),
        &format!(
            "[project]\nname = \"{name}\"\nlanguage = \"0.1\"\n\
             members = [\"crates/*\"]\ndefault-members = [\"crates/app\"]\n"
        ),
    )?;
    let app = root.join("crates").join("app");
    let greet = root.join("crates").join("greet");
    std::fs::create_dir_all(app.join("src")).map_err(|error| error.to_string())?;
    std::fs::create_dir_all(app.join("tests")).map_err(|error| error.to_string())?;
    std::fs::create_dir_all(greet.join("src")).map_err(|error| error.to_string())?;
    std::fs::create_dir_all(greet.join("tests")).map_err(|error| error.to_string())?;
    write(
        app.join("seal.toml"),
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n\
         [dependencies]\ngreet = { path = \"../greet\" }\n",
    )?;
    write(
        app.join("src").join("main.sl"),
        "use greet::answer;\n\nfunc main() -> i32 {\n  answer()\n}\n",
    )?;
    write(
        greet.join("seal.toml"),
        "[package]\nname = \"greet\"\nversion = \"0.1.0\"\n\n\
         [package.api]\nexpose = [\"root\"]\n",
    )?;
    write(
        greet.join("src").join("lib.sl"),
        "public func answer() -> i32 {\n  0\n}\n",
    )?;
    write_lockfile(root)
}

fn write_lockfile(root: &Path) -> Result<(), String> {
    let project = seal_project::Project::load(root)
        .map_err(|error| format!("cannot finish the project: {error}"))?;
    project
        .write_lockfile()
        .map(|_| ())
        .map_err(|error| error.to_string())
}

pub(crate) fn write(path: PathBuf, contents: &str) -> Result<(), String> {
    std::fs::write(&path, contents)
        .map_err(|error| format!("cannot write `{}`: {error}", path.display()))
}
