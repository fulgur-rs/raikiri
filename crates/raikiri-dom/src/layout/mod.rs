//! Single-page layout driver — publicly provides `layout_single_page`.
//!
//! Pipeline: uses the cascade (raikiri-style) output, Document arena, and PageBox
//! to drive taffy with `<body>` as the root. Paragraphs are laid out by the shodo
//! inline engine, which taffy measures as leaves; without fonts set on the
//! Document, the installed system fonts are used.
//!
//! Single-page and paged-layout entry points are public;
//! implementation helpers remain crate-private.

use crate::page_projection::records::*;
use raikiri_traits::{NodeId, NodeKind, ReplacedResolver};
use std::collections::{HashMap, HashSet};

use crate::document::Document;
use crate::fragment::{FragmentationContext, MulticolStyle};
use crate::node::{MulticolTextFragment, NodeData, NodeFlags};
use crate::taffy_impl::block_flow::context::BlockContext;
use raikiri_style::property::{
    AlignSelfValue, BoxSizing as StyleBoxSizing, BreakBetween, CalcLengthPercentage, ClearValue,
    ColumnCountValue, ColumnFillValue, ContentAlignmentValue, Direction, DisplayValue,
    FlexDirectionValue, FlexWrapValue, FloatValue, GridAutoFlowValue, GridLineValue,
    GridRepeatCount, GridTemplateAreasValue, Length, LengthOrAuto, LengthPercentageOrCalc,
    OverflowValue, PositionValue, PropertyKey, PropertyValue, SelfAlignmentValue, WritingMode,
};
use raikiri_style::{
    CascadeResult, ComputedColumnWidth, ComputedFlexBasis, ComputedGridTemplateTracks,
    ComputedGridTrackBreadth, ComputedGridTrackListComponent, ComputedGridTrackSize,
    ComputedLength, ComputedLengthPercentage, ComputedLengthPercentageOrAuto,
    ComputedLengthPercentageOrCalc, ComputedLengthPercentageOrNormal, ComputedLineHeight,
    ComputedValues,
};
use raikiri_traits::{LayoutError, PageBox};
use taffy::{
    AlignContent as TaffyAlignContent, AlignItems as TaffyAlignItems, AvailableSpace,
    BoxSizing as TaffyBoxSizing, Clear as TaffyClear, Dimension, Direction as TaffyDirection,
    Display, ExpandedDimension, ExpandedLengthPercentage, FlexDirection as TaffyFlexDirection,
    FlexWrap as TaffyFlexWrap, Float as TaffyFloat, GridAutoFlow as TaffyGridAutoFlow,
    GridPlacement, GridTemplateArea as TaffyGridTemplateArea, GridTemplateComponent,
    GridTemplateRepetition, Layout as TaffyLayout, LayoutInput, LayoutOutput, LayoutPartialTree,
    LengthPercentage, LengthPercentageAuto, Line as TaffyLine, MaxTrackSizingFunction,
    MinTrackSizingFunction, NodeId as TaffyNodeId, Overflow as TaffyOverflow, Point,
    Position as TaffyPosition, Rect, RepetitionCount as TaffyRepetitionCount, RequestedAxis,
    RunMode, Size, SizingMode, TrackSizingFunction, style_helpers as taffy_style_helpers,
};

pub(crate) mod box_style_data;
pub(crate) mod context;
pub(crate) mod retained_styles;
pub(crate) mod style_views;
pub(crate) mod table;

fn style_dimension_length(value: Dimension) -> Option<f32> {
    resolve_dimension(value, None).filter(|value| value.is_finite())
}

mod body_overflow;
mod box_tree;
mod bridge;
pub(crate) mod cache_context;
mod compact_length;
mod depth;
mod fixed;
pub(crate) mod ifc;
mod initial_containing_block;
pub(crate) mod multicol;
pub(crate) mod oof_containing_block;
mod page;
mod page_pipeline;
mod relative;
mod root_box;
pub(crate) mod sanitize;
mod spanner_wrappers;
#[cfg(test)]
mod static_position;
#[cfg(test)]
pub(crate) mod test_support;

pub(crate) use body_overflow::viewport_overflow_source;
pub(crate) use box_tree::{BoxParentScope, box_parent};
pub(crate) use compact_length::{
    resolve_dimension, resolve_length_percentage, resolve_length_percentage_auto,
};
use multicol::*;
pub(crate) use page::used_style_length_percentage;
use page::*;
use page_pipeline::*;
use sanitize::*;

#[cfg(test)]
pub(crate) use bridge::apply_computed_to_style;
pub use depth::{MAX_LAYOUT_DEPTH, validate_layout_depth};
pub(crate) use multicol::spanning::is_spanner;
pub(crate) use multicol::{
    compute_multicol_layout, multicol_definite_dimension, root_column_fragments,
};
#[doc(hidden)]
pub(crate) use page::find_body;
pub(crate) use page::is_floating_box_for_pagination;
pub(crate) use sanitize::LayoutWarn;
// only reached via an intra-doc link from outside layout/, not real code
pub use ifc::ch::{measure_ch_advance, measure_normal_line_height};
pub(crate) use sanitize::sanitize_taffy;

#[doc(hidden)]
pub use fixed::generates_inline_box;
pub use fixed::{establishes_fixed_containing_block, has_applied_transform, is_transformable};
pub(crate) use fixed::{
    establishes_fixed_containing_block_for_layout, is_transformable_for_layout,
};
/// Boxes of the inline elements of a paragraph laid out by the inline engine.
#[doc(hidden)]
pub use ifc::inline_boxes::{BoxRect, GeneratedBlockPiece, InlineBoxPiece};
/// Glyph positions of the lines of a paragraph laid out by the inline engine.
#[doc(hidden)]
pub use ifc::positioned_runs::{
    AnnotationRun, LineGlyph, PositionedContentImage, PositionedLine, PositionedLines,
    PositionedMarker, PositionedRun, cumulative_offset,
};
#[doc(hidden)]
pub use ifc::root::IfcBuildMode;
/// Text outside paragraphs shaped by the inline engine.
#[doc(hidden)]
pub use ifc::standalone::{StandaloneAlign, StandaloneStyle, StandaloneText};
#[doc(hidden)]
pub use ifc::style::relative_offset;
/// Lines of the text nodes of a paragraph laid out by the inline engine.
pub use ifc::text_lines::{IfcTextLine, IfcTextLines};
pub use page::{
    InitialPageContext, InitialPageContextError, InitialPageProbeResources, PageContentInsets,
    PageMargins, first_page_name, page_content_insets, page_content_insets_for_page, page_margins,
    page_margins_for_page, resolve_initial_page_context,
};
pub use page_pipeline::{
    PageImageOptions, PageLayoutControl, PageLayoutOptions, PageSlice, layout_pages,
    layout_pages_with_options, layout_single_page, layout_single_page_with_resolver,
    layout_single_page_with_resolver_and_base_url, pages_follow_x_axis, relayout_text_for_width,
};
pub use relative::placed_without_relative_offset;
#[doc(hidden)]
pub use relative::used_relative_offset;
pub(crate) use relative::{
    Precedence, layout_relative_insets, placed_without_relative_offset_for_layout,
    used_relative_offset_for_layout,
};
pub use root_box::{RootBox, root_box};

pub(crate) use page_pipeline::{
    line_center_on_page, page_fragment_events_from_pages, project_slices_with_control,
};

pub(crate) use page_pipeline::column_projection::{
    ParagraphCache, ProjectionWork, first_line_restyles, generated_piece_style,
};
