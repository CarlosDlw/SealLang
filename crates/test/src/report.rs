//! Machine-readable test reports: JUnit XML and NDJSON.

use std::fmt::Write as _;
use std::path::Path;

use crate::error::TestError;
use crate::model::{Report, Status};

/// Renders a JUnit XML document with one test suite.
pub fn junit(report: &Report, suite: &str) -> String {
    let mut xml = String::new();
    let failures = report.failed();
    let skipped = report.skipped();
    let _ = writeln!(xml, r#"<?xml version="1.0" encoding="UTF-8"?>"#);
    let _ = writeln!(
        xml,
        r#"<testsuites tests="{}" failures="{}" skipped="{}" time="{:.3}">"#,
        report.outcomes.len(),
        failures,
        skipped,
        report.duration.as_secs_f64()
    );
    let _ = writeln!(
        xml,
        r#"  <testsuite name="{}" tests="{}" failures="{}" skipped="{}" time="{:.3}">"#,
        escape(suite),
        report.outcomes.len(),
        failures,
        skipped,
        report.duration.as_secs_f64()
    );
    for outcome in &report.outcomes {
        let classname = format!("{}::{}", outcome.package, outcome.target);
        let _ = writeln!(
            xml,
            r#"    <testcase name="{}" classname="{}" time="{:.3}">"#,
            escape(&outcome.name),
            escape(&classname),
            outcome.duration.as_secs_f64()
        );
        match outcome.status {
            Status::Passed => {}
            Status::Skipped => {
                let _ = writeln!(
                    xml,
                    r#"      <skipped message="{}"/>"#,
                    escape(&outcome.message)
                );
            }
            Status::Failed | Status::Timeout => {
                let kind = outcome.status.as_str();
                let _ = writeln!(
                    xml,
                    r#"      <failure type="{kind}" message="{}">{}</failure>"#,
                    escape(&outcome.message),
                    escape(&outcome.message)
                );
            }
        }
        let _ = writeln!(xml, "    </testcase>");
    }
    let _ = writeln!(xml, "  </testsuite>");
    let _ = writeln!(xml, "</testsuites>");
    xml
}

/// Writes a JUnit XML document, creating its parent directory.
pub fn write_junit(report: &Report, suite: &str, path: &Path) -> Result<(), TestError> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).map_err(|error| {
            TestError::report(format!(
                "cannot create the report directory `{}`: {error}",
                parent.display()
            ))
        })?;
    }
    std::fs::write(path, junit(report, suite)).map_err(|error| {
        TestError::report(format!(
            "cannot write the JUnit report `{}`: {error}",
            path.display()
        ))
    })
}

/// Renders the report as newline-delimited JSON, one object per outcome plus a
/// final summary object.
pub fn json(report: &Report) -> Result<String, TestError> {
    let mut text = String::new();
    for outcome in &report.outcomes {
        let value = serde_json::json!({
            "event": "test",
            "name": outcome.name,
            "package": outcome.package,
            "target": outcome.target,
            "kind": outcome.kind.as_str(),
            "status": outcome.status.as_str(),
            "duration_ms": outcome.duration.as_millis() as u64,
            "attempts": outcome.attempts,
            "exit_code": outcome.exit_code,
            "signal": outcome.signal,
            "message": outcome.message,
        });
        let line = serde_json::to_string(&value).map_err(|error| {
            TestError::report(format!("cannot serialize a test result: {error}"))
        })?;
        text.push_str(&line);
        text.push('\n');
    }
    let summary = serde_json::json!({
        "event": "summary",
        "tests": report.outcomes.len(),
        "passed": report.passed(),
        "failed": report.failed(),
        "skipped": report.skipped(),
        "duration_ms": report.duration.as_millis() as u64,
    });
    let line = serde_json::to_string(&summary)
        .map_err(|error| TestError::report(format!("cannot serialize the summary: {error}")))?;
    text.push_str(&line);
    text.push('\n');
    Ok(text)
}

/// Writes the NDJSON report, creating its parent directory.
pub fn write_json(report: &Report, path: &Path) -> Result<(), TestError> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).map_err(|error| {
            TestError::report(format!(
                "cannot create the report directory `{}`: {error}",
                parent.display()
            ))
        })?;
    }
    std::fs::write(path, json(report)?).map_err(|error| {
        TestError::report(format!(
            "cannot write the JSON report `{}`: {error}",
            path.display()
        ))
    })
}

/// Escapes text for an XML attribute or body.
fn escape(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&apos;"),
            // XML 1.0 forbids most control characters.
            character if character < ' ' && !matches!(character, '\n' | '\r' | '\t') => {
                escaped.push('\u{fffd}');
            }
            character => escaped.push(character),
        }
    }
    escaped
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{escape, junit};
    use crate::model::{Kind, Outcome, Report, Status};

    fn report(status: Status) -> Report {
        Report {
            outcomes: vec![Outcome {
                name: "a::b<c".to_owned(),
                package: "demo".to_owned(),
                target: "unit".to_owned(),
                kind: Kind::Test,
                status,
                duration: Duration::from_millis(12),
                attempts: 1,
                message: "expected 1 & got <2>".to_owned(),
                exit_code: Some(1),
                signal: None,
            }],
            duration: Duration::from_millis(15),
        }
    }

    #[test]
    fn xml_escapes_attributes_and_text() {
        assert_eq!(escape("a&b<c>\"d'"), "a&amp;b&lt;c&gt;&quot;d&apos;");
    }

    #[test]
    fn junit_marks_a_failure() {
        let xml = junit(&report(Status::Failed), "demo");
        assert!(xml.contains(r#"failures="1""#));
        assert!(xml.contains("<failure type=\"failed\""));
        assert!(xml.contains("a::b&lt;c"));
    }

    #[test]
    fn junit_marks_a_skip() {
        let xml = junit(&report(Status::Skipped), "demo");
        assert!(xml.contains(r#"skipped="1""#));
        assert!(xml.contains("<skipped"));
    }
}
