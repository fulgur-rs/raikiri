//! Borrowed style queries for Taffy, independent of the DOM adapter.

mod block;
mod calc;
mod context;
mod core;
pub use core::preferred_aspect_ratio;
mod flex;
mod grid;
pub use grid::{Areas, GridRepetition, LineNames, TemplateTracks, Tracks};
mod length;
mod overrides;
mod physical_sizing;
mod prepared;
mod projection;
mod resolve;
pub mod values;
mod view;
pub use calc::*;
pub use context::*;
pub use overrides::*;
pub use prepared::*;
pub use projection::TaffyStyle;
pub use view::*;

#[cfg(test)]
mod tests;
