//! Borrowed style queries for Taffy, independent of the DOM adapter.

mod calc;
mod context;
mod core;
pub use core::preferred_aspect_ratio;
mod flex;
mod grid;
pub use grid::{Areas, GridRepetition, LineNames, TemplateTracks, Tracks};
mod legacy_sizing;
mod length;
mod overrides;
mod prepared;
mod view;
pub use calc::*;
pub use context::*;
pub use legacy_sizing::LegacySizingUndo;
pub use overrides::*;
pub use prepared::*;
pub use view::*;

#[cfg(test)]
mod tests;
