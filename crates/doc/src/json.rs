//! JSON rendering for `seal doc`.

use crate::{DocError, DocResult, Documentation};

/// Renders the documentation as stable, pretty JSON.
pub fn render(documentation: &Documentation) -> DocResult<String> {
    serde_json::to_string_pretty(documentation)
        .map(|text| format!("{text}\n"))
        .map_err(|error| DocError::Json(format!("cannot serialize documentation: {error}")))
}

/// Writes the JSON document to `path`.
pub fn write(documentation: &Documentation, path: &std::path::Path) -> DocResult<()> {
    let text = render(documentation)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| DocError::Io {
            path: parent.display().to_string(),
            message: error.to_string(),
        })?;
    }
    std::fs::write(path, text).map_err(|error| DocError::Io {
        path: path.display().to_string(),
        message: error.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_is_stable() {
        let documentation = Documentation {
            version: 1,
            package: "demo".to_owned(),
            package_version: "1.0.0".to_owned(),
            target: "x86_64-unknown-linux-gnu".to_owned(),
            modules: Vec::new(),
        };
        assert_eq!(
            render(&documentation).unwrap(),
            render(&documentation).unwrap()
        );
        assert!(render(&documentation).unwrap().starts_with('{'));
    }
}
