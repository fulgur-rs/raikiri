//! Shared helpers for testharness-based tests.
//!
//! The fake `resources/testharness.js` stand-in lives in a single fixture
//! (`tests/fixtures/fake-testharness.js`, included with `include_str!` by
//! each test module), while this module owns the one way test code locates
//! the checkout's real `resources/testharness.js` for the `#[ignore]`d
//! end-to-end cases.

use std::path::PathBuf;

/// Path to the checkout's real `resources/testharness.js`, under the sparse
/// WPT checkout from `scripts/wpt/fetch.sh`.
pub(crate) fn real_testharness_js_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt/resources/testharness.js")
}

/// Read the checkout's real `resources/testharness.js`, panicking with the
/// missing path when the sparse checkout is absent.
pub(crate) fn read_real_testharness_js() -> String {
    let path = real_testharness_js_path();
    std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

#[cfg(test)]
mod tests;
