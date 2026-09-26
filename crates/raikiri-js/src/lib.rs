//! JavaScript runtime support for WPT scripts, backed by the pure-Rust Boa
//! engine.
//!
//! The [`runtime`] module provides native DOM interfaces bound to a
//! `raikiri_dom::Document` through the embedder-supplied
//! [`runtime::DocumentHost`] trait; inline style values are parsed and
//! serialized with `raikiri_style`. The [`testharness`] module runs WPT
//! testharness scripts on that runtime.

pub mod runtime;
pub mod testharness;

/// One WPT `test()` call's outcome, as recorded by the JS-side harness shim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestOutcome {
    /// The `test()` call's name argument, verbatim.
    pub name: String,
    /// Whether the test function ran without throwing.
    pub passed: bool,
    /// The thrown error's string form, empty when `passed` is true.
    pub message: String,
}
