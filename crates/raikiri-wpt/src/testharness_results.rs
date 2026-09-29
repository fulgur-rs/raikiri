//! Structured named results for comparing compile-time JS backends.
use crate::testharness_page::SubtestOutcome;
use serde::Serialize;
use std::path::{Path, PathBuf};
/// Ordered results or an explicit page-level execution error.
#[derive(Serialize)]
pub struct ResultRecord {
    /// WPT-relative file identifier.
    pub test_id: String,
    /// Ordered assertions, retaining duplicate names.
    pub tests: Vec<NamedResult>,
    /// Page-level execution failure.
    pub error: Option<String>,
}
/// Owned named assertion.
#[derive(Serialize)]
pub struct NamedResult {
    /// Unmodified test name.
    pub name: String,
    /// Whether the status is PASS.
    pub passed: bool,
    /// Message including non-PASS status word.
    pub message: String,
}
impl ResultRecord {
    /// Capture one file's complete outcome.
    pub fn new(test_id: &str, tests: &[SubtestOutcome], error: Option<String>) -> Self {
        Self {
            test_id: test_id.into(),
            tests: tests
                .iter()
                .map(|t| NamedResult {
                    name: t.name.clone(),
                    passed: t.passed,
                    message: t.message.clone(),
                })
                .collect(),
            error,
        }
    }
}
/// Remove the optional output argument before the runner's existing parser.
pub fn extract_output(args: &mut Vec<String>) -> Result<Option<PathBuf>, String> {
    let mut output = None;
    while let Some(i) = args.iter().position(|a| a == "--results-json") {
        if output.is_some() {
            return Err("duplicate --results-json".into());
        }
        if i + 1 >= args.len() || args[i + 1].starts_with("--") {
            return Err("--results-json requires a path".into());
        }
        output = Some(PathBuf::from(args.remove(i + 1)));
        args.remove(i);
    }
    Ok(output)
}
/// Write results when an output path was selected.
pub fn write_results(path: Option<&Path>, records: &[ResultRecord]) -> Result<(), String> {
    if let Some(path) = path {
        let file = std::fs::File::create(path).map_err(|e| e.to_string())?;
        serde_json::to_writer(file, records).map_err(|e| e.to_string())?;
    }
    Ok(())
}
#[cfg(test)]
mod tests;
