//! Runs a WPT CSS parsing test file's `test_invalid_value` assertions
//! through `raikiri_js`, aggregating per-file PASS/FAIL counts.
//!
//! Scope: `test_invalid_value` only (see `raikiri_js`'s crate doc for why
//! `test_valid_value`/`test_valid_selector`/`test_valid_rule` are out of
//! scope for now).

use std::fs;
use std::path::Path;

/// One `css/*/parsing/*.html` file's `test_invalid_value` outcomes.
#[derive(Debug)]
pub struct ParsingFileOutcome {
    /// The file's path relative to the WPT root, forward-slash separated.
    pub test_id: String,
    /// One entry per `test_invalid_value` assertion the file declares, in
    /// source order.
    pub outcomes: Vec<raikiri_js::TestOutcome>,
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
    /// The file's inline script has no `test_invalid_value(` call, so
    /// there is nothing for this harness to run.
    NoInvalidValueCalls,
    /// Reading the test file or `parsing-testcommon.js` failed.
    Io(String),
    /// The JS harness itself reported an error.
    Harness(raikiri_js::HarnessError),
}

impl std::fmt::Display for ParsingFileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ParsingFileError::NoInvalidValueCalls => {
                write!(
                    f,
                    "no test_invalid_value( calls in this file's inline script"
                )
            }
            ParsingFileError::Io(msg) => write!(f, "I/O error: {msg}"),
            ParsingFileError::Harness(e) => write!(f, "{e}"),
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
/// is reliable, and it avoids depending on `raikiri-html`'s DOM-building
/// pipeline just to pull out inline script text.
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

/// Run one `css/*/parsing/*.html` file's `test_invalid_value` assertions.
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
    if !script.contains("test_invalid_value(") {
        return Err(ParsingFileError::NoInvalidValueCalls);
    }
    let parsing_testcommon = fs::read_to_string(wpt_root.join("css/support/parsing-testcommon.js"))
        .map_err(|e| ParsingFileError::Io(e.to_string()))?;

    let outcomes = raikiri_js::run_invalid_value_script(&parsing_testcommon, &script)
        .map_err(ParsingFileError::Harness)?;

    Ok(ParsingFileOutcome {
        test_id: path_to_test_id(relative_path),
        outcomes,
    })
}

fn path_to_test_id(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inline_scripts_skips_src_tags_and_keeps_bare_ones() {
        let html = r#"<script src="/resources/testharness.js"></script>
<script src="/css/support/parsing-testcommon.js"></script>
<script>
test_invalid_value("box-sizing", "margin-box");
</script>"#;
        let extracted = inline_scripts(html);
        assert!(extracted.contains("test_invalid_value(\"box-sizing\", \"margin-box\")"));
        assert!(!extracted.contains("testharness.js"));
    }

    #[test]
    fn inline_scripts_concatenates_multiple_bare_tags() {
        let html = "<script>a();</script><script>b();</script>";
        let extracted = inline_scripts(html);
        assert!(extracted.contains("a();"));
        assert!(extracted.contains("b();"));
    }

    #[test]
    fn inline_scripts_returns_empty_for_no_scripts() {
        assert_eq!(inline_scripts("<html><body>hi</body></html>"), "");
    }

    #[test]
    fn run_parsing_invalid_file_reports_no_invalid_value_calls() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("css/some-cat/parsing")).unwrap();
        fs::write(
            dir.path().join("css/some-cat/parsing/only-valid.html"),
            "<script>test_valid_value(\"color\", \"red\");</script>",
        )
        .unwrap();
        let result = run_parsing_invalid_file(
            dir.path(),
            Path::new("css/some-cat/parsing/only-valid.html"),
        );
        assert!(matches!(result, Err(ParsingFileError::NoInvalidValueCalls)));
    }

    #[test]
    fn run_parsing_invalid_file_on_real_fixture_is_seven_of_seven() {
        let wpt_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
        let outcome = run_parsing_invalid_file(
            &wpt_root,
            Path::new("css/css-sizing/parsing/box-sizing-invalid.html"),
        )
        .unwrap();
        assert_eq!(
            outcome.test_id,
            "css/css-sizing/parsing/box-sizing-invalid.html"
        );
        assert_eq!(outcome.total(), 7);
        assert!(outcome.all_passed(), "{:?}", outcome.outcomes);
    }
}
