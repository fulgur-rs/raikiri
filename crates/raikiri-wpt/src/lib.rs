//! raikiri-wpt — WPT harness with blitz oracle (initial skeleton).
//!
//! Current scaffold: module structure + expectations parsers + type stubs.
//! Actual reftest execution and oracle diff aggregation are future work
//! (see design spec §12.9 / §12.10).

pub mod expectations;
pub mod lint;
pub mod oracle;
pub mod reftest;
pub mod runner;
