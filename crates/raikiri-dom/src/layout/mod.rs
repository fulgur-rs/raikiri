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
    GridRepeatCount, GridTemplateAreasValue, Length, LengthOrAuto, LengthPercentageOrCalc,
    OverflowValue, PositionValue, PropertyKey, PropertyValue, SelfAlignmentValue, WritingMode,
};
use raikiri_style::{
    CascadeResult, ChLengthProvenance, ComputedColumnWidth, ComputedFlexBasis,
    ComputedGridTemplateTracks, ComputedGridTrackBreadth, ComputedGridTrackListComponent,
    ComputedGridTrackSize, ComputedLength, ComputedLengthPercentage,
    ComputedLengthPercentageOrAuto, ComputedLengthPercentageOrCalc,
    ComputedLengthPercentageOrNormal, ComputedLineHeight, ComputedValues,
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
    TrackSizingFunction, style_helpers as taffy_style_helpers,
};

pub(crate) mod table;

fn style_dimension_length(value: Dimension) -> Option<f32> {
    resolve_dimension(value, None).filter(|value| value.is_finite())
}

/// The used width a measured `ch` width was given before Taffy: a length, or
/// a calc() sum whose percentage resolves against `parent_width`.
fn measured_ch_width(
    doc: &Document,
    cascade: &CascadeResult,
    id: usize,
    parent_width: f32,
) -> Option<f32> {
    cascade.computed[id].width_ch.as_ref()?;
    let width = doc.nodes[id].style.size.width;
    let raw = width.into_raw();
    if raw.is_calc() {
        Some(crate::taffy_impl::resolve_calc(
            raw.calc_value(),
            parent_width,
        ))
    } else {
        style_dimension_length(width)
    }
}

mod body_block_margins;
mod box_tree;
mod bridge;
mod compact_length;
mod depth;
mod fixed;
pub(crate) mod ifc;
pub(crate) mod multicol;
mod page;
mod page_pipeline;
mod relative;
mod root_box;
pub(crate) mod sanitize;
mod static_position;
#[cfg(test)]
pub(crate) mod test_support;

use body_block_margins::*;
pub(crate) use box_tree::{BoxParentScope, box_parent};
pub(crate) use compact_length::{
    resolve_dimension, resolve_length_percentage, resolve_length_percentage_auto,
};
use fixed::place_fixed_boxes_in_page_area;
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
pub(crate) use page::is_floating_box_for_pagination;
#[doc(hidden)]
pub(crate) use page::{find_body, is_body_element};
pub(crate) use sanitize::LayoutWarn;
// only reached via an intra-doc link from outside layout/, not real code
pub use ifc::ch::{measure_ch_advance, measure_normal_line_height};
pub(crate) use sanitize::sanitize_taffy;
pub(crate) use sanitize::sanitize_taffy_layout;

pub use fixed::{establishes_fixed_containing_block, is_transformable};
/// Boxes of the inline elements of a paragraph laid out by the inline engine.
#[doc(hidden)]
pub use ifc::inline_boxes::{BoxRect, InlineBoxPiece};
/// Glyph positions of the lines of a paragraph laid out by the inline engine.
#[doc(hidden)]
pub use ifc::positioned_runs::{
    LineGlyph, PositionedContentImage, PositionedLine, PositionedLines, PositionedMarker,
    PositionedRun, cumulative_offset,
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
pub use relative::placed_without_relative_offset;
#[doc(hidden)]
pub use relative::used_relative_offset;
pub(crate) use relative::{Precedence, used_relative_insets};
pub use root_box::{RootBox, root_box};

pub(crate) use page_pipeline::{
    line_center_on_page, page_fragment_events_from_pages, project_slices_with_control,
};

pub(crate) use page_pipeline::column_projection::{
    ParagraphCache, ProjectionWork, generated_piece_style,
};
