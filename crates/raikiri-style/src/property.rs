//! CSS property value types and per-property parsers.
//!
//! For the canonical list of supported properties, see the `parse_value`
//! match arms together with the `properties!` table in `decl.rs`, which
//! `parse_value` falls through to (or [`supported_property_names`], which
//! merges both). For unknown property names or
//! invalid values, `parse_value` returns `None` (the spec-compliant silent
//! drop; the caller in rule.rs drops the entire declaration).
//!
//! `parse_value` is called from `DeclParser::parse_value` in rule.rs.

mod types;
pub use types::*;

mod longhand_trait;
pub(crate) use longhand_trait::{AbsolutizeCx, Longhand};

mod decl;
pub use decl::*;

mod parse;
pub use parse::*;

mod names;
pub use names::*;

mod calc_serialize;

mod serialize;
pub use serialize::*;

#[cfg(test)]
mod tests;
