//! Runs a WPT CSS parsing test file's `test_invalid_value`/`test_valid_value`
//! assertions, aggregating per-file PASS/FAIL counts.
//!
//! The real `css/support/parsing-testcommon.js` and the file's inline script
//! run on the native DOM runtime against a live Raikiri document built from
//! the test page, so `document.getElementById('target')` and element style
//! declarations behave as they do for any other testharness page.

use std::fs;
use std::path::Path;

use raikiri_js::TestOutcome;
use raikiri_js::testharness::{TestHarnessError, run_testharness_scripts_on_host};

use crate::reftest::{DEFAULT_REFTTEST_HEIGHT, DEFAULT_REFTTEST_WIDTH, prepare_wpt_live_document};
use crate::wpt_host::WptDocumentHost;

/// The name suffix `parsing-testcommon.js` gives every `test()` registered by
/// `test_invalid_value`.
const INVALID_VALUE_NAME_SUFFIX: &str = " should not set the property value";

/// Known-valid, property-agnostic sanity check run before the helper and the
/// test script: `color: red` must round-trip through an element's inline
/// style. It catches an inert style binding (a setter that silently no-ops,
/// or a getter that always reads back empty) that would otherwise make every
/// `test_invalid_value` assertion pass for the wrong reason. A failure throws,
/// so the whole file is reported as an error instead of a result.
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

/// Which JavaScript implementation executes the parsing tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Engine {
    /// The standalone CSS-only shim context.
    Legacy,
    /// Native DOM interfaces over a live `WptDocumentHost` document.
    Native,
}

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

    /// Outcomes registered by `test_invalid_value`, identified by the test
    /// name `parsing-testcommon.js` gives them.
    pub fn invalid_value_outcomes(&self) -> impl Iterator<Item = &TestOutcome> {
        self.outcomes
            .iter()
            .filter(|o| o.name.ends_with(INVALID_VALUE_NAME_SUFFIX))
    }

    /// Number of passing `test_invalid_value` outcomes.
    pub fn invalid_value_passed(&self) -> usize {
        self.invalid_value_outcomes().filter(|o| o.passed).count()
    }

    /// Number of `test_invalid_value` outcomes.
    pub fn invalid_value_total(&self) -> usize {
        self.invalid_value_outcomes().count()
    }
}

/// Why [`run_parsing_invalid_file`] could not produce an outcome.
#[derive(Debug)]
pub enum ParsingFileError {
    /// The file's inline script has neither a `test_invalid_value(` nor a
    /// `test_valid_value(` call, so there is nothing for this runner to run.
    NoInvalidValueCalls,
    /// Reading the test file or `parsing-testcommon.js` failed.
    Io(String),
    /// Building the live document for the test page failed.
    Document(String),
    /// The legacy shim reported an error.
    Legacy(raikiri_js::HarnessError),
    /// The native testharness run reported an error.
    Harness(TestHarnessError),
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
            ParsingFileError::Document(msg) => write!(f, "live document: {msg}"),
            ParsingFileError::Legacy(e) => write!(f, "{e}"),
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

/// Run one `css/*/parsing/*.html` file on the native DOM runtime.
///
/// `wpt_root` is the fetched WPT checkout root (`target/wpt` by
/// convention; see `scripts/wpt/fetch.sh`); `relative_path` is the file's
/// path under it (for example
/// `css/css-sizing/parsing/box-sizing-invalid.html`).
pub fn run_parsing_invalid_file(
    wpt_root: &Path,
    relative_path: &Path,
) -> Result<ParsingFileOutcome, ParsingFileError> {
    run_parsing_invalid_file_with(wpt_root, relative_path, Engine::Native)
}

/// Run one `css/*/parsing/*.html` file on the selected `engine`.
pub fn run_parsing_invalid_file_with(
    wpt_root: &Path,
    relative_path: &Path,
    engine: Engine,
) -> Result<ParsingFileOutcome, ParsingFileError> {
    let page_path = wpt_root.join(relative_path);
    let html = fs::read_to_string(&page_path).map_err(|e| ParsingFileError::Io(e.to_string()))?;
    let script = inline_scripts(&html);
    if !script.contains("test_invalid_value(") && !script.contains("test_valid_value(") {
        return Err(ParsingFileError::NoInvalidValueCalls);
    }
    let parsing_testcommon = fs::read_to_string(wpt_root.join("css/support/parsing-testcommon.js"))
        .map_err(|e| ParsingFileError::Io(e.to_string()))?;

    let outcomes = match engine {
        Engine::Legacy => raikiri_js::run_invalid_value_script(&parsing_testcommon, &script)
            .map_err(ParsingFileError::Legacy)?,
        Engine::Native => {
            let page_base = page_path.parent().unwrap_or(wpt_root);
            let setup = prepare_wpt_live_document(
                &html,
                DEFAULT_REFTTEST_WIDTH,
                DEFAULT_REFTTEST_HEIGHT,
                page_base,
                wpt_root,
            )
            .map_err(ParsingFileError::Document)?;
            run_testharness_scripts_on_host(
                &[POSITIVE_CONTROL_JS, &parsing_testcommon, &script],
                WptDocumentHost::new(setup, wpt_root),
            )
            .map_err(ParsingFileError::Harness)?
        }
    };

    Ok(ParsingFileOutcome {
        test_id: path_to_test_id(relative_path),
        outcomes,
    })
}

/// A file whose `test_invalid_value` results got worse on the native engine,
/// or got better (see [`compare`]).
#[derive(Debug, PartialEq, Eq)]
pub struct Difference {
    /// WPT-relative test ID.
    pub test_id: String,
    /// Legacy summary (`passed/total` over `test_invalid_value` outcomes, or
    /// the error).
    pub legacy: String,
    /// Native summary, in the same form.
    pub native: String,
}

/// Regressions and improvements between the two engines' results for one
/// set of files.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Comparison {
    /// Files where legacy ran without error and native errored, or native
    /// passed fewer `test_invalid_value` assertions.
    pub regressions: Vec<Difference>,
    /// Files where legacy errored and native did not, or native passed more
    /// `test_invalid_value` assertions.
    pub improvements: Vec<Difference>,
}

fn invalid_value_summary(result: &Result<ParsingFileOutcome, ParsingFileError>) -> String {
    match result {
        Ok(outcome) => format!(
            "{}/{}",
            outcome.invalid_value_passed(),
            outcome.invalid_value_total()
        ),
        Err(error) => format!("error: {error}"),
    }
}

/// Compare the legacy and native results of the same file, counting only
/// `test_invalid_value` outcomes. `results` pairs a test ID with its legacy
/// and native results.
pub fn compare<'a, I>(results: I) -> Comparison
where
    I: IntoIterator<
        Item = (
            &'a str,
            &'a Result<ParsingFileOutcome, ParsingFileError>,
            &'a Result<ParsingFileOutcome, ParsingFileError>,
        ),
    >,
{
    let mut comparison = Comparison::default();
    for (test_id, legacy, native) in results {
        let bucket = match (legacy, native) {
            (Ok(_), Err(_)) => Some(&mut comparison.regressions),
            (Err(_), Ok(_)) => Some(&mut comparison.improvements),
            (Ok(old), Ok(new)) if new.invalid_value_passed() < old.invalid_value_passed() => {
                Some(&mut comparison.regressions)
            }
            (Ok(old), Ok(new)) if new.invalid_value_passed() > old.invalid_value_passed() => {
                Some(&mut comparison.improvements)
            }
            _ => None,
        };
        if let Some(bucket) = bucket {
            bucket.push(Difference {
                test_id: test_id.to_owned(),
                legacy: invalid_value_summary(legacy),
                native: invalid_value_summary(native),
            });
        }
    }
    comparison
}

fn path_to_test_id(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

#[cfg(test)]
mod tests;
