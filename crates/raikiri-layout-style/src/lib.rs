//! Borrowed style queries for Taffy, independent of the DOM adapter.

mod block;
mod box_style;
mod calc;
mod context;
mod core;
pub use core::preferred_aspect_ratio;
mod flex;
mod length;
mod overrides;
mod physical_sizing;
mod placement;
mod prepared;
mod projection;
mod resolve;
pub mod values;
mod view;
pub use box_style::BoxStyle;
pub use calc::*;
pub use context::*;
pub use overrides::*;
pub use prepared::*;
pub use projection::{Areas, GridRepetition, LineNames, TemplateTracks, Tracks};
pub use projection::{LeafStyle, TaffyStyle, ToTaffy};
pub use values::resolve_projected_calc;
pub use view::*;

#[cfg(test)]
mod tests;
