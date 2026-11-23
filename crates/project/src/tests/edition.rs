//! The package edition changes the resolved package instance.

use seal_plan::PlanProfile;
use seal_version::Edition;

use crate::cfg::CfgEnvironment;

use super::{fixture, load};

#[test]
fn changing_the_edition_changes_the_instance_hash() {
    let fixture = fixture(&[
        (
            "seal.toml",
            "[project]\nname = \"demo\"\n\n[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2026\"\n",
        ),
        ("src/main.sl", "func main() -> i32 { 0 }\n"),
    ]);
    let mut project = load(&fixture);
    let cfg = CfgEnvironment::for_triple(&seal_target::host_triple());
    let before = crate::plan::instance_hash(project.package(0), &[], &cfg, &PlanProfile::default());
    project.packages[0].edition = Edition::new(2029);
    let after = crate::plan::instance_hash(project.package(0), &[], &cfg, &PlanProfile::default());
    assert_ne!(before, after);
}
