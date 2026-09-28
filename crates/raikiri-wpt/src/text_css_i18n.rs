//! Executes the testharness-only tests in `css/css-text/i18n` against Raikiri.
//!
//! Reftest files in the same directory remain on the visual runner. This
//! module covers testharness pages, which query DOM geometry. Each page runs
//! end to end with the checkout's real `resources/testharness.js`: its
//! `<script>` elements, inline or `src`, run in document order against a
//! live Raikiri document.

use std::fs;
use std::path::{Path, PathBuf};

use crate::testharness_page::SubtestOutcome;

const TEST_DIR: &str = "css/css-text/i18n";

/// Per-file result from a CSS Text i18n testharness page.
#[derive(Debug)]
pub struct TestHarnessFileResult {
    /// The WPT-relative test ID.
    pub test_id: String,
    /// Results for every `test()` call. Empty only when `error` is set.
    pub outcomes: Vec<SubtestOutcome>,
    /// A page, layout, or harness-level error, separate from assertion failures.
    pub error: Option<String>,
}

impl TestHarnessFileResult {
    /// Number of passing assertions.
    pub fn passed(&self) -> usize {
        self.outcomes
            .iter()
            .filter(|outcome| outcome.passed)
            .count()
    }

    /// Number of assertions that ran.
    pub fn total(&self) -> usize {
        self.outcomes.len()
    }

    /// Whether all assertions ran and passed.
    pub fn all_passed(&self) -> bool {
        self.error.is_none() && self.total() > 0 && self.passed() == self.total()
    }
}

/// Why the CSS Text i18n test suite could not be discovered or executed.
#[derive(Debug)]
pub enum TestHarnessRunError {
    /// The selected WPT directory is missing or cannot be read.
    Io(String),
    /// The pinned tree had no testharness pages in the selected directory.
    NoTests,
}

impl std::fmt::Display for TestHarnessRunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(message) => write!(f, "I/O error: {message}"),
            Self::NoTests => write!(f, "no testharness pages found under {TEST_DIR}"),
        }
    }
}

impl std::error::Error for TestHarnessRunError {}

/// Run every testharness HTML page under `css/css-text/i18n` with the real
/// `testharness.js`.
///
/// Each page runs against a live Raikiri document (`WptDocumentHost`). DOM
/// and style writes update arena nodes; geometry reads lazily recascade and
/// relayout that same document before returning current page-scene fragments.
pub fn run_css_text_i18n(
    wpt_root: &Path,
) -> Result<Vec<TestHarnessFileResult>, TestHarnessRunError> {
    let test_root = wpt_root.join(TEST_DIR);
    if !test_root.is_dir() {
        return Err(TestHarnessRunError::Io(format!(
            "{} is not a directory",
            test_root.display()
        )));
    }

    let files = discover_testharness_files(&test_root)?;
    if files.is_empty() {
        return Err(TestHarnessRunError::NoTests);
    }

    let results = files
        .iter()
        .map(|path| run_testharness_file(path, wpt_root))
        .collect();
    Ok(results)
}

/// Run one page end to end with the checkout's real `testharness.js`.
fn run_testharness_file(path: &Path, wpt_root: &Path) -> TestHarnessFileResult {
    // `run_testharness_page` joins its path onto `wpt_root`, so a discovered
    // path (already under a possibly relative `wpt_root`) goes in relative.
    let relative = path.strip_prefix(wpt_root).unwrap_or(path);
    let test_id = relative.to_string_lossy().replace('\\', "/");
    match crate::testharness_page::run_testharness_page(relative, wpt_root) {
        Ok(outcomes) => TestHarnessFileResult {
            test_id,
            outcomes,
            error: None,
        },
        Err(error) => TestHarnessFileResult {
            test_id,
            outcomes: Vec::new(),
            error: Some(error.to_string()),
        },
    }
}

fn discover_testharness_files(test_root: &Path) -> Result<Vec<PathBuf>, TestHarnessRunError> {
    let mut stack = vec![test_root.to_path_buf()];
    let mut files = Vec::new();
    while let Some(directory) = stack.pop() {
        let entries = fs::read_dir(&directory).map_err(|error| {
            TestHarnessRunError::Io(format!("{}: {error}", directory.display()))
        })?;
        for entry in entries {
            let entry = entry.map_err(|error| TestHarnessRunError::Io(error.to_string()))?;
            let path = entry.path();
            if path.is_dir() {
                if path
                    .file_name()
                    .is_some_and(|name| name == "reference" || name == "support")
                {
                    continue;
                }
                stack.push(path);
                continue;
            }
            let Some(extension) = path.extension().and_then(|value| value.to_str()) else {
                continue;
            };
            if !matches!(
                extension.to_ascii_lowercase().as_str(),
                "html" | "htm" | "xhtml"
            ) {
                continue;
            }
            let source = fs::read_to_string(&path)
                .map_err(|error| TestHarnessRunError::Io(format!("{}: {error}", path.display())))?;
            if source.to_ascii_lowercase().contains("testharness.js") {
                files.push(path);
            }
        }
    }
    files.sort();
    Ok(files)
}

#[cfg(test)]
mod tests;
