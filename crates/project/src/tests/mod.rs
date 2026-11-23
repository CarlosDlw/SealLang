//! Project-layer tests for features, cfg, profiles, and lockfile behavior.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::{PlanOptions, Project};

pub(crate) mod cfg;
pub(crate) mod edition;
pub(crate) mod features;
pub(crate) mod lockfile;
pub(crate) mod profiles;

static NEXT: AtomicU64 = AtomicU64::new(0);

pub(crate) struct Fixture {
    pub(crate) root: PathBuf,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

pub(crate) fn fixture(files: &[(&str, &str)]) -> Fixture {
    let root = std::env::temp_dir().join(format!(
        "seal-project-unit-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("temporary root");
    for (path, contents) in files {
        let path = root.join(path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("fixture parent");
        }
        std::fs::write(path, contents).expect("fixture file");
    }
    Fixture { root }
}

pub(crate) fn load(fixture: &Fixture) -> Project {
    Project::load(&fixture.root).expect("the fixture is a valid project")
}

pub(crate) fn options(features: &[&str]) -> PlanOptions {
    PlanOptions {
        features: features
            .iter()
            .map(|feature| (*feature).to_owned())
            .collect(),
        cfgs: Vec::new(),
        ..PlanOptions::default()
    }
}
