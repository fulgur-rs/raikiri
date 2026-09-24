//! Single-page layout driver — `layout_single_page` を pub 提供。
//!
//! Pipeline: cascade (raikiri-style) 出力 + Document arena + PageBox から
//! taffy compute_root_layout を駆動し、text intrinsic size は parley 0.10 の
//! 最小統合で pre-shape する。現在の scope は単一 A4 ページ、ASCII Latin、
//! parley system font default (byte-identical cross-machine は将来 font pinning で対応予定)。
//!
//! Single-page, paged-layout, and neutral page-fragment entry points are public;
//! implementation helpers remain crate-private.

use raikiri_traits::{
    NodeId, NodeKind, PageFragment, PageFragmentEvent, PageFragmentGeometry,
    PageFragmentGeometryTable, PageFragmentInsets, PageFragmentItem, PageFragmentKind,
    PageFragmentLineRange, PageFragmentLink, PageFragmentLinkEvent, PageFragmentOrientation,
    PageFragmentPageGeometry, PageFragmentRect, ReplacedResolver,
};
use rayon::prelude::*;
use std::collections::{HashMap, HashSet};

use crate::document::Document;
use crate::fragment::{FragmentationContext, MulticolStyle};
use crate::node::{MulticolTextFragment, NodeData, NodeFlags};
use parley::{
    Alignment, AlignmentOptions, FontContext, FontFamily, FontStyle, FontWeight, IndentOptions,
    InlineBox, InlineBoxKind, Layout, LayoutContext, LineHeight,
    OverflowWrap as ParleyOverflowWrap, PositionedLayoutItem, StyleProperty,
    TextWrapMode as ParleyTextWrapMode, WordBreak as ParleyWordBreak,
};
use raikiri_style::property::{
    AlignSelfValue, BackgroundImage, BoxSizing as StyleBoxSizing, BreakBetween,
    CalcLengthPercentage, ClearValue, ColumnCountValue, ContentAlignmentValue, Direction,
    DisplayValue, FlexDirectionValue, FlexWrapValue, FloatValue, FontStyle as StyleFontStyle,
    GridAutoFlowValue, GridLineValue, GridRepeatCount, GridTemplateAreasValue, Hyphens, Length,
    LengthOrAuto, LineBreak, OverflowValue, OverflowWrap, PositionValue, PropertyKey,
    PropertyValue, RubyPosition, SelfAlignmentValue, TextAlign, TextAutospace, TextJustify,
    TextTransform, TextWrapMode, VerticalAlign, WhiteSpace, WordBreak, WritingMode,
};
use raikiri_style::{
    CascadeResult, ChLengthProvenance, ComputedColumnWidth, ComputedFlexBasis,
    ComputedGridTemplateTracks, ComputedGridTrackBreadth, ComputedGridTrackListComponent,
    ComputedGridTrackSize, ComputedLength, ComputedLengthPercentage,
    ComputedLengthPercentageOrAuto, ComputedLengthPercentageOrNormal, ComputedLineHeight,
    ComputedTabSize, ComputedValues,
};
use raikiri_traits::{LayoutError, PageBox};
use taffy::{
    AlignContent as TaffyAlignContent, AlignItems as TaffyAlignItems, AlignSelf as TaffyAlignSelf,
    AvailableSpace, BlockContext, BoxSizing as TaffyBoxSizing, Clear as TaffyClear, CompactLength,
    Dimension, Direction as TaffyDirection, Display, ExpandedDimension, ExpandedLengthPercentage,
    FlexDirection as TaffyFlexDirection, FlexWrap as TaffyFlexWrap, Float as TaffyFloat,
    GridAutoFlow as TaffyGridAutoFlow, GridPlacement, GridTemplateArea as TaffyGridTemplateArea,
    GridTemplateComponent, GridTemplateRepetition, Layout as TaffyLayout, LayoutInput,
    LayoutOutput, LayoutPartialTree, LengthPercentage, LengthPercentageAuto, Line as TaffyLine,
    MaxTrackSizingFunction, MinTrackSizingFunction, NodeId as TaffyNodeId,
    Overflow as TaffyOverflow, Point, Position as TaffyPosition, Rect,
    RepetitionCount as TaffyRepetitionCount, RequestedAxis, RunMode, Size, SizingMode,
    TrackSizingFunction, compute_block_layout, compute_root_layout,
    style_helpers as taffy_style_helpers,
};

pub(crate) mod table;

fn style_dimension_length(value: Dimension) -> Option<f32> {
    let raw = value.into_raw();
    (raw.tag() == CompactLength::LENGTH_TAG)
        .then_some(raw.value())
        .filter(|value| value.is_finite())
}

mod bridge;
mod inline_text;
mod multicol;
mod page;
mod page_pipeline;
mod sanitize;

#[allow(unused_imports)]
use bridge::*;
#[allow(unused_imports)]
use inline_text::*;
#[allow(unused_imports)]
use multicol::*;
#[allow(unused_imports)]
use page::*;
#[allow(unused_imports)]
use page_pipeline::*;
#[allow(unused_imports)]
use sanitize::*;

// only reached via an intra-doc link from outside layout/, not real code
#[allow(unused_imports)]
pub(crate) use sanitize::LAYOUT_WARN_CAP;
pub(crate) use sanitize::LayoutWarn;
// only reached via an intra-doc link from outside layout/, not real code
#[allow(unused_imports)]
pub(crate) use bridge::apply_computed_to_style;
pub(crate) use multicol::compute_multicol_layout;
pub(crate) use page::find_body;
// only reached via an intra-doc link from outside layout/, not real code
#[allow(unused_imports)]
pub(crate) use inline_text::preshape_text;
// only reached via an intra-doc link from outside layout/, not real code
#[allow(unused_imports)]
pub(crate) use sanitize::sanitize_taffy;
pub(crate) use sanitize::sanitize_taffy_layout;

pub use page::{
    InitialPageContext, InitialPageContextError, InitialPageProbeResources, PageContentInsets,
    PageMargins, first_page_name, page_content_insets, page_margins, resolve_initial_page_context,
};
pub use page_pipeline::{
    PageSlice, layout_page_fragments, layout_pages, layout_pages_with_page_geometry,
    layout_pages_with_page_geometry_and_resolver,
    layout_pages_with_page_geometry_and_resolver_and_base_url, layout_pages_with_page_steps,
    layout_pages_with_resolver, layout_pages_with_resolver_and_base_url, layout_single_page,
    layout_single_page_with_resolver, layout_single_page_with_resolver_and_base_url,
    page_fragment_events_from_pages, page_fragment_geometry_table, page_fragments_from_slices,
    page_fragments_from_slices_with_page_geometry, relayout_text_for_width,
    resolve_page_fragment_geometry,
};
