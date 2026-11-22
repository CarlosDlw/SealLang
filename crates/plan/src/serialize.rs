//! Reading and writing the versioned compile plan.
//!
//! The project layer writes a plan before invoking the driver, and the driver
//! reads the same file back. Both operations validate the schema at the
//! boundary so a malformed or partially written plan is never compiled.

use std::path::Path;

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
    pub fn from_json(text: &str) -> PlanResult<Self> {
        let plan: Self = serde_json::from_str(text).map_err(|error| {
            PlanError::schema(format!("the compile plan is not valid JSON: {error}"))
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
            }],
            primary: 0,
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
            package: 0,
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
    fn malformed_json_is_a_schema_error() {
        let error =
            CompilePlan::from_json("{\"version\": 1}").expect_err("an incomplete plan is rejected");
        assert_eq!(error.code(), "E8001");
    }
}
