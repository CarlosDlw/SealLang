//! Shell completions and the manual page.
//!
//! Both artifacts are generated from the same clap metadata the parser uses, so
//! a command or flag cannot exist in one place and be missing from the other.
//! The generators intentionally produce the stable, common subset of each shell
//! format instead of relying on runtime-only completion helpers.

use std::path::Path;
use std::process::ExitCode;

use clap::{Command, CommandFactory};

use crate::cli::{Cli, CompletionsArgs, ManArgs, Shell};
use crate::ui;

pub(crate) fn completions(args: &CompletionsArgs) -> ExitCode {
    let command = Cli::command();
    let script = match args.shell {
        Shell::Bash => bash(&command),
        Shell::Zsh => zsh(&command),
        Shell::Fish => fish(&command),
        Shell::Powershell => powershell(&command),
    };
    write(args.output.as_deref(), &script)
}

pub(crate) fn man(args: &ManArgs) -> ExitCode {
    write(args.output.as_deref(), &manual(&Cli::command()))
}

fn write(path: Option<&Path>, text: &str) -> ExitCode {
    let Some(path) = path else {
        print!("{text}");
        return ExitCode::SUCCESS;
    };
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
        && let Err(error) = std::fs::create_dir_all(parent)
    {
        return ui::error(
            "E8111",
            &format!("cannot create `{}`: {error}", parent.display()),
        );
    }
    match std::fs::write(path, text) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => ui::error(
            "E8111",
            &format!("cannot write `{}`: {error}", path.display()),
        ),
    }
}

fn words(command: &Command) -> (Vec<String>, Vec<String>) {
    let names: Vec<String> = command
        .get_subcommands()
        .map(|subcommand| subcommand.get_name().to_owned())
        .collect();
    let mut flags: Vec<String> = command
        .get_arguments()
        .filter_map(|argument| {
            argument
                .get_long()
                .map(|long| format!("--{long}"))
                .or_else(|| argument.get_short().map(|short| format!("-{short}")))
        })
        .collect();
    flags.sort();
    flags.dedup();
    (names, flags)
}

fn bash(command: &Command) -> String {
    let (commands, flags) = words(command);
    format!(
        "# bash completion for seal\n\
         _seal() {{\n\
         \x20   local cur prev\n\
         \x20   COMPREPLY=()\n\
         \x20   cur=\"${{COMP_WORDS[COMP_CWORD]}}\"\n\
         \x20   prev=\"${{COMP_WORDS[COMP_CWORD-1]}}\"\n\
         \x20   local commands=\"{commands}\"\n\
         \x20   local flags=\"{flags}\"\n\
         \x20   if [[ $COMP_CWORD -eq 1 ]]; then\n\
         \x20       COMPREPLY=( $(compgen -W \"$commands $flags\" -- \"$cur\") )\n\
         \x20   else\n\
         \x20       COMPREPLY=( $(compgen -W \"$commands $flags\" -- \"$cur\") )\n\
         \x20   fi\n\
         \x20   return 0\n\
         }}\n\
         complete -F _seal seal\n",
        commands = commands.join(" "),
        flags = flags.join(" "),
    )
}

fn zsh(command: &Command) -> String {
    let (commands, flags) = words(command);
    let entries: Vec<String> = commands
        .iter()
        .map(|name| format!("'{name}:{name}'"))
        .collect();
    format!(
        "#compdef seal\n\
         _seal() {{\n\
         \x20   local -a commands\n\
         \x20   commands=({})\n\
         \x20   if (( CURRENT == 2 )); then\n\
         \x20       _describe 'seal command' commands\n\
         \x20   else\n\
         \x20       _describe 'seal command' commands\n\
         \x20       compadd -- {}\n\
         \x20   fi\n\
         }}\n\
         compdef _seal seal\n",
        entries.join(" "),
        flags.join(" "),
    )
}

fn fish(command: &Command) -> String {
    let mut text = String::from("# fish completion for seal\n");
    for subcommand in command.get_subcommands() {
        text.push_str(&format!(
            "complete -c seal -f -n '__fish_use_subcommand' -a '{}' -d '{}'\n",
            escape_fish(subcommand.get_name()),
            escape_fish(&about(subcommand)),
        ));
    }
    let flags: Vec<String> = command
        .get_arguments()
        .filter_map(|argument| argument.get_long().map(str::to_owned))
        .collect();
    for flag in flags {
        text.push_str(&format!("complete -c seal -l '{flag}'\n"));
    }
    text
}

fn powershell(command: &Command) -> String {
    let (commands, flags) = words(command);
    let mut values = commands;
    values.extend(flags);
    let values: Vec<String> = values
        .iter()
        .map(|value| format!("'{}'", value.replace('\'', "''")))
        .collect();
    format!(
        "Register-ArgumentCompleter -Native -CommandName seal -ScriptBlock {{\n\
         \x20   param($wordToComplete, $commandAst, $cursorPosition)\n\
         \x20   $completions = @({})\n\
         \x20   $completions | Where-Object {{ $_ -like \"$wordToComplete*\" }} | ForEach-Object {{\n\
         \x20       [System.Management.Automation.CompletionResult]::new($_, $_, 'ParameterValue', $_)\n\
         \x20   }}\n\
         }}\n",
        values.join(", "),
    )
}

fn manual(command: &Command) -> String {
    let mut text = String::new();
    text.push_str(".TH SEAL 1 \"\" \"seal ");
    text.push_str(env!("CARGO_PKG_VERSION"));
    text.push_str("\" \"User Commands\"\n");
    text.push_str(".SH NAME\nseal \\- the Seal project tool\n");
    text.push_str(".SH SYNOPSIS\n.B seal\n[OPTIONS] <COMMAND> [ARGS]\n");
    text.push_str(".SH DESCRIPTION\n");
    text.push_str(&roff(&about(command)));
    text.push_str("\n.SH COMMANDS\n");
    for subcommand in command.get_subcommands() {
        text.push_str(".TP\n.B ");
        text.push_str(&roff(subcommand.get_name()));
        text.push('\n');
        text.push_str(&roff(&about(subcommand)));
        text.push('\n');
    }
    text.push_str(".SH OPTIONS\n");
    for argument in command.get_arguments() {
        let Some(long) = argument.get_long() else {
            continue;
        };
        text.push_str(".TP\n.B \\-\\-");
        text.push_str(&roff(long));
        text.push('\n');
        let help = argument
            .get_help()
            .map(ToString::to_string)
            .unwrap_or_default();
        text.push_str(&roff(&help));
        text.push('\n');
    }
    text.push_str(".SH FILES\n");
    text.push_str("\\fIseal.toml\\fR project manifest, \\fIseal.lock\\fR lockfile.\n");
    text
}

fn about(command: &Command) -> String {
    command
        .get_about()
        .map(ToString::to_string)
        .unwrap_or_default()
}

fn escape_fish(text: &str) -> String {
    text.replace('\\', "\\\\").replace('\'', "\\'")
}

fn roff(text: &str) -> String {
    let mut escaped = text.replace('\\', "\\\\");
    escaped = escaped.replace('-', "\\-");
    if escaped.starts_with('.') || escaped.starts_with('\'') {
        escaped.insert_str(0, "\\&");
    }
    escaped
}
