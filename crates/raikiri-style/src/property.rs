//! CSS property value types and per-property parsers.
//!
//! For the canonical list of supported properties, see the `parse_value`
//! match arms (the single source of truth). For unknown property names or
//! invalid values, `parse_value` returns `None` (the spec-compliant silent
//! drop; the caller in rule.rs drops the entire declaration).
//!
//! `parse_value` is called from `DeclParser::parse_value` in rule.rs.

mod types;
pub use types::*;

mod parse;
pub use parse::*;

mod names;
pub use names::*;

mod calc_serialize;

mod media_math;
pub(crate) use media_math::{
    MathTypes, MediaNumericType, is_math_function_name, math_value_has_type, parse_math_function,
    product_operator, sum_operator,
};

mod serialize;
pub use serialize::*;

mod serialize_alpha;
pub use serialize_alpha::serialize_alpha_value;

mod serialize_position;
pub use serialize_position::serialize_position_value;

#[cfg(test)]
mod tests;
