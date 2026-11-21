//! Dependency audit: advisories, licenses, sources, yanked versions, checksums,
//! and provenance.

use std::collections::BTreeSet;

use semver::{Version, VersionReq};
use serde::{Deserialize, Serialize};

use crate::client::RegistryClient;
use crate::config::RegistryConfig;
use crate::download::verify_cached;
use crate::error::{RegistryError, RegistryResult};
use crate::index::validate_checksum;
use crate::tuf::TufSession;

/// One advisory from the registry feed.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Advisory {
    /// A stable advisory id.
    pub id: String,
    /// The affected package name.
    pub package: String,
    /// A semver requirement selecting affected versions.
    pub affected: String,
    /// `low`, `moderate`, `high`, or `critical`.
    #[serde(default)]
    pub severity: String,
    /// A human-readable summary.
    #[serde(default)]
    pub description: String,
    /// An optional link.
    #[serde(default)]
    pub url: Option<String>,
}

/// The advisory feed.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AdvisoryFeed {
    /// The feed schema version.
    pub format: u32,
    /// Advisories in the feed.
    #[serde(default)]
    pub advisories: Vec<Advisory>,
}

/// The fields audit needs for one locked package.
#[derive(Clone, Debug)]
pub struct AuditInput {
    /// Package name.
    pub name: String,
    /// Package version.
    pub version: Version,
    /// Lockfile source string.
    pub source: String,
    /// The license expression.
    pub license: Option<String>,
    /// The archive digest.
    pub checksum: String,
    /// The package root, for checksum verification.
    pub root: std::path::PathBuf,
    /// Whether the index marks this version yanked.
    pub yanked: bool,
    /// The provenance digest, when published.
    pub provenance: Option<String>,
}

/// One audit finding.
#[derive(Clone, Debug, Serialize)]
pub struct Violation {
    /// The stable diagnostic code.
    pub code: String,
    /// The affected package.
    pub package: String,
    /// The explanation.
    pub message: String,
}

/// The audit report.
#[derive(Clone, Debug, Serialize)]
pub struct AuditReport {
    /// Report schema version.
    pub version: u32,
    /// Checked packages.
    pub packages: usize,
    /// Findings; empty means the policy passes.
    pub violations: Vec<Violation>,
    /// Advisories that matched, including ones not elevated by policy.
    pub advisories: Vec<Advisory>,
}

/// Loads the advisory feed, using TUF when available.
pub fn load_advisories(
    client: &RegistryClient,
    config: &RegistryConfig,
    tuf: Option<&TufSession>,
) -> RegistryResult<Vec<Advisory>> {
    let bytes = match tuf {
        Some(tuf) => {
            let path = "advisories.json";
            tuf.target_from(client, config, path)?
        }
        None => client.fetch_url(&config.advisories)?,
    };
    let feed: AdvisoryFeed = serde_json::from_slice(&bytes)
        .map_err(|error| RegistryError::audit(format!("invalid advisory feed: {error}")))?;
    if feed.format != 1 {
        return Err(RegistryError::audit(format!(
            "the advisory feed uses format {}, but this client understands 1",
            feed.format
        )));
    }
    for advisory in &feed.advisories {
        if advisory.id.is_empty() || advisory.package.is_empty() {
            return Err(RegistryError::audit(
                "an advisory has an empty id or package name",
            ));
        }
        VersionReq::parse(&advisory.affected).map_err(|error| {
            RegistryError::audit(format!(
                "the advisory `{}` has an invalid affected range: {error}",
                advisory.id
            ))
        })?;
    }
    Ok(feed.advisories)
}

/// Runs the audit.
pub fn audit(
    inputs: &[AuditInput],
    advisories: &[Advisory],
    policy: &seal_manifest::RegistryPolicyManifest,
) -> RegistryResult<AuditReport> {
    let mut violations = Vec::new();
    let mut matched = BTreeSet::new();
    for input in inputs {
        if let Err(error) = verify_cached(&input.root, &input.checksum) {
            violations.push(Violation {
                code: error.code().to_owned(),
                package: format!("{} {}", input.name, input.version),
                message: error.message().to_owned(),
            });
        }
        if input.yanked {
            violations.push(Violation {
                code: "E8033".to_owned(),
                package: format!("{} {}", input.name, input.version),
                message: "the locked version is yanked; update to a supported release".to_owned(),
            });
        }
        if let Some(license) = &input.license {
            if !license_allowed(license, policy) {
                violations.push(Violation {
                    code: "E8034".to_owned(),
                    package: format!("{} {}", input.name, input.version),
                    message: format!("the license `{license}` is not allowed by policy"),
                });
            }
        } else if !policy.allow_licenses.is_empty() {
            violations.push(Violation {
                code: "E8034".to_owned(),
                package: format!("{} {}", input.name, input.version),
                message: "the package declares no license, but the policy requires one".to_owned(),
            });
        }
        if !policy.allow_sources.is_empty()
            && !policy
                .allow_sources
                .iter()
                .any(|prefix| input.source.starts_with(prefix))
        {
            violations.push(Violation {
                code: "E8035".to_owned(),
                package: format!("{} {}", input.name, input.version),
                message: format!("the source `{}` is not allowed by policy", input.source),
            });
        }
        if policy.require_provenance && input.provenance.is_none() {
            violations.push(Violation {
                code: "E8037".to_owned(),
                package: format!("{} {}", input.name, input.version),
                message: "the package has no provenance metadata".to_owned(),
            });
        }
        if let Some(provenance) = &input.provenance
            && let Err(error) = validate_checksum(provenance)
        {
            violations.push(Violation {
                code: error.code().to_owned(),
                package: format!("{} {}", input.name, input.version),
                message: error.message().to_owned(),
            });
        }
        for advisory in advisories {
            if advisory.package != input.name {
                continue;
            }
            let Ok(requirement) = VersionReq::parse(&advisory.affected) else {
                continue;
            };
            if !requirement.matches(&input.version) {
                continue;
            }
            matched.insert(advisory.id.clone());
            violations.push(Violation {
                code: "E8036".to_owned(),
                package: format!("{} {}", input.name, input.version),
                message: format!(
                    "{}: {}{}",
                    advisory.id,
                    advisory.severity,
                    if advisory.description.is_empty() {
                        String::new()
                    } else {
                        format!(" — {}", advisory.description)
                    }
                ),
            });
        }
    }
    let advisories = advisories
        .iter()
        .filter(|advisory| matched.contains(&advisory.id))
        .cloned()
        .collect();
    Ok(AuditReport {
        version: 1,
        packages: inputs.len(),
        violations,
        advisories,
    })
}

fn license_allowed(license: &str, policy: &seal_manifest::RegistryPolicyManifest) -> bool {
    let tokens: Vec<&str> = license
        .split(['(', ')'])
        .flat_map(|part| part.split(" OR ").flat_map(|part| part.split(" AND ")))
        .map(str::trim)
        .filter(|token| !token.is_empty() && *token != "WITH")
        .collect();
    if !policy.deny_licenses.is_empty()
        && tokens
            .iter()
            .any(|token| policy.deny_licenses.iter().any(|denied| denied == token))
    {
        return false;
    }
    policy.allow_licenses.is_empty()
        || tokens
            .iter()
            .all(|token| policy.allow_licenses.iter().any(|allowed| allowed == token))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy() -> seal_manifest::RegistryPolicyManifest {
        seal_manifest::RegistryPolicyManifest {
            allow_licenses: vec!["MIT".to_owned(), "Apache-2.0".to_owned()],
            ..Default::default()
        }
    }

    #[test]
    fn license_expressions_are_split_into_tokens() {
        assert!(license_allowed("MIT OR Apache-2.0", &policy()));
        assert!(!license_allowed("GPL-3.0", &policy()));
        assert!(!license_allowed("MIT AND GPL-3.0", &policy()));
    }

    #[test]
    fn a_denied_license_wins_over_an_allow_list() {
        let mut policy = policy();
        policy.deny_licenses.push("MIT".to_owned());
        assert!(!license_allowed("MIT", &policy));
    }
}
