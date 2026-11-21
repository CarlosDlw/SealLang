//! Content-addressed, cache-first project builds.
//!
//! A build is a small graph of actions: `frontend` turns one compile plan into
//! an object and link metadata, `link` turns that object into an executable,
//! `generate` runs a generator executable and validates its outputs, and
//! `native-probe` resolves a declared native dependency. Every action key is a
//! digest of its inputs, hashed environment, and display command, so a repeated
//! build becomes a cache hit without hashing project-wide state.
//!
//! Generation and native probing run before planning the consumers they feed:
//! their output digests are inputs of the downstream frontend and link keys.
//! That is a dynamic-dependency build, not a shortcut: no consumer is keyed
//! before the bytes it consumes exist.
//!
//! # Example
//!
//! ```ignore
//! let builder = seal_build::Builder::new(&project, seal_build::BuildOptions::default());
//! let report = builder.build(&[seal_build::JobRequest {
//!     package: 0,
//!     target: "app".to_owned(),
//!     options: seal_project::PlanOptions::default(),
//! }])?;
//! ```

#![warn(missing_docs)]

mod action;
mod context;
mod error;
mod graph;
mod tasks;

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use seal_cas::{ActionCache, ActionInput, ActionRecord, DEFAULT_CACHE_DIR, Digest};
use seal_manifest::TargetKind;
use seal_project::{PlanOptions, Project, Target};
use seal_sandbox::Sandbox;

pub use action::NativeResult;
pub use error::{BuildError, BuildResult};
pub use tasks::{TaskOutcome, TaskReport, TaskRequest, TaskRunner};

use action::{
    Action, CodegenAction, FrontendAction, GenerateAction, LinkAction, NativeAction, slash_path,
};
use context::ActionContext;
use graph::ActionGraph;

/// One target selected for a build.
#[derive(Clone, Debug)]
pub struct JobRequest {
    /// The package index.
    pub package: usize,
    /// The target name.
    pub target: String,
    /// Feature, profile, and target selection.
    pub options: PlanOptions,
}

/// One progress event emitted while actions run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProgressEvent {
    /// An action is about to run.
    Started {
        /// The action kind.
        kind: String,
        /// The human-readable label.
        label: String,
        /// The content key.
        key: String,
        /// The display command.
        command: Vec<String>,
    },
    /// An action was served from the cache.
    Cached {
        /// The action kind.
        kind: String,
        /// The human-readable label.
        label: String,
        /// The content key.
        key: String,
        /// The display command.
        command: Vec<String>,
    },
    /// An action finished successfully.
    Finished {
        /// The action kind.
        kind: String,
        /// The human-readable label.
        label: String,
        /// The content key.
        key: String,
        /// Elapsed milliseconds.
        millis: u64,
    },
    /// A cache record was corrupt and will be rebuilt.
    CacheCorrupt {
        /// The action kind.
        kind: String,
        /// The human-readable label.
        label: String,
        /// The content key.
        key: String,
        /// Why the record was discarded.
        reason: String,
    },
}

/// A progress callback shared by all workers.
pub type ProgressFn = dyn Fn(ProgressEvent) + Send + Sync;

/// One action's timing and cache result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActionTiming {
    /// The action kind.
    pub kind: String,
    /// The human-readable label.
    pub label: String,
    /// The content key.
    pub key: String,
    /// Elapsed milliseconds; zero for a cache hit.
    pub millis: u64,
    /// Whether the action was served from the cache.
    pub cached: bool,
}

/// Options for one builder.
#[derive(Clone)]
pub struct BuildOptions {
    /// Maximum actions running at once.
    pub jobs: usize,
    /// Whether to read and write the action cache.
    pub use_cache: bool,
    /// The sandbox backend.
    pub sandbox: Sandbox,
    /// The cache root; defaults to `<project>/seal/cache`.
    pub cache_root: Option<PathBuf>,
    /// Extra linker arguments applied to every link action.
    pub link_args: Vec<OsString>,
    /// An optional progress callback.
    pub progress: Option<Arc<ProgressFn>>,
}

impl Default for BuildOptions {
    fn default() -> Self {
        Self {
            jobs: std::thread::available_parallelism()
                .map(|value| value.get())
                .unwrap_or(1),
            use_cache: true,
            sandbox: Sandbox::detect(),
            cache_root: None,
            link_args: Vec::new(),
            progress: None,
        }
    }
}

impl std::fmt::Debug for BuildOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BuildOptions")
            .field("jobs", &self.jobs)
            .field("use_cache", &self.use_cache)
            .field("cache_root", &self.cache_root)
            .field("link_args", &self.link_args)
            .finish_non_exhaustive()
    }
}

/// One artifact produced by a build.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BuiltTarget {
    /// The package name.
    pub package: String,
    /// The target name.
    pub target: String,
    /// The artifact path.
    pub path: PathBuf,
    /// The final action key.
    pub key: String,
    /// Whether the final action was a cache hit.
    pub cached: bool,
}

/// The complete result of a build.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BuildReport {
    /// One entry per selected target, in request order.
    pub outputs: Vec<BuiltTarget>,
    /// Every action timing, sorted by kind and label.
    pub timings: Vec<ActionTiming>,
    /// Native dependencies resolved during the build.
    pub native: Vec<NativeResult>,
    /// Wall-clock duration.
    pub duration: Duration,
}

impl BuildReport {
    /// Returns the number of cache hits.
    pub fn hits(&self) -> usize {
        self.timings.iter().filter(|timing| timing.cached).count()
    }

    /// Returns the number of actions that ran.
    pub fn misses(&self) -> usize {
        self.timings.iter().filter(|timing| !timing.cached).count()
    }
}

/// A cache-first project builder.
pub struct Builder<'a> {
    project: &'a Project,
    options: BuildOptions,
}

impl<'a> Builder<'a> {
    /// Creates a builder for `project`.
    pub fn new(project: &'a Project, options: BuildOptions) -> Self {
        Self { project, options }
    }

    /// Builds every requested target.
    pub fn build(&self, jobs: &[JobRequest]) -> BuildResult<BuildReport> {
        let start = Instant::now();
        let cache_root = self
            .options
            .cache_root
            .clone()
            .unwrap_or_else(|| self.project.root.join(DEFAULT_CACHE_DIR));
        let cache = ActionCache::open(&cache_root)?;
        let temp_root = cache_root.join("tmp");
        fs::create_dir_all(&temp_root).map_err(|error| {
            BuildError::cache(format!("cannot create `{}`: {error}", temp_root.display()))
        })?;
        let context = Arc::new(ActionContext::new(
            cache,
            self.options.sandbox.clone(),
            temp_root,
            self.options.use_cache,
            self.options.progress.clone(),
        ));

        let (generators, regular): (Vec<&JobRequest>, Vec<&JobRequest>) = jobs
            .iter()
            .partition(|job| self.target(job).kind == TargetKind::Generator);
        let generated = self.build_generators(&context, &generators)?;
        let native = self.resolve_native(&context, &regular)?;
        let generated_by_job: BTreeMap<(usize, String), Vec<PathBuf>> = generated
            .iter()
            .map(|entry| {
                (
                    (entry.job.package, entry.job.target.clone()),
                    entry.artifacts.clone(),
                )
            })
            .collect();
        let mut outputs: Vec<BuiltTarget> =
            generated.into_iter().map(|entry| entry.built).collect();
        outputs.extend(self.build_regular(&context, &regular, &generated_by_job, &native)?);
        Ok(BuildReport {
            outputs,
            timings: context.timings(),
            native,
            duration: start.elapsed(),
        })
    }

    fn target(&self, job: &JobRequest) -> &Target {
        self.project
            .package(job.package)
            .target(&job.target)
            .expect("the CLI selects existing targets")
    }

    fn label(&self, job: &JobRequest) -> String {
        format!("{}.{}", self.project.package(job.package).name, job.target)
    }

    fn plan_options(&self, job: &JobRequest) -> PlanOptions {
        let mut options = job.options.clone();
        options.mode = seal_plan::BuildMode::Build;
        options
    }

    fn build_generators(
        &self,
        context: &Arc<ActionContext>,
        generators: &[&JobRequest],
    ) -> BuildResult<Vec<GeneratorOutput>> {
        if generators.is_empty() {
            return Ok(Vec::new());
        }
        let mut graph = ActionGraph::new();
        let mut frontends = Vec::new();
        for job in generators {
            let options = self.plan_options(job);
            let plan = self
                .project
                .compile_plan(job.package, &job.target, &options)?;
            let count = plan.profile.codegen_units.max(1);
            let frontend = Arc::new(FrontendAction::new(
                self.label(job),
                plan,
                count,
                &self.project.root,
                &context.temp_root,
            )?);
            let key = graph.add(Action::Frontend(frontend), Vec::new())?;
            frontends.push((*job, key, count));
        }
        let records = graph.execute(context, self.options.jobs)?;
        let mut result = Vec::new();
        for (job, frontend_key, count) in frontends {
            let target = self.target(job);
            let path = target_output(self.project, target);
            let link_key = {
                let frontend = records.get(&frontend_key).ok_or_else(|| {
                    BuildError::action(format!("the frontend `{}` produced no record", job.target))
                })?;
                let mono = frontend
                    .output("mono")
                    .ok_or_else(|| BuildError::action("the frontend produced no mono artifact"))?;
                let metadata = frontend
                    .output("link-metadata")
                    .ok_or_else(|| BuildError::action("the frontend produced no link metadata"))?;
                let objects =
                    self.codegen_objects(context, &self.label(job), mono.digest, count)?;
                let mut link_graph = ActionGraph::new();
                let link = Arc::new(LinkAction::new(
                    self.label(job),
                    objects,
                    metadata.digest,
                    Vec::new(),
                    self.options.link_args.clone(),
                    target.linker.as_ref().map(OsString::from),
                    &context.temp_root,
                ));
                let key = link_graph.add(Action::Link(link), Vec::new())?;
                let link_records = link_graph.execute(context, 1)?;
                let record = link_records
                    .get(&key)
                    .ok_or_else(|| BuildError::action("the generator link produced no record"))?;
                materialize_record(
                    context,
                    record,
                    "executable",
                    &path,
                    &format!("generator `{}`", job.target),
                )?;
                key
            };
            let link_record = context
                .cache
                .lookup(link_key)?
                .ok_or_else(|| BuildError::action("the generator executable disappeared"))?;
            let tool = link_record
                .output("executable")
                .map(|output| output.digest)
                .ok_or_else(|| BuildError::action("the generator executable disappeared"))?;
            let output_globs = target.generator_outputs.clone();
            self.reject_output_glob_collisions(job, &output_globs)?;
            let output_dirs =
                output_directories(&self.project.package(job.package).root, &output_globs)?;
            let inputs: Vec<ActionInput> = target
                .generator_inputs
                .iter()
                .map(|input| {
                    ActionInput::from_file(
                        input
                            .strip_prefix(&self.project.package(job.package).root)
                            .map(slash_path)
                            .unwrap_or_else(|_| input.display().to_string()),
                        input,
                    )
                })
                .collect::<Result<_, _>>()?;
            let generate = Arc::new(GenerateAction::new(
                &self.project.package(job.package).root,
                &job.target,
                tool,
                inputs,
                output_globs,
                output_dirs,
                &context.temp_root,
            )?);
            let mut generate_graph = ActionGraph::new();
            let generate_key = generate_graph.add(Action::Generate(generate), Vec::new())?;
            let generated = generate_graph.execute(context, 1)?;
            let record = generated
                .get(&generate_key)
                .ok_or_else(|| BuildError::action("the generator produced no record"))?;
            let mut artifacts = Vec::new();
            for output in &record.outputs {
                let artifact = self.project.package(job.package).root.join(&output.name);
                materialize_record(
                    context,
                    record,
                    &output.name,
                    &artifact,
                    &format!("generator `{}`", job.target),
                )?;
                artifacts.push(artifact);
            }
            self.reject_generated_collisions(job, &artifacts)?;
            artifacts.sort();
            result.push(GeneratorOutput {
                job: job.clone(),
                artifacts,
                built: BuiltTarget {
                    package: self.project.package(job.package).name.clone(),
                    target: job.target.clone(),
                    path,
                    key: link_key.to_hex(),
                    cached: context.was_cached(link_key),
                },
            });
        }
        Ok(result)
    }

    fn build_regular(
        &self,
        context: &Arc<ActionContext>,
        jobs: &[&JobRequest],
        generated: &BTreeMap<(usize, String), Vec<PathBuf>>,
        native: &[NativeResult],
    ) -> BuildResult<Vec<BuiltTarget>> {
        if jobs.is_empty() {
            return Ok(Vec::new());
        }
        let mut graph = ActionGraph::new();
        let mut frontends = Vec::new();
        for job in jobs {
            let target = self.target(job);
            let mut options = self.plan_options(job);
            let mut extras = Vec::new();
            for generator in &target.generated {
                if let Some(files) = generated.get(&(job.package, generator.clone())) {
                    extras.extend(files.iter().cloned());
                }
            }
            extras.sort();
            extras.dedup();
            options.extra_files = extras;
            let plan = self
                .project
                .compile_plan(job.package, &job.target, &options)?;
            let path = plan
                .output
                .clone()
                .unwrap_or_else(|| target_output(self.project, target));
            let count = if target.is_executable() {
                plan.profile.codegen_units.max(1)
            } else {
                1
            };
            let frontend = Arc::new(FrontendAction::new(
                self.label(job),
                plan,
                count,
                &self.project.root,
                &context.temp_root,
            )?);
            let key = graph.add(Action::Frontend(frontend), Vec::new())?;
            frontends.push((*job, key, path, count));
        }
        let records = graph.execute(context, self.options.jobs)?;

        let mut codegen_graph = ActionGraph::new();
        let mut codegen_keys = Vec::new();
        for (job, frontend_key, path, count) in &frontends {
            let frontend = records.get(frontend_key).ok_or_else(|| {
                BuildError::action(format!("the frontend `{}` produced no record", job.target))
            })?;
            let mono = frontend
                .output("mono")
                .ok_or_else(|| BuildError::action("the frontend produced no mono artifact"))?;
            let units = mono_units(context, mono.digest)?;
            if units.len() != *count as usize {
                return Err(BuildError::action(format!(
                    "the frontend for `{}` wrote {} units but the build expected {count}",
                    job.target,
                    units.len()
                )));
            }
            let mut keys = Vec::new();
            for (cgu, unit) in units.into_iter().enumerate() {
                let action = Arc::new(CodegenAction::new(
                    self.label(job),
                    mono.digest,
                    unit,
                    cgu as u32,
                    &context.temp_root,
                ));
                keys.push(codegen_graph.add(Action::Codegen(action), Vec::new())?);
            }
            codegen_keys.push((*job, *frontend_key, path.clone(), keys));
        }
        let codegen_records = codegen_graph.execute(context, self.options.jobs)?;

        let mut link_graph = ActionGraph::new();
        let mut links = Vec::new();
        let mut outputs = Vec::new();
        for (job, frontend_key, path, keys) in &codegen_keys {
            let target = self.target(job);
            let frontend = records.get(frontend_key).ok_or_else(|| {
                BuildError::action(format!("the frontend `{}` produced no record", job.target))
            })?;
            if !target.is_executable() {
                let first = keys
                    .first()
                    .ok_or_else(|| BuildError::action("the codegen graph produced no unit"))?;
                let record = codegen_records
                    .get(first)
                    .ok_or_else(|| BuildError::action("the codegen produced no record"))?;
                materialize_record(
                    context,
                    record,
                    "object",
                    path,
                    &format!("target `{}`", job.target),
                )?;
                outputs.push(built_target(self.project, job, path, *first, context));
                continue;
            }
            let objects = keys
                .iter()
                .map(|key| {
                    codegen_records
                        .get(key)
                        .and_then(|record| record.output("object"))
                        .map(|output| output.digest)
                        .ok_or_else(|| {
                            BuildError::action(format!(
                                "the codegen for `{}` produced no object",
                                job.target
                            ))
                        })
                })
                .collect::<BuildResult<Vec<_>>>()?;
            let metadata = frontend
                .output("link-metadata")
                .ok_or_else(|| BuildError::action("the frontend produced no link metadata"))?;
            let link = Arc::new(LinkAction::new(
                self.label(job),
                objects,
                metadata.digest,
                self.native_for(job, native),
                self.options.link_args.clone(),
                target.linker.as_ref().map(OsString::from),
                &context.temp_root,
            ));
            let key = link_graph.add(Action::Link(link), Vec::new())?;
            links.push((*job, key, path.clone()));
        }
        let link_records = link_graph.execute(context, self.options.jobs)?;
        for (job, key, path) in links {
            let record = link_records
                .get(&key)
                .ok_or_else(|| BuildError::action("the link produced no record"))?;
            materialize_record(
                context,
                record,
                "executable",
                &path,
                &format!("target `{}`", job.target),
            )?;
            outputs.push(built_target(self.project, job, &path, key, context));
        }
        Ok(outputs)
    }

    /// Runs every code generation unit and returns the object digests in order.
    fn codegen_objects(
        &self,
        context: &Arc<ActionContext>,
        label: &str,
        mono: Digest,
        count: u32,
    ) -> BuildResult<Vec<Digest>> {
        let units = mono_units(context, mono)?;
        if units.len() != count as usize {
            return Err(BuildError::action(format!(
                "the frontend for `{label}` wrote {} units but the build expected {count}",
                units.len()
            )));
        }
        let mut graph = ActionGraph::new();
        let mut keys = Vec::with_capacity(count as usize);
        for (cgu, unit) in units.into_iter().enumerate() {
            let action = Arc::new(CodegenAction::new(
                label,
                mono,
                unit,
                cgu as u32,
                &context.temp_root,
            ));
            keys.push(graph.add(Action::Codegen(action), Vec::new())?);
        }
        let records = graph.execute(context, self.options.jobs)?;
        keys.iter()
            .map(|key| {
                records
                    .get(key)
                    .and_then(|record| record.output("object"))
                    .map(|output| output.digest)
                    .ok_or_else(|| {
                        BuildError::action(format!("the codegen for `{label}` produced no object"))
                    })
            })
            .collect()
    }

    fn native_for(&self, job: &JobRequest, native: &[NativeResult]) -> Vec<NativeResult> {
        let target = self.target(job);
        let package = &self.project.package(job.package).name;
        native
            .iter()
            .filter(|entry| &entry.package == package && target.native.contains(&entry.alias))
            .cloned()
            .collect()
    }

    fn resolve_native(
        &self,
        context: &Arc<ActionContext>,
        jobs: &[&JobRequest],
    ) -> BuildResult<Vec<NativeResult>> {
        let mut wanted: BTreeSet<(usize, String)> = BTreeSet::new();
        for job in jobs {
            for alias in &self.target(job).native {
                wanted.insert((job.package, alias.clone()));
            }
        }
        if wanted.is_empty() {
            return Ok(Vec::new());
        }
        let pkg_config = which("pkg-config");
        let pkg_config_path = std::env::var("PKG_CONFIG_PATH").ok();
        let mut graph = ActionGraph::new();
        let mut keys = Vec::new();
        for (package_index, alias) in &wanted {
            let package = self.project.package(*package_index);
            let declaration = package.manifest.native.get(alias).ok_or_else(|| {
                BuildError::native(format!(
                    "the package `{}` has no native dependency `{alias}`",
                    package.name
                ))
            })?;
            let action = Arc::new(NativeAction::new(
                &package.name,
                alias,
                &package.root,
                declaration,
                pkg_config.as_deref(),
                pkg_config_path.as_deref(),
            )?);
            keys.push(graph.add(Action::Native(action), Vec::new())?);
        }
        let records = graph.execute(context, self.options.jobs)?;
        let mut results = Vec::new();
        for key in keys {
            let record = records
                .get(&key)
                .ok_or_else(|| BuildError::native("the native probe produced no record"))?;
            let output = record
                .output("result")
                .ok_or_else(|| BuildError::native("the native probe produced no result"))?;
            let bytes = context.cache.cas().get(output.digest)?;
            let result: NativeResult = serde_json::from_slice(&bytes).map_err(|error| {
                BuildError::native(format!("invalid native probe result: {error}"))
            })?;
            results.push(result);
        }
        results.sort_by(|left, right| {
            left.package
                .cmp(&right.package)
                .then_with(|| left.alias.cmp(&right.alias))
        });
        Ok(results)
    }

    fn reject_output_glob_collisions(&self, job: &JobRequest, globs: &[String]) -> BuildResult<()> {
        let package = self.project.package(job.package);
        let mut builder = globset::GlobSetBuilder::new();
        for glob in globs {
            builder.add(globset::Glob::new(glob).map_err(|error| {
                BuildError::generator(format!("invalid output glob `{glob}`: {error}"))
            })?);
        }
        let matcher = builder
            .build()
            .map_err(|error| BuildError::generator(format!("invalid output globs: {error}")))?;
        for file in &package.files {
            let relative = file
                .strip_prefix(&package.root)
                .map(slash_path)
                .unwrap_or_else(|_| file.display().to_string());
            if matcher.is_match(&relative) {
                return Err(BuildError::generator(format!(
                    "the generator `{}` output glob matches the source file `{relative}`; generated files cannot overwrite sources",
                    job.target
                )));
            }
        }
        Ok(())
    }

    fn reject_generated_collisions(
        &self,
        job: &JobRequest,
        artifacts: &[PathBuf],
    ) -> BuildResult<()> {
        let package = self.project.package(job.package);
        let mut seen = BTreeSet::new();
        for artifact in artifacts {
            if package.files.contains(artifact) {
                return Err(BuildError::generator(format!(
                    "the generator `{}` output `{}` collides with a source file of `{}`",
                    job.target,
                    artifact.display(),
                    package.name
                )));
            }
            if !seen.insert(artifact.clone()) {
                return Err(BuildError::conflict(format!(
                    "the generator `{}` produced `{}` twice",
                    job.target,
                    artifact.display()
                )));
            }
        }
        Ok(())
    }
}

struct GeneratorOutput {
    job: JobRequest,
    artifacts: Vec<PathBuf>,
    built: BuiltTarget,
}

fn target_output(project: &Project, target: &Target) -> PathBuf {
    if let Some(path) = &target.output {
        return project.root.join(path);
    }
    let directory = project
        .root
        .join("seal")
        .join("target")
        .join(target.profile.clone().unwrap_or_else(|| "dev".to_owned()));
    let name = match target.kind {
        TargetKind::Lib | TargetKind::Staticlib | TargetKind::Cdylib => {
            format!("lib{}.o", target.name)
        }
        _ => target.name.clone(),
    };
    directory.join(name)
}

/// Returns one digest per `.sealmono` unit, in CGU order.
fn mono_units(context: &ActionContext, mono: Digest) -> BuildResult<Vec<Digest>> {
    let path = context.cache.cas().object_path(mono);
    let archive = seal_metadata::read_archive_file(&path)
        .map_err(|error| BuildError::action(format!("cannot read the mono artifact: {error}")))?;
    Ok(archive
        .modules()
        .iter()
        .map(|bytes| Digest::of_bytes(bytes))
        .collect())
}

fn built_target(
    project: &Project,
    job: &JobRequest,
    path: &Path,
    key: Digest,
    context: &ActionContext,
) -> BuiltTarget {
    BuiltTarget {
        package: project.package(job.package).name.clone(),
        target: job.target.clone(),
        path: path.to_path_buf(),
        key: key.to_hex(),
        cached: context.was_cached(key),
    }
}

fn output_directories(package_root: &Path, globs: &[String]) -> BuildResult<Vec<PathBuf>> {
    let mut directories = BTreeSet::new();
    for glob in globs {
        let first = glob.find(['*', '?', '[']).unwrap_or(glob.len());
        let prefix = &glob[..first];
        let directory = prefix.rsplit_once('/').map(|(directory, _)| directory);
        let Some(directory) = directory.filter(|directory| !directory.is_empty()) else {
            return Err(BuildError::generator(format!(
                "the output glob `{glob}` must name a directory"
            )));
        };
        directories.insert(package_root.join(directory));
    }
    Ok(directories.into_iter().collect())
}

fn materialize_record(
    context: &ActionContext,
    record: &ActionRecord,
    name: &str,
    destination: &Path,
    label: &str,
) -> BuildResult<()> {
    let output = record.output(name).ok_or_else(|| {
        BuildError::action(format!("the action `{label}` produced no `{name}` output"))
    })?;
    context
        .cache
        .cas()
        .materialize(output.digest, destination)?;
    set_mode(destination, output.mode)?;
    Ok(())
}

fn set_mode(path: &Path, mode: u32) -> BuildResult<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(mode)).map_err(|error| {
            BuildError::action(format!(
                "cannot set the mode of `{}`: {error}",
                path.display()
            ))
        })?;
    }
    let _ = mode;
    Ok(())
}

fn which(program: &str) -> Option<PathBuf> {
    if program.contains(['/', '\\']) {
        let path = PathBuf::from(program);
        return path.is_file().then_some(path);
    }
    let path = std::env::var_os("PATH")?;
    for directory in std::env::split_paths(&path) {
        let candidate = directory.join(program);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

/// Removes build outputs and the local cache.
pub fn clean(root: &Path) -> BuildResult<Vec<PathBuf>> {
    let mut removed = Vec::new();
    for relative in ["seal/cache", "seal/target"] {
        let path = root.join(relative);
        if path.exists() {
            fs::remove_dir_all(&path).map_err(|error| {
                BuildError::cache(format!("cannot remove `{}`: {error}", path.display()))
            })?;
            removed.push(path);
        }
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::output_directories;

    #[test]
    fn output_directories_come_from_the_literal_prefix() {
        let root = Path::new("/project");
        let directories = output_directories(
            root,
            &[
                "generated/**/*.sl".to_owned(),
                "src/generated/*.sl".to_owned(),
            ],
        )
        .expect("valid globs");
        assert_eq!(
            directories,
            vec![root.join("generated"), root.join("src/generated")]
        );
    }

    #[test]
    fn an_output_glob_without_a_directory_is_rejected() {
        assert!(output_directories(Path::new("/project"), &["*.sl".to_owned()]).is_err());
    }
}
