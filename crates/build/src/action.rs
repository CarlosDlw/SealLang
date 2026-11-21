//! The concrete actions of a project build.
//!
//! Every action has a content key computed from its input digests, hashed
//! environment, and display command. Execution returns an [`ActionRecord`]
//! whose outputs are already stored in the content-addressed store, so storing
//! the record is the commit point of the action.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use seal_cas::{ActionInput, ActionOutput, ActionRecord, Digest};
use seal_codegen::ObjectLink;
use seal_manifest::NativeManifest;
use seal_plan::{BuildMode, CompilePlan};
use seal_sandbox::{SandboxPolicy, SandboxRequest};
use seal_target::{PanicStrategy, RuntimePolicy};
use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

use crate::context::ActionContext;
use crate::error::{BuildError, BuildResult};

/// One node of the action graph.
#[derive(Clone, Debug)]
pub(crate) enum Action {
    /// Analyze one target and write its `.sealmono` artifact.
    Frontend(Arc<FrontendAction>),
    /// Turn one `.sealmono` unit into one native object.
    Codegen(Arc<CodegenAction>),
    /// Link one object into an executable.
    Link(Arc<LinkAction>),
    /// Run a generator executable and collect its declared outputs.
    Generate(Arc<GenerateAction>),
    /// Resolve one native dependency.
    Native(Arc<NativeAction>),
}

impl Action {
    /// Returns the record kind.
    pub(crate) fn kind(&self) -> &'static str {
        match self {
            Self::Frontend(_) => "frontend",
            Self::Codegen(_) => "codegen-cgu",
            Self::Link(_) => "link",
            Self::Generate(_) => "generate",
            Self::Native(_) => "native-probe",
        }
    }

    /// Returns the human-readable label.
    pub(crate) fn label(&self) -> &str {
        match self {
            Self::Frontend(action) => &action.label,
            Self::Codegen(action) => &action.label,
            Self::Link(action) => &action.label,
            Self::Generate(action) => &action.label,
            Self::Native(action) => &action.label,
        }
    }

    /// Returns the input files of this action.
    pub(crate) fn inputs(&self) -> &[ActionInput] {
        match self {
            Self::Frontend(action) => &action.inputs,
            Self::Codegen(action) => &action.inputs,
            Self::Link(action) => &action.inputs,
            Self::Generate(action) => &action.inputs,
            Self::Native(action) => &action.inputs,
        }
    }

    /// Returns the hashed environment of this action.
    pub(crate) fn environment(&self) -> BTreeMap<String, Digest> {
        match self {
            Self::Frontend(action) => action.environment(),
            Self::Codegen(action) => action.environment(),
            Self::Link(action) => action.environment(),
            Self::Generate(action) => action.environment(),
            Self::Native(action) => action.environment.clone(),
        }
    }

    /// Returns the display command used for cache explanation.
    pub(crate) fn command(&self) -> Vec<String> {
        match self {
            Self::Frontend(action) => action.command(),
            Self::Codegen(action) => action.command(),
            Self::Link(action) => action.command(),
            Self::Generate(action) => action.command(),
            Self::Native(action) => action.command(),
        }
    }

    pub(crate) fn execute(&self, context: &ActionContext) -> BuildResult<ActionRecord> {
        match self {
            Self::Frontend(action) => action.execute(context),
            Self::Codegen(action) => action.execute(context),
            Self::Link(action) => action.execute(context),
            Self::Generate(action) => action.execute(context),
            Self::Native(action) => action.execute(context),
        }
    }
}

/// The `frontend` action: one compile plan to one object and link metadata.
#[derive(Clone, Debug)]
pub(crate) struct FrontendAction {
    label: String,
    plan: CompilePlan,
    cgu_count: u32,
    project_root: PathBuf,
    inputs: Vec<ActionInput>,
    temp_root: PathBuf,
}

impl FrontendAction {
    /// Creates the action for one build plan.
    pub(crate) fn new(
        label: impl Into<String>,
        mut plan: CompilePlan,
        cgu_count: u32,
        project_root: &Path,
        temp_root: &Path,
    ) -> BuildResult<Self> {
        plan.mode = BuildMode::Build;
        plan.profile.codegen_units = cgu_count.max(1);
        let mut key_plan = plan.clone();
        key_plan.output = None;
        let plan_bytes = serde_json::to_vec(&key_plan).map_err(|error| {
            BuildError::action(format!("cannot encode the compile plan: {error}"))
        })?;
        let mut inputs = vec![ActionInput {
            path: "plan".to_owned(),
            digest: Digest::of_bytes(&plan_bytes),
        }];
        for package in &plan.packages {
            for file in &package.files {
                let name = relative_name(&file.path, project_root);
                inputs.push(ActionInput::from_file(name, &file.path)?);
            }
        }
        Ok(Self {
            label: label.into(),
            plan,
            cgu_count: cgu_count.max(1),
            project_root: project_root.to_path_buf(),
            inputs,
            temp_root: temp_root.to_path_buf(),
        })
    }

    fn environment(&self) -> BTreeMap<String, Digest> {
        BTreeMap::new()
    }

    fn command(&self) -> Vec<String> {
        vec![
            "sealc".to_owned(),
            "build".to_owned(),
            "--emit=mono".to_owned(),
            format!("--target={}", self.plan.target),
            format!("--cgu={}", self.cgu_count),
            self.label.clone(),
        ]
    }

    fn execute(&self, context: &ActionContext) -> BuildResult<ActionRecord> {
        static SEQUENCE: AtomicU64 = AtomicU64::new(0);
        let work = self.temp_root.join(format!(
            "frontend-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&work).map_err(|error| {
            BuildError::action(format!("cannot create `{}`: {error}", work.display()))
        })?;
        let mono = work.join("artifact.sealmono");
        let metadata = work.join("link.json");
        let plan_path = work.join("plan.json");
        let mut plan = self.plan.clone();
        plan.output = Some(mono.clone());
        plan.write(&plan_path).map_err(|error| {
            BuildError::action(format!("cannot write the compile plan: {error}"))
        })?;

        let arguments = vec![
            "sealc".to_owned(),
            "build".to_owned(),
            "--plan-file".to_owned(),
            plan_path.display().to_string(),
            "--emit=mono".to_owned(),
            "--remap-path-prefix".to_owned(),
            format!("{}=.", self.project_root.display()),
            "--link-metadata".to_owned(),
            metadata.display().to_string(),
        ];
        // The driver still shares process-wide LLVM state. Serializing the
        // frontend keeps concurrent links, generators, and probes meaningful
        // while codegen stays single-threaded until the split pipeline lands.
        let status = {
            let _guard = FRONTEND_LOCK
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            seal_driver::run_from(arguments)
        };
        if status != std::process::ExitCode::SUCCESS {
            let _ = fs::remove_dir_all(&work);
            return Err(BuildError::action(format!(
                "the frontend action `{}` failed",
                self.label
            )));
        }
        let mono_digest = context.cache.cas().put_file(&mono)?;
        let metadata_bytes = match fs::read(&metadata) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                b"{\"version\":1,\"libraries\":[]}".to_vec()
            }
            Err(error) => {
                let _ = fs::remove_dir_all(&work);
                return Err(BuildError::action(format!(
                    "cannot read link metadata for `{}`: {error}",
                    self.label
                )));
            }
        };
        let metadata_digest = context.cache.cas().put_bytes(&metadata_bytes)?;
        let _ = fs::remove_dir_all(&work);
        Ok(ActionRecord::new(
            "frontend",
            self.inputs.clone(),
            vec![
                ActionOutput::data("mono", mono_digest),
                ActionOutput::data("link-metadata", metadata_digest),
            ],
        )
        .with_command(self.command()))
    }
}

static FRONTEND_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// The `codegen-cgu` action: one `.sealmono` unit to one object.
#[derive(Clone, Debug)]
pub(crate) struct CodegenAction {
    label: String,
    mono: Digest,
    cgu: u32,
    inputs: Vec<ActionInput>,
    temp_root: PathBuf,
}

impl CodegenAction {
    /// Creates an action for one code generation unit.
    ///
    /// `profile` hashes the target, entry, optimization level, debug flag, and
    /// unit count. The unit digest alone is not enough: the same bitcode can be
    /// passed to code generation at different optimization levels, and those
    /// requests must not share a cache entry.
    pub(crate) fn new(
        label: impl Into<String>,
        mono: Digest,
        profile: Digest,
        unit: Digest,
        cgu: u32,
        temp_root: &Path,
    ) -> Self {
        let label = format!("{}.cgu{cgu}", label.into());
        Self {
            label,
            mono,
            cgu,
            inputs: vec![
                ActionInput {
                    path: "mono-profile".to_owned(),
                    digest: profile,
                },
                ActionInput {
                    path: format!("mono:cgu{cgu}"),
                    digest: unit,
                },
            ],
            temp_root: temp_root.to_path_buf(),
        }
    }

    fn environment(&self) -> BTreeMap<String, Digest> {
        BTreeMap::new()
    }

    fn command(&self) -> Vec<String> {
        vec![
            "sealc".to_owned(),
            "codegen".to_owned(),
            format!("--cgu={}", self.cgu),
            self.label.clone(),
        ]
    }

    fn execute(&self, context: &ActionContext) -> BuildResult<ActionRecord> {
        static SEQUENCE: AtomicU64 = AtomicU64::new(0);
        let work = self.temp_root.join(format!(
            "codegen-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&work).map_err(|error| {
            BuildError::action(format!("cannot create `{}`: {error}", work.display()))
        })?;
        let mono = context.cache.cas().object_path(self.mono);
        let object = work.join("artifact.o");
        let arguments = vec![
            "sealc".to_owned(),
            "codegen".to_owned(),
            "--mono".to_owned(),
            mono.display().to_string(),
            "--cgu".to_owned(),
            self.cgu.to_string(),
            "-o".to_owned(),
            object.display().to_string(),
        ];
        let status = seal_driver::run_from(arguments);
        if status != std::process::ExitCode::SUCCESS {
            let _ = fs::remove_dir_all(&work);
            return Err(BuildError::action(format!(
                "the codegen action `{}` failed",
                self.label
            )));
        }
        let digest = context.cache.cas().put_file(&object)?;
        let _ = fs::remove_dir_all(&work);
        Ok(ActionRecord::new(
            "codegen-cgu",
            self.inputs.clone(),
            vec![ActionOutput::data("object", digest)],
        )
        .with_command(self.command()))
    }
}

/// The artifact a link action produces.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LinkKind {
    /// A native executable with the C `main` wrapper.
    Executable,
    /// A static C library containing the objects and runtime.
    StaticLib,
    /// A shared C library containing the objects and runtime.
    Cdylib,
}

impl LinkKind {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Executable => "exe",
            Self::StaticLib => "staticlib",
            Self::Cdylib => "cdylib",
        }
    }

    pub(crate) const fn output_name(self) -> &'static str {
        match self {
            Self::Executable => "executable",
            Self::StaticLib | Self::Cdylib => "library",
        }
    }
}

/// Target, runtime, and reproducibility settings shared by every link.
#[derive(Clone, Debug)]
pub(crate) struct LinkSettings {
    pub(crate) runtime: RuntimePolicy,
    pub(crate) panic: PanicStrategy,
    pub(crate) target: String,
    pub(crate) linker: Option<String>,
    pub(crate) linker_script: Option<PathBuf>,
    pub(crate) sysroot: Option<PathBuf>,
    pub(crate) source_date_epoch: u64,
    pub(crate) reproducible: bool,
    pub(crate) toolchain: String,
    pub(crate) link_args: Vec<OsString>,
}

impl LinkSettings {
    pub(crate) fn from_plan(plan: &CompilePlan) -> Self {
        let target = plan.target_spec.as_ref();
        Self {
            runtime: target
                .and_then(|target| RuntimePolicy::parse(&target.runtime))
                .unwrap_or_default(),
            panic: target
                .and_then(|target| PanicStrategy::parse(&target.panic))
                .unwrap_or_default(),
            target: plan.target.clone(),
            linker: target.and_then(|target| target.linker.clone()),
            linker_script: target.and_then(|target| target.linker_script.clone()),
            sysroot: target.and_then(|target| target.sysroot.clone()),
            source_date_epoch: target.map_or(0, |target| target.source_date_epoch),
            reproducible: target.is_none_or(|target| target.reproducible),
            toolchain: plan.metadata.toolchain.clone(),
            link_args: target
                .map(|target| target.link_args.iter().map(OsString::from).collect())
                .unwrap_or_default(),
        }
    }
}

/// Inputs collected before a [`LinkAction`] is created.
#[derive(Clone, Debug)]
pub(crate) struct LinkActionSpec<'a> {
    /// The artifact kind.
    pub(crate) kind: LinkKind,
    /// The object digests, in order.
    pub(crate) objects: Vec<Digest>,
    /// The frontend link metadata digest.
    pub(crate) metadata: Digest,
    /// Resolved native dependencies.
    pub(crate) native: Vec<NativeResult>,
    /// Extra linker arguments from the command line.
    pub(crate) link_args: Vec<OsString>,
    /// An explicit linker program.
    pub(crate) linker: Option<OsString>,
    /// Target, runtime, and reproducibility settings.
    pub(crate) settings: LinkSettings,
    /// The directory for temporary link files.
    pub(crate) temp_root: &'a Path,
}

/// The `link` action: one or more objects plus runtime and native libraries.
#[derive(Clone, Debug)]
pub(crate) struct LinkAction {
    label: String,
    kind: LinkKind,
    objects: Vec<Digest>,
    metadata: Digest,
    native: Vec<NativeResult>,
    link_args: Vec<OsString>,
    linker: Option<OsString>,
    runtime: RuntimePolicy,
    panic: PanicStrategy,
    target: String,
    linker_script: Option<PathBuf>,
    sysroot: Option<PathBuf>,
    source_date_epoch: u64,
    reproducible: bool,
    toolchain: String,
    inputs: Vec<ActionInput>,
    temp_root: PathBuf,
}

impl LinkAction {
    /// Creates a link action from a frontend record and resolved native inputs.
    pub(crate) fn new(label: impl Into<String>, spec: LinkActionSpec<'_>) -> Self {
        let LinkActionSpec {
            kind,
            objects,
            metadata,
            native,
            mut link_args,
            linker,
            mut settings,
            temp_root,
        } = spec;
        let linker = linker.or_else(|| settings.linker.take().map(OsString::from));
        link_args.extend(settings.link_args.iter().cloned());
        for entry in &native {
            link_args.extend(entry.link_args.iter().map(OsString::from));
        }
        let mut inputs = vec![ActionInput {
            path: "link-metadata".to_owned(),
            digest: metadata,
        }];
        for (index, object) in objects.iter().enumerate() {
            inputs.push(ActionInput {
                path: format!("object:{index}"),
                digest: *object,
            });
        }
        for entry in &native {
            inputs.push(ActionInput {
                path: format!("native:{}", entry.alias),
                digest: Digest::of_bytes(
                    serde_json::to_string(entry).unwrap_or_default().as_bytes(),
                ),
            });
        }
        Self {
            label: label.into(),
            kind,
            objects,
            metadata,
            native,
            link_args,
            linker,
            runtime: settings.runtime,
            panic: settings.panic,
            target: settings.target,
            linker_script: settings.linker_script,
            sysroot: settings.sysroot,
            source_date_epoch: settings.source_date_epoch,
            reproducible: settings.reproducible,
            toolchain: settings.toolchain,
            inputs,
            temp_root: temp_root.to_path_buf(),
        }
    }

    fn environment(&self) -> BTreeMap<String, Digest> {
        BTreeMap::new()
    }

    fn command(&self) -> Vec<String> {
        let mut command = vec![
            "link".to_owned(),
            self.kind.as_str().to_owned(),
            self.label.clone(),
            format!("--target={}", self.target),
            format!("--runtime={}", self.runtime),
            format!("--panic={}", self.panic),
            format!("--toolchain={}", self.toolchain),
            format!("--epoch={}", self.source_date_epoch),
            format!("--reproducible={}", self.reproducible),
        ];
        if let Some(linker) = &self.linker {
            command.push(format!("--linker={}", linker.to_string_lossy()));
        }
        if let Some(script) = &self.linker_script {
            command.push(format!("--linker-script={}", script.display()));
        }
        if let Some(sysroot) = &self.sysroot {
            command.push(format!("--sysroot={}", sysroot.display()));
        }
        for argument in &self.link_args {
            command.push(format!("--link-arg={}", argument.to_string_lossy()));
        }
        for native in &self.native {
            command.push(format!("native:{}", native.alias));
        }
        command
    }

    fn execute(&self, context: &ActionContext) -> BuildResult<ActionRecord> {
        static SEQUENCE: AtomicU64 = AtomicU64::new(0);
        let work = self.temp_root.join(format!(
            "link-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&work).map_err(|error| {
            BuildError::action(format!("cannot create `{}`: {error}", work.display()))
        })?;
        let output = work.join("artifact");
        let metadata = context.cache.cas().get(self.metadata)?;
        let libraries = read_source_libraries(&metadata)?;
        let mut libraries = libraries;
        let mut search_paths = Vec::new();
        let mut link_args = Vec::new();
        for native in &self.native {
            libraries.extend(native.libs.iter().cloned());
            search_paths.extend(native.search_paths.iter().cloned());
            link_args.extend(native.link_args.iter().map(OsString::from));
        }
        link_args.extend(self.link_args.iter().cloned());
        let objects: Vec<PathBuf> = self
            .objects
            .iter()
            .map(|digest| context.cache.cas().object_path(*digest))
            .collect();
        let runtime_bytes = (self.runtime == RuntimePolicy::Default
            && self.kind != LinkKind::StaticLib)
            .then(seal_codegen::runtime_archive);
        let runtime = runtime_bytes;
        let linker = self.linker.as_deref();
        let result = match self.kind {
            LinkKind::StaticLib => {
                let request = seal_codegen::ObjectArchive {
                    objects: &objects,
                    output: &output,
                    runtime: (self.runtime == RuntimePolicy::Default)
                        .then(seal_codegen::runtime_archive),
                };
                seal_codegen::archive_objects(&request)
            }
            LinkKind::Executable | LinkKind::Cdylib => {
                let request = ObjectLink {
                    objects: &objects,
                    output: &output,
                    runtime,
                    libraries: &libraries,
                    search_paths: &search_paths,
                    link_args: &link_args,
                    linker,
                    shared: self.kind == LinkKind::Cdylib,
                    freestanding: self.runtime == RuntimePolicy::None,
                    target: &self.target,
                    linker_script: self.linker_script.as_deref(),
                    sysroot: self.sysroot.as_deref(),
                    source_date_epoch: self.source_date_epoch,
                    reproducible: self.reproducible,
                };
                seal_codegen::link_objects(&request)
            }
        };
        if let Err(error) = result {
            let _ = fs::remove_dir_all(&work);
            return Err(BuildError::action(format!(
                "the link action `{}` failed: {error}",
                self.label
            )));
        }
        let digest = context.cache.cas().put_file(&output)?;
        let _ = fs::remove_dir_all(&work);
        let artifact = if self.kind == LinkKind::Executable {
            ActionOutput::executable(self.kind.output_name(), digest)
        } else {
            ActionOutput::data(self.kind.output_name(), digest)
        };
        Ok(
            ActionRecord::new("link", self.inputs.clone(), vec![artifact])
                .with_command(self.command()),
        )
    }
}

fn read_source_libraries(bytes: &[u8]) -> BuildResult<Vec<String>> {
    #[derive(Deserialize)]
    struct Metadata {
        #[serde(default)]
        libraries: Vec<String>,
    }
    if bytes.is_empty() {
        return Ok(Vec::new());
    }
    let metadata: Metadata = serde_json::from_slice(bytes)
        .map_err(|error| BuildError::action(format!("invalid link metadata: {error}")))?;
    Ok(metadata.libraries)
}

/// The `generate` action: one generator executable and its declared outputs.
#[derive(Clone, Debug)]
pub(crate) struct GenerateAction {
    label: String,
    package_root: PathBuf,
    target: String,
    tool: Digest,
    inputs: Vec<ActionInput>,
    output_globs: Vec<String>,
    output_dirs: Vec<PathBuf>,
    temp_root: PathBuf,
}

impl GenerateAction {
    /// Creates a generator action.
    pub(crate) fn new(
        package_root: &Path,
        target: &str,
        tool: Digest,
        inputs: Vec<ActionInput>,
        output_globs: Vec<String>,
        output_dirs: Vec<PathBuf>,
        temp_root: &Path,
    ) -> BuildResult<Self> {
        if output_globs.is_empty() || output_dirs.is_empty() {
            return Err(BuildError::generator(format!(
                "the generator `{target}` declares no output directory"
            )));
        }
        Ok(Self {
            label: target.to_owned(),
            package_root: package_root.to_path_buf(),
            target: target.to_owned(),
            tool,
            inputs,
            output_globs,
            output_dirs,
            temp_root: temp_root.to_path_buf(),
        })
    }

    fn environment(&self) -> BTreeMap<String, Digest> {
        let mut env = BTreeMap::new();
        env.insert("PATH".to_owned(), Digest::of_bytes(b"/usr/bin:/bin"));
        env
    }

    fn command(&self) -> Vec<String> {
        vec![
            "generate".to_owned(),
            self.target.clone(),
            self.output_globs.join(","),
        ]
    }

    fn execute(&self, context: &ActionContext) -> BuildResult<ActionRecord> {
        static SEQUENCE: AtomicU64 = AtomicU64::new(0);
        let work = self.temp_root.join(format!(
            "generate-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&work).map_err(|error| {
            BuildError::generator(format!("cannot create `{}`: {error}", work.display()))
        })?;
        let tool = work.join(&self.target);
        materialize_executable(context, self.tool, &tool)?;
        let mut builder = globset::GlobSetBuilder::new();
        for glob in &self.output_globs {
            builder.add(globset::Glob::new(glob).map_err(|error| {
                BuildError::generator(format!("invalid output glob `{glob}`: {error}"))
            })?);
        }
        let matcher = builder
            .build()
            .map_err(|error| BuildError::generator(format!("invalid output globs: {error}")))?;

        // Generators write into a staged tree with the package layout, never
        // directly into the package. A failed or undeclared output therefore
        // cannot damage a source file, and the sandbox exposes exactly the
        // declared inputs instead of the whole package root.
        let stage = work.join("stage");
        let mut staged_outputs = Vec::new();
        for directory in &self.output_dirs {
            let relative = directory.strip_prefix(&self.package_root).map_err(|_| {
                BuildError::generator(format!(
                    "the output directory `{}` leaves the package",
                    directory.display()
                ))
            })?;
            let staged = stage.join(relative);
            fs::create_dir_all(&staged).map_err(|error| {
                BuildError::generator(format!("cannot create `{}`: {error}", staged.display()))
            })?;
            staged_outputs.push(staged);
        }
        let inputs: Vec<PathBuf> = self
            .inputs
            .iter()
            .map(|input| self.package_root.join(&input.path))
            .collect();
        let output_paths: Vec<String> = staged_outputs
            .iter()
            .map(|directory| directory.display().to_string())
            .collect();
        let mut policy = SandboxPolicy::new()
            .read(&tool)
            .write(&stage)
            .working_dir(&stage)
            .allow_portable(true)
            .env("PATH", "/usr/bin:/bin")
            .env(
                "SEAL_GENERATOR_INPUTS",
                inputs
                    .iter()
                    .map(|path| path.display().to_string())
                    .collect::<Vec<_>>()
                    .join(" "),
            )
            .env("SEAL_GENERATOR_OUTPUTS", output_paths.join(" "))
            .env("SEAL_GENERATOR_TARGET", &self.target);
        for input in &inputs {
            policy = policy.read(input);
        }
        let request = SandboxRequest::new(&tool)
            .policy(policy)
            .args(Vec::<OsString>::new());
        context.sandbox.run_checked(&request)?;

        let mut produced = Vec::new();
        for entry in WalkDir::new(&stage).follow_links(false) {
            let entry = entry.map_err(|error| {
                BuildError::generator(format!("cannot inspect `{}`: {error}", stage.display()))
            })?;
            if !entry.file_type().is_file() {
                continue;
            }
            let relative = entry.path().strip_prefix(&stage).map_err(|_| {
                BuildError::generator(format!(
                    "the generator wrote outside the staging directory: `{}`",
                    entry.path().display()
                ))
            })?;
            let name = slash_path(relative);
            if !matcher.is_match(&name) {
                let _ = fs::remove_dir_all(&work);
                return Err(BuildError::generator(format!(
                    "the generator `{}` wrote the undeclared output `{name}`",
                    self.target
                )));
            }
            produced.push((name, entry.path().to_path_buf()));
        }
        produced.sort_by(|left, right| left.0.cmp(&right.0));
        if produced.is_empty() {
            let _ = fs::remove_dir_all(&work);
            return Err(BuildError::generator(format!(
                "the generator `{}` produced none of its declared outputs: {}",
                self.target,
                self.output_globs.join(", ")
            )));
        }
        remove_existing_outputs(&self.package_root, &self.output_dirs, &matcher)?;
        let mut outputs = Vec::with_capacity(produced.len());
        for (name, path) in &produced {
            let digest = context.cache.cas().put_file(path)?;
            outputs.push(ActionOutput::data(name.clone(), digest));
        }
        let _ = fs::remove_dir_all(&work);
        Ok(
            ActionRecord::new("generate", self.inputs.clone(), outputs)
                .with_command(self.command()),
        )
    }
}

fn remove_existing_outputs(
    package_root: &Path,
    directories: &[PathBuf],
    matcher: &globset::GlobSet,
) -> BuildResult<()> {
    for directory in directories {
        if !directory.exists() {
            fs::create_dir_all(directory).map_err(|error| {
                BuildError::generator(format!(
                    "cannot create the output directory `{}`: {error}",
                    directory.display()
                ))
            })?;
            continue;
        }
        for entry in WalkDir::new(directory).follow_links(false) {
            let entry = entry.map_err(|error| {
                BuildError::generator(format!("cannot inspect `{}`: {error}", directory.display()))
            })?;
            if !entry.file_type().is_file() {
                continue;
            }
            let relative = entry
                .path()
                .strip_prefix(package_root)
                .map(slash_path)
                .unwrap_or_default();
            if matcher.is_match(&relative) {
                let _ = fs::remove_file(entry.path());
            }
        }
    }
    Ok(())
}

/// One resolved native dependency.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NativeResult {
    /// The package that declares the alias.
    pub package: String,
    /// The alias from `[native.<alias>]`.
    pub alias: String,
    /// The pkg-config package, when one was declared.
    pub pkg_config: Option<String>,
    /// Whether static libraries were requested.
    pub static_link: bool,
    /// Library names, in link order.
    pub libs: Vec<String>,
    /// Directories searched by the linker.
    pub search_paths: Vec<PathBuf>,
    /// Extra linker flags.
    pub link_args: Vec<String>,
    /// The command that produced this result, for diagnostics.
    pub command: Vec<String>,
}

/// The `native-probe` action.
#[derive(Clone, Debug)]
pub(crate) struct NativeAction {
    label: String,
    package: String,
    alias: String,
    package_root: PathBuf,
    declaration: NativeManifest,
    pkg_config: Option<PathBuf>,
    inputs: Vec<ActionInput>,
    environment: BTreeMap<String, Digest>,
}

impl NativeAction {
    /// Creates a native probe action and hashes the pkg-config executable.
    pub(crate) fn new(
        package: &str,
        alias: &str,
        package_root: &Path,
        declaration: &NativeManifest,
        pkg_config: Option<&Path>,
        pkg_config_path: Option<&str>,
    ) -> BuildResult<Self> {
        let declaration_bytes = serde_json::to_vec(declaration).map_err(|error| {
            BuildError::native(format!("cannot encode the native declaration: {error}"))
        })?;
        let mut inputs = vec![ActionInput {
            path: "declaration".to_owned(),
            digest: Digest::of_bytes(&declaration_bytes),
        }];
        if let Some(path) = pkg_config {
            inputs.push(ActionInput::from_file("pkg-config", path)?);
        }
        let mut environment = BTreeMap::new();
        if let Some(value) = pkg_config_path {
            environment.insert(
                "PKG_CONFIG_PATH".to_owned(),
                Digest::of_bytes(value.as_bytes()),
            );
        }
        Ok(Self {
            label: format!("native.{alias}"),
            package: package.to_owned(),
            alias: alias.to_owned(),
            package_root: package_root.to_path_buf(),
            declaration: declaration.clone(),
            pkg_config: pkg_config.map(Path::to_path_buf),
            inputs,
            environment,
        })
    }

    fn command(&self) -> Vec<String> {
        let mut command = vec!["pkg-config".to_owned()];
        if self.declaration.static_link {
            command.push("--static".to_owned());
        }
        command.push("--libs".to_owned());
        if let Some(name) = &self.declaration.pkg_config {
            command.push(name.clone());
        }
        command
    }

    fn execute(&self, context: &ActionContext) -> BuildResult<ActionRecord> {
        let mut result = NativeResult {
            package: self.package.clone(),
            alias: self.alias.clone(),
            pkg_config: self.declaration.pkg_config.clone(),
            static_link: self.declaration.static_link,
            libs: self.declaration.libs.clone(),
            search_paths: self
                .declaration
                .search_paths
                .iter()
                .map(|path| self.package_root.join(path))
                .collect(),
            link_args: Vec::new(),
            command: self.command(),
        };
        if let Some(name) = &self.declaration.pkg_config {
            let program = self.pkg_config.as_ref().ok_or_else(|| {
                BuildError::native(format!(
                    "the native dependency `{}` needs pkg-config, but it was not found on PATH",
                    self.alias
                ))
            })?;
            let mut arguments = vec![OsString::from("--libs")];
            if self.declaration.static_link {
                arguments.push(OsString::from("--static"));
            }
            arguments.push(OsString::from(name));
            let mut policy = SandboxPolicy::new()
                .read(&self.package_root)
                .working_dir(&self.package_root)
                .allow_portable(true)
                .env("PATH", "/usr/bin:/bin");
            if let Some(value) = std::env::var_os("PKG_CONFIG_PATH") {
                policy = policy.env("PKG_CONFIG_PATH", value.to_string_lossy().into_owned());
            }
            let request = SandboxRequest::new(program).policy(policy).args(arguments);
            let output = context.sandbox.run_checked(&request).map_err(|error| {
                BuildError::native(format!(
                    "pkg-config failed for `{name}` (native `{}`): {error}",
                    self.alias
                ))
            })?;
            let (libs, paths, flags) = parse_pkg_config(&output.stdout_text())?;
            result.libs.extend(libs);
            result.search_paths.extend(paths);
            result.link_args.extend(flags);
            result.command.push(name.clone());
        }
        let bytes = serde_json::to_vec(&result).map_err(|error| {
            BuildError::native(format!("cannot encode the native result: {error}"))
        })?;
        let digest = context.cache.cas().put_bytes(&bytes)?;
        Ok(ActionRecord::new(
            "native-probe",
            self.inputs.clone(),
            vec![ActionOutput::data("result", digest)],
        )
        .with_command(self.command()))
    }
}

fn parse_pkg_config(text: &str) -> BuildResult<(Vec<String>, Vec<PathBuf>, Vec<String>)> {
    let mut libs = Vec::new();
    let mut paths = Vec::new();
    let mut flags = Vec::new();
    for token in text.split_whitespace() {
        if let Some(rest) = token.strip_prefix("-l") {
            if rest.is_empty() {
                return Err(BuildError::native("pkg-config returned an empty `-l` flag"));
            }
            libs.push(rest.to_owned());
        } else if let Some(rest) = token.strip_prefix("-L") {
            if rest.is_empty() {
                return Err(BuildError::native("pkg-config returned an empty `-L` flag"));
            }
            paths.push(PathBuf::from(rest));
        } else if token.starts_with("-Wl,") || token.starts_with("-pthread") {
            flags.push(token.to_owned());
        } else {
            return Err(BuildError::native(format!(
                "pkg-config returned the unsupported flag `{token}`"
            )));
        }
    }
    Ok((libs, paths, flags))
}

fn materialize_executable(
    context: &ActionContext,
    digest: Digest,
    destination: &Path,
) -> BuildResult<()> {
    context.cache.cas().materialize(digest, destination)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(destination, fs::Permissions::from_mode(0o755)).map_err(|error| {
            BuildError::action(format!(
                "cannot make `{}` executable: {error}",
                destination.display()
            ))
        })?;
    }
    Ok(())
}

fn relative_name(path: &Path, root: &Path) -> String {
    path.strip_prefix(root)
        .map(slash_path)
        .unwrap_or_else(|_| path.display().to_string())
}

/// Renders a path with forward slashes for deterministic cache names.
pub(crate) fn slash_path(path: &Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(test)]
mod tests {
    use super::parse_pkg_config;

    #[test]
    fn pkg_config_flags_are_partitioned() {
        let (libs, paths, flags) =
            parse_pkg_config("-L/usr/lib -lz -pthread -Wl,-rpath,/x").expect("valid output");
        assert_eq!(libs, vec!["z"]);
        assert_eq!(paths, vec![std::path::PathBuf::from("/usr/lib")]);
        assert_eq!(flags, vec!["-pthread", "-Wl,-rpath,/x"]);
    }

    #[test]
    fn an_unsupported_pkg_config_flag_is_rejected() {
        assert!(parse_pkg_config("-DFOO").is_err());
    }
}
