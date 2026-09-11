//! raikiri-wpt — WPT harness with blitz oracle.
//!
//! Provides expectations parsers, reftest discovery/render/diff, runner
//! dispatch, and the blitz oracle delta. See `reftest`, `runner`, and
//! `oracle` modules for the execution path (spec §12.4, §12.9, §12.10).

pub mod expectations;
pub mod lint;
pub mod oracle;
pub mod reftest;
pub mod runner;
