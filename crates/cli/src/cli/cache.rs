//! `seal cache`: inspect, verify, prune, and explain the local cache.

use std::path::Path;
use std::process::ExitCode;

use seal_cas::{ActionCache, DEFAULT_CACHE_DIR, Digest};

use crate::cli::{CacheArgs, CacheCommand, Cli};
use crate::ui;

/// Runs one cache operation.
pub(crate) fn cache(cli: &Cli, root: &Path, args: &CacheArgs) -> ExitCode {
    let project = match crate::cli::load(root) {
        Ok(project) => project,
        Err(code) => return code,
    };
    let root = project.root.join(DEFAULT_CACHE_DIR);
    let cache = match ActionCache::open(&root) {
        Ok(cache) => cache,
        Err(error) => return ui::error(error.code(), &error.to_string()),
    };
    match &args.command {
        CacheCommand::Stats => stats(cli, &cache),
        CacheCommand::Verify => verify(cli, &cache),
        CacheCommand::Prune {
            max_size,
            older_than,
        } => prune(cli, &cache, *max_size, *older_than),
        CacheCommand::Explain { key } => explain(cli, &cache, key),
    }
}

fn stats(cli: &Cli, cache: &ActionCache) -> ExitCode {
    let objects = match cache.cas().stats() {
        Ok(stats) => stats,
        Err(error) => return ui::error(error.code(), &error.to_string()),
    };
    let actions = match cache.stats() {
        Ok(stats) => stats,
        Err(error) => return ui::error(error.code(), &error.to_string()),
    };
    println!("cache: {}", cache.cas().root().display());
    println!("objects: {} ({} bytes)", objects.objects, objects.bytes);
    println!("actions: {} ({} bytes)", actions.records, actions.bytes);
    ui::verbose(cli, "run `seal cache verify` to hash every object");
    ExitCode::SUCCESS
}

fn verify(cli: &Cli, cache: &ActionCache) -> ExitCode {
    let objects = match cache.cas().verify() {
        Ok(report) => report,
        Err(error) => return ui::error(error.code(), &error.to_string()),
    };
    let actions = match cache.verify() {
        Ok(report) => report,
        Err(error) => return ui::error(error.code(), &error.to_string()),
    };
    println!(
        "verified {} object(s); {} corrupt",
        objects.checked,
        objects.corrupt.len()
    );
    println!(
        "verified {} action record(s); {} corrupt",
        actions.checked,
        actions.corrupt.len()
    );
    for object in &objects.corrupt {
        eprintln!(
            "warning[E8040]: `{}` hashes to `{}`, expected `{}`",
            object.path.display(),
            object.actual,
            object.expected
        );
    }
    for path in &actions.corrupt {
        eprintln!(
            "warning[E8040]: action record `{}` is corrupt",
            path.display()
        );
    }
    if objects.corrupt.is_empty() && actions.corrupt.is_empty() {
        ui::verbose(cli, "the cache is valid");
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}

fn prune(
    cli: &Cli,
    cache: &ActionCache,
    max_size: Option<u64>,
    older_than: Option<u64>,
) -> ExitCode {
    if let Some(seconds) = older_than {
        match cache.prune_older_than(seconds) {
            Ok(removed) => ui::status(cli, &format!("removed {removed} action record(s)")),
            Err(error) => return ui::error(error.code(), &error.to_string()),
        }
    }
    if let Some(max_size) = max_size {
        match cache.cas().prune_lru(max_size) {
            Ok(report) => {
                ui::status(
                    cli,
                    &format!(
                        "removed {} object(s), reclaimed {} bytes; {} bytes remain",
                        report.removed, report.bytes, report.remaining_bytes
                    ),
                );
            }
            Err(error) => return ui::error(error.code(), &error.to_string()),
        }
    }
    ExitCode::SUCCESS
}

fn explain(cli: &Cli, cache: &ActionCache, key: &str) -> ExitCode {
    let key = match Digest::parse(key) {
        Ok(key) => key,
        Err(error) => return ui::error(error.code(), &error.to_string()),
    };
    let record = match cache.lookup(key) {
        Ok(Some(record)) => record,
        Ok(None) => {
            return ui::error("E8040", &format!("no action record for the key `{key}`"));
        }
        Err(error) => return ui::error(error.code(), &error.to_string()),
    };
    println!("key: {key}");
    println!("kind: {}", record.kind);
    if !record.command.is_empty() {
        println!("command: {}", record.command.join(" "));
    }
    println!("inputs:");
    for input in &record.inputs {
        println!("  {} {}", input.digest, input.path);
    }
    println!("environment:");
    for (name, digest) in &record.env {
        println!("  {name} {digest}");
    }
    println!("outputs:");
    for output in &record.outputs {
        println!("  {} {} ({:o})", output.digest, output.name, output.mode);
    }
    ui::verbose(cli, "environment values are stored only as digests");
    ExitCode::SUCCESS
}
