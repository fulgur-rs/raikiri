//! raikiri-html — html5ever wrapper (`RaikiriTreeSink`) + `parse` /
//! `parse_with_sink` entrypoints。cascade 前の [`UncascadedDocument`] を produce
//! する薄い parser layer (責務は parse だけ、cascade / layout は含まない)。
//!
//! Note: `parse` / `parse_with_sink` become linked items in Task 4; kept as
//! plain code text here for pristine intermediate `cargo doc` output.

mod types;

pub use types::{ParseOptions, UncascadedDocument};
