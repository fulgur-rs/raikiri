//! Document-script runtime: a JavaScript realm with native DOM bindings,
//! backed by the pure-Rust Boa engine.
//!
//! The [`runtime`] module provides [`runtime::DomRuntime`], a realm bound to
//! one embedder-supplied document through [`runtime::DocumentHost`]; inline
//! style values are parsed and serialized with `raikiri_style`.
//!
//! Create a runtime with [`runtime::DomRuntime::new`], or
//! [`runtime::DomRuntime::with_options`] for explicit resource
//! [`runtime::Limits`]. Evaluate a classic script with
//! [`runtime::DomRuntime::evaluate`], or run a whole document end to end
//! with [`runtime::DomRuntime::run_document`], which runs every classic
//! `<script>` in tree order, fires `DOMContentLoaded` and `load`, drains the
//! event loop, and returns a [`runtime::RunReport`]. A failed evaluation is
//! [`runtime::RuntimeError`]; a run stopped by a resource limit reports
//! [`runtime::Abort`] instead, after which the runtime refuses to run
//! anything else.

pub mod runtime;
