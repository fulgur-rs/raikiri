//! Borrowed style queries for Taffy, independent of the DOM adapter.

mod calc;
mod context;
mod prepared;
pub use calc::*;
pub use context::*;
pub use prepared::*;

#[cfg(test)]
mod tests;
