//! Deterministic CycloneDX 1.5 SBOM generation.

use sha2::{Digest, Sha256};

use crate::error::{RegistryError, RegistryResult};

/// One component for the SBOM.
#[derive(Clone, Debug)]
pub struct Component {
    /// Package name.
    pub name: String,
    /// Package version.
    pub version: String,
    /// Lockfile source string.
    pub source: String,
    /// Archive checksum, `sha256:<hex>`.
    pub checksum: String,
    /// License expression.
    pub license: Option<String>,
    /// Enabled features.
    pub features: Vec<String>,
    /// Target names compiled for the package.
    pub targets: Vec<String>,
}

/// The project and toolchain metadata for the SBOM.
#[derive(Clone, Debug)]
pub struct SbomInput {
    /// Project name.
    pub project: String,
    /// Project version.
    pub project_version: String,
    /// Toolchain version.
    pub toolchain: String,
    /// Components, sorted by name and version.
    pub components: Vec<Component>,
}

/// Generates a CycloneDX JSON document.
pub fn generate(input: &SbomInput) -> RegistryResult<serde_json::Value> {
    let mut components: Vec<&Component> = input.components.iter().collect();
    components.sort_by(|left, right| {
        left.name
            .cmp(&right.name)
            .then_with(|| left.version.cmp(&right.version))
    });
    let mut serial = Sha256::new();
    serial.update(input.project.as_bytes());
    for component in &components {
        serial.update(component.source.as_bytes());
        serial.update(component.checksum.as_bytes());
    }
    let serial = hex::encode(serial.finalize());
    let serial = format!(
        "urn:uuid:{}-{}-{}-{}-{}",
        &serial[0..8],
        &serial[8..12],
        &serial[12..16],
        &serial[16..20],
        &serial[20..32]
    );
    let component_values: Vec<serde_json::Value> = components
        .iter()
        .map(|component| -> RegistryResult<serde_json::Value> {
            let digest = component.checksum.strip_prefix("sha256:").ok_or_else(|| {
                RegistryError::sbom(format!(
                    "the checksum for `{}` is not SHA-256",
                    component.name
                ))
            })?;
            let properties = vec![
                property("seal:features", &component.features.join(",")),
                property("seal:source", &component.source),
                property("seal:targets", &component.targets.join(",")),
            ];
            let mut value = serde_json::json!({
                "type": "library",
                "bom-ref": component.source,
                "name": component.name,
                "version": component.version,
                "purl": format!("pkg:seal/{}@{}", encode_name(&component.name), component.version),
                "hashes": [{"alg": "SHA-256", "content": digest}],
                "properties": properties,
            });
            if let Some(license) = &component.license {
                value["licenses"] = serde_json::json!([{"expression": license}]);
            }
            Ok(value)
        })
        .collect::<RegistryResult<Vec<_>>>()?;
    let document = serde_json::json!({
        "bomFormat": "CycloneDX",
        "specVersion": "1.5",
        "serialNumber": serial,
        "version": 1,
        "metadata": {
            "tools": {
                "components": [{
                    "type": "application",
                    "name": "seal",
                    "version": input.toolchain,
                }]
            },
            "component": {
                "type": "application",
                "bom-ref": format!("seal:project:{}", input.project),
                "name": input.project,
                "version": input.project_version,
            }
        },
        "components": component_values,
    });
    Ok(document)
}

fn property(name: &str, value: &str) -> serde_json::Value {
    serde_json::json!({"name": name, "value": value})
}

fn encode_name(name: &str) -> String {
    name.replace('@', "%40").replace('/', "%2F")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sbom_is_deterministic_and_contains_hashes() {
        let input = SbomInput {
            project: "demo".to_owned(),
            project_version: "0.1.0".to_owned(),
            toolchain: "0.1.0".to_owned(),
            components: vec![Component {
                name: "foo".to_owned(),
                version: "1.2.0".to_owned(),
                source: "registry+https://example.com/#foo@1.2.0".to_owned(),
                checksum: format!("sha256:{}", "a".repeat(64)),
                license: Some("MIT".to_owned()),
                features: vec!["std".to_owned()],
                targets: vec!["foo".to_owned()],
            }],
        };
        let first = generate(&input).unwrap();
        let second = generate(&input).unwrap();
        assert_eq!(first, second);
        assert_eq!(first["bomFormat"], "CycloneDX");
        assert_eq!(first["components"][0]["name"], "foo");
        assert_eq!(
            first["components"][0]["hashes"][0]["content"],
            "a".repeat(64)
        );
    }
}
