//! The names layout algorithms use for style values.
//!
//! Style queries return keywords, lengths and small value types, and the
//! algorithms that read them import those types from here instead of from the
//! layout library. The names and constructors are the library's own.

pub use crate::resolve::{CalcHandle, IntoEdges, LengthExt, LengthIdentity};
pub use crate::resolve::{MaybeResolve, ResolveOrZero};
pub use taffy::AlignContentKeyword;
pub use taffy::{AlignContent, AlignItems, AlignItemsKeyword, AlignSelf, AlignmentSafety};
pub use taffy::{BoxGenerationMode, BoxSizing, Clear, Contain, ContainingBlockClaims};
pub use taffy::{CompactLength, Dimension, LengthPercentage, LengthPercentageAuto};
pub use taffy::{Direction, Display, Float, FloatDirection, JustifyContent, JustifyItems};
pub use taffy::{ExpandedDimension, ExpandedLengthPercentage, ExpandedLengthPercentageAuto};
pub use taffy::{Overflow, Position, TextAlign};

/// `Size` of style values: a width and a height of lengths, keywords or flags.
pub use taffy::Size as StyleSize;

/// `Rect` of style values: four edges of lengths.
pub use taffy::Rect as StyleRect;

/// `Point` of style values: an x and a y of keywords, such as per-axis overflow.
pub use taffy::Point as StylePoint;

/// `Line` of style values: a start and an end, such as a grid placement.
pub use taffy::Line as StyleLine;

/// The position scheme of a box, under the name layout algorithms already use.
pub use taffy::Position as TaffyPosition;
