//! The compile plan itself.

use std::path::PathBuf;

use seal_target::{CAbiDefinition, TargetDefinition, TargetSpec, TargetTriple};
use seal_version::{Edition, LanguageVersion};
use serde::{Deserialize, Serialize};

use crate::{PlanError, PlanResult};

/// The plan schema version understood by this compiler.
///
/// Version 3 adds the per-package edition. Version 2 plans are upgraded to the
/// current edition in memory; version 1 plans are read as `stdlib = "none"`
/// with no prelude.
pub const FORMAT_VERSION: u32 = 3;

/// What the compiler should produce.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BuildMode {
    /// Analyze without writing an artifact.
    Check,
    /// Write the artifact named by [`CompilePlan::output`].
    Build,
}

/// One source file in a package plan.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanFile {
    /// Absolute path to the source file.
    pub path: PathBuf,
    /// Whether this is the package's entry file.
    pub primary: bool,
}

/// One dependency edge in a package plan.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanDependency {
    /// The local name imports use.
    pub alias: String,
    /// Index into [`CompilePlan::packages`]; it must precede the dependent.
    pub package: usize,
}

/// The package prelude selected for one package.
///
/// A prelude names exactly one source: either a user package in the plan or a
/// package of the standard library. The standard-library form keeps the plan
/// independent of the installed layout; the driver resolves the name after it
/// loads a standard library with the digest named by the plan.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanPrelude {
    /// Index into [`CompilePlan::packages`], for a prelude from a user package.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package: Option<usize>,
    /// Standard-library package name, for a prelude from the standard library.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stdlib: Option<String>,
    /// Module path inside that package, such as `prelude` or `root`.
    pub module: String,
}

/// The standard-library mode selected for one package.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PlanStdlibMode {
    /// The hosted standard library, which includes `core`.
    Std,
    /// The dependency-free core library.
    Core,
    /// No standard library is injected.
    #[default]
    None,
}

/// The standard-library identity a plan depends on.
///
/// The plan deliberately carries no absolute standard-library paths and no
/// sources. The same plan therefore compiles against any installation whose
/// descriptor, language, version, and digest match.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanStdlib {
    /// Language version the standard library targets, such as `0.1`.
    pub language: String,
    /// Standard-library release version.
    pub version: String,
    /// Stable content digest of the descriptor, manifests, and sources.
    pub digest: String,
}

/// Configuration values selected for this compilation.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CfgOptions {
    /// Features enabled for the package instance.
    pub features: Vec<String>,
    /// Target operating system, as used by `cfg(target.os)`.
    pub target_os: String,
    /// Target architecture, as used by `cfg(target.arch)`.
    pub target_arch: String,
    /// Target environment, as used by `cfg(target.env)`.
    pub target_env: String,
    /// Pointer width in bits.
    pub pointer_width: u8,
    /// Whether `cfg(debug_assertions)` is active.
    pub debug_assertions: bool,
    /// Whether `cfg(test)` is active.
    pub test: bool,
    /// Whether `cfg(bench)` is active.
    pub bench: bool,
    /// Declared environment values available to `cfg(env("..."))`.
    #[serde(default)]
    pub env: std::collections::BTreeMap<String, String>,
}

/// One package of the compilation, in dependency order.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanPackage {
    /// The package name.
    pub name: String,
    /// The package version.
    pub version: String,
    /// The compatibility edition selected by the package.
    #[serde(default)]
    pub edition: String,
    /// The compiled package instance.
    pub instance: u32,
    /// Absolute path to the package root.
    pub root: PathBuf,
    /// The package's source files.
    pub files: Vec<PlanFile>,
    /// Module paths exposed by the package API. Empty exposes the root.
    #[serde(default)]
    pub exposed: Vec<String>,
    /// Dependency aliases and their package indexes.
    #[serde(default)]
    pub dependencies: Vec<PlanDependency>,
    /// Features selected for this instance.
    #[serde(default)]
    pub cfg: CfgOptions,
    /// The module whose public items are implicitly in scope in this package.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prelude: Option<PlanPrelude>,
    /// The standard-library mode injected into this package.
    #[serde(default)]
    pub stdlib: PlanStdlibMode,
}

impl PlanPackage {
    /// Returns the parsed edition after the plan has been validated.
    ///
    /// A version 2 plan with no edition is the current edition; a version 3
    /// plan without one is rejected by [`CompilePlan::validate`].
    pub fn edition(&self) -> Edition {
        if self.edition.is_empty() {
            Edition::CURRENT
        } else {
            Edition::parse(&self.edition).unwrap_or(Edition::CURRENT)
        }
    }
}

/// The profile values that affect the artifact.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PlanProfile {
    /// Optimization level, 0 through 3.
    pub opt_level: u8,
    /// Whether debug information is emitted.
    pub debug: bool,
    /// Whether debug assertions are active.
    pub debug_assertions: bool,
    /// Link-time optimization mode: `off`, `thin`, or `full`.
    pub lto: String,
    /// Number of code generation units.
    pub codegen_units: u32,
    /// Whether symbols are stripped.
    pub strip: bool,
    /// Whether incremental compilation is enabled.
    pub incremental: bool,
}

impl Default for PlanProfile {
    fn default() -> Self {
        Self {
            opt_level: 0,
            debug: false,
            debug_assertions: true,
            lto: "off".to_owned(),
            codegen_units: 1,
            strip: false,
            incremental: false,
        }
    }
}

/// Information a tool needs without reading any source.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PlanMetadata {
    /// The project name.
    pub project: String,
    /// The target name.
    pub target: String,
    /// The target kind (`bin`, `lib`, `test`, ...).
    pub kind: String,
    /// The source root of the primary package.
    pub root: String,
    /// The toolchain version that produced the plan.
    pub toolchain: String,
}

/// The target properties that affect code generation and linking.
///
/// The plan carries an explicit representation instead of the target crate's
/// type so a plan file names every property it relies on. `None` keeps the
/// historical behavior: derive the properties from `target`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PlanTarget {
    /// The target triple.
    pub triple: String,
    /// Pointer width in bits (`32` or `64`).
    pub pointer_width: u8,
    /// Byte order (`little` or `big`).
    pub endianness: String,
    /// Explicit LLVM data layout, when the target declares one.
    pub data_layout: Option<String>,
    /// Whether plain C `char` is signed.
    pub c_char_signed: bool,
    /// Width of C `long` in bits.
    pub c_long_width: u16,
    /// Width of C `wchar_t` in bits.
    pub c_wchar_width: u16,
    /// Whether C `wchar_t` is signed.
    pub c_wchar_signed: bool,
    /// Runtime policy (`default` or `none`).
    pub runtime: String,
    /// Panic policy (`abort` or `trap`).
    pub panic: String,
    /// Linker program used for executable targets.
    pub linker: Option<String>,
    /// Sysroot passed to the C toolchain.
    pub sysroot: Option<PathBuf>,
    /// Linker script passed to the C toolchain.
    pub linker_script: Option<PathBuf>,
    /// Extra linker arguments declared by the target.
    pub link_args: Vec<String>,
    /// Timestamp used by reproducible artifacts.
    pub source_date_epoch: u64,
    /// Whether outputs are canonicalized for reproducibility.
    pub reproducible: bool,
}

impl PlanTarget {
    /// Captures a target specification and the target's link settings.
    pub fn from_spec(
        spec: &TargetSpec,
        link_args: Vec<String>,
        source_date_epoch: u64,
        reproducible: bool,
    ) -> Self {
        let abi = spec.c_abi();
        Self {
            triple: spec.triple().as_str().to_owned(),
            pointer_width: spec.pointer_width(),
            endianness: match spec.endianness() {
                seal_target::Endianness::Little => "little",
                seal_target::Endianness::Big => "big",
            }
            .to_owned(),
            data_layout: spec.data_layout().map(ToOwned::to_owned),
            c_char_signed: abi.char_signed(),
            c_long_width: abi.long_width(),
            c_wchar_width: abi.wchar_width(),
            c_wchar_signed: abi.wchar_signed(),
            runtime: spec.runtime().as_str().to_owned(),
            panic: spec.panic().as_str().to_owned(),
            linker: spec.linker().map(ToOwned::to_owned),
            sysroot: spec.sysroot().map(PathBuf::from),
            linker_script: spec.linker_script().map(PathBuf::from),
            link_args,
            source_date_epoch,
            reproducible,
        }
    }

    /// Resolves the plan representation back into a validated target.
    pub fn to_spec(&self) -> PlanResult<TargetSpec> {
        if self
            .link_args
            .iter()
            .any(|argument| argument.is_empty() || argument.contains('\0'))
        {
            return Err(PlanError::invalid(
                "the plan target declares an empty or NUL-containing link argument",
            ));
        }
        let definition = TargetDefinition {
            triple: self.triple.clone(),
            data_layout: self.data_layout.clone(),
            pointer_width: Some(self.pointer_width),
            endianness: Some(self.endianness.clone()),
            c_abi: Some(CAbiDefinition {
                char_signed: self.c_char_signed,
                long_width: self.c_long_width,
                wchar_width: self.c_wchar_width,
                wchar_signed: self.c_wchar_signed,
            }),
            runtime: Some(self.runtime.clone()),
            panic: Some(self.panic.clone()),
            linker: self.linker.clone(),
            sysroot: self.sysroot.clone(),
            linker_script: self.linker_script.clone(),
        };
        definition
            .resolve()
            .map_err(|error| PlanError::invalid(error.to_string()))
    }
}

/// One validated compilation request.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompilePlan {
    /// The schema version; must equal [`FORMAT_VERSION`].
    pub version: u32,
    /// The target triple.
    pub target: String,
    /// Artifact options.
    pub profile: PlanProfile,
    /// Whether to check or build.
    pub mode: BuildMode,
    /// Packages in dependency order. A package depends only on earlier ones.
    pub packages: Vec<PlanPackage>,
    /// Index of the package being compiled.
    pub primary: usize,
    /// The standard-library identity every non-`none` package depends on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stdlib: Option<PlanStdlib>,
    /// Entry function path, such as `root::main` or `app::main`.
    #[serde(default)]
    pub entry: Option<String>,
    /// Non-semantic metadata for tools.
    #[serde(default)]
    pub metadata: PlanMetadata,
    /// Artifact path selected by the project layer.
    #[serde(default)]
    pub output: Option<PathBuf>,
    /// Target properties for custom, freestanding, and cross builds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_spec: Option<PlanTarget>,
}

impl CompilePlan {
    /// Returns the primary package.
    pub fn primary_package(&self) -> &PlanPackage {
        &self.packages[self.primary]
    }

    /// Validates every structural invariant the driver relies on.
    pub fn validate(&self) -> PlanResult<()> {
        if !(2..=FORMAT_VERSION).contains(&self.version) {
            return Err(PlanError::version(self.version));
        }
        if TargetTriple::parse(&self.target).is_err() {
            return Err(PlanError::invalid(format!(
                "the compile plan names an unsupported target `{}`",
                self.target
            )));
        }
        if let Some(target_spec) = &self.target_spec {
            let spec = target_spec.to_spec()?;
            if spec.triple().as_str() != self.target {
                return Err(PlanError::invalid(format!(
                    "the plan target specification names `{}` but the plan compiles for `{}`",
                    spec.triple(),
                    self.target
                )));
            }
        }
        if self.packages.is_empty() {
            return Err(PlanError::invalid("the compile plan contains no packages"));
        }
        if self.primary >= self.packages.len() {
            return Err(PlanError::invalid(format!(
                "the primary package index {} is outside the {} package(s) of the plan",
                self.primary,
                self.packages.len()
            )));
        }
        if self.profile.opt_level > 3 {
            return Err(PlanError::invalid(format!(
                "the profile requests optimization level {}, but only 0 through 3 exist",
                self.profile.opt_level
            )));
        }
        if self.profile.codegen_units == 0 {
            return Err(PlanError::invalid(
                "the profile requests zero code generation units",
            ));
        }
        if !matches!(self.profile.lto.as_str(), "off" | "thin" | "full") {
            return Err(PlanError::invalid(format!(
                "the profile names the unknown LTO mode `{}`",
                self.profile.lto
            )));
        }
        if let Some(entry) = &self.entry {
            validate_entry(entry)?;
        }
        if self.mode == BuildMode::Build && self.output.is_none() {
            return Err(PlanError::invalid(
                "a build plan must name the artifact path in `output`",
            ));
        }
        if let Some(stdlib) = &self.stdlib {
            let language = LanguageVersion::parse(&stdlib.language).map_err(|error| {
                PlanError::stdlib_mismatch(format!(
                    "the plan names the standard-library language `{}`, which is invalid: {error}",
                    stdlib.language
                ))
            })?;
            if language != LanguageVersion::CURRENT {
                return Err(PlanError::stdlib_mismatch(format!(
                    "the plan targets standard-library language `{language}`, but this compiler implements `{}`",
                    LanguageVersion::CURRENT
                )));
            }
            semver::Version::parse(&stdlib.version).map_err(|error| {
                PlanError::stdlib_mismatch(format!(
                    "the plan names the standard-library version `{}`, which is not semantic: {error}",
                    stdlib.version
                ))
            })?;
            if stdlib.digest.len() != 64
                || !stdlib.digest.bytes().all(|byte| byte.is_ascii_hexdigit())
            {
                return Err(PlanError::stdlib_mismatch(
                    "the plan standard-library digest is not 64 hexadecimal digits",
                ));
            }
        }

        let mut names = std::collections::BTreeSet::new();
        let mut files: std::collections::BTreeMap<PathBuf, (String, u32)> =
            std::collections::BTreeMap::new();
        for (index, package) in self.packages.iter().enumerate() {
            if package.name.trim().is_empty() {
                return Err(PlanError::invalid(format!(
                    "package {index} has an empty name"
                )));
            }
            if !names.insert((package.name.clone(), package.instance)) {
                return Err(PlanError::invalid(format!(
                    "package `{}` instance {} appears twice in the plan",
                    package.name, package.instance
                )));
            }
            if package.files.is_empty() {
                return Err(PlanError::invalid(format!(
                    "package `{}` has no source files",
                    package.name
                )));
            }
            if package.edition.is_empty() {
                if self.version >= 3 {
                    return Err(PlanError::invalid(format!(
                        "package `{}` names no edition",
                        package.name
                    )));
                }
            } else {
                Edition::parse(&package.edition).map_err(|error| {
                    PlanError::invalid(format!(
                        "package `{}` names the edition `{}`, which is invalid: {error}",
                        package.name, package.edition
                    ))
                })?;
            }
            if package.stdlib != PlanStdlibMode::None && self.stdlib.is_none() {
                return Err(PlanError::stdlib_inconsistent(format!(
                    "the package `{}` selects the `{}` standard library, but the plan names no standard-library identity",
                    package.name,
                    match package.stdlib {
                        PlanStdlibMode::Std => "std",
                        PlanStdlibMode::Core => "core",
                        PlanStdlibMode::None => "none",
                    }
                )));
            }
            if let Some(prelude) = &package.prelude {
                if prelude.module.is_empty() || prelude.module.split("::").any(str::is_empty) {
                    return Err(PlanError::stdlib_inconsistent(format!(
                        "the prelude of package `{}` names an empty module path",
                        package.name
                    )));
                }
                match (&prelude.package, &prelude.stdlib) {
                    (Some(index), None) => {
                        if *index >= self.packages.len() {
                            return Err(PlanError::stdlib_inconsistent(format!(
                                "the prelude of package `{}` names package {} outside the plan",
                                package.name, index
                            )));
                        }
                    }
                    (None, Some(name)) => {
                        if name.trim().is_empty() {
                            return Err(PlanError::stdlib_inconsistent(format!(
                                "the prelude of package `{}` names an empty standard-library package",
                                package.name
                            )));
                        }
                        if self.stdlib.is_none() || package.stdlib == PlanStdlibMode::None {
                            return Err(PlanError::stdlib_inconsistent(format!(
                                "the prelude of package `{}` comes from the standard library, but the package does not select one",
                                package.name
                            )));
                        }
                    }
                    _ => {
                        return Err(PlanError::stdlib_inconsistent(format!(
                            "the prelude of package `{}` must name exactly one package or standard-library source",
                            package.name
                        )));
                    }
                }
            }
            if package.files.iter().filter(|file| file.primary).count() > 1 {
                return Err(PlanError::invalid(format!(
                    "package `{}` names more than one primary file",
                    package.name
                )));
            }
            let mut own_files = std::collections::BTreeSet::new();
            for file in &package.files {
                if !own_files.insert(file.path.clone()) {
                    return Err(PlanError::invalid(format!(
                        "the source file `{}` appears twice in package `{}`",
                        file.path.display(),
                        package.name
                    )));
                }
                if let Some((name, _instance)) =
                    files.insert(file.path.clone(), (package.name.clone(), package.instance))
                {
                    if name != package.name {
                        return Err(PlanError::invalid(format!(
                            "the source file `{}` appears in more than one package",
                            file.path.display()
                        )));
                    }
                }
            }
            let mut aliases = std::collections::BTreeSet::new();
            for dependency in &package.dependencies {
                if dependency.package >= index {
                    return Err(PlanError::invalid(format!(
                        "package `{}` depends on package {} which does not precede it",
                        package.name, dependency.package
                    )));
                }
                if !aliases.insert(dependency.alias.clone()) {
                    return Err(PlanError::invalid(format!(
                        "package `{}` uses the dependency alias `{}` twice",
                        package.name, dependency.alias
                    )));
                }
            }
        }
        Ok(())
    }
}

fn validate_entry(entry: &str) -> PlanResult<()> {
    if entry.is_empty() {
        return Err(PlanError::invalid("the entry path is empty"));
    }
    if entry.split("::").any(str::is_empty) {
        return Err(PlanError::invalid(format!(
            "the entry path `{entry}` has an empty segment"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan() -> CompilePlan {
        CompilePlan {
            version: FORMAT_VERSION,
            target: "x86_64-unknown-linux-gnu".to_owned(),
            profile: PlanProfile::default(),
            mode: BuildMode::Check,
            packages: vec![PlanPackage {
                name: "demo".to_owned(),
                version: "0.1.0".to_owned(),
                edition: "2026".to_owned(),
                instance: 0,
                root: PathBuf::from("/demo"),
                files: vec![PlanFile {
                    path: PathBuf::from("/demo/src/main.sl"),
                    primary: true,
                }],
                exposed: vec!["root".to_owned()],
                dependencies: Vec::new(),
                cfg: CfgOptions::default(),
                prelude: None,
                stdlib: PlanStdlibMode::None,
            }],
            primary: 0,
            stdlib: None,
            entry: Some("root::main".to_owned()),
            metadata: PlanMetadata::default(),
            output: None,
            target_spec: None,
        }
    }

    #[test]
    fn an_unknown_schema_version_is_rejected() {
        let mut plan = plan();
        plan.version = FORMAT_VERSION + 1;
        assert_eq!(plan.validate().unwrap_err().code(), "E8001");
    }

    #[test]
    fn a_dependency_must_point_backwards() {
        let mut plan = plan();
        plan.packages[0].dependencies.push(PlanDependency {
            alias: "self".to_owned(),
            package: 0,
        });
        assert_eq!(plan.validate().unwrap_err().code(), "E8010");
    }

    #[test]
    fn a_prelude_must_name_a_package_in_the_plan() {
        let mut plan = plan();
        plan.packages[0].prelude = Some(PlanPrelude {
            package: Some(1),
            stdlib: None,
            module: "root".to_owned(),
        });
        assert_eq!(plan.validate().unwrap_err().code(), "E8141");
    }

    #[test]
    fn a_prelude_module_path_cannot_be_empty() {
        let mut plan = plan();
        plan.packages[0].prelude = Some(PlanPrelude {
            package: Some(0),
            stdlib: None,
            module: String::new(),
        });
        assert_eq!(plan.validate().unwrap_err().code(), "E8141");
        plan.packages[0].prelude = Some(PlanPrelude {
            package: Some(0),
            stdlib: None,
            module: "a::".to_owned(),
        });
        assert_eq!(plan.validate().unwrap_err().code(), "E8141");
    }

    #[test]
    fn a_build_plan_requires_an_output() {
        let mut plan = plan();
        plan.mode = BuildMode::Build;
        assert_eq!(plan.validate().unwrap_err().code(), "E8010");
    }

    #[test]
    fn two_packages_cannot_share_a_file() {
        let mut plan = plan();
        let mut other = plan.packages[0].clone();
        other.name = "other".to_owned();
        other.instance = 1;
        plan.packages.push(other);
        assert_eq!(plan.validate().unwrap_err().code(), "E8010");
    }

    #[test]
    fn an_unsupported_target_is_rejected() {
        let mut plan = plan();
        plan.target = "plan9".to_owned();
        assert_eq!(plan.validate().unwrap_err().code(), "E8010");
    }

    #[test]
    fn a_target_specification_is_validated_and_must_match_the_target() {
        let mut plan = plan();
        plan.target_spec = Some(PlanTarget {
            triple: plan.target.clone(),
            pointer_width: 64,
            endianness: "little".to_owned(),
            c_char_signed: true,
            c_long_width: 64,
            c_wchar_width: 32,
            c_wchar_signed: true,
            runtime: "default".to_owned(),
            panic: "abort".to_owned(),
            reproducible: true,
            ..PlanTarget::default()
        });
        plan.validate().expect("valid target spec");
        plan.target_spec.as_mut().expect("spec").triple = "aarch64-unknown-linux-gnu".to_owned();
        assert!(plan.validate().is_err());
    }

    #[test]
    fn a_freestanding_target_specification_round_trips() {
        let mut plan = plan();
        plan.target_spec = Some(PlanTarget {
            triple: plan.target.clone(),
            pointer_width: 64,
            endianness: "little".to_owned(),
            runtime: "none".to_owned(),
            panic: "trap".to_owned(),
            linker: Some("cc".to_owned()),
            linker_script: Some(PathBuf::from("kernel.ld")),
            link_args: vec!["-no-pie".to_owned()],
            c_char_signed: true,
            c_long_width: 64,
            c_wchar_width: 32,
            c_wchar_signed: true,
            reproducible: true,
            ..PlanTarget::default()
        });
        plan.validate().expect("valid freestanding plan");
        let spec = plan
            .target_spec
            .as_ref()
            .expect("spec")
            .to_spec()
            .expect("resolves");
        assert!(spec.is_freestanding());
        assert_eq!(spec.panic().as_str(), "trap");
    }

    #[test]
    fn a_nul_link_argument_is_rejected() {
        let mut plan = plan();
        plan.target_spec = Some(PlanTarget {
            triple: plan.target.clone(),
            pointer_width: 64,
            endianness: "little".to_owned(),
            runtime: "default".to_owned(),
            panic: "abort".to_owned(),
            link_args: vec!["bad\0argument".to_owned()],
            ..PlanTarget::default()
        });
        assert!(plan.validate().is_err());
    }

    #[test]
    fn an_invalid_target_specification_is_rejected() {
        let mut plan = plan();
        plan.target_spec = Some(PlanTarget {
            triple: plan.target.clone(),
            pointer_width: 48,
            endianness: "little".to_owned(),
            runtime: "none".to_owned(),
            panic: "abort".to_owned(),
            ..PlanTarget::default()
        });
        assert!(plan.validate().is_err());
    }
}
