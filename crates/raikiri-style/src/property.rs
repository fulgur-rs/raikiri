//! CSS property value types and per-property parsers.
//!
//! For the canonical list of supported properties, see the `parse_value`
//! match arms (the single source of truth). For unknown property names or
//! invalid values, `parse_value` returns `None` (the spec-compliant silent
//! drop; the caller in rule.rs drops the entire declaration).
//!
//! `parse_value` is called from `DeclParser::parse_value` in rule.rs.

#[macro_use]
mod macros;

mod types;
pub use types::*;

mod parse;
pub use parse::*;

mod names;
pub use names::*;

mod calc_serialize;

mod serialize;
pub use serialize::*;

#[cfg(test)]
mod tests;
