//! Evaluation of typed `when` conditions for targets and dependencies.

use seal_manifest::CfgSpec;
use seal_target::TargetTriple;

/// The configuration facts available while loading a project.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CfgEnvironment {
    os: String,
    arch: String,
    env: String,
    pointer_width: u8,
}

impl CfgEnvironment {
    /// Creates the environment for a target triple.
    pub(crate) fn for_triple(triple: &TargetTriple) -> Self {
        let spec = triple.spec();
        Self {
            os: spec.triple().os().as_str().to_owned(),
            arch: spec.triple().arch().as_str().to_owned(),
            env: String::new(),
            pointer_width: spec.pointer_width(),
        }
    }

    /// Evaluates a manifest condition.
    pub(crate) fn matches(&self, condition: &CfgSpec) -> bool {
        match condition {
            CfgSpec::Os(value) => self.os == *value,
            CfgSpec::Arch(value) => self.arch == *value,
            CfgSpec::Env(value) => self.env == *value,
            CfgSpec::PointerWidth(value) => self.pointer_width == *value,
            CfgSpec::All(conditions) => conditions.iter().all(|condition| self.matches(condition)),
            CfgSpec::Any(conditions) => conditions.iter().any(|condition| self.matches(condition)),
            CfgSpec::Not(condition) => !self.matches(condition),
        }
    }

    /// Returns the target OS.
    pub(crate) fn os(&self) -> &str {
        &self.os
    }

    /// Returns the target architecture.
    pub(crate) fn arch(&self) -> &str {
        &self.arch
    }

    /// Returns the target environment.
    pub(crate) fn env(&self) -> &str {
        &self.env
    }

    /// Returns the target pointer width.
    pub(crate) fn pointer_width(&self) -> u8 {
        self.pointer_width
    }
}

#[cfg(test)]
mod tests {
    use seal_manifest::CfgSpec;

    use super::CfgEnvironment;

    fn environment() -> CfgEnvironment {
        CfgEnvironment {
            os: "linux".to_owned(),
            arch: "x86_64".to_owned(),
            env: String::new(),
            pointer_width: 64,
        }
    }

    #[test]
    fn all_and_any_conditions_compose() {
        let environment = environment();
        let condition = CfgSpec::All(vec![
            CfgSpec::Os("linux".to_owned()),
            CfgSpec::Any(vec![
                CfgSpec::Arch("x86_64".to_owned()),
                CfgSpec::Arch("aarch64".to_owned()),
            ]),
        ]);
        assert!(environment.matches(&condition));
        assert!(!environment.matches(&CfgSpec::Not(Box::new(condition))));
    }

    #[test]
    fn pointer_width_and_target_env_are_typed() {
        let environment = environment();
        assert!(environment.matches(&CfgSpec::PointerWidth(64)));
        assert!(!environment.matches(&CfgSpec::PointerWidth(32)));
        assert!(environment.matches(&CfgSpec::Env(String::new())));
    }
}
