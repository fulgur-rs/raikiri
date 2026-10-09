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
use raikiri_style::property::{
    AlignSelfValue, BoxSizing as StyleBoxSizing, BreakBetween, CalcLengthPercentage, ClearValue,
    ColumnCountValue, ColumnFillValue, ContentAlignmentValue, Direction, DisplayValue,
    FlexDirectionValue, FlexWrapValue, FloatValue, GridAutoFlowValue, GridLineValue,
    GridRepeatCount, GridTemplateAreasValue, Length, LengthOrAuto, OverflowValue, PositionValue,
    PropertyKey, PropertyValue, RubyPosition, SelfAlignmentValue, WritingMode,
};
use raikiri_style::{
    CascadeResult, ChLengthProvenance, ComputedColumnWidth, ComputedFlexBasis,
    ComputedGridTemplateTracks, ComputedGridTrackBreadth, ComputedGridTrackListComponent,
    ComputedGridTrackSize, ComputedLength, ComputedLengthPercentage,
    ComputedLengthPercentageOrAuto, ComputedLengthPercentageOrNormal, ComputedLineHeight,
    ComputedValues,
};
use raikiri_traits::{LayoutError, PageBox};
use taffy::{
    AlignContent as TaffyAlignContent, AlignItems as TaffyAlignItems, AvailableSpace, BlockContext,
    BoxSizing as TaffyBoxSizing, Clear as TaffyClear, CompactLength, Dimension,
    Direction as TaffyDirection, Display, ExpandedDimension, ExpandedLengthPercentage,
    FlexDirection as TaffyFlexDirection, FlexWrap as TaffyFlexWrap, Float as TaffyFloat,
    GridAutoFlow as TaffyGridAutoFlow, GridPlacement, GridTemplateArea as TaffyGridTemplateArea,
    GridTemplateComponent, GridTemplateRepetition, Layout as TaffyLayout, LayoutInput,
    LayoutOutput, LayoutPartialTree, LengthPercentage, LengthPercentageAuto, Line as TaffyLine,
    MaxTrackSizingFunction, MinTrackSizingFunction, NodeId as TaffyNodeId,
    Overflow as TaffyOverflow, Point, Position as TaffyPosition, Rect,
    RepetitionCount as TaffyRepetitionCount, RequestedAxis, RunMode, Size, SizingMode,
    TrackSizingFunction, compute_block_layout, style_helpers as taffy_style_helpers,
};

pub(crate) mod table;

fn style_dimension_length(value: Dimension) -> Option<f32> {
    let raw = value.into_raw();
    (raw.tag() == CompactLength::LENGTH_TAG)
        .then_some(raw.value())
        .filter(|value| value.is_finite())
}

mod body_block_margins;
mod bridge;
mod depth;
pub(crate) mod ifc;
mod multicol;
mod page;
mod page_pipeline;
pub(crate) mod sanitize;
#[cfg(test)]
pub(crate) mod test_support;

use body_block_margins::*;
use multicol::*;
pub(crate) use page::used_style_length_percentage;
use page::*;
use page_pipeline::*;
use sanitize::*;

pub(crate) use bridge::apply_computed_to_style;
pub use depth::{MAX_LAYOUT_DEPTH, validate_layout_depth};
pub(crate) use multicol::{
    compute_multicol_layout, multicol_definite_dimension, root_column_fragments,
};
#[doc(hidden)]
pub(crate) use page::find_body;
pub(crate) use page::is_floating_box_for_pagination;
pub(crate) use sanitize::LayoutWarn;
// only reached via an intra-doc link from outside layout/, not real code
pub use ifc::ch::measure_ch_advance;
pub(crate) use sanitize::sanitize_taffy;
pub(crate) use sanitize::sanitize_taffy_layout;

/// Boxes of the inline elements of a paragraph laid out by the inline engine.
#[doc(hidden)]
pub use ifc::inline_boxes::{BoxRect, InlineBoxPiece};
/// Glyph positions of the lines of a paragraph laid out by the inline engine.
#[doc(hidden)]
pub use ifc::positioned_runs::{
    LineGlyph, PositionedLine, PositionedLines, PositionedMarker, PositionedRun, cumulative_offset,
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
    PageLayoutControl, PageSlice, layout_pages, layout_pages_with_page_geometry,
    layout_pages_with_page_geometry_and_control, layout_pages_with_page_geometry_and_resolver,
    layout_pages_with_page_geometry_and_resolver_and_base_url,
    layout_pages_with_page_geometry_and_resolver_and_base_url_and_control,
    layout_pages_with_page_steps, layout_pages_with_resolver,
    layout_pages_with_resolver_and_base_url, layout_pages_with_resolver_and_base_url_and_control,
    layout_single_page, layout_single_page_with_resolver,
    layout_single_page_with_resolver_and_base_url, relayout_text_for_width,
};

pub(crate) use page_pipeline::{
    line_center_on_page, page_fragment_events_from_pages, project_slices_with_control,
};

pub(crate) use page_pipeline::column_projection::{ParagraphCache, ProjectionWork};
