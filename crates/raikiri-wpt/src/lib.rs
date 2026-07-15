//! raikiri-wpt — WPT harness with blitz oracle (M1 skeleton).
//!
//! M1 scaffold: module structure + expectations parsers + type stubs.
//! Actual reftest execution and oracle diff aggregation land in M3
//! (see design spec §12.9 / §12.10).

pub mod expectations;
pub mod lint;
pub mod oracle;
pub mod reftest;
pub mod runner;
