//! Runs a WPT CSS parsing test file's `test_invalid_value`/`test_valid_value`
//! assertions, aggregating per-file PASS/FAIL counts.
//!
//! Each page runs end to end with the checkout's real
//! `resources/testharness.js`: its `<script>` elements, inline or `src`
//! (including `css/support/parsing-testcommon.js`), run in document order on
//! the native DOM runtime against a live Raikiri document built from the
//! page, so `document.getElementById('target')` and element style
//! declarations behave as they do for any other testharness page.

use std::fs;
use std::path::Path;

use raikiri_js::TestOutcome;

use crate::testharness_page::{PageError, run_testharness_page_with_preamble};

/// Known-valid, property-agnostic sanity check run before any of the page's
/// scripts: `color: red` must round-trip through an element's inline
/// style. It catches an inert style binding (a setter that silently no-ops,
/// or a getter that always reads back empty) that would otherwise make every
/// `test_invalid_value` assertion pass for the wrong reason. A failure throws,
/// so the whole file is reported as [`ParsingFileError::PositiveControl`]
/// instead of a result.
const POSITIVE_CONTROL_JS: &str = r#"
(function () {
    var div = document.createElement("div");
    div.style["color"] = "red";
    var got = div.style.getPropertyValue("color");
    if (got !== "red") {
        throw new Error("positive control failed: color round-trip got " + JSON.stringify(got));
    }
})();
"#;

/// One `css/*/parsing/*.html` file's outcomes.
#[derive(Debug)]
pub struct ParsingFileOutcome {
    /// The file's path relative to the WPT root, forward-slash separated.
    pub test_id: String,
    /// One entry per `test()` call the file's scripts registered, in
    /// registration order.
    pub outcomes: Vec<TestOutcome>,
}

impl ParsingFileOutcome {
    /// Number of assertions that passed.
    pub fn passed(&self) -> usize {
        self.outcomes.iter().filter(|o| o.passed).count()
    }

    /// Total number of assertions.
    pub fn total(&self) -> usize {
        self.outcomes.len()
    }

    /// Whether every assertion passed (and there was at least one).
    pub fn all_passed(&self) -> bool {
        !self.outcomes.is_empty() && self.passed() == self.total()
    }
}

/// Why [`run_parsing_invalid_file`] could not produce an outcome.
#[derive(Debug)]
pub enum ParsingFileError {
    /// The file's inline script has neither a `test_invalid_value(` nor a
    /// `test_valid_value(` call, so there is nothing for this runner to run.
    NoParsingTestCalls,
    /// Reading the test file failed.
    Io(String),
    /// The positive control run ahead of the page's scripts failed, so the
    /// style binding cannot be trusted and the page was not run.
    PositiveControl(String),
    /// The page ran but produced no trustworthy results (a harness-level
    /// error, a resource limit, a host failure, or no tests at all).
    Page(String),
}

impl std::fmt::Display for ParsingFileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ParsingFileError::NoParsingTestCalls => {
                write!(
                    f,
                    "no test_invalid_value( or test_valid_value( calls in this file's inline script"
                )
            }
            ParsingFileError::Io(msg) => write!(f, "I/O error: {msg}"),
            ParsingFileError::PositiveControl(msg) => write!(f, "positive control: {msg}"),
            ParsingFileError::Page(msg) => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for ParsingFileError {}

/// Concatenate the bodies of every `<script>` tag in `html` that has no
/// `src` attribute (a `src`-bearing tag loads an external file, such as
/// `testharness.js`, which this extractor has nothing to say about).
///
/// A manual scan rather than full HTML tokenization: WPT parsing test
/// fixtures are simple enough (no nested `<script>`-like text inside
/// string literals containing the literal substring `</script>`) that this
/// is reliable.
pub fn inline_scripts(html: &str) -> String {
    let mut out = String::new();
    let mut search = 0usize;
    while let Some(rel_start) = html[search..].find("<script") {
        let tag_start = search + rel_start;
        let Some(tag_end_rel) = html[tag_start..].find('>') else {
            break;
        };
        let tag_end = tag_start + tag_end_rel;
        let tag = &html[tag_start..=tag_end];
        let has_src = tag.to_ascii_lowercase().contains("src=");
        let body_start = tag_end + 1;
        let Some(close_rel) = html[body_start..].find("</script>") else {
            break;
        };
        let body_end = body_start + close_rel;
        if !has_src {
            out.push_str(&html[body_start..body_end]);
            out.push('\n');
        }
        search = body_end + "</script>".len();
    }
    out
}

/// Run one `css/*/parsing/*.html` file with the real `testharness.js`,
/// after the positive control.
///
/// `wpt_root` is the fetched WPT checkout root (`target/wpt` by
/// convention; see `scripts/wpt/fetch.sh`); `relative_path` is the file's
/// path under it (for example
/// `css/css-sizing/parsing/box-sizing-invalid.html`).
pub fn run_parsing_invalid_file(
    wpt_root: &Path,
    relative_path: &Path,
) -> Result<ParsingFileOutcome, ParsingFileError> {
    let html = fs::read_to_string(wpt_root.join(relative_path))
        .map_err(|e| ParsingFileError::Io(e.to_string()))?;
    let script = inline_scripts(&html);
    if !script.contains("test_invalid_value(") && !script.contains("test_valid_value(") {
        return Err(ParsingFileError::NoParsingTestCalls);
    }
    let outcomes = run_page_after_control(wpt_root, relative_path, POSITIVE_CONTROL_JS)?;
    Ok(ParsingFileOutcome {
        test_id: path_to_test_id(relative_path),
        outcomes,
    })
}

/// Run the whole page with the real `testharness.js`, `control` first.
fn run_page_after_control(
    wpt_root: &Path,
    relative_path: &Path,
    control: &str,
) -> Result<Vec<TestOutcome>, ParsingFileError> {
    run_testharness_page_with_preamble(relative_path, wpt_root, control).map_err(
        |error| match error {
            PageError::Preamble(message) => ParsingFileError::PositiveControl(message),
            other => ParsingFileError::Page(other.to_string()),
        },
    )
}

fn path_to_test_id(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

#[cfg(test)]
mod tests;
