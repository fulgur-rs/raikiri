//! Inline formatting context integration: raikiri DOM and cascade in, shodo
//! paragraphs out.

pub(crate) mod assign;
pub(crate) mod boxes;
pub(crate) mod ch;
pub(crate) mod error;
pub(crate) mod flow;
pub(crate) mod font;
pub(crate) mod projection;
pub(crate) mod root;
pub(crate) mod style;

#[cfg(test)]
mod parity_tests;
#[cfg(test)]
pub(crate) mod test_support;

#[cfg(test)]
mod tests;
