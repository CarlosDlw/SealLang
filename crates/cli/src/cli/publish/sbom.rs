//! Building the CycloneDX document attached to a publication.

use seal_project::Project;
use seal_registry::sbom::{Component, SbomInput, generate as encode};

use crate::ui;

/// Generates the SBOM for one package and its resolved dependencies.
pub(crate) fn generate(
    project: &Project,
    index: usize,
) -> Result<serde_json::Value, crate::ui::Failure> {
    let package = project.package(index);
    let lockfile = project.resolve_lockfile(seal_target::host_triple().as_str());
    let mut components = Vec::new();
    for (position, dependency) in project.packages.iter().enumerate() {
        if position == index {
            continue;
        }
        let source = dependency.source.lock_source(
            &project.root,
            &dependency.root,
            &dependency.name,
            &dependency.version,
        );
        let Some(locked) = lockfile
            .packages
            .iter()
            .find(|locked| locked.name == dependency.name && locked.version == dependency.version)
        else {
            return Err(ui::failure(
                "E8031",
                format!(
                    "the dependency `{}` {} is missing from the resolved lockfile",
                    dependency.name, dependency.version
                ),
            ));
        };
        let license = dependency
            .manifest
            .package
            .as_ref()
            .and_then(|manifest| manifest.license.clone());
        components.push(Component {
            name: dependency.name.clone(),
            version: dependency.version.clone(),
            source,
            checksum: locked.checksum.clone(),
            license,
            features: locked.features.clone(),
            targets: dependency
                .targets
                .iter()
                .map(|target| target.name.clone())
                .collect(),
        });
    }
    let input = SbomInput {
        project: package.name.clone(),
        project_version: package.version.clone(),
        toolchain: seal_toolchain::EMBEDDED_VERSION.to_owned(),
        components,
    };
    encode(&input).map_err(|error| ui::failure(error.code(), error.message()))
}
