//! Inline formatting context integration: raikiri DOM and cascade in, shodo
//! paragraphs out.

pub(crate) mod assign;
pub(crate) mod error;
pub(crate) mod font;
pub(crate) mod projection;
pub(crate) mod root;
pub(crate) mod style;

#[cfg(test)]
mod test_support;

#[cfg(test)]
mod tests;
