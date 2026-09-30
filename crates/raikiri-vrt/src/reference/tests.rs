//! Separated unit tests for the reference module.
//!
//! Previously inline `mod type_tests` / `mod defense_tests` inside
//! `reference.rs`; moved here per repo `tests.rs` separation rule so
//! `cargo-llvm-cov` default `--ignore-filename-regex` excludes test-only
//! lines from coverage (see AGENTS.md).

use super::*;

mod compare_tolerance_tests;
mod defense_tests;
mod diff_artifact_tests;
mod display_tests;
mod load_edge_tests;
mod type_tests;
mod update_goldens_tests;
