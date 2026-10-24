//! The names layout algorithms use for style values.
//!
//! Style queries return keywords, lengths and small value types, and the
//! algorithms that read them import those types from here. They describe the
//! used style of a box in the layout engine's own terms; the projection onto
//! the layout library's types is [`ToTaffy`] and [`crate::TaffyStyle`].

mod align;
mod calc_ref;
mod keyword;
mod length;
mod to_taffy;

pub use crate::block::{BaselineSet, baseline_set_of};
pub use crate::resolve::{MaybeResolve, ResolveOrZero};
pub use align::{AlignContent, AlignContentKeyword, AlignItems, AlignItemsKeyword};
pub use align::{AlignSelf, AlignmentSafety};
pub use calc_ref::{CalcRef, resolve_projected_calc};
pub use keyword::{BoxGenerationMode, BoxSizing, Clear, Contain, ContainingBlockClaims};
pub use keyword::{Direction, Display, Float, Overflow, Position};
pub use length::LengthPercentageAuto;
pub use length::{Dimension, ExpandedDimension, ExpandedLengthPercentage};
pub use length::{ExpandedLengthPercentageAuto, LengthIdentity, LengthPercentage};
pub use to_taffy::ToTaffy;
