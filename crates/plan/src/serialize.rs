//! Reading and writing the versioned compile plan.
//!
//! The project layer writes a plan before invoking the driver, and the driver
//! reads the same file back. Both operations validate the schema at the
//! boundary so a malformed or partially written plan is never compiled.

use std::path::Path;

use seal_version::Edition;

use crate::{CompilePlan, PlanError, PlanResult};

impl CompilePlan {
    /// Reads and validates a plan from a JSON file.
    pub fn load(path: &Path) -> PlanResult<Self> {
        let text = std::fs::read_to_string(path).map_err(|error| {
            PlanError::io(format!(
                "cannot read compile plan `{}`: {error}",
                path.display()
            ))
        })?;
        Self::from_json(&text)
    }

    /// Parses and validates a plan from JSON text.
    ///
    /// Version 1 plans are migrated in place: they gain the current schema
    /// number, every package keeps `stdlib = "none"`, and the plan names no
    /// standard library. Version 2 plans gain the current edition for every
    /// package. Both preserve the behavior of plans written before the field
    /// existed.
    pub fn from_json(text: &str) -> PlanResult<Self> {
        let mut value: serde_json::Value = serde_json::from_str(text).map_err(|error| {
            PlanError::schema(format!("the compile plan is not valid JSON: {error}"))
        })?;
        let version = value
            .get("version")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| PlanError::schema("the compile plan has no integer `version` field"))?;
        if version <= 2 {
            value["version"] = serde_json::Value::from(crate::FORMAT_VERSION);
            if let Some(packages) = value
                .get_mut("packages")
                .and_then(serde_json::Value::as_array_mut)
            {
                for package in packages {
                    if let Some(object) = package.as_object_mut() {
                        object.entry("edition").or_insert_with(|| {
                            serde_json::Value::from(Edition::CURRENT.to_string())
                        });
                    }
                }
            }
        }
        let plan: Self = serde_json::from_value(value).map_err(|error| {
            PlanError::schema(format!(
                "the compile plan does not match the schema: {error}"
            ))
        })?;
        plan.validate()?;
        Ok(plan)
    }

    /// Serializes the plan as stable, pretty JSON.
    pub fn to_json(&self) -> PlanResult<String> {
        serde_json::to_string_pretty(self).map_err(|error| {
            PlanError::schema(format!("cannot serialize the compile plan: {error}"))
        })
    }

    /// Writes the plan atomically, creating the parent directory when needed.
    pub fn write(&self, path: &Path) -> PlanResult<()> {
        let text = self.to_json()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| {
                PlanError::io(format!(
                    "cannot create the plan directory `{}`: {error}",
                    parent.display()
                ))
            })?;
        }
        let temporary = path.with_extension("json.tmp");
        std::fs::write(&temporary, text).map_err(|error| {
            PlanError::io(format!(
                "cannot write the compile plan `{}`: {error}",
                temporary.display()
            ))
        })?;
        std::fs::rename(&temporary, path).map_err(|error| {
            PlanError::io(format!(
                "cannot replace the compile plan `{}`: {error}",
                path.display()
            ))
        })
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use crate::{
        BuildMode, CfgOptions, CompilePlan, FORMAT_VERSION, PlanFile, PlanMetadata, PlanPackage,
        PlanPrelude, PlanProfile,
    };

    fn fixture_path(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("the clock is after the epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("seal-plan-{}-{nonce}-{name}", std::process::id()))
    }

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
                stdlib: crate::PlanStdlibMode::None,
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
    fn a_plan_round_trips_through_json() {
        let mut plan = plan();
        plan.packages[0].prelude = Some(PlanPrelude {
            package: Some(0),
            stdlib: None,
            module: "root".to_owned(),
        });
        let text = plan.to_json().expect("the plan serializes");
        assert_eq!(
            CompilePlan::from_json(&text).expect("the plan parses"),
            plan
        );
    }

    #[test]
    fn writing_creates_the_parent_directory_and_replaces_the_file() {
        let root = fixture_path("write");
        let path = root.join("nested").join("plan.json");
        plan().write(&path).expect("the plan is written");
        assert!(path.is_file());
        assert_eq!(CompilePlan::load(&path).expect("the plan loads"), plan());

        let mut changed = plan();
        changed.entry = Some("app::main".to_owned());
        changed.write(&path).expect("the plan is replaced");
        assert_eq!(
            CompilePlan::load(&path).expect("the replaced plan loads"),
            changed
        );
        assert!(
            !path.with_extension("json.tmp").exists(),
            "the temporary file is renamed"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_version_one_plan_migrates_to_no_stdlib() {
        let text = "{\"version\":1,\"target\":\"x86_64-unknown-linux-gnu\",\"profile\":{},\"mode\":\"check\",\"packages\":[{\"name\":\"demo\",\"version\":\"0.1.0\",\"instance\":0,\"root\":\"/demo\",\"files\":[{\"path\":\"/demo/src/main.sl\",\"primary\":true}]}],\"primary\":0}";
        let plan = CompilePlan::from_json(text).expect("the legacy plan loads");
        assert_eq!(plan.version, FORMAT_VERSION);
        assert_eq!(plan.packages[0].edition, "2026");
        assert!(plan.stdlib.is_none());
        assert_eq!(plan.packages[0].stdlib, crate::PlanStdlibMode::None);
        assert!(plan.packages[0].prelude.is_none());
    }

    #[test]
    fn a_version_two_plan_gains_the_current_edition() {
        let text = "{\"version\":2,\"target\":\"x86_64-unknown-linux-gnu\",\"profile\":{},\"mode\":\"check\",\"packages\":[{\"name\":\"demo\",\"version\":\"0.1.0\",\"instance\":0,\"root\":\"/demo\",\"files\":[{\"path\":\"/demo/src/main.sl\",\"primary\":true}]}],\"primary\":0}";
        let plan = CompilePlan::from_json(text).expect("the version two plan loads");
        assert_eq!(plan.version, FORMAT_VERSION);
        assert_eq!(plan.packages[0].edition, "2026");
    }

    #[test]
    fn the_edition_is_part_of_the_serialized_plan() {
        let plan = plan();
        let first = plan.to_json().expect("the plan serializes");
        let mut changed = plan;
        changed.packages[0].edition = "2029".to_owned();
        let second = changed.to_json().expect("the changed plan serializes");
        assert_ne!(first, second);
        assert!(second.contains("\"edition\": \"2029\""));
    }

    #[test]
    fn a_current_plan_requires_a_supported_edition() {
        let mut plan = plan();
        plan.packages[0].edition = "2027".to_owned();
        assert_eq!(plan.validate().unwrap_err().code(), "E8010");

        plan.packages[0].edition.clear();
        assert_eq!(plan.validate().unwrap_err().code(), "E8010");
    }

    #[test]
    fn a_version_two_plan_round_trips_the_standard_library() {
        let mut plan = plan();
        plan.stdlib = Some(crate::PlanStdlib {
            language: "0.1".to_owned(),
            version: "1.2.3".to_owned(),
            digest: "a".repeat(64),
        });
        plan.packages[0].stdlib = crate::PlanStdlibMode::Std;
        plan.packages[0].prelude = Some(PlanPrelude {
            package: None,
            stdlib: Some("std".to_owned()),
            module: "prelude".to_owned(),
        });
        let text = plan.to_json().expect("the plan serializes");
        assert_eq!(
            CompilePlan::from_json(&text).expect("the plan parses"),
            plan
        );
    }

    #[test]
    fn a_stdlib_mode_without_stdlib_metadata_is_e8141() {
        let mut plan = plan();
        plan.packages[0].stdlib = crate::PlanStdlibMode::Std;
        assert_eq!(plan.validate().unwrap_err().code(), "E8141");
    }

    #[test]
    fn an_invalid_stdlib_digest_is_e8141() {
        let mut plan = plan();
        plan.stdlib = Some(crate::PlanStdlib {
            language: "0.1".to_owned(),
            version: "1.2.3".to_owned(),
            digest: "not-a-digest".to_owned(),
        });
        assert_eq!(plan.validate().unwrap_err().code(), "E8141");
    }

    #[test]
    fn malformed_json_is_a_schema_error() {
        let error =
            CompilePlan::from_json("{\"version\": 1}").expect_err("an incomplete plan is rejected");
        assert_eq!(error.code(), "E8001");
    }
}
