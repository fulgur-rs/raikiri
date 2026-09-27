//! JavaScript runtime support for WPT scripts, backed by the pure-Rust Boa
//! engine.
//!
//! The [`runtime`] module provides native DOM interfaces bound to a
//! `raikiri_dom::Document` through the embedder-supplied
//! [`runtime::DocumentHost`] trait; inline style values are parsed and
//! serialized with `raikiri_style`.

pub mod runtime;

/// One WPT subtest's outcome, as reported by the testharness.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestOutcome {
    /// The subtest's name, verbatim.
    pub name: String,
    /// Whether the subtest passed.
    pub passed: bool,
    /// The harness's message for the subtest, usually empty when `passed`
    /// is true.
    pub message: String,
}
