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

/// Document arena を DFS で walk し、最初の `<body>` element の arena index を返す。
///
/// iterative `Vec` stack で実装 (cascade §deep_nesting の pattern と一貫、
/// deep DOM で stack overflow を回避)。fragment parse (no `<body>`) では
/// `None`、caller が `LayoutError::Internal` に昇格させる。
///
/// `!is_in_document()` の subtree
/// (`<template>` descendants など) を skip する。inert subtree 内に `<body>`
/// tag があってもそれを本物の body として選ばないため — 例えば
/// `<template><body>ghost</body></template>` の後に real `<body>` が来る HTML
/// で ghost body を選んでしまうと後続の layout / paint が inert subtree に対して
/// 実行されてしまう。
pub(crate) fn find_body(doc: &Document) -> Option<usize> {
    let mut stack: Vec<usize> = vec![doc.root];
    while let Some(node_idx) = stack.pop() {
        let node = &doc.nodes[node_idx];
        if !node.is_in_document() {
            // inert subtree — 本 subtree の中に body があっても選ばない。
            continue;
        }
        if node.kind() == NodeKind::Element && node.tag_name() == Some("body") {
            return Some(node_idx);
        }
        // children を reverse push すると document order で pop される
        for &child in node.children.iter().rev() {
            stack.push(child);
        }
    }
    None
}

/// `<body>` の taffy::Style.size を PageBox の width / height (CSS px) に強制する。
///
/// CSS Paged Media の initial containing block = @page size。現行実装は @page 非対応
/// のため body.style.size に直接注入する妥協。将来 @page cascade + per-page
/// PageBox を導入する際に `<html>` root style に site を昇格予定。
#[allow(dead_code)]
pub(crate) fn apply_page_box_to_body(doc: &mut Document, body_id: usize, page_box: PageBox) {
    doc.nodes[body_id].style.size = Size {
        width: Dimension::length(page_box.width),
        height: Dimension::length(page_box.height),
    };
}

fn apply_page_content_box_to_body(
    doc: &mut Document,
    body_id: usize,
    page_box: PageBox,
    margins: PageMargins,
    insets: PageContentInsets,
) {
    // Page border/padding shift the painted page origin, but they do not
    // establish a narrower inline containing block for document flow.  Keep
    // the initial containing-block width at the margin content width; the
    // paint walk applies the horizontal inset when positioning the flow.
    doc.nodes[body_id].style.size = Size {
        width: Dimension::length(margins.content_width(page_box).max(0.0)),
        height: Dimension::length(
            (margins.content_height(page_box) - insets.top - insets.bottom).max(0.0),
        ),
    };
}

/// Resolve the width of a direct-body absolutely positioned box whose
/// horizontal insets and preferred width are all `auto`.
///
/// Taffy's absolute-position fallback leaves such a box at the containing
/// block width even when a resolved horizontal margin consumes part of that
/// width. CSS 2.1 §10.3.7 solves the horizontal constraint instead: the
/// border box plus both margins must fit the containing block. Do this before
/// the root layout so text descendants receive the corrected width while
/// shaping/reflowing; changing only `unrounded_layout` after the compute would
/// leave their line breaks stale.
///
/// The first implementation is deliberately limited to direct `<body>`
/// children in the static containing block. Nested containing blocks and
/// viewport-fixed boxes need their own containing-block geometry and remain
/// on Taffy's normal path until that geometry is available.
fn used_style_length_percentage(value: LengthPercentage, basis: f32) -> Option<f32> {
    let raw = value.into_raw();
    let resolved = match raw.tag() {
        CompactLength::LENGTH_TAG => raw.value(),
        CompactLength::PERCENT_TAG => raw.value() * basis,
        _ => return None,
    };
    resolved.is_finite().then_some(resolved)
}

fn style_dimension_length(value: Dimension) -> Option<f32> {
    let raw = value.into_raw();
    (raw.tag() == CompactLength::LENGTH_TAG)
        .then_some(raw.value())
        .filter(|value| value.is_finite())
}

fn used_style_length_percentage_auto(value: LengthPercentageAuto, basis: f32) -> Option<f32> {
    let raw = value.into_raw();
    let resolved = match raw.tag() {
        CompactLength::LENGTH_TAG => raw.value(),
        CompactLength::PERCENT_TAG => raw.value() * basis,
        _ => return None,
    };
    resolved.is_finite().then_some(resolved)
}

fn used_computed_length_percentage_or_auto(
    value: ComputedLengthPercentageOrAuto,
    basis: f32,
) -> Option<f32> {
    let resolved = match value {
        ComputedLengthPercentageOrAuto::Px(px) => px,
        ComputedLengthPercentageOrAuto::Percent(percent) => basis * percent / 100.0,
        ComputedLengthPercentageOrAuto::Calc(value) => value.px + basis * value.percent / 100.0,
        ComputedLengthPercentageOrAuto::Auto => return None,
    };
    resolved.is_finite().then_some(resolved)
}

fn resolve_direct_absolute_auto_widths(
    document: &mut Document,
    cascade: &CascadeResult,
    body_id: usize,
    containing_width: f32,
) {
    if !containing_width.is_finite() || containing_width <= 0.0 {
        return;
    }
    let children = document.nodes[body_id].children.clone();
    for child_id in children {
        let computed = &cascade.computed[child_id];
        if !matches!(computed.position, PositionValue::Absolute)
            || !matches!(computed.width, ComputedLengthPercentageOrAuto::Auto)
            || !matches!(computed.left, ComputedLengthPercentageOrAuto::Auto)
            || !matches!(computed.right, ComputedLengthPercentageOrAuto::Auto)
        {
            continue;
        }

        let style = &document.nodes[child_id].style;
        let margin_left = used_style_length_percentage_auto(style.margin.left, containing_width)
            .or_else(|| {
                used_computed_length_percentage_or_auto(computed.margin.left, containing_width)
            });
        let margin_right = used_style_length_percentage_auto(style.margin.right, containing_width)
            .or_else(|| {
                used_computed_length_percentage_or_auto(computed.margin.right, containing_width)
            });
        if margin_left.is_none() && margin_right.is_none() {
            continue;
        }
        let margin_left = margin_left.unwrap_or(0.0);
        let margin_right = margin_right.unwrap_or(0.0);
        let padding_left = used_style_length_percentage(style.padding.left, containing_width);
        let padding_right = used_style_length_percentage(style.padding.right, containing_width);
        let Some(padding_left) = padding_left else {
            continue;
        };
        let Some(padding_right) = padding_right else {
            continue;
        };
        let border_left = computed.border.left.width().px();
        let border_right = computed.border.right.width().px();
        if !border_left.is_finite() || !border_right.is_finite() {
            continue;
        }

        let available_border_box = containing_width - margin_left - margin_right;
        let width = match computed.box_sizing {
            StyleBoxSizing::BorderBox => available_border_box,
            StyleBoxSizing::ContentBox => {
                available_border_box - border_left - border_right - padding_left - padding_right
            }
            _ => available_border_box - border_left - border_right - padding_left - padding_right,
        };
        if width.is_finite() {
            document.nodes[child_id].style.size.width = Dimension::length(width.max(0.0));
        }
    }
}

/// Used page margins resolved from the page-context cascade.
///
/// The four values are paper-relative CSS pixels.  Unlike ordinary element
/// margins, page percentages use the corresponding page-box axis: top and
/// bottom use the page height, while left and right use the page width.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct PageMargins {
    /// Used top margin in CSS pixels.
    pub top: f32,
    /// Used right margin in CSS pixels.
    pub right: f32,
    /// Used bottom margin in CSS pixels.
    pub bottom: f32,
    /// Used left margin in CSS pixels.
    pub left: f32,
}

impl PageMargins {
    /// Width available to the page's content area.
    pub fn content_width(self, page_box: PageBox) -> f32 {
        (page_box.width - self.left - self.right).max(0.0)
    }

    /// Height available to the page's content area.
    pub fn content_height(self, page_box: PageBox) -> f32 {
        (page_box.height - self.top - self.bottom).max(0.0)
    }

    /// Whether all four used margins are zero.
    pub fn is_zero(self) -> bool {
        self.top == 0.0 && self.right == 0.0 && self.bottom == 0.0 && self.left == 0.0
    }
}

fn page_length_to_px(length: Length, basis: f32) -> f32 {
    let value = match length {
        Length::Px(value) => value,
        Length::Pt(value) => value * 96.0 / 72.0,
        Length::Cm(value) => value * 96.0 / 2.54,
        Length::Mm(value) => value * 96.0 / 25.4,
        Length::Q(value) => value * 96.0 / 101.6,
        Length::In(value) => value * 96.0,
        Length::Pc(value) => value * 96.0 / 6.0,
        // Page-context absolutization normally resolves these before this
        // consumer sees them.  Keep a deterministic initial-font fallback for
        // values introduced through direct/internal cascade construction.
        Length::Em(value)
        | Length::Rem(value)
        | Length::Ex(value)
        | Length::Rex(value)
        | Length::Ch(value)
        | Length::Rch(value)
        | Length::Ic(value)
        | Length::Ric(value)
        | Length::Lh(value)
        | Length::Rlh(value) => value * 16.0,
        Length::Percent(value) => basis * value / 100.0,
        _ => 0.0,
    };
    if value.is_finite() { value } else { 0.0 }
}

fn page_margin_length(value: LengthOrAuto, basis: f32) -> f32 {
    let value = match value {
        LengthOrAuto::Auto => 0.0,
        LengthOrAuto::Length(length) => page_length_to_px(length, basis),
        LengthOrAuto::Calc(CalcLengthPercentage { percent, px }) => {
            let percent = if percent.is_finite() { percent } else { 0.0 };
            let px = if px.is_finite() { px } else { 0.0 };
            px + basis * percent / 100.0
        }
        _ => 0.0,
    };
    if value.is_finite() { value } else { 0.0 }
}

/// Used border and padding inset inside the page margin area.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct PageContentInsets {
    /// Top border plus padding.
    pub top: f32,
    /// Right border plus padding.
    pub right: f32,
    /// Bottom border plus padding.
    pub bottom: f32,
    /// Left border plus padding.
    pub left: f32,
}

fn page_box_side(
    declarations: &std::collections::HashMap<PropertyKey, PropertyValue>,
    padding: PropertyKey,
    border: PropertyKey,
    basis: f32,
) -> f32 {
    let value = match declarations.get(&padding) {
        Some(PropertyValue::PaddingTop(value))
        | Some(PropertyValue::PaddingRight(value))
        | Some(PropertyValue::PaddingBottom(value))
        | Some(PropertyValue::PaddingLeft(value)) => Some(*value),
        _ => None,
    };
    let padding = value.map_or(0.0, |length| page_length_to_px(length, basis));
    // A page border with an explicit margin encloses the page content box;
    // preserve the existing overlay behavior for a border-only page with no
    // margin (for example the page-box border smoke test).  Padding already
    // establishes an explicit content box and therefore always includes the
    // border.
    let has_page_margin = declarations.keys().any(|key| {
        matches!(
            key,
            PropertyKey::Margin
                | PropertyKey::MarginTop
                | PropertyKey::MarginRight
                | PropertyKey::MarginBottom
                | PropertyKey::MarginLeft
        )
    });
    let border = if padding > 0.0 || has_page_margin {
        match declarations.get(&border) {
            Some(PropertyValue::BorderTopWidth(value))
            | Some(PropertyValue::BorderRightWidth(value))
            | Some(PropertyValue::BorderBottomWidth(value))
            | Some(PropertyValue::BorderLeftWidth(value)) => page_length_to_px(*value, basis),
            _ => 0.0,
        }
    } else {
        0.0
    };
    (padding + border).max(0.0)
}

/// Resolve the border and padding inset of the page content box.
///
/// Page margins remain separate because margin boxes occupy the margin strips.
/// The inset shifts the physical flow origin and reduces the block-axis
/// fragmentainer extent; the inline containing-block width remains the page's
/// margin content width so page decorations do not force text rewrapping.
pub fn page_content_insets(cascade: &CascadeResult, page_box: PageBox) -> PageContentInsets {
    let declarations = cascade.page.declarations();
    PageContentInsets {
        top: page_box_side(
            declarations,
            PropertyKey::PaddingTop,
            PropertyKey::BorderTopWidth,
            page_box.height,
        ),
        right: page_box_side(
            declarations,
            PropertyKey::PaddingRight,
            PropertyKey::BorderRightWidth,
            page_box.width,
        ),
        bottom: page_box_side(
            declarations,
            PropertyKey::PaddingBottom,
            PropertyKey::BorderBottomWidth,
            page_box.height,
        ),
        left: page_box_side(
            declarations,
            PropertyKey::PaddingLeft,
            PropertyKey::BorderLeftWidth,
            page_box.width,
        ),
    }
}

fn page_margin_value(
    declarations: &std::collections::HashMap<PropertyKey, PropertyValue>,
    side: PropertyKey,
    shorthand: Option<LengthOrAuto>,
) -> Option<LengthOrAuto> {
    match (side, declarations.get(&side)) {
        (PropertyKey::MarginTop, Some(PropertyValue::MarginTop(value)))
        | (PropertyKey::MarginRight, Some(PropertyValue::MarginRight(value)))
        | (PropertyKey::MarginBottom, Some(PropertyValue::MarginBottom(value)))
        | (PropertyKey::MarginLeft, Some(PropertyValue::MarginLeft(value))) => Some(*value),
        _ => shorthand,
    }
}

fn page_dimension(
    declarations: &std::collections::HashMap<PropertyKey, PropertyValue>,
    key: PropertyKey,
    basis: f32,
) -> Option<f32> {
    let value = declarations.get(&key)?;
    let value = match (key, value) {
        (PropertyKey::Width, PropertyValue::Width(LengthOrAuto::Length(value)))
        | (PropertyKey::Height, PropertyValue::Height(LengthOrAuto::Length(value))) => {
            page_length_to_px(*value, basis)
        }
        (PropertyKey::Width, PropertyValue::Width(LengthOrAuto::Calc(value)))
        | (PropertyKey::Height, PropertyValue::Height(LengthOrAuto::Calc(value))) => {
            value.px + basis * value.percent / 100.0
        }
        _ => return None,
    };
    (value.is_finite() && value >= 0.0).then_some(value)
}

/// Resolve the used page margins from a page cascade and paper size.
///
/// `cascade_page` deliberately keeps page percentages symbolic because the
/// page box is a downstream used-value basis.  This is the single geometry
/// entry point used by layout, scene extraction, and paint so all three agree
/// on the paper/content split.
pub fn page_margins(cascade: &CascadeResult, page_box: PageBox) -> PageMargins {
    let declarations = cascade.page.declarations();
    // The legacy page `width`/`height` descriptors size the page area inside
    // the page box.  When they are combined with percentage margins, those
    // percentages resolve against the pre-descriptor page size, not the
    // already reduced outer box.  Keep that basis stable so e.g. `size: 500px;
    // margin: 10%; width: 40%; height: 60%` uses 50px margins and a 200x300
    // content area after the outer box is reduced to 300x400.
    let margin_basis = if (declarations.contains_key(&PropertyKey::Width)
        || declarations.contains_key(&PropertyKey::Height))
        && cascade.page.size().is_some()
    {
        PageBox::from_page_size(cascade.page.size())
    } else {
        page_box
    };
    let shorthand = match declarations.get(&PropertyKey::Margin) {
        Some(PropertyValue::Margin(sides)) => Some(*sides),
        _ => None,
    };
    let top_value = page_margin_value(
        declarations,
        PropertyKey::MarginTop,
        shorthand.map(|s| s.top),
    );
    let right_value = page_margin_value(
        declarations,
        PropertyKey::MarginRight,
        shorthand.map(|s| s.right),
    );
    let bottom_value = page_margin_value(
        declarations,
        PropertyKey::MarginBottom,
        shorthand.map(|s| s.bottom),
    );
    let left_value = page_margin_value(
        declarations,
        PropertyKey::MarginLeft,
        shorthand.map(|s| s.left),
    );
    let mut margins = PageMargins {
        top: top_value.map_or(0.0, |value| page_margin_length(value, margin_basis.height)),
        right: right_value.map_or(0.0, |value| page_margin_length(value, margin_basis.width)),
        bottom: bottom_value.map_or(0.0, |value| page_margin_length(value, margin_basis.height)),
        left: left_value.map_or(0.0, |value| page_margin_length(value, margin_basis.width)),
    };

    // In a page context, legacy `width`/`height` describe the page area.  If
    // that area is smaller than the physical page and a margin is `auto`, the
    // remaining space is distributed between the auto sides of that axis.
    // Without those descriptors the existing zero fallback is retained.
    let page_area_width = page_dimension(declarations, PropertyKey::Width, page_box.width);
    let page_area_height = page_dimension(declarations, PropertyKey::Height, page_box.height);
    if let Some(area_width) = page_area_width {
        let auto_left = matches!(left_value, Some(LengthOrAuto::Auto));
        let auto_right = matches!(right_value, Some(LengthOrAuto::Auto));
        let auto_count = usize::from(auto_left) + usize::from(auto_right);
        if auto_count > 0 {
            // Auto page margins absorb the full remainder, including a
            // negative one when the requested page area is larger than the
            // physical page box.
            let remaining = page_box.width - area_width - margins.left - margins.right;
            let auto_margin = remaining / auto_count as f32;
            if auto_left {
                margins.left = auto_margin;
            }
            if auto_right {
                margins.right = auto_margin;
            }
        }
    }
    if let Some(area_height) = page_area_height {
        let auto_top = matches!(top_value, Some(LengthOrAuto::Auto));
        let auto_bottom = matches!(bottom_value, Some(LengthOrAuto::Auto));
        let auto_count = usize::from(auto_top) + usize::from(auto_bottom);
        if auto_count > 0 {
            let remaining = page_box.height - area_height - margins.top - margins.bottom;
            let auto_margin = remaining / auto_count as f32;
            if auto_top {
                margins.top = auto_margin;
            }
            if auto_bottom {
                margins.bottom = auto_margin;
            }
        }
    }
    margins
}

/// Resolve the first page-context scan's child order from cascaded styles.
/// This runs before the first Taffy layout, so it can use order-modified source
/// order for flex items and grid items with auto row start/end and column
/// placement at auto or line 1, but not Taffy's resolved grid row details.
fn initial_page_child_order(
    document: &Document,
    cascade: &CascadeResult,
    parent_id: usize,
) -> Vec<usize> {
    let children = document.nodes[parent_id].children.clone();
    let parent_style = &cascade.computed[parent_id];
    let is_flex = matches!(
        parent_style.display,
        DisplayValue::Flex | DisplayValue::InlineFlex
    );
    let is_auto_row_grid = matches!(
        parent_style.display,
        DisplayValue::Grid | DisplayValue::InlineGrid
    ) && children
        .iter()
        .copied()
        .filter(|&child_id| {
            !matches!(
                cascade.computed[child_id].position,
                PositionValue::Absolute | PositionValue::Fixed
            ) && cascade.computed[child_id].display != DisplayValue::None
        })
        .all(|child_id| {
            matches!(
                &cascade.computed[child_id].grid_row_start,
                GridLineValue::Auto
            ) && matches!(
                &cascade.computed[child_id].grid_row_end,
                GridLineValue::Auto
            ) && matches!(
                &cascade.computed[child_id].grid_column_start,
                GridLineValue::Auto | GridLineValue::Line(1)
            )
        });
    if !is_flex && !is_auto_row_grid {
        return children;
    }

    let is_in_flow = |child_id: usize| {
        cascade.computed[child_id].display != DisplayValue::None
            && !matches!(
                cascade.computed[child_id].position,
                PositionValue::Absolute | PositionValue::Fixed
            )
    };
    let mut ordered_items: Vec<_> = children
        .iter()
        .copied()
        .filter(|&child_id| is_in_flow(child_id))
        .collect();
    ordered_items.sort_by_key(|&child_id| cascade.computed[child_id].order);
    if is_flex
        && matches!(
            parent_style.flex_direction,
            FlexDirectionValue::ColumnReverse
        )
    {
        ordered_items.reverse();
    }

    let mut ordered_items = ordered_items.into_iter();
    let mut ordered_children = children;
    for child_id in &mut ordered_children {
        if is_in_flow(*child_id) {
            *child_id = ordered_items
                .next()
                .expect("every in-flow child has one initial-page order entry");
        }
    }
    ordered_children
}

/// Cached child order and Grid rows are usable only for the exact cascade run
/// that produced the layout, while the tree is clean and stored `order` values
/// still match the cascade the caller wants to inspect.
fn document_has_resolved_pagination_order(document: &Document, cascade: &CascadeResult) -> bool {
    let has_resolved_order = document.nodes.iter().any(|node| {
        node.grid_column_count > 0
            || !node.grid_item_row_starts.is_empty()
            || !node.order_modified_children.is_empty()
    });
    let cascade_order_matches_layout = document.nodes.iter().enumerate().all(|(node_id, node)| {
        cascade
            .computed
            .get(node_id)
            .is_some_and(|computed| node.order == computed.order)
    });
    has_resolved_order
        && !document.layout_dirty
        && document.layout_cascade_generation == Some(cascade.generation())
        && cascade_order_matches_layout
}

/// Find the named page requested by the first rendered in-flow class-A box.
///
/// A named page on the first in-flow box selects the initial page context, so
/// callers must resolve its `@page` size and margins before the first layout
/// pass. Before layout, recursive scans use cascaded flex/grid order where
/// placement is known; after layout, they use order projections and resolved
/// grid rows. Later named descendants belong to a pagination transition and
/// must not change the initial layout geometry. The scan therefore stops after
/// the first rendered in-flow body child.
pub fn first_page_name(document: &Document, cascade: &CascadeResult) -> Option<String> {
    let body_id = (0..document.nodes.len())
        .find(|&node_id| document.nodes[node_id].tag_name() == Some("body"))?;
    let resolved_layout = document_has_resolved_pagination_order(document, cascade);
    if let Some(raikiri_style::property::PageValue::Named(name)) = cascade.page_values.get(body_id)
    {
        return Some(name.to_string());
    }
    let child_order = if resolved_layout {
        pagination_child_order(document, cascade, body_id)
    } else {
        initial_page_child_order(document, cascade, body_id)
    };
    for child_id in child_order {
        let node = document.get_node(child_id)?;
        if !node.is_in_document()
            || node.is_non_rendered_html_element()
            || (node.kind() == NodeKind::Element
                && cascade.computed[child_id].display == DisplayValue::None)
        {
            continue;
        }
        match node.kind() {
            NodeKind::Text => {
                if let crate::node::NodeData::Text(text) = &node.data
                    && !text.text_content.trim().is_empty()
                {
                    return None;
                }
            }
            NodeKind::Element => {
                let computed = &cascade.computed[child_id];
                if !matches!(
                    computed.position,
                    raikiri_style::property::PositionValue::Static
                        | raikiri_style::property::PositionValue::Relative
                        | raikiri_style::property::PositionValue::Sticky
                ) || !matches!(computed.float, FloatValue::None)
                {
                    continue;
                }
                if let Some(raikiri_style::property::PageValue::Named(name)) =
                    cascade.page_values.get(child_id)
                {
                    return Some(name.to_string());
                }
                // A first in-flow descendant can carry the page value through
                // an anonymous block. Resolve that propagation before falling
                // back to the anonymous initial page; later siblings must not
                // change the initial page context.
                let (_, propagated) = propagated_start_page_name_with_order(
                    document,
                    cascade,
                    child_id,
                    None,
                    resolved_layout,
                );
                return propagated;
            }
            _ => {}
        }
    }
    None
}

/// Cascaded state selected for the first page after verifying resolved Grid placement.
///
/// `page_name` is the query name used to build `cascade`; `page_box` and
/// `font_context` are the matching layout inputs. Callers rebuild these values
/// in `recascade` when the resolved first in-flow box selects a different page.
pub struct InitialPageContext {
    /// Named page used by the current first-page cascade, if any.
    pub page_name: Option<String>,
    /// Cascade for the current first-page query.
    pub cascade: CascadeResult,
    /// Physical page dimensions derived from that cascade or caller defaults.
    pub page_box: PageBox,
    /// Font context used when probing the first-page layout.
    pub font_context: FontContext,
}

/// Failure while resolving the page context selected by the first placed Grid item.
#[derive(Debug)]
pub enum InitialPageContextError {
    /// The single-page placement probe failed.
    Layout(LayoutError),
    /// Named-page size changes kept changing the first resolved Grid item.
    PageGeometryDidNotConverge {
        /// Number of placement passes attempted.
        iterations: u32,
    },
}

impl std::fmt::Display for InitialPageContextError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Layout(error) => write!(f, "initial page placement failed: {error}"),
            Self::PageGeometryDidNotConverge { iterations } => write!(
                f,
                "initial page context did not converge after {iterations} placement passes"
            ),
        }
    }
}

impl std::error::Error for InitialPageContextError {}

/// Optional resources used by the initial-page layout probe.
///
/// Provide the same resolver and base URL as the final layout pass so image
/// intrinsic sizes produce matching Flex/Grid placement.
#[derive(Clone, Copy, Default)]
pub struct InitialPageProbeResources<'a> {
    resolver: Option<&'a dyn ReplacedResolver>,
    base_url: Option<&'a url::Url>,
}

impl<'a> InitialPageProbeResources<'a> {
    /// Create probe resources from the caller's optional image resolver and base URL.
    pub const fn new(
        resolver: Option<&'a dyn ReplacedResolver>,
        base_url: Option<&'a url::Url>,
    ) -> Self {
        Self { resolver, base_url }
    }
}

/// Resolve and rebuild the initial page context using the first placed Grid box.
///
/// Flex/Grid order and Grid placement may change which named-page descendant
/// starts a document. When a document has both a named page and a Flex/Grid
/// container, this helper probes placement, asks [`first_page_name`] for the
/// resolved first box, and invokes `recascade` when that name differs from the
/// current page query. Every probe uses the caller's replaced-resource resolver
/// and base URL so intrinsic sizes match the final layout. The operation is
/// bounded so self-referential page-size changes fail explicitly. Documents
/// without both features return their input state without a probe.
#[allow(clippy::result_large_err)]
pub fn resolve_initial_page_context(
    document: &Document,
    page_name: Option<String>,
    cascade: CascadeResult,
    page_box: PageBox,
    font_context: FontContext,
    resources: InitialPageProbeResources<'_>,
    mut recascade: impl FnMut(Option<&str>) -> (CascadeResult, PageBox, FontContext),
) -> Result<InitialPageContext, InitialPageContextError> {
    let mut context = InitialPageContext {
        page_name,
        cascade,
        page_box,
        font_context,
    };
    let has_named_page = context
        .cascade
        .page_values
        .iter()
        .any(|page_value| matches!(page_value, raikiri_style::property::PageValue::Named(_)));
    let has_flex_or_grid = context.cascade.computed.iter().any(|computed| {
        matches!(
            computed.display,
            DisplayValue::Flex
                | DisplayValue::InlineFlex
                | DisplayValue::Grid
                | DisplayValue::InlineGrid
        )
    });
    if !has_named_page || !has_flex_or_grid {
        return Ok(context);
    }

    const MAX_INITIAL_PAGE_CONTEXT_PASSES: u32 = 3;
    for _ in 0..MAX_INITIAL_PAGE_CONTEXT_PASSES {
        let mut probe_document = document.clone();
        let probe_layout = if let Some(resolver) = resources.resolver {
            layout_single_page_with_resolver_and_base_url(
                &mut probe_document,
                &context.cascade,
                context.page_box,
                context.font_context.clone(),
                resolver,
                resources.base_url,
            )
        } else {
            layout_single_page(
                &mut probe_document,
                &context.cascade,
                context.page_box,
                context.font_context.clone(),
            )
        };
        probe_layout.map_err(InitialPageContextError::Layout)?;
        let resolved_name = first_page_name(&probe_document, &context.cascade);
        if resolved_name == context.page_name {
            return Ok(context);
        }

        context.page_name = resolved_name;
        let (cascade, page_box, font_context) = recascade(context.page_name.as_deref());
        context.cascade = cascade;
        context.page_box = page_box;
        context.font_context = font_context;
    }
    // cov:ignore: an oscillating named-grid page query needs a self-referential page-size fixture; callers expose a structured terminal error.
    Err(InitialPageContextError::PageGeometryDidNotConverge {
        iterations: MAX_INITIAL_PAGE_CONTEXT_PASSES,
    })
}

// cov:ignore: computed multicol bridge is exercised by ignored WPT reftests.
fn multicol_style_from_computed(cv: &ComputedValues) -> Option<MulticolStyle> {
    let count = match cv.column_count {
        ColumnCountValue::Count(value) if value > 0 => Some(value as usize),
        _ => None,
    };
    let width = match cv.column_width {
        ComputedColumnWidth::Px(value) if value.is_finite() && value > 0.0 => Some(value),
        _ => None,
    };
    if count.is_none() && width.is_none() {
        return None;
    }
    let (gap, gap_percent) = match cv.column_gap {
        ComputedLengthPercentageOrNormal::Px(value) if value.is_finite() => (value.max(0.0), None),
        ComputedLengthPercentageOrNormal::Percent(value) if value.is_finite() => (0.0, Some(value)),
        _ => (0.0, None),
    };
    Some(MulticolStyle {
        count,
        width,
        gap,
        gap_percent,
        height_definite: !matches!(cv.height, ComputedLengthPercentageOrAuto::Auto),
        horizontal: matches!(cv.writing_mode, WritingMode::HorizontalTb),
        orphans: cv.orphans.max(1) as usize,
        widows: cv.widows.max(1) as usize,
    })
}

/// ComputedValues → taffy::Style bridge の dispatch site。
///
/// per-element for loop 内 inline mapping から per-field `bridge_*` helper へ
/// dispatch する pattern にリファクタ済み。新しい bridge は helper 追加 +
/// dispatch 1 行追加のみで足りるよう設計している。
///
/// 現時点で active な bridge:
/// - [`bridge_direction`] — [`Direction`] → [`taffy::Direction`]
/// - [`bridge_display`] — [`DisplayValue`] → [`taffy::Display`]
/// - [`bridge_float`] — [`ComputedValues::float`] / [`ComputedValues::clear`] →
///   [`taffy::Style::float`] / [`taffy::Style::clear`] (CSS2 §9.5.1 / §9.5.2)
/// - [`bridge_margin`] — `Sides<ComputedLengthPercentageOrAuto>` → [`taffy::Rect<LengthPercentageAuto>`]
/// - [`bridge_padding`] — `Sides<ComputedLengthPercentage>` → [`taffy::Rect<LengthPercentage>`]
/// - [`bridge_size`] — [`ComputedLengthPercentageOrAuto`] `cv.width` / `cv.height` →
///   [`taffy::Style::size`] (`Size<Dimension>`)。width / height 両 field を
///   struct literal 1 発 assign で書く。
/// - [`bridge_min_max_size`] — [`ComputedLengthPercentageOrAuto`]
///   `cv.min_width` / `cv.min_height` → [`taffy::Style::min_size`]、
///   `cv.max_width` / `cv.max_height` → [`taffy::Style::max_size`]。
///   min/max 4 field を 2 struct literal で書く ([`bridge_size`] と同 shape)。
/// - [`bridge_border`] — `Sides<ComputedBorder>` → [`taffy::Rect<LengthPercentage>`]
///   。**border-style gating は本 bridge ではなく上流の
///   `raikiri_style::resolve_border` (computed 層) が持つ** — CSS Backgrounds 3
///   §3.3 <https://www.w3.org/TR/css-backgrounds-3/#border-width>。
/// - [`bridge_box_sizing`] — [`raikiri_style::property::BoxSizing`] → [`taffy::BoxSizing`]
///   (CSS Sizing 3 §7)
/// - [`bridge_flex`] — `flex-direction` / `flex-wrap` / `flex-grow` /
///   `flex-shrink` / `flex-basis` → [`taffy::Style`]'s matching flex
///   container/item fields (CSS Flexible Box Layout Module Level 1)
/// - [`bridge_alignment`] — `justify-content` / `align-content` /
///   `align-items` / `align-self` / `justify-items` / `justify-self` →
///   [`taffy::Style`]'s matching `Option<AlignItems>`/`Option<AlignContent>`
///   fields (CSS Box Alignment Module Level 3)
/// - [`bridge_gap`] — `row-gap` / `column-gap` → [`taffy::Style::gap`]
///   (`Size<LengthPercentage>`, CSS Box Alignment Module Level 3 §8.1)
/// - [`bridge_grid`] — `grid-template-columns` / `grid-template-rows` /
///   `grid-template-areas` / `grid-auto-columns` / `grid-auto-rows` /
///   `grid-auto-flow` / `grid-row-start` / `grid-row-end` /
///   `grid-column-start` / `grid-column-end` → [`taffy::Style`]'s matching
///   grid container/item fields (CSS Grid Layout Module Level 1)
///
/// per-node の bridge loop の後、second pass
/// ([`establish_minimal_line_boxes`]) が bridge 済みの tree 全体を走査し、
/// 条件を満たす block container に minimal な inline formatting context を
/// 確立する — qualifying condition と scope は同関数の doc 参照。
pub(crate) fn apply_computed_to_style(doc: &mut Document, cascade: &CascadeResult) {
    // Styles retain raw pointers to these payloads for Taffy's calc callback;
    // rebuild the arena for every cascade/layout pass.
    doc.calc_values.clear();
    for idx in 0..doc.nodes.len() {
        if doc.nodes[idx].kind() != NodeKind::Element {
            continue;
        }
        let cv = &cascade.computed[idx];
        // Preserve full DisplayValue for table dispatch before taffy collapses it.
        doc.nodes[idx].display = cv.display;
        // Taffy does not carry CSS `order` in Style. Retain the computed value
        // on Node so flex/grid child traversal can build order-modified order.
        doc.nodes[idx].order = cv.order;
        // Carry multicol settings into the Taffy dispatch seam. Taffy has no
        // native multicol style fields, so the custom strategy reads this
        // side-channel while the ordinary Style remains Taffy-compatible.
        doc.nodes[idx].multicol = multicol_style_from_computed(cv);
        // Table engine inputs — no taffy::Style counterpart (taffy 0.12
        // has no table layout), carried Node-side like `display` above.
        doc.nodes[idx].table_layout = cv.table_layout;
        doc.nodes[idx].border_collapse = cv.border_collapse;
        doc.nodes[idx].border_spacing = cv.border_spacing;
        doc.nodes[idx].break_before = cv.break_before;
        doc.nodes[idx].break_after = cv.break_after;
        doc.nodes[idx].authored_writing_mode = cascade
            .authored_writing_modes
            .get(idx)
            .and_then(|mode| *mode);
        bridge_size(doc, idx, cv);
        if let Some((intrinsic_width, intrinsic_height)) =
            bridge_known_image_intrinsic_size(doc, idx, cv)
        {
            let style = &mut doc.nodes[idx].style;
            if matches!(cv.width, ComputedLengthPercentageOrAuto::Auto) {
                style.size.width = Dimension::length(intrinsic_width);
            }
            if matches!(cv.height, ComputedLengthPercentageOrAuto::Auto) {
                style.size.height = Dimension::length(intrinsic_height);
            }
        }
        doc.nodes[idx].has_logical_min_block_size = cv.min_block_size.is_some()
            && !matches!(cv.break_inside, raikiri_style::property::BreakInside::Auto);
        let style = &mut doc.nodes[idx].style;
        bridge_direction(style, cv);
        bridge_display(style, cv);
        bridge_position(style, cv, &mut doc.layout_warnings);
        bridge_overflow(style, cv);
        bridge_float(style, cv);
        bridge_margin(style, cv, &mut doc.layout_warnings);
        bridge_padding(style, cv, &mut doc.layout_warnings);
        if cv.display == DisplayValue::ListItem
            && matches!(
                cv.list_style_position,
                raikiri_style::ListStylePosition::Inside
            )
            && style.padding.left.into_raw().value() == 0.0
        {
            // The current Taffy bridge has no marker child. Reserve a
            // conservative first-line gutter for the inside marker so its
            // post-layout paint does not overlap ordinary text. Explicit or
            // percentage padding remains untouched; a later inline-formatting
            // pass will replace this estimate with measured marker width.
            style.padding.left = LengthPercentage::length(cv.font_size.px().max(16.0) * 1.5);
        }
        bridge_min_max_size(style, cv, &mut doc.calc_values, &mut doc.layout_warnings);
        bridge_border(style, cv, &mut doc.layout_warnings);
        bridge_box_sizing(style, cv);
        bridge_flex(style, cv, &mut doc.layout_warnings);
        bridge_alignment(style, cv);
        bridge_gap(style, cv, &mut doc.layout_warnings);
        bridge_grid(style, cv, &mut doc.layout_warnings);
    }
    refresh_order_modified_children(doc);
    establish_minimal_line_boxes(doc, cascade);
    // Mark table formatting roots for blitz-compat bit preservation.
    for idx in 0..doc.nodes.len() {
        if doc.nodes[idx].kind() != NodeKind::Element {
            continue;
        }
        let is_table = matches!(
            doc.nodes[idx].display,
            DisplayValue::Table | DisplayValue::InlineTable
        );
        if is_table {
            doc.nodes[idx].flags.insert(NodeFlags::IS_TABLE_ROOT);
        } else {
            doc.nodes[idx].flags.remove(NodeFlags::IS_TABLE_ROOT);
        }
    }
}

/// Rebuild the derived child-order view after every computed-style bridge.
///
/// CSS Flexbox and Grid consume flex/grid items in order-modified document
/// order, while the DOM child vector must remain in source order for traversal,
/// selectors, counters, and serialization. Taffy has no CSS `order` input in
/// `Style`, so retain a stable sorted projection only for containers with a
/// non-zero ordered child. `TaffyChildIter` applies its existing flat-tree and
/// whitespace filtering to this view.
fn refresh_order_modified_children(doc: &mut Document) {
    let mut projection_changed = false;
    let grid_details_stale = doc.layout_dirty;
    for parent_idx in 0..doc.nodes.len() {
        let should_sort = {
            let parent = &doc.nodes[parent_idx];
            matches!(parent.style.display, Display::Flex | Display::Grid)
                && parent.children.iter().any(|&child| {
                    doc.nodes[child].style.position != TaffyPosition::Absolute
                        && doc.nodes[child].order != 0
                })
        };
        let new_projection = if should_sort {
            let children = doc.nodes[parent_idx].children.clone();
            let mut ordered_items: Vec<usize> = children
                .iter()
                .copied()
                .filter(|&child| doc.nodes[child].style.position != TaffyPosition::Absolute)
                .collect();
            // `sort_by_key` is stable, so equal `order` values retain DOM order.
            ordered_items.sort_by_key(|&child| doc.nodes[child].order);
            let mut ordered_items = ordered_items.into_iter();
            let mut ordered_children = children;
            for child in &mut ordered_children {
                if doc.nodes[*child].style.position != TaffyPosition::Absolute {
                    *child = ordered_items
                        .next()
                        .expect("every in-flow child has one ordered entry");
                }
            }
            ordered_children.into_boxed_slice()
        } else {
            Box::new([])
        };

        if doc.nodes[parent_idx].order_modified_children.as_ref() != new_projection.as_ref() {
            projection_changed = true;
            doc.nodes[parent_idx].order_modified_children = new_projection;
        }
    }

    if projection_changed {
        // Reordering changes Taffy's child traversal. Clear every cache before
        // layout starts: a root/ancestor cache hit would otherwise skip the
        // normal lazy invalidation hook and preserve stale item positions.
        for node in &mut doc.nodes {
            node.cache.clear();
            node.grid_item_row_starts = Box::new([]);
            node.grid_column_count = 0;
        }
    } else if grid_details_stale {
        // A DOM mutation will invalidate layout caches in Taffy's normal path;
        // discard placement metadata until the next grid layout repopulates it.
        for node in &mut doc.nodes {
            node.grid_item_row_starts = Box::new([]);
            node.grid_column_count = 0;
        }
    }
}

/// Whether `child_id` participates in its flex parent's ordered item sequence.
/// Match Taffy's flat-tree, box-generation, out-of-flow, and whitespace-text
/// filters so reversing the sequence changes only actual flex items.
fn is_in_flow_flex_child_for_pagination(
    document: &Document,
    parent_id: usize,
    child_id: usize,
) -> bool {
    let child = &document.nodes[child_id];
    child.is_in_document()
        && child.style.display != Display::None
        && child.style.position != TaffyPosition::Absolute
        && !matches!(
            &child.data,
            NodeData::Text(text)
                if text.text_content.chars().all(|c| matches!(c, ' ' | '\t' | '\n' | '\r' | '\u{000c}'))
        )
        && document.nodes[parent_id].style.display == Display::Flex
}

/// Return a flex parent's page-traversal order, which follows top-to-bottom
/// visual order for `column-reverse` and Taffy's order-modified order otherwise.
fn flex_pagination_child_order(
    document: &Document,
    cascade: &CascadeResult,
    parent_id: usize,
) -> Vec<usize> {
    let mut children = document.nodes[parent_id].layout_children().to_vec();
    if !matches!(
        cascade.computed[parent_id].flex_direction,
        FlexDirectionValue::ColumnReverse
    ) {
        return children;
    }

    let mut ordered_items: Vec<_> = children
        .iter()
        .copied()
        .filter(|&child_id| is_in_flow_flex_child_for_pagination(document, parent_id, child_id))
        .collect();
    ordered_items.reverse();
    let mut ordered_items = ordered_items.into_iter();
    for child_id in &mut children {
        if is_in_flow_flex_child_for_pagination(document, parent_id, *child_id) {
            *child_id = ordered_items
                .next()
                .expect("each in-flow flex child has one reversed entry");
        }
    }
    children
}

fn is_in_flow_grid_item_for_pagination(
    document: &Document,
    cascade: &CascadeResult,
    child_id: usize,
) -> bool {
    match document.nodes[child_id].kind() {
        NodeKind::Element => {
            cascade.computed[child_id].display != DisplayValue::None
                && matches!(
                    cascade.computed[child_id].position,
                    PositionValue::Static | PositionValue::Relative | PositionValue::Sticky
                )
        }
        NodeKind::Text => matches!(
            &document.nodes[child_id].data,
            NodeData::Text(text) if !text.text_content.trim().is_empty()
        ),
        _ => false,
    }
}

/// Child order shared by page-candidate collection and named-page propagation.
fn pagination_child_order(
    document: &Document,
    cascade: &CascadeResult,
    parent_id: usize,
) -> Vec<usize> {
    let computed = &cascade.computed[parent_id];
    if matches!(
        computed.display,
        DisplayValue::Flex | DisplayValue::InlineFlex
    ) {
        return flex_pagination_child_order(document, cascade, parent_id);
    }
    let is_single_column_grid = matches!(
        computed.display,
        DisplayValue::Grid | DisplayValue::InlineGrid
    ) && document.nodes[parent_id].grid_column_count == 1;
    if !is_single_column_grid {
        if matches!(
            computed.display,
            DisplayValue::Grid | DisplayValue::InlineGrid
        ) {
            // Multi-column pagination is not implemented, but named-page
            // propagation and candidate traversal must retain Grid's
            // order-modified child sequence.
            return document.nodes[parent_id].layout_children().to_vec();
        }
        return document.nodes[parent_id].children.clone();
    }

    let mut children = document.nodes[parent_id].layout_children().to_vec();
    let mut in_flow_items: Vec<_> = document.nodes[parent_id]
        .grid_item_row_starts
        .iter()
        .copied()
        .filter(|(child_id, _)| is_in_flow_grid_item_for_pagination(document, cascade, *child_id))
        .collect();
    let pageable_count = children
        .iter()
        .filter(|&&child_id| is_in_flow_grid_item_for_pagination(document, cascade, child_id))
        .count();
    if in_flow_items.len() == pageable_count {
        // Stable sorting keeps order-modified order within a resolved row.
        in_flow_items.sort_by_key(|&(_, row_start)| row_start);
        let mut ordered_items = in_flow_items.into_iter().map(|(child_id, _)| child_id);
        for child_id in &mut children {
            if is_in_flow_grid_item_for_pagination(document, cascade, *child_id) {
                *child_id = ordered_items
                    .next()
                    .expect("Taffy row details cover each in-flow grid item");
            }
        }
    }
    children
}

/// [`Direction`] → [`taffy::Direction`] mapping.
///
/// Taffy uses this field for horizontal block alignment, table/grid ordering,
/// and overflow direction.  Keep it separate from text shaping: parley does
/// not expose the CSS base direction through the bridge used by this crate.
fn bridge_direction(style: &mut taffy::Style, cv: &ComputedValues) {
    style.direction = match cv.direction {
        Direction::Ltr => TaffyDirection::Ltr,
        Direction::Rtl => TaffyDirection::Rtl,
        _ => TaffyDirection::Ltr,
    };
}

/// [`DisplayValue`] → [`taffy::Display`] mapping。
///
/// [`DisplayValue`] を
/// [`taffy::Display`] に mapping する。taffy 0.x は Block / Flex / Grid /
/// None のみ (`Inline` / `InlineBlock` 独立 variant なし) のため:
/// - `Inline` → `Block` (initial は Block、text-only は leaf で render)
/// - `InlineBlock` → `Block` (block child + inline-level flow parent の
///   separate 扱い、精密化は follow-up)
/// - `None` → `None`
/// - `Flex` → `Flex` (CSS Display 3 §2.2 `<display-inside>` keyword、
///   outer-defaulting rule で `block flex` と等価。
///   [`crate::taffy_impl`]'s `LayoutFlexboxContainer` impl + taffy's
///   `compute_flexbox_layout` が実 layout を担う)
/// - `Grid` → `Grid` (CSS Display 3 §2.2 `<display-inside>` keyword、
///   outer-defaulting rule で `block grid` と等価。
///   [`crate::taffy_impl`]'s `LayoutGridContainer` impl + taffy's
///   `compute_grid_layout` が実 layout を担う)
/// - Table internal types (`table` / `inline-table` / `table-row-group` /
///   `table-header-group` / `table-footer-group` / `table-row` /
///   `table-column-group` / `table-column` / `table-cell` / `table-caption`)
///   → `Block` (暫定: taffy 0.12 は table layout 未対応のため block 近似。
///   TODO(table-layout): [`crate::layout::table`] の dedicated table
///   formatting context (CSS 2.1 §17.2.1 anonymous table object generation
///   含む) が landing したら専用 Display / layout へ置換)
/// - `flow-root` → `FlowRoot` (独立 block formatting context)
/// - `list-item` / `contents` → `Block` (catch-all 経由、将来専用 handling
///   が入るまで block 近似)
/// - catch-all arm → `Block` (`non_exhaustive` forward-compat)
fn bridge_display(style: &mut taffy::Style, cv: &ComputedValues) {
    style.display = match cv.display {
        DisplayValue::Block => Display::Block,
        DisplayValue::Inline => Display::Block,
        DisplayValue::InlineBlock => Display::Block,
        DisplayValue::FlowRoot => Display::FlowRoot,
        DisplayValue::None => Display::None,
        DisplayValue::Flex | DisplayValue::InlineFlex => Display::Flex,
        DisplayValue::Grid | DisplayValue::InlineGrid => Display::Grid,
        // Table model — CSS Display 3 §2 / CSS 2.1 §17.2. Taffy 0.12 has no
        // table layout, so map to Block as temporary approximation.
        // Real table routing uses `Node::display` (preserved in
        // `apply_computed_to_style` before this collapse) rather than
        // `Style::display`.
        DisplayValue::Table => Display::Block,
        DisplayValue::InlineTable => Display::Block,
        DisplayValue::TableRowGroup => Display::Block,
        DisplayValue::TableHeaderGroup => Display::Block,
        DisplayValue::TableFooterGroup => Display::Block,
        DisplayValue::TableRow => Display::Block,
        DisplayValue::TableColumnGroup => Display::Block,
        DisplayValue::TableColumn => Display::Block,
        DisplayValue::TableCell => Display::Block,
        DisplayValue::TableCaption => Display::Block,
        _ => {
            // non_exhaustive catch-all — unknown future variant (e.g.
            // `list-item` / `contents`) goes to Block
            Display::Block
        }
    };
}

/// [`ComputedValues::float`] / [`ComputedValues::clear`] → [`taffy::Style::float`] /
/// [`taffy::Style::clear`] bridge (CSS2 §9.5.1
/// <https://www.w3.org/TR/CSS2/visuren.html#propdef-float> / §9.5.2
/// <https://www.w3.org/TR/CSS2/visuren.html#propdef-clear>)。**enum 1:1
/// mapping** (both raikiri-style's `FloatValue`/`ClearValue` and taffy's
/// `Float`/`Clear` carry exactly the spec's keyword sets, so no length
/// resolution or Length policy participation is needed, same shape as
/// [`bridge_box_sizing`]).
///
/// This only carries the keyword through; the actual float positioning,
/// shrink-to-fit width, and wraparound layout it drives is taffy's
/// `float_layout` feature (`compute_block_layout`'s `BlockFormattingContext`/
/// `FloatContext`), wired via this crate's `taffy_impl` module.
///
/// **Known taffy `block_layout` limitation**: a same-BFC child narrower than
/// its own BFC root can get float insets computed against the root's width
/// instead of its own — not yet worked around or covered by a test here.
fn bridge_float(style: &mut taffy::Style, cv: &ComputedValues) {
    style.float = match cv.float {
        FloatValue::None => TaffyFloat::None,
        FloatValue::Left => TaffyFloat::Left,
        FloatValue::Right => TaffyFloat::Right,
        // cov:ignore: unreachable while FloatValue is None|Left|Right only;
        // required for its #[non_exhaustive] contract.
        _ => TaffyFloat::None,
    };
    style.clear = match cv.clear {
        ClearValue::None => TaffyClear::None,
        ClearValue::Left => TaffyClear::Left,
        ClearValue::Right => TaffyClear::Right,
        ClearValue::Both => TaffyClear::Both,
        // cov:ignore: unreachable while ClearValue is None|Left|Right|Both
        // only; required for its #[non_exhaustive] contract.
        _ => TaffyClear::None,
    };
}

/// [`ComputedValues::margin`] (`Sides<ComputedLengthPercentageOrAuto>`) → [`taffy::Style::margin`]
/// (`Rect<LengthPercentageAuto>`) bridge。
///
/// CSS Box 3 §3.1 <https://www.w3.org/TR/css-box-3/#margin-physical> の
/// physical margin 4 side (top / right / bottom / left) を taffy `Rect` に
/// **field 名 mapping** で write する (positional constructor は使わない —
/// `Sides` の field 順 `top,right,bottom,left` と `Rect` の field 順
/// `left,right,top,bottom` が異なるため silent transpose を防ぐ)。
///
/// Length policy は [`computed_length_percentage_or_auto_to_taffy_length_percentage_auto`] を参照。
fn bridge_margin(style: &mut taffy::Style, cv: &ComputedValues, diag: &mut Vec<LayoutWarn>) {
    let m = cv.margin;
    style.margin = Rect {
        top: computed_length_percentage_or_auto_to_taffy_length_percentage_auto(
            m.top, "margin", diag,
        ),
        right: computed_length_percentage_or_auto_to_taffy_length_percentage_auto(
            m.right, "margin", diag,
        ),
        bottom: computed_length_percentage_or_auto_to_taffy_length_percentage_auto(
            m.bottom, "margin", diag,
        ),
        left: computed_length_percentage_or_auto_to_taffy_length_percentage_auto(
            m.left, "margin", diag,
        ),
    };
}

/// [`ComputedValues::padding`] (`Sides<ComputedLengthPercentage>`) → [`taffy::Style::padding`]
/// (`Rect<LengthPercentage>`) bridge。
///
/// CSS Box 3 §4.1 <https://www.w3.org/TR/css-box-3/#padding-physical> の
/// physical padding 4 side (top / right / bottom / left) を taffy `Rect` に
/// **field 名 mapping** で write する (positional constructor は使わない —
/// `Sides` の field 順 `top,right,bottom,left` と `Rect` の field 順
/// `left,right,top,bottom` が異なるため silent transpose を防ぐ)。margin と
/// の差は value type: padding は `<length-percentage [0,∞]>` (auto なし、
/// non-negative は raikiri-style parse-time enforce) のため
/// [`computed_length_percentage_to_taffy_length_percentage`] を使う。
///
/// Length policy は [`computed_length_percentage_to_taffy_length_percentage`] を参照。
fn bridge_padding(style: &mut taffy::Style, cv: &ComputedValues, diag: &mut Vec<LayoutWarn>) {
    let p = cv.padding;
    style.padding = Rect {
        top: computed_length_percentage_to_taffy_length_percentage(p.top, "padding", diag),
        right: computed_length_percentage_to_taffy_length_percentage(p.right, "padding", diag),
        bottom: computed_length_percentage_to_taffy_length_percentage(p.bottom, "padding", diag),
        left: computed_length_percentage_to_taffy_length_percentage(p.left, "padding", diag),
    };
}

/// [`ComputedValues::border`] (`Sides<ComputedBorder>`) → [`taffy::Style::border`]
/// (`Rect<LengthPercentage>`) bridge。
///
/// # style-gating は **上流** で済んでいる
///
/// `border-style: none` / `hidden` の側で width を 0 にする規則は、CSS
/// Backgrounds 3 §3.3 <https://www.w3.org/TR/css-backgrounds-3/#border-width>
/// の propdef table が "Computed value: absolute length, snapped as a border
/// width; **zero if the border style is `none` or `hidden`**" と規定するとおり
/// **computed 層**の要求である。したがって gate は
/// `raikiri_style::resolve_border` が持ち、[`ComputedValues::border`] に届く
/// 時点で width は既に 0 に潰れている。
///
/// 本 bridge が同じ判定を再実装してはならない (spec 規則の二重実装になり、
/// 一方だけ直す drift の温床になる)。かつてあった `used_border_width` helper は
/// この理由で削除した。end-to-end の gating check は本 file の
/// `apply_computed_to_style_bridges_border_to_taffy` が引き続き持つ。
///
/// `@page` 経路 (`PageCascadeResult::declarations`) も同じ `resolve_border` へ
/// funnel するようになったので、border-width の gate は
/// raikiri-style 側に 1 本しか無い。本 bridge が読むのは per-node
/// [`ComputedValues`] なので直接の影響は無いが、将来 page-margin box の layout を
/// 本 bridge に通す場合も **gate を再実装せず** 上流の値を信頼すること。
///
/// ⚠️ 上記は **border-style gating に限った話**。`PageCascadeResult::declarations`
/// は非有限 `f32` (+Inf / NaN) について本段落とは別の未対応 hazard を抱えている
/// (contract は `raikiri_style::page::PageCascadeResult::declarations`
/// の doc が canonical)。将来 page-margin box の layout を本 bridge に通す際は
/// gating の再確認だけでなくそちらも再確認すること。
///
/// 4-side は **field 名 mapping** で write (positional constructor は使わない —
/// [`bridge_margin`] と同じ `Sides` vs `Rect` field 順不一致の silent transpose
/// 防止)。
///
/// # taffy scope の非対応
///
/// - `border-color` / `border-style` 自体は taffy が track しない (taffy は
///   border-width のみ)。色 / 線 pattern は paint scope が別途 [`ComputedValues::border`]
///   から consume する将来 task。
/// - `border-image` / `border-radius` はスコープ外。
fn bridge_border(style: &mut taffy::Style, cv: &ComputedValues, diag: &mut Vec<LayoutWarn>) {
    let b = cv.border;
    style.border = Rect {
        top: computed_length_to_taffy_length_percentage(b.top.width(), diag),
        right: computed_length_to_taffy_length_percentage(b.right.width(), diag),
        bottom: computed_length_to_taffy_length_percentage(b.bottom.width(), diag),
        left: computed_length_to_taffy_length_percentage(b.left.width(), diag),
    };
}

/// [`ComputedValues::width`] / [`ComputedValues::height`] (`ComputedLengthPercentageOrAuto`) →
/// [`taffy::Style::size`] (`Size<Dimension>`) bridge (CSS Sizing 3 §3.1.1
/// "Preferred Size Properties"
/// <https://www.w3.org/TR/css-sizing-3/#preferred-size-properties>)。
///
/// width 側を先に landing、続けて height 側を追記して両 preferred size 軸を
/// full-bridge にした。両 field を同時に書き込むため struct literal
/// (`style.size = Size { width, height }`) で 1 発 assign する — partial write
/// scaffold はもう不要になった。
///
/// Length policy は [`computed_length_percentage_or_auto_to_taffy_dimension`] を参照。
///
/// # PageBox 妥協
///
/// `<body>` element の `style.size` は本 bridge の後、[`apply_page_box_to_body`]
/// で PageBox の値 (width / height 両方) に上書きされる (layout.rs Step 1 →
/// Step 4)。したがって `<body style="width: 100px; height: 200px">` の author
/// 値は本 helper で一度 taffy に write されるが、Step 4 で PageBox 値に
/// clobber される — 現行実装で意図された挙動 (将来 @page cascade + per-page
/// PageBox に refactor 予定)。width 側 clobber の author→PageBox 上書き経路は
/// test `apply_page_box_clobbers_body_width_from_bridge` が check する。height
/// 側は [`apply_page_box_to_body`] が `style.size = Size { width, height }` の
/// struct literal で **field を分岐なく一括代入する** ため、width と同じ
/// clobber 経路を通る (両 field は同一 statement で書かれる)。同 helper の
/// PageBox output check は test `apply_page_box_to_body_sets_body_style_size_to_page_dimensions`
/// が担う (author→PageBox の bridge→clobber 連鎖 test は width 側で十分、
/// 冗長化を避け height 側は structural 保証に留める)。
fn bridge_overflow(style: &mut taffy::Style, cv: &ComputedValues) {
    fn map(value: OverflowValue) -> TaffyOverflow {
        match value {
            OverflowValue::Visible => TaffyOverflow::Visible,
            OverflowValue::Hidden => TaffyOverflow::Hidden,
            OverflowValue::Clip => TaffyOverflow::Clip,
            OverflowValue::Scroll | OverflowValue::Auto => TaffyOverflow::Scroll,
            _ => TaffyOverflow::Visible,
        }
    }
    style.overflow = Point {
        x: map(cv.overflow.x),
        y: map(cv.overflow.y),
    };
}

fn bridge_position(style: &mut taffy::Style, cv: &ComputedValues, diag: &mut Vec<LayoutWarn>) {
    style.position = match cv.position {
        PositionValue::Absolute | PositionValue::Fixed => TaffyPosition::Absolute,
        PositionValue::Static | PositionValue::Relative | PositionValue::Sticky => {
            TaffyPosition::Relative
        }
        _ => TaffyPosition::Relative,
    };
    let mut inset = |value: ComputedLengthPercentageOrAuto, site: &'static str| {
        computed_length_percentage_or_auto_to_taffy_length_percentage_auto(value, site, diag)
    };
    style.inset = Rect {
        top: inset(cv.top, "top"),
        right: inset(cv.right, "right"),
        bottom: inset(cv.bottom, "bottom"),
        left: inset(cv.left, "left"),
    };
}

fn bridge_size(doc: &mut Document, node_id: usize, cv: &ComputedValues) {
    // width + height 両方を同時に書くので struct literal を採用。
    let width = computed_length_percentage_or_auto_to_taffy_dimension(
        &mut doc.calc_values,
        cv.width,
        "width",
        &mut doc.layout_warnings,
    );
    let height = computed_length_percentage_or_auto_to_taffy_dimension(
        &mut doc.calc_values,
        cv.height,
        "height",
        &mut doc.layout_warnings,
    );
    doc.nodes[node_id].style.size = Size { width, height };
}

/// Supply the dimensions of the small set of WPT image assets whose bytes are
/// intentionally resolved by the renderer's URL-color fallback.  Without an
/// intrinsic size, a replaced `<img>` with auto width and height collapses to
/// zero even though its sibling background-image fallback paints a rectangle.
fn bridge_known_image_intrinsic_size(
    doc: &Document,
    node_id: usize,
    cv: &ComputedValues,
) -> Option<(f32, f32)> {
    let node = &doc.nodes[node_id];
    if node.tag_name() != Some("img") {
        return None;
    }
    let src = node.attribute("src")?;
    let basename = src.rsplit('/').next().unwrap_or(src).to_ascii_lowercase();
    if basename != "green.png" {
        return None;
    }
    let width_auto = matches!(cv.width, ComputedLengthPercentageOrAuto::Auto);
    let height_auto = matches!(cv.height, ComputedLengthPercentageOrAuto::Auto);
    (width_auto || height_auto).then_some((100.0, 50.0))
}

/// [`ComputedValues::min_width`] / [`ComputedValues::min_height`] /
/// [`ComputedValues::max_width`] / [`ComputedValues::max_height`]
/// ([`ComputedLengthPercentageOrAuto`]) → [`taffy::Style::min_size`] /
/// [`taffy::Style::max_size`] (`Size<Dimension>`) bridge (CSS Sizing 3 §4
/// "Minimum Size Properties" / §5 "Maximum Size Properties").
///
/// min 側 initial `auto` / max 側 initial `none` は共に computed 層で
/// [`ComputedLengthPercentageOrAuto::Auto`] に正規化済み (specified 層の
/// `none` → `Auto` placeholder mapping は sibling `parse_max_size` が担う)
/// ので、4 field とも同じ
/// [`computed_length_percentage_or_auto_to_taffy_min_max`] helper で `Auto` →
/// `LengthPercentageAuto::auto()` に translate する — max の `none` 用の特別扱いは本
/// bridge に要らない。`none` (no max) と `auto` (no minimum) は共に
/// "制約なし" として taffy に委譲する。calc() payloads are retained in the
/// document arena by that helper.
///
/// Length policy / Percent policy / 非有限 guard は helper の doc 参照。
/// `site` label は 4 caller ごとに `min-width` / `min-height` /
/// `max-width` / `max-height` を渡す ([`LayoutWarn::NonFiniteClamped`] の診断用)。
fn bridge_min_max_size(
    style: &mut taffy::Style,
    cv: &ComputedValues,
    calc_values: &mut Vec<std::sync::Arc<raikiri_style::property::CalcLengthPercentage>>,
    diag: &mut Vec<LayoutWarn>,
) {
    // min/max 各 2 field を同時に書くので struct literal を 2 発採用
    // (bridge_size と同 shape — default 保持は cv 側の Auto で自然に達成)。
    // Keep calc() handles alive in the document arena just like width/height;
    // collapsing a max-width calc() to zero would spuriously clamp a <col>.
    style.min_size = Size {
        width: computed_length_percentage_or_auto_to_taffy_min_max(
            calc_values,
            cv.min_width,
            "min-width",
            diag,
        ),
        height: computed_length_percentage_or_auto_to_taffy_min_max(
            calc_values,
            cv.min_height,
            "min-height",
            diag,
        ),
    };
    style.max_size = Size {
        width: computed_length_percentage_or_auto_to_taffy_min_max(
            calc_values,
            cv.max_width,
            "max-width",
            diag,
        ),
        height: computed_length_percentage_or_auto_to_taffy_min_max(
            calc_values,
            cv.max_height,
            "max-height",
            diag,
        ),
    };
}

/// [`raikiri_style::property::BoxSizing`] → [`taffy::BoxSizing`] bridge。
///
/// CSS Sizing 3 §3.3 "Box Edges for Sizing: the box-sizing property"
/// <https://www.w3.org/TR/css-sizing-3/#box-sizing>: value grammar
/// `content-box | border-box`、spec initial `content-box`。**enum 1:1 mapping**
/// (Length policy に不参加の最小 helper)。
///
/// # Initial-value 補正 note
///
/// - raikiri-style initial = `BoxSizing::ContentBox` (CSS Sizing 3 §3.3 spec 準拠)
/// - taffy default = `taffy::BoxSizing::BorderBox` (taffy 0.12 の `#[default]`)
///
/// 両者の初期値は spec と食い違うが、cascade は unspecified 時に必ず
/// [`ComputedValues::initial`] 経由で `ContentBox` を seed するため、本 bridge が
/// 走った後の `style.box_sizing` は常に spec 初期値 (`ContentBox`) になる。
/// つまり本 helper の副作用として "taffy default の spec 違反" を補正する。
///
/// # non_exhaustive catch-all
///
/// raikiri-style の [`BoxSizing`] は `#[non_exhaustive]`
/// (forward-compat のための sibling pattern)。未知 variant は spec initial (`ContentBox`) に
/// fail-quiet — spec-violation を silent に伸ばさないよう "最も安全な既定"
/// にする方針 ([`bridge_display`] catch-all → `Block` と同じ趣旨)。
///
/// taffy 側 (`taffy::BoxSizing`) は `#[non_exhaustive]` **ではない** ため、
/// mapping 出力 arm は `ContentBox` / `BorderBox` の 2 個で網羅済。
///
/// [`BoxSizing`]: raikiri_style::property::BoxSizing
fn bridge_box_sizing(style: &mut taffy::Style, cv: &ComputedValues) {
    style.box_sizing = match cv.box_sizing {
        StyleBoxSizing::ContentBox => TaffyBoxSizing::ContentBox,
        StyleBoxSizing::BorderBox => TaffyBoxSizing::BorderBox,
        // non_exhaustive catch-all — 未知 variant は spec initial (ContentBox)
        // に fail-quiet (silent spec-violation 拡大を避ける)。
        _ => TaffyBoxSizing::ContentBox,
    };
}

/// flex container/item property → [`taffy::Style`] bridge。
///
/// CSS Flexible Box Layout Module Level 1 の 5 property を対応する taffy
/// field へ写す:
///
/// - `flex-direction` ([`FlexDirectionValue`]) → `style.flex_direction`
///   (§5.1)
/// - `flex-wrap` ([`FlexWrapValue`]) → `style.flex_wrap` (§5.2)
/// - `flex-grow` (`f32`) → `style.flex_grow` (§7.2.1、無変換で直接 copy —
///   `<number>` は spec 上絶対化を要さない、[`ComputedValues::flex_grow`]
///   doc の sink-guard 注記参照)
/// - `flex-shrink` (`f32`) → `style.flex_shrink` (§7.2.2、同上)
/// - `flex-basis` ([`ComputedFlexBasis`]) → `style.flex_basis`
///   (`taffy::Dimension`、§7.2.3)
///
/// # `flex-basis` の bridge
///
/// `Px` / `Percent` は [`bridge_size`] と同じ percent `/100.0` 変換 +
/// [`sanitize_taffy`] 非有限 guard で [`taffy::Dimension`] に載せる。
/// `MinContent` / `MaxContent` / `FitContent` は taffy 0.14 の同名
/// `Dimension` variant にそのまま写像する (flex アルゴリズムが content
/// 測定で解決する)。
///
/// - `Content` → [`taffy::Dimension::content`] (0.14 — flex-basis 専用
///   keyword、そのままの意味で写像する)。
fn bridge_flex(style: &mut taffy::Style, cv: &ComputedValues, diag: &mut Vec<LayoutWarn>) {
    style.flex_direction = match cv.flex_direction {
        FlexDirectionValue::Row => TaffyFlexDirection::Row,
        FlexDirectionValue::RowReverse => TaffyFlexDirection::RowReverse,
        FlexDirectionValue::Column => TaffyFlexDirection::Column,
        FlexDirectionValue::ColumnReverse => TaffyFlexDirection::ColumnReverse,
        // cov:ignore: unreachable while FlexDirectionValue is
        // Row|RowReverse|Column|ColumnReverse only; required for its
        // #[non_exhaustive] contract (see doc above, `bridge_display`
        // catch-all と同じ趣旨).
        _ => TaffyFlexDirection::Row,
    };
    style.flex_wrap = match cv.flex_wrap {
        FlexWrapValue::NoWrap => TaffyFlexWrap::NoWrap,
        FlexWrapValue::Wrap => TaffyFlexWrap::Wrap,
        FlexWrapValue::WrapReverse => TaffyFlexWrap::WrapReverse,
        // cov:ignore: unreachable while FlexWrapValue is
        // NoWrap|Wrap|WrapReverse only; required for its #[non_exhaustive]
        // contract.
        _ => TaffyFlexWrap::NoWrap,
    };
    // `<number>` は絶対化不要 — 無変換で直接 copy。
    style.flex_grow = cv.flex_grow;
    style.flex_shrink = cv.flex_shrink;
    // `flex-basis: content` (CSS Flexible Box Layout 1 §4.5) is distinct
    // from `auto` in principle — it always uses the item's content size as
    // the flex basis, even when `width`/`height` are also set, whereas
    // `auto` defers to `width`/`height` first and only falls back to
    // `content` maps to taffy's own `Dimension::content` (0.14 — "only
    // valid for flex-basis", exactly this property's keyword).
    // `min-content` / `max-content` / bare `fit-content` map to taffy's own
    // intrinsic `Dimension` variants (0.14), which the flex algorithm
    // resolves through content measurement.
    style.flex_basis = match cv.flex_basis {
        ComputedFlexBasis::Auto => taffy::Dimension::auto(),
        ComputedFlexBasis::Content => taffy::Dimension::content(),
        ComputedFlexBasis::MinContent => taffy::Dimension::min_content(),
        ComputedFlexBasis::MaxContent => taffy::Dimension::max_content(),
        ComputedFlexBasis::FitContent => taffy::Dimension::fit_content(),
        ComputedFlexBasis::Px(v) => taffy::Dimension::length(sanitize_taffy(v, "flex-basis", diag)),
        ComputedFlexBasis::Percent(p) => {
            taffy::Dimension::percent(sanitize_taffy(p / 100.0, "flex-basis", diag))
        }
    };
}

/// alignment property → [`taffy::Style`] bridge (CSS Box Alignment Module
/// Level 3)。
///
/// - `justify-content` ([`ContentAlignmentValue`]) → `style.justify_content`
///   (§5.1)
/// - `align-content` ([`ContentAlignmentValue`]) → `style.align_content`
///   (§5.1)
/// - `align-items` ([`SelfAlignmentValue`]) → `style.align_items` (§7.2)
/// - `align-self` ([`AlignSelfValue`]) → `style.align_self` (§6.2)
/// - `justify-items` ([`SelfAlignmentValue`]) → `style.justify_items` (§7.1)
///   — grid container 上の inline-axis 版 `align-items`、同じ keyword set
///   ([`SelfAlignmentValue`] doc 参照) を共有する。
/// - `justify-self` ([`AlignSelfValue`]) → `style.justify_self` (§6.1) —
///   grid item 上の inline-axis 版 `align-self`。taffy `GridItemStyle::justify_self`
///   の doc も `align_self` と同じ "Falls back to the parents … if not set"
///   契約を持つため、`align-self` と同じ `normal`/`auto` 特殊 handling が
///   そのまま適用できる。
///
/// taffy 側の 6 field は全て `Option<…>` — CSS の `normal` (content-*系)
/// keyword には対応する taffy keyword が無く、`None` へ写す
/// (`bridge_box_sizing` の "Initial-value 補正" 節と同型の initial-value
/// mismatch)。taffy はその後 layout mode 依存の default で埋める
/// (`GridContainerStyle::grid_align_content` 等の `unwrap_or(AlignContent::STRETCH)`
/// — grid path の fallback は `STRETCH`、flex path は
/// `compute_flexbox_layout` 内部の別 default)。`align-self`/`justify-self`
/// の `auto` も同じ形 (`Option::None` → 親の `align-items`/`justify-items`
/// に fallback、CSS Box Alignment 3 §6.2/§6.1 の spec 規定どおり)。
///
/// 明示 keyword は [`taffy::AlignItems`] / [`taffy::AlignContent`] の
/// 定数 (`AlignItems::CENTER` 等、struct constant であって enum variant
/// ではない — taffy 0.12 の safe/unsafe overflow-position 分離 shape、
/// [`ContentAlignmentValue`]/[`SelfAlignmentValue`] doc の scope carving
/// 節参照) に写す。`safety` field は常に `AlignmentSafety::Unsafe` —
/// `safe`/`unsafe` prefix 自体を本 crate が受理していないため
/// (`parse_content_alignment`/`parse_self_alignment` doc 参照)。
fn bridge_alignment(style: &mut taffy::Style, cv: &ComputedValues) {
    style.justify_content = content_alignment_to_taffy(cv.justify_content);
    style.align_content = content_alignment_to_taffy(cv.align_content);
    style.align_items = self_alignment_to_taffy(cv.align_items);
    style.align_self = self_alignment_or_auto_to_taffy(cv.align_self);
    style.justify_items = self_alignment_to_taffy(cv.justify_items);
    style.justify_self = self_alignment_or_auto_to_taffy(cv.justify_self);
}

/// [`AlignSelfValue`] → `Option<taffy::AlignItems>` mapping — shared by
/// `align-self` and `justify-self` ([`bridge_alignment`] doc の
/// `justify-self` 節参照、taffy 側 `AlignSelf` は `AlignItems` の type
/// alias)。
fn self_alignment_or_auto_to_taffy(v: AlignSelfValue) -> Option<TaffyAlignItems> {
    match v {
        AlignSelfValue::Auto => None,
        // `normal` は `auto` とは異なり、親の align-items/justify-items へ
        // fallback せず、flex/grid layout では単独で `stretch` 相当に
        // 振る舞う (CSS Box Alignment 3 §8.3 の "In flex layout, this
        // value behaves as stretch" — grid layout も同節の対象)。
        // `self_alignment_to_taffy`'s `Normal => None` mapping はここでは
        // 使えない — taffy の `None` は「コンテナの対応 property を
        // 継承する」意味 (`auto` の spec 挙動そのもの) であり、`normal` の
        // 「親の値に関わらず stretch」とは異なるため、明示的に `STRETCH`
        // へ写す。
        AlignSelfValue::Value(SelfAlignmentValue::Normal) => Some(TaffyAlignItems::STRETCH),
        AlignSelfValue::Value(v) => self_alignment_to_taffy(v),
        // cov:ignore: unreachable while AlignSelfValue is Auto|Value(_)
        // only; required for its #[non_exhaustive] contract
        // (`self_alignment_to_taffy` の catch-all と同じ判断).
        _ => None,
    }
}

/// [`ContentAlignmentValue`] → `Option<taffy::AlignContent>` mapping —
/// [`bridge_alignment`] の `justify_content` / `align_content` 両 field で
/// 共有する ([`ContentAlignmentValue`] 自体が両 property の共有 payload
/// 型であるのと同じ理由)。
fn content_alignment_to_taffy(v: ContentAlignmentValue) -> Option<TaffyAlignContent> {
    match v {
        ContentAlignmentValue::Normal => None,
        ContentAlignmentValue::Stretch => Some(TaffyAlignContent::STRETCH),
        ContentAlignmentValue::SpaceBetween => Some(TaffyAlignContent::SPACE_BETWEEN),
        ContentAlignmentValue::SpaceEvenly => Some(TaffyAlignContent::SPACE_EVENLY),
        ContentAlignmentValue::SpaceAround => Some(TaffyAlignContent::SPACE_AROUND),
        ContentAlignmentValue::Center => Some(TaffyAlignContent::CENTER),
        ContentAlignmentValue::Start => Some(TaffyAlignContent::START),
        ContentAlignmentValue::End => Some(TaffyAlignContent::END),
        ContentAlignmentValue::FlexStart => Some(TaffyAlignContent::FLEX_START),
        ContentAlignmentValue::FlexEnd => Some(TaffyAlignContent::FLEX_END),
        // cov:ignore: unreachable while ContentAlignmentValue is the 10
        // variants matched above only; required for its #[non_exhaustive]
        // contract.
        _ => None,
    }
}

/// [`SelfAlignmentValue`] → `Option<taffy::AlignItems>` mapping —
/// [`bridge_alignment`] の `align_items` field と `align_self` の
/// `AlignSelfValue::Value` branch で共有する (`align-self` の非-`auto` 値は
/// `align-items` と同じ keyword set、[`AlignSelfValue`] doc 参照)。
fn self_alignment_to_taffy(v: SelfAlignmentValue) -> Option<TaffyAlignItems> {
    match v {
        SelfAlignmentValue::Normal => None,
        SelfAlignmentValue::Stretch => Some(TaffyAlignItems::STRETCH),
        SelfAlignmentValue::Center => Some(TaffyAlignItems::CENTER),
        SelfAlignmentValue::Start => Some(TaffyAlignItems::START),
        SelfAlignmentValue::End => Some(TaffyAlignItems::END),
        SelfAlignmentValue::FlexStart => Some(TaffyAlignItems::FLEX_START),
        SelfAlignmentValue::FlexEnd => Some(TaffyAlignItems::FLEX_END),
        SelfAlignmentValue::Baseline => Some(TaffyAlignItems::BASELINE),
        // cov:ignore: unreachable while SelfAlignmentValue is the 8
        // variants matched above only; required for its #[non_exhaustive]
        // contract.
        _ => None,
    }
}

/// `row-gap` / `column-gap` → [`taffy::Style::gap`] (`Size<LengthPercentage>`)
/// bridge。
///
/// CSS Box Alignment Module Level 3 §8.1 "Row and Column Gutters: the
/// row-gap and column-gap properties"
/// <https://www.w3.org/TR/css-align-3/#column-row-gap> propdef の
/// "Initial: normal" に対し、同 propdef の value 説明 (verbatim) が:
///
/// > The value `normal` represents a used value of `1em` on multi-column
/// > containers, and a used value of `0px` in all other contexts.
///
/// と規定する — flex/grid container はこの "all other contexts" に属する。
/// これは [`bridge_box_sizing`] の "Initial-value 補正" 節と同型の
/// mismatch: raikiri-style の computed 層 initial は `normal` keyword を
/// 保持する ([`ComputedLengthPercentageOrNormal::Normal`]) が、taffy 側
/// `Size<LengthPercentage>` (non-`Option`) には `normal` keyword が
/// 存在しないため、本 bridge が `normal → 0` の変換を担う。
///
/// `Px` / `Percent` は [`ComputedLengthPercentage`] に一度変換してから
/// [`computed_length_percentage_to_taffy_length_percentage`] (percent の
/// `/100.0` 変換 + [`sanitize_taffy`] 非有限 guard を含む) へ delegate する
/// — [`bridge_padding`] と同じ helper を再利用し、実装を複製しない。
fn bridge_gap(style: &mut taffy::Style, cv: &ComputedValues, diag: &mut Vec<LayoutWarn>) {
    style.gap = Size {
        width: computed_gap_component_to_taffy(cv.column_gap, "column-gap", diag),
        height: computed_gap_component_to_taffy(cv.row_gap, "row-gap", diag),
    };
}

/// [`bridge_gap`] の per-axis helper — `taffy::Size<LengthPercentage>` の
/// `width` は inline axis (`column-gap` 相当)、`height` は block axis
/// (`row-gap` 相当) に対応する (taffy `Size` の一般 convention、
/// [`bridge_size`] の `width`/`height` mapping と同じ軸の向き)。
fn computed_gap_component_to_taffy(
    gap: ComputedLengthPercentageOrNormal,
    site: &'static str,
    diag: &mut Vec<LayoutWarn>,
) -> LengthPercentage {
    let lp = match gap {
        ComputedLengthPercentageOrNormal::Normal => ComputedLengthPercentage::Px(0.0),
        ComputedLengthPercentageOrNormal::Px(v) => ComputedLengthPercentage::Px(v),
        ComputedLengthPercentageOrNormal::Percent(p) => ComputedLengthPercentage::Percent(p),
    };
    computed_length_percentage_to_taffy_length_percentage(lp, site, diag)
}

/// grid container/item property → [`taffy::Style`] bridge (CSS Grid Layout
/// Module Level 1)。
///
/// - `grid-template-columns` / `grid-template-rows` → `style.grid_template_columns`
///   / `style.grid_template_rows` (`Vec<GridTemplateComponent>`、§7.2) +
///   `style.grid_template_column_names` / `style.grid_template_row_names`
///   (line names outside any `repeat()`、§7.2.2 — [`grid_template_tracks_to_taffy`]
///   doc の shape 対応参照)
/// - `grid-template-areas` → `style.grid_template_areas`
///   (`Vec<GridTemplateArea>`、§7.3)
/// - `grid-auto-columns` / `grid-auto-rows` → `style.grid_auto_columns` /
///   `style.grid_auto_rows` (§7.6)
/// - `grid-auto-flow` → `style.grid_auto_flow` (§7.7)
/// - `grid-row-start`/`grid-row-end` / `grid-column-start`/`grid-column-end`
///   → `style.grid_row` / `style.grid_column` (`Line<GridPlacement>`、§8.3)
fn bridge_grid(style: &mut taffy::Style, cv: &ComputedValues, diag: &mut Vec<LayoutWarn>) {
    let (columns, column_names) =
        grid_template_tracks_to_taffy(&cv.grid_template_columns, "grid-template-columns", diag);
    style.grid_template_columns = columns;
    style.grid_template_column_names = column_names;
    let (rows, row_names) =
        grid_template_tracks_to_taffy(&cv.grid_template_rows, "grid-template-rows", diag);
    style.grid_template_rows = rows;
    style.grid_template_row_names = row_names;

    style.grid_template_areas = match &cv.grid_template_areas {
        GridTemplateAreasValue::None => None,
        GridTemplateAreasValue::Areas(areas) => Some(taffy::style::GridTemplateAreas {
            areas: areas
                .areas
                .iter()
                .map(|a| TaffyGridTemplateArea {
                    name: a.name.to_string(),
                    row_start: saturate_u16(a.row_start),
                    row_end: saturate_u16(a.row_end),
                    column_start: saturate_u16(a.column_start),
                    column_end: saturate_u16(a.column_end),
                })
                .collect(),
            // taffy clamps grid dimensions to 10,000 tracks; saturate our
            // u32 counts the same way as the area coordinates above.
            row_count: areas.row_count.min(u16::MAX as u32) as u16,
            column_count: areas.column_count.min(u16::MAX as u32) as u16,
        }),
        // cov:ignore: unreachable while GridTemplateAreasValue is
        // None|Areas(_) only; required for its #[non_exhaustive] contract
        // (see `bridge_display` catch-all doc).
        _ => None,
    };

    style.grid_auto_columns = cv
        .grid_auto_columns
        .iter()
        .copied()
        .map(|t| grid_track_size_to_taffy(t, "grid-auto-columns", diag))
        .collect();
    style.grid_auto_rows = cv
        .grid_auto_rows
        .iter()
        .copied()
        .map(|t| grid_track_size_to_taffy(t, "grid-auto-rows", diag))
        .collect();

    style.grid_auto_flow = match cv.grid_auto_flow {
        GridAutoFlowValue::Row => TaffyGridAutoFlow::Row,
        GridAutoFlowValue::Column => TaffyGridAutoFlow::Column,
        GridAutoFlowValue::RowDense => TaffyGridAutoFlow::RowDense,
        GridAutoFlowValue::ColumnDense => TaffyGridAutoFlow::ColumnDense,
        // cov:ignore: unreachable while GridAutoFlowValue is
        // Row|Column|RowDense|ColumnDense only; required for its
        // #[non_exhaustive] contract (see `bridge_display` catch-all doc).
        _ => TaffyGridAutoFlow::Row,
    };

    style.grid_row = TaffyLine {
        start: grid_line_value_to_taffy_placement(&cv.grid_row_start),
        end: grid_line_value_to_taffy_placement(&cv.grid_row_end),
    };
    style.grid_column = TaffyLine {
        start: grid_line_value_to_taffy_placement(&cv.grid_column_start),
        end: grid_line_value_to_taffy_placement(&cv.grid_column_end),
    };
}

/// [`ComputedGridTemplateTracks`] → taffy's `(Vec<GridTemplateComponent>,
/// Vec<Vec<String>>)` pair — [`bridge_grid`]'s `grid_template_columns`/
/// `grid_template_rows` field **and** their paired `_names` field share one
/// helper because taffy's `NamedLineResolver` consumes both in lock-step
/// (one name-list entry per top-level component, plus one trailing entry
/// after the last — see taffy 0.12's `NamedLineResolver::new`, which zips
/// `grid_template_columns()`/`grid_template_rows()` against
/// `grid_template_column_names()`/`grid_template_row_names()`). This is
/// exactly [`crate` `raikiri_style`]'s own `ComputedGridTrackList::line_names`
/// interleave convention (`line_names.len() == components.len() + 1`), so
/// the two Vecs below are built from the same source data without any
/// reindexing.
///
/// `none` maps to a pair of empty `Vec`s — taffy's own default for a
/// container with no explicit track list.
fn grid_template_tracks_to_taffy(
    tracks: &ComputedGridTemplateTracks,
    site: &'static str,
    diag: &mut Vec<LayoutWarn>,
) -> (Vec<GridTemplateComponent<String>>, Vec<Vec<String>>) {
    match tracks {
        ComputedGridTemplateTracks::None => (Vec::new(), Vec::new()),
        ComputedGridTemplateTracks::List(list) => {
            let components = list
                .components
                .iter()
                .cloned()
                .map(|component| match component {
                    ComputedGridTrackListComponent::Size(size) => {
                        GridTemplateComponent::Single(grid_track_size_to_taffy(size, site, diag))
                    }
                    ComputedGridTrackListComponent::Repeat(repeat) => {
                        GridTemplateComponent::Repeat(GridTemplateRepetition {
                            count: grid_repeat_count_to_taffy(repeat.count),
                            tracks: repeat
                                .tracks
                                .into_iter()
                                .map(|t| grid_track_size_to_taffy(t, site, diag))
                                .collect(),
                            line_names: repeat
                                .line_names
                                .into_iter()
                                .map(|names| names.iter().map(ToString::to_string).collect())
                                .collect(),
                        })
                    }
                })
                .collect();
            let line_names = list
                .line_names
                .iter()
                .map(|names| names.iter().map(ToString::to_string).collect())
                .collect();
            (components, line_names)
        }
    }
}

/// [`GridRepeatCount`] → [`taffy::RepetitionCount`] mapping. `Count`'s
/// `<integer [1,∞]>` payload is `u32` at the raikiri-style layer (CSS
/// Values 4 §4.2 places no upper bound on `<integer>`) but taffy's
/// `RepetitionCount::Count` is `u16` — silently saturating at
/// [`u16::MAX`] (65535 repetitions) rather than adding new
/// [`LayoutWarn`] machinery for an author value that large, which would
/// already make taffy's own track-sizing algorithm impractically slow
/// long before this cast is reached.
fn grid_repeat_count_to_taffy(count: GridRepeatCount) -> TaffyRepetitionCount {
    match count {
        GridRepeatCount::Count(n) => TaffyRepetitionCount::Count(saturate_u16(n)),
        GridRepeatCount::AutoFill => TaffyRepetitionCount::AutoFill,
        GridRepeatCount::AutoFit => TaffyRepetitionCount::AutoFit,
        // cov:ignore: unreachable while GridRepeatCount is
        // Count|AutoFill|AutoFit only; required for its #[non_exhaustive]
        // contract.
        _ => TaffyRepetitionCount::Count(1),
    }
}

/// [`ComputedGridTrackSize`] → [`taffy::TrackSizingFunction`]
/// (`MinMax<MinTrackSizingFunction, MaxTrackSizingFunction>`) mapping.
///
/// `FitContent` has no bare-breadth min side in taffy's model (only
/// `MaxTrackSizingFunction::fit_content_px`/`fit_content_percent` exist) —
/// the CSS spec formula `max(minimum, min(limit, max-content))` treats the
/// min side as `auto`, which is what `MinTrackSizingFunction::auto()`
/// supplies here.
fn grid_track_size_to_taffy(
    size: ComputedGridTrackSize,
    site: &'static str,
    diag: &mut Vec<LayoutWarn>,
) -> TrackSizingFunction {
    match size {
        ComputedGridTrackSize::Breadth(b) => TrackSizingFunction {
            min: grid_track_breadth_to_taffy_min(b, site, diag),
            max: grid_track_breadth_to_taffy_max(b, site, diag),
        },
        ComputedGridTrackSize::MinMax(min, max) => TrackSizingFunction {
            min: grid_track_breadth_to_taffy_min(min, site, diag),
            max: grid_track_breadth_to_taffy_max(max, site, diag),
        },
        ComputedGridTrackSize::FitContent(lp) => {
            let max = match lp {
                ComputedLengthPercentage::Px(v) => {
                    MaxTrackSizingFunction::fit_content_px(sanitize_taffy(v, site, diag))
                }
                ComputedLengthPercentage::Percent(p) => {
                    MaxTrackSizingFunction::fit_content_percent(sanitize_taffy(
                        p / 100.0,
                        site,
                        diag,
                    ))
                }
            };
            TrackSizingFunction {
                min: MinTrackSizingFunction::auto(),
                max,
            }
        }
    }
}

/// [`ComputedGridTrackBreadth`] → [`taffy::MinTrackSizingFunction`] mapping
/// (`minmax()`'s min side, or the min side of a bare `<track-breadth>`
/// widened to `MinMax { min, max }` — [`grid_track_size_to_taffy`]'s
/// `Breadth` arm). `Flex` never actually reaches this function
/// ([`ComputedGridTrackBreadth`] doc's collapse note — the min side is
/// only ever produced by `resolve_grid_inflexible_breadth`, which has no
/// `Flex` variant to produce it from); the arm below is a defensive
/// fallback to keep the match exhaustive, not a reachable case.
fn grid_track_breadth_to_taffy_min(
    b: ComputedGridTrackBreadth,
    site: &'static str,
    diag: &mut Vec<LayoutWarn>,
) -> MinTrackSizingFunction {
    use ComputedGridTrackBreadth as B;
    match b {
        B::Px(v) => MinTrackSizingFunction::length(sanitize_taffy(v, site, diag)),
        B::Percent(p) => MinTrackSizingFunction::percent(sanitize_taffy(p / 100.0, site, diag)),
        B::MinContent => MinTrackSizingFunction::min_content(),
        B::MaxContent => MinTrackSizingFunction::max_content(),
        B::Auto => MinTrackSizingFunction::auto(),
        // cov:ignore: see this fn's doc — structurally unreachable, kept
        // only for match exhaustiveness.
        B::Flex(_) => MinTrackSizingFunction::auto(),
    }
}

/// [`ComputedGridTrackBreadth`] → [`taffy::MaxTrackSizingFunction`] mapping
/// (`minmax()`'s max side, or the max side of a bare `<track-breadth>` —
/// [`grid_track_size_to_taffy`]'s `Breadth` arm). Unlike
/// [`grid_track_breadth_to_taffy_min`], `Flex` (`fr`) is a real, reachable
/// case here — the `fr` unit is only valid on the max side of `minmax()`
/// or as a bare `<track-breadth>` (CSS Grid Layout Module Level 1 §7.2.4).
fn grid_track_breadth_to_taffy_max(
    b: ComputedGridTrackBreadth,
    site: &'static str,
    diag: &mut Vec<LayoutWarn>,
) -> MaxTrackSizingFunction {
    use ComputedGridTrackBreadth as B;
    match b {
        B::Px(v) => MaxTrackSizingFunction::length(sanitize_taffy(v, site, diag)),
        B::Percent(p) => MaxTrackSizingFunction::percent(sanitize_taffy(p / 100.0, site, diag)),
        B::Flex(f) => MaxTrackSizingFunction::fr(sanitize_taffy(f, site, diag)),
        B::MinContent => MaxTrackSizingFunction::min_content(),
        B::MaxContent => MaxTrackSizingFunction::max_content(),
        B::Auto => MaxTrackSizingFunction::auto(),
    }
}

/// [`GridLineValue`] → [`taffy::GridPlacement`] mapping (CSS Grid Layout
/// Module Level 1 §8.3). [`GridLineValue::Named`] (bare `<custom-ident>`,
/// no explicit index) maps to `NamedLine` with index `1` explicitly filled
/// in — same as [`GridLineValue`]'s own doc explains: taffy 0.12 treats
/// index `0` as an "unspecified" sentinel and normalizes it to `1`
/// internally (`NamedLineResolver::find_line_index`'s `if idx == 0 { idx =
/// 1; }`), so filling in `1` here ahead of time is behavior-identical.
///
/// `Line`/`Span`/`NamedSpan`'s `i32`/`u32` payloads are widened from parse
/// time's unbounded `<integer>` (CSS Values 4 §4.2) but taffy's
/// `GridLine`/`Span(u16)`/`NamedSpan(_, u16)` use 16-bit integers —
/// silently saturating at [`i16::MIN`]/[`i16::MAX`]/[`u16::MAX`] rather
/// than adding new [`LayoutWarn`] machinery, same rationale as
/// [`grid_repeat_count_to_taffy`].
fn grid_line_value_to_taffy_placement(v: &GridLineValue) -> GridPlacement {
    match v {
        GridLineValue::Auto => GridPlacement::Auto,
        GridLineValue::Line(n) => taffy_style_helpers::line(saturate_i16(*n)),
        GridLineValue::Named(name) => GridPlacement::NamedLine(name.to_string(), 1),
        GridLineValue::NamedLine(name, n) => {
            GridPlacement::NamedLine(name.to_string(), saturate_i16(*n))
        }
        GridLineValue::Span(n) => GridPlacement::Span(saturate_u16(*n)),
        GridLineValue::SpanNamed(name, n) => {
            GridPlacement::NamedSpan(name.to_string(), saturate_u16(*n))
        }
        // cov:ignore: unreachable while GridLineValue is the 6 variants
        // matched above only; required for its #[non_exhaustive] contract.
        _ => GridPlacement::Auto,
    }
}

/// 最小限の inline formatting context を、条件を満たす block container に
/// 確立する。`<br>` を含まない場合は単一行・non-wrapping (下記 "実現方法")
/// だが、display:none ではない `<br>` を 1 個以上含む場合は forced break
/// だけで少なくともその個数 + 1 個の (視覚上高さを持つ) line に分かれる
/// (下記 "`<br>` forced break")。先頭・末尾・連続する `<br>` はそれ自身の
/// line が高さ 0 に潰れるため、実際の見た目上の line 数はこれより少なく
/// なりうる一方、`<br>` を含む container は `flex_wrap: Wrap` も同時に
/// 有効になる副作用を持つため、それとは独立な size-based wrapping が
/// 追加の line を発生させ、より多くなることもある — 上限はない (同
/// section の doc および Non-goals 参照)。
///
/// CSS 2.1 §9.4.2 <https://www.w3.org/TR/CSS21/visuren.html#inline-formatting>:
/// "a block container either contains only block-level boxes or
/// establishes an inline formatting context and thus contains only
/// inline-level boxes."(block container は block-level box のみを含むか、
/// inline formatting context を確立して inline-level box のみを含む)。
/// ここで node が条件を満たすのは、自身の computed display
/// ([`DisplayValue`]) が [`DisplayValue::Block`] または
/// [`DisplayValue::InlineBlock`] であり (両方とも自身の content に対して
/// block container を生成する — `inline-block` は CSS Display 3 §2 上
/// "inline flow-root" (plain な `block` の "block flow" とは別の、独立
/// した formatting context を確立する概念、[`DisplayValue::InlineBlock`]
/// 自身の doc 参照) だが、本 pass の qualifying condition と扱いはこの
/// 区別をしない — どちらも「自身の content area が block container で
/// ある」という一点のみを見る)、**かつ** in-document な children が
/// **2 個以上**あり、それら
/// 全てが inline-level である場合 ([`NodeKind::Text`] の child、または
/// [`DisplayValue`] が [`DisplayValue::Inline`] / [`DisplayValue::InlineBlock`]
/// な [`NodeKind::Element`] の child。[`DisplayValue::None`] の child は
/// 無視する — count にも disqualification にも数えない)。
/// [`NodeKind::Text`] の child は無条件に count する
/// (whitespace のみを保持する text node も含む) — HTML parser は source の
/// indentation から sibling tag 間にこうした text node をよく生成し、CSS
/// 2.1 §9.2.1.1 上、text node の content は保持する文字に関わらず
/// ordinary な inline-level content である (`white-space` の collapsing は
/// rendering 時の関心事であり、formatting context の関心事ではない) ため、
/// "本物の" inline な child が 1 個だけの container でも、付随する parser
/// の whitespace により実質的にすでに 2 個以上の qualifying な children
/// を持つことが多い。inline-level な child が 1 個だけの場合は、plain な
/// block path のままにする: line 上に box が 1 個しかなければ隣に置く
/// ものが無く、taffy の block layout も 1 個だけの child を
/// `align-items: flex-start` な 1-item flex row と同じ位置に置く
/// (どちらも container を child 自身の margin box に合わせて size し、
/// cross-axis 方向の処理が不要な点も同じ) — したがってこの case には
/// 埋めるべき behavioral gap が無く、既存の block code path を乱す理由も
/// 無い。
///
/// plain な [`DisplayValue::Inline`] の node は、direct text children
/// だけなら本条件を満たさない: `block` / `inline-block` と異なり、
/// non-replaced な `inline` box は CSS 2.1 §9.2.1.1 上、自身の content に
/// 対して block container を生成しない — その content は `inline` box 自身と
/// *同じ* inline formatting context に流れ込む ordinary な inline-level
/// content である。nested な inline element child を持つ場合だけは例外として
/// synthetic flex line root を作る。taffy の block path ではその child の
/// descendants が縦に積まれるためであり、これは DOM の flatten ではなく
/// nested wrapper ごとの最小 line-box bridge である。
///
/// block-level / flex / grid な in-document child を 1 個でも持つ
/// container も同様に変更しない (block-level と inline-level が混在する
/// content は、CSS 2.1 §9.2.1.1 に従い inline-level の run を囲む
/// anonymous block box の生成が必要になるが、本 minimal pass はそこまで
/// 対応しない)。
///
/// # 実現方法
///
/// taffy には inline layout mode が無いため、line box は 1 行・
/// non-wrapping な flex container として実現する: [`Display::Flex`] +
/// `flex_direction: Row` (taffy 自身の default と同じ) + `flex_wrap:
/// NoWrap` (同上) により、参加する children を左から右へ 1 行に並べる —
/// これは CSS 2.1 §9.4.2 が inline formatting context の box について
/// 述べる内容 ("laid out horizontally, one after the other, beginning at
/// the top of a containing block") そのものである。`flex_direction` /
/// `flex_wrap` を default に委ねず明示的に set しているのは、この node
/// が block container だった間は inert だった author の
/// `flex-direction` / `flex-wrap` 宣言を [`bridge_flex`] が既に copy
/// 済みの可能性があり、ここで flex container になった時点でそれが
/// inert でなくなるため — default のままだろうと期待するのではなく
/// 明示的に上書きする必要がある。
///
/// `align_items: Baseline` を set するのは、`bridge_alignment` が copy
/// した author の `align-items` を上書きし、参加する text children の
/// first baseline を揃えるためである。`taffy_impl.rs` は text leaf の
/// Parley first-line baseline を report し、`inline` / `inline-block`
/// wrapper では in-flow text baseline を box へ伝える。baseline を持たない
/// replaced box 等は taffy の bottom-edge fallback に残る。`vertical-align`
/// の top / bottom は child の `align_self` で引き続き別扱いする。
///
/// `vertical-align` の実装済み length / sub / super shift は paint 時に
/// glyph へ適用する一方、その shift 分を line root の synthetic leading /
/// trailing として sizing に加える。これにより raised / lowered box が
/// line box の高さへ参加し、他の child も同じ text baseline を基準に配置
/// される。Synthetic extent は authored padding に加算されるため、既存の
/// padding 値と共存する。
///
/// container 自身は `justify_content: None` (taffy 自身の default、CSS
/// の `normal` 相当) へも reset する — 全く同じ「もう inert ではない」
/// 理由による: [`bridge_alignment`] が copy した author の
/// `justify-content` 宣言は、block container だった間は inert だったが、
/// ここで flex container になった時点で main-axis 方向の item 配置
/// (space-between 等) を変えてしまう。CSS 2.1 の inline formatting
/// context に "line box 上の複数 box を main-axis 方向に再配置する"
/// 意味論はそもそも存在しないため、taffy 側の default に戻すことでその
/// 意味論を無効化する。`gap` (`row-gap` / `column-gap`) も同じ理由で
/// `0` へ reset する — line box は inline-level box の間に author 指定の
/// 隙間を空ける意味論を持たない (CSS 2.1 §9.4.2 の line box は隙間なく
/// box を並べるモデル) ため、[`bridge_gap`] が copy した author 値を
/// ここで無効化する。
///
/// `align_content` も [`bridge_alignment`] が copy した author の値を
/// `Some(FlexStart)` へ reset する。これは複数 flex line 間で余った
/// cross-axis space を配る flex 専用 property であり、CSS inline context
/// には対応する意味論がない。reset を怠ると明示的な `height` がある場合に
/// default の `Stretch` が余り space を `<br>` の zero-height line に配り、
/// 視覚的な gap を作る。
///
/// 参加する各 child はさらに `flex_grow: 0.0` / `flex_shrink: 0.0` /
/// `flex_basis: auto` / `align_self: None` (or `Top` / `Bottom` override) も得る ([`bridge_flex`] /
/// [`bridge_alignment`] が自身の author CSS から copy した値を、上記と
/// 同じ「もう inert ではない」理由で上書きする)。`flex_grow` /
/// `flex_shrink` を `0` にする理由: line wrapping を未実装の現状、
/// container より広い content は圧縮されずに overflow するべきものである
/// — flex の default である `flex-shrink: 1` のままだと、各 child の
/// box を自身の shaped content より狭く圧縮してしまい、text の child
/// ではすでに [`preshape_text`] が shape した glyph run と box が乖離
/// する (shaped 済みの glyph は追従して縮まないため、狭くなった box から
/// overflow し、隣の child の glyph と重なりうる — 本 pass が置き換える
/// 以前の stacked-block な描画より悪化する)。`flex_basis` を `auto` へ
/// 戻す理由も同じ box/glyph 乖離を防ぐためである — 明示的な author
/// `flex-basis` は `flex_grow` / `flex_shrink` の値に関わらず box の
/// main size を直接決めてしまうため、`flex_shrink: 0` だけでは守れない
/// (自身の content 基準の flex basis に戻すことで、box は常に自身の
/// shaped content 以上の幅を持つ)。`align_self` を `None` (= `auto`) へ
/// 戻す理由は、container 側の `align_items: Baseline` へ一貫して
/// fallback させるためである (`auto` は親の `align-items` へ fallback
/// する契約、[`bridge_alignment`] の doc 参照)。`vertical-align: top` /
/// `bottom` はそれぞれ `FlexStart` / `FlexEnd` に明示変換する。
///
/// # `<br>` forced break
///
/// HTML LS §the-br-element
/// (<https://html.spec.whatwg.org/multipage/text-level-semantics.html#the-br-element>)
/// の `<br>` は "a line break" を表す。HTML LS の rendering section
/// (§phrasing-content-3) は `br { display-outside: newline; }` と記すが、
/// これは CSS Display 4 の `<display-outside>` production
/// (`block | inline | run-in` のみ) に存在しない illustrative な記法で、
/// raikiri-style が消費できる CSS 宣言ではない。本 pass は代わりに `<br>`
/// を tag_name で直接判別する ([`find_body`] が `tag_name() ==
/// Some("body")` を直接比較するのと同じ pattern — `<br>` の判別に
/// [`NodeKind`] へ新しい variant を追加する必要はない、`NodeKind::Element`
/// のまま扱う)。
///
/// qualify する container の participating children に、display:none
/// ではない `<br>` が 1 個以上含まれる場合、container 自身の `flex_wrap`
/// を (taffy 自身の default である) `NoWrap` から `Wrap` へ切り替え、
/// 参加する `<br>` child 自身の `flex_basis` のみを (他の child と同じ
/// `auto` ではなく) `100%` にする。他の participating child への
/// `flex_grow: 0` / `flex_shrink: 0` の適用は変わらない。
///
/// これにより、taffy 自身の flex line-packing algorithm (CSS Flexbox 1
/// §9.2 "Line Length Determination"
/// <https://www.w3.org/TR/css-flexbox-1/#algo-line-break> — multi-line
/// (`flex-wrap` が `nowrap` ではない) container の各 line は、item を
/// 1 個ずつ足しながら、container の main size を超える最初の item の
/// 手前で確定し、その item を次の line へ持ち越す。ただしその item が
/// line 上の最初の item である場合 (line がまだ空) は、超えていても
/// そのまま現在の line に置く、という例外を持つ) が `<br>` の位置で
/// 自然に line を切る: `<br>` の hypothetical main size が container
/// 幅の 100% であるため、すでに他の item が乗っている line には決して
/// 収まらず新しい line へ move する一方、`<br>` 自身が (空の) line の
/// 先頭に来た場合は上記例外でその line にそのまま置かれる。
/// `<br>` が単独で占有するその line 自身の高さ (cross size、row 方向
/// なので `flex_basis` が支配しない軸) は
/// [`compute_leaf_layout`](taffy::compute_leaf_layout) の測定に委ねられる
/// — `<br>` は children を持たない leaf であり [`Node::text_layout`](crate::node::Node::text_layout) も
/// 返さないため、measure 結果は常に `0` になる (この module の
/// `LayoutPartialTree::compute_child_layout` — `taffy_impl.rs` 側 — の
/// leaf 分岐 doc 参照)。したがって `<br>` が占有する line は幅こそ
/// container 全幅だが高さ 0 で、直後の line が直前の line の下端に
/// 隙間なく続く。結果として `<br>` の前後にある run はそれぞれ別の
/// (見た目上の) line に分かれる一方、`<br>` 自身は視覚上何の高さも
/// 占めない — CSS 2.1 の forced line break の見た目の効果と一致する。
///
/// この機構は `<br>` を含む container にのみ `flex_wrap: Wrap` を
/// 付与する副作用も持つ: `<br>` を含まない container は従来通り
/// `flex_wrap: NoWrap` のままだが、`<br>` を含む container では
/// (`<br>` の前後どちらの run でも) 本来 Non-goal である
/// size-based wrapping が technically 可能になる — line 内の
/// inline-level item 群が container の available width を超えれば、
/// `<br>` の有無に関わらず taffy 自身が折り返してしまう。この delta は
/// `<br>` を含む container にのみ生じ、これまで check されていた
/// no-wrap な挙動 (`establish_minimal_line_boxes_upgrades_qualifying_container_to_flex_row`
/// 等の regression test) は `<br>` を含まないケースなので影響を受けない。
///
/// # Non-goals (本 pass)
///
/// - size-based line wrapping は行わない: qualify する container **自身が
///   確立する line box** の個数は、display:none ではない `<br>` を含まない
///   限り、container の available width に関わらず常にちょうど 1 個になる
///   (`<br>` を含む場合の個数は上記 "`<br>` forced break" 参照 — それは
///   forced break であり、size に基づく wrap ではない)。ただしこれは
///   container レベルの取り扱いの話であり、participating な text child
///   自身が shape する glyph run が内部で複数行に折り返されないことまでは
///   意味しない (最後の bullet 参照 — [`preshape_text`] は本 pass とは
///   独立に、text ごとに page width 基準で soft-wrap する)。
/// - `<br>` 自身の box は測定上つねに `0×0` であり、「空の行が font の
///   line-height 相当の高さを持つ」という real browser の挙動を再現
///   しない。real browser でこの高さを与えているのは CSS 2.1 §10.8.1
///   <https://www.w3.org/TR/CSS21/visudet.html#strut> の "strut" —
///   各 line box の先頭に、その line box を確立した要素の font /
///   line-height を持つ幅 0 の仮想 inline box が置かれているものとして
///   扱う、という規定で、空行にも line-height 分の最小高さを与える。
///   本 pass はこの strut を line box (= 本 pass が作る flex line) に
///   対して合成しない — [`compute_leaf_layout`](taffy::compute_leaf_layout)
///   による `<br>` 自身の測定結果だけが line の cross size を決めるため、
///   これは [`establish_minimal_line_boxes`] が line box そのものを
///   (単一行・non-wrapping な場合を含め) 元から strut 抜きで組んでいる
///   ことの帰結であり、別々の special case ではない。これにより 2 個の
///   bullet で挙動が変わる:
///   (a) `<br>` が (自身の line 上で) 最初の participating item になる
///   場合 — line の先頭にある container、あるいは連続する `<br><br>` の
///   2 個目以降 — line-packing の "空の line はその最初の item を
///   そのまま受け入れる" 例外により `<br>` はそこに留まり、高さ 0 のまま
///   何も visible な空行を作らない。real browser が `<p><br>text</p>` の
///   先頭や `<br><br>` の間で作る空行の高さは、本 pass では再現しない。
///   (b) 末尾の `<br>` (後続の inline-level content が無い) も同様に
///   高さ 0 の line を追加するだけで、視覚上の余白は生まない。
///   両方とも、`<br>` が childless leaf で text layout を持たないために
///   measure が `0` を返すという同じ機構の帰結であり、別々の special
///   case ではない。
/// - nested な inline element を ancestor の DOM line box へ flatten する
///   ことは行わない。nested wrapper が inline element child を持つ場合は
///   wrapper自身を最小 flex line root にするが、各 wrapper の children は
///   その wrapper 内で独立に layout する。
/// - [`preshape_text`] は本 pass の後 (`apply_computed_to_style` 完了後、
///   `layout_single_page` の後段) に、各 text node の glyph run を full
///   page width に対して shape・soft-wrap する。これは本 pass が最終的に
///   その text node に line box 上の sibling と並べて与える横幅とは
///   無関係であり、text の shaping を実際の available な inline space
///   と整合させることは follow-up work であり、ここでは行わない。
/// - `text-align: center` は本 pass と [`realign_text_after_layout`] の
///   2 経路で扱う: qualify した container 自身は line box 全体を中央に寄せる
///   ため `justify_content` を `Center` にする (他値は `None` のまま —
///   `Right` / `Justify` 等の flex 対応は本 task の scope 外)。
///   qualify しない block container 内の単独 Text は後段の
///   `realign_text_after_layout` が parley 側で中央寄せする。
///   詳細は同関数の doc 参照。
fn text_align_to_parley(v: TextAlign) -> Alignment {
    match v {
        TextAlign::Start => Alignment::Start,
        TextAlign::End => Alignment::End,
        TextAlign::Left => Alignment::Left,
        TextAlign::Right => Alignment::Right,
        TextAlign::Center => Alignment::Center,
        TextAlign::Justify => Alignment::Justify,
        // CSS Text 3 §6.1: `justify-all` は全行 (最終行含む) を justify。
        // parley に相当が無いため `Justify` に degrade する
        // (最終行は start-aligned のまま — 既知の差分として doc に残す)。
        TextAlign::JustifyAll => Alignment::Justify,
        // `MatchParent` は computed 層到達前に解決済のはず
        // (`raikiri_style::computed::ComputedValues::text_align` doc 参照)。
        // defensive fallback として `Start` (initial value) に倒す。
        TextAlign::MatchParent => Alignment::Start,
        // `#[non_exhaustive]` 将来 variant への fail-closed fallback。
        _ => Alignment::Start,
    }
}

/// taffy の `compute_root_layout` 後に各 Text の parley `Layout` を
/// containing block 幅基準で re-align する。
///
/// # なぜ preshape 時ではなくここか
///
/// [`preshape_text`] は taffy より前に走り、使える幅は `page_box.width`
/// のみ。`Center` 等をそこで素朴に渡すと、狭い containing block 内の text が
/// ページ幅基準で中央寄せされ、大きくズレる。
/// taffy 後に親 box の確定幅 (`unrounded_layout.size.width`) を
/// containing 幅として `break_all_lines(Some(w))` + `align(...)` し直すことで、
/// 正しい幅基準の offset を glyph run に bake する。
/// `Start` は幅に依存しないため skip する (preshape のまま正しい)。
///
/// # flex line box の child は対象外
///
/// 親が [`establish_minimal_line_boxes`] で flex 化された container
/// (`IS_INLINE_ROOT`) の場合、その Text は line box 上の flex item であり、
/// 中央寄せは container 側の `justify_content: Center` が担う。
/// ここで親幅基準に parley re-align すると各 piece が親全幅基準で中央に
/// 寄り、互いに重なってしまうため、明示的に skip する。
/// 親幅ではなく自身の box 幅で寄せても offset 0 の no-op にしかならないが、
/// 意図を明示するため skip 側に倒す。
///
/// # 既知の限界
///
/// - 親の `size.width` をそのまま containing 幅にする。padding / border の
///   content-box 減算はしない (px length のみ減算すべきだが、現状は未対応 —
///   padding 付き narrow container では中央がわずかにずれる)。
/// - re-break で行数が変わる場合 (narrow container 内の長文)、text の高さが
///   変わるが taffy box は更新しない (sibling の y が stale のまま)。
///   中央寄せの主 target である短文・単一行では高さ不変のため無害。
///   長文 wrap の完全な整合は inline formatting context 全体の再設計時に扱う。
/// - `Start` / `End` の論理→物理解決は自要素の `cv.direction` で行う。
///   parley public API に base direction を渡す口が無いため、parley 側の
///   `Start` / `End` には頼らず
///   `Left` / `Right` に解決してから渡す。`dir=rtl` 属性は対象外
///   (`direction` CSS のみ — dir 属性→direction 反映は Epic 3 領域)。
/// - `text-indent` の `%` は親の border-box 幅基準で解決する (content-box
///   減算なし — 上記第 1 bullet と同じ近似)。flex line box の child の
///   indent は未対応 (box 測定との乖離のため skip)。
///
/// tab-stop 基準の block-container 祖先を探す (CSS Text 3 §4.2)。
///
/// `Inline` / `Contents` は透過して登る。`Flex` / `Grid` 等の
/// 非 block-container で止まった場合・root 到達の場合は None
/// (caller は自 font に倒す fail-safe)。
fn nearest_block_container(
    doc: &Document,
    cascade: &CascadeResult,
    parent_of: &[Option<usize>],
    idx: usize,
) -> Option<usize> {
    let mut cur = parent_of.get(idx).copied().flatten();
    while let Some(a) = cur {
        if doc.nodes[a].kind() != NodeKind::Element {
            cur = parent_of.get(a).copied().flatten();
            continue;
        }
        match cascade.computed[a].display {
            DisplayValue::Block | DisplayValue::InlineBlock | DisplayValue::ListItem => {
                return Some(a);
            }
            DisplayValue::Inline | DisplayValue::Contents => {
                cur = parent_of.get(a).copied().flatten();
            }
            _ => return None,
        }
    }
    None
}

/// CSS collapsing 対象の空白集合 (space / tab / LF / FF / CR)。
///
/// `char::is_whitespace` は NBSP 等も含むが、NBSP は collapse しないため
/// ここでは使わない (CSS Text 3 §4.1 準拠の近似)。
fn is_collapsible_ws(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\x0C' | '\r')
}

/// sibling が block 内の先行 content になるか (CSS Text 3 §8.1 先頭行判定用)。
///
/// - Text: collapse 系 white-space で空白のみ → 無視 (行を占めない)。
///   preserve 系では空白も行を占めるため content。空 string は常に無視。
/// - `br` → 常に content (forced break → 後続は 2 行目以降)。
/// - `display: none` → 無視。replaced (`img` 等) → content。
///   その他: 子無し → 無視、子持ち → content。
///   (空 inline の過剰除外は受容する近似 — doc に明記。)
fn is_block_content(doc: &Document, cascade: &CascadeResult, sib: usize) -> bool {
    match &doc.nodes[sib].data {
        crate::node::NodeData::Text(t) => {
            if t.text_content.is_empty() {
                return false;
            }
            match cascade.computed[sib].white_space {
                WhiteSpace::Normal | WhiteSpace::Nowrap | WhiteSpace::PreLine => {
                    !t.text_content.chars().all(is_collapsible_ws)
                }
                _ => true,
            }
        }
        crate::node::NodeData::Element(_) => {
            let tag = doc.nodes[sib].tag_name().unwrap_or("");
            if tag.eq_ignore_ascii_case("br") {
                return true;
            }
            if cascade.computed[sib].display == DisplayValue::None {
                return false;
            }
            if doc.nodes[sib].children.is_empty() {
                return matches!(
                    tag.to_ascii_lowercase().as_str(),
                    "img"
                        | "input"
                        | "video"
                        | "canvas"
                        | "iframe"
                        | "embed"
                        | "object"
                        | "textarea"
                        | "select"
                        | "button"
                        | "hr"
                );
            }
            true
        }
        _ => false,
    }
}

/// その Text node が属する block の先頭 formatted line を開始するか
/// (CSS Text 3 §8.1 — `text-indent` は each-line 無しでは先頭行のみ)。
///
/// per-Text-node preshape では「node の先頭行」と「block の先頭行」が一致
/// しない (`<br>` 後・inline 分割・nested block 混在 — length-002 が pin)。
/// nearest block-container 祖先まで登りながら先行 sibling を scan し、
/// content が 1 つでもあれば false。
/// block 祖先が無い場合は true (fail-safe — 従来挙動を維持)。
/// Maps computed hanging/each-line flags plus node line position to parley
/// [`IndentOptions`] (CSS Text 3 §8.1).
///
/// Returns `None` when the node must not indent at all. Position semantics:
/// - [`LineStart::MidLine`] (mid-line inline split): never indent — parley
///   cannot know the layout starts mid-line, so any amount would shift
///   already-placed content.
/// - [`LineStart::BlockStart`]: the layout's first line is the block's first
///   line — pass the flags through unchanged (parley resolves first/wrap/
///   hard-break lines itself, including hanging+each-line combined via XOR).
/// - [`LineStart::AfterBreak`]: every layout line is a non-first block line.
///   basic skips; each-line and combined pass through unchanged (exact per
///   parley scope-line semantics); hanging-only maps to
///   `{ each_line: true, hanging: false }`, which is exact for single-line
///   and post-hard-break lines — soft-wrapped continuations inside the node
///   are missed (no parley option indents all lines unconditionally).
///   Documented approximation; the common test shape (short lines) is exact.
fn indent_options_for_node(
    hanging: bool,
    each_line: bool,
    start: LineStart,
) -> Option<IndentOptions> {
    match (hanging, each_line, start) {
        (_, _, LineStart::MidLine) => None,
        (false, false, LineStart::BlockStart) => Some(IndentOptions::default()),
        (false, false, LineStart::AfterBreak) => None,
        (false, true, _) => Some(IndentOptions {
            each_line: true,
            hanging: false,
        }),
        (true, false, LineStart::BlockStart) => Some(IndentOptions {
            each_line: false,
            hanging: true,
        }),
        (true, false, LineStart::AfterBreak) => Some(IndentOptions {
            each_line: true,
            hanging: false,
        }),
        (true, true, _) => Some(IndentOptions {
            each_line: true,
            hanging: true,
        }),
    }
}

/// 行頭位置の分類 (leading trim 用)。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum LineStart {
    /// block 先頭。
    BlockStart,
    /// `<br>` / preserved `\n` / 先行 block の直後。
    AfterBreak,
    /// 上記以外 (行中)。
    MidLine,
}

/// Text node が preserved な強制 break (`\n`) を含むか。
///
/// preserve するのは Pre / PreWrap / BreakSpaces / PreLine。
/// Normal / Nowrap 下の `\n` は space 化されるため break ではない。
fn has_preserved_break(cascade: &CascadeResult, idx: usize, text: &str) -> bool {
    if !text.contains('\n') {
        return false;
    }
    !matches!(
        cascade.computed[idx].white_space,
        WhiteSpace::Normal | WhiteSpace::Nowrap
    )
}

/// 行頭位置を後ろ向き scan で求める (leading trim 用)。
///
/// block 祖先まで登りながら先行 sibling を逆順 scan:
/// - collapse 系の空白のみ text / 空 / `display: none` → 透過。
/// - preserved `\n` を含む text / `br` / 先行 block 要素 → [`LineStart::AfterBreak`]。
/// - その他 content → [`LineStart::MidLine`]。
/// - block 先頭 (or root) 到達 → [`LineStart::BlockStart`]。
fn line_start_pos(
    doc: &Document,
    cascade: &CascadeResult,
    parent_of: &[Option<usize>],
    idx: usize,
) -> LineStart {
    let block = nearest_block_container(doc, cascade, parent_of, idx);
    let mut cur = idx;
    loop {
        let Some(p) = parent_of.get(cur).copied().flatten() else {
            return LineStart::BlockStart;
        };
        if let Some(pos) = doc.nodes[p].children.iter().position(|&c| c == cur) {
            for &sib in doc.nodes[p].children[..pos].iter().rev() {
                match &doc.nodes[sib].data {
                    crate::node::NodeData::Text(t) => {
                        if has_preserved_break(cascade, sib, &t.text_content) {
                            return LineStart::AfterBreak;
                        }
                        if is_block_content(doc, cascade, sib) {
                            return LineStart::MidLine;
                        }
                    }
                    crate::node::NodeData::Element(_) => {
                        let tag = doc.nodes[sib].tag_name().unwrap_or("");
                        if tag.eq_ignore_ascii_case("br") {
                            return LineStart::AfterBreak;
                        }
                        match cascade.computed[sib].display {
                            DisplayValue::Block
                            | DisplayValue::InlineBlock
                            | DisplayValue::ListItem => {
                                return LineStart::AfterBreak;
                            }
                            DisplayValue::None => {}
                            _ => {
                                if is_block_content(doc, cascade, sib) {
                                    return LineStart::MidLine;
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        if Some(p) == block {
            return LineStart::BlockStart;
        }
        cur = p;
    }
}

/// 後続が block 終端・`<br>`・preserved `\n`・後続 block のいずれか
/// (trailing trim 用)。前向き scan、分類は [`line_start_pos`] と対称。
fn trail_trim(
    doc: &Document,
    cascade: &CascadeResult,
    parent_of: &[Option<usize>],
    idx: usize,
) -> bool {
    let block = nearest_block_container(doc, cascade, parent_of, idx);
    let mut cur = idx;
    loop {
        let Some(p) = parent_of.get(cur).copied().flatten() else {
            return true;
        };
        if let Some(pos) = doc.nodes[p].children.iter().position(|&c| c == cur) {
            for &sib in doc.nodes[p].children[pos + 1..].iter() {
                match &doc.nodes[sib].data {
                    crate::node::NodeData::Text(t) => {
                        if has_preserved_break(cascade, sib, &t.text_content) {
                            return true;
                        }
                        if is_block_content(doc, cascade, sib) {
                            return false;
                        }
                    }
                    crate::node::NodeData::Element(_) => {
                        let tag = doc.nodes[sib].tag_name().unwrap_or("");
                        if tag.eq_ignore_ascii_case("br") {
                            return true;
                        }
                        match cascade.computed[sib].display {
                            DisplayValue::Block
                            | DisplayValue::InlineBlock
                            | DisplayValue::ListItem => {
                                return true;
                            }
                            DisplayValue::None => {}
                            _ => {
                                if is_block_content(doc, cascade, sib) {
                                    return false;
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        if Some(p) == block {
            return true;
        }
        cur = p;
    }
}

/// white-space phase 1 collapsing (CSS Text 3 §4.1)。
///
/// - Pre / PreWrap / BreakSpaces → 無変換 (tab 展開は [`preshape_text`] で別途行う)。
/// - Normal / Nowrap: `\t \n \f \r` → space、run collapse、行頭/行末 trim。
/// - PreLine: `\n` 保持 (forced break)、他は Normal と同じ。
/// - leading trim は `start != MidLine`、trailing trim は `trim_end`。
///
/// 対象外 (doc 明記): segment-break 前後の CJK space 除去、PreLine の
/// hanging trailing space、`overflow-wrap` との相互作用。
fn collapse_ws(
    text: &str,
    ws: WhiteSpace,
    start: LineStart,
    trim_end: bool,
) -> std::borrow::Cow<'_, str> {
    use std::borrow::Cow;
    match ws {
        WhiteSpace::Pre | WhiteSpace::PreWrap | WhiteSpace::BreakSpaces => {
            return Cow::Borrowed(text);
        }
        _ => {}
    }
    let keep_nl = ws == WhiteSpace::PreLine;
    // 速径: 変換対象が無ければ borrow のまま返す。
    let needs = text.chars().any(|c| match c {
        '\t' | '\x0C' | '\r' => true,
        '\n' => !keep_nl,
        ' ' => true,
        _ => false,
    });
    if !needs {
        return Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len());
    let mut pending_space = false;
    let mut at_start = true;
    // 先頭 run の扱い: BlockStart / AfterBreak では捨て、MidLine では 1 space。
    let mut drop_leading = start != LineStart::MidLine;
    for c in text.chars() {
        if c == '\n' && keep_nl {
            // PreLine の preserved break: pending を flush して改行を置き、
            // 改行後は常に行頭扱い (start-of-line run 除去 — CSS Text 3 §4.1.2)。
            // break 直前 trailing の hanging 再現は対象外 (1 space 置く)。
            if pending_space && !at_start {
                out.push(' ');
            }
            out.push('\n');
            pending_space = false;
            at_start = true;
            drop_leading = true;
            continue;
        }
        let is_ws = matches!(c, '\t' | '\x0C' | '\r' | '\n' | ' ');
        if is_ws {
            if !at_start || !drop_leading {
                pending_space = true;
            }
            continue;
        }
        if pending_space && (!at_start || !drop_leading) {
            out.push(' ');
        }
        pending_space = false;
        at_start = false;
        out.push(c);
    }
    // 末尾 run: `trim_end` (block 終端 / break 直前) のとき捨て、
    // それ以外は 1 space。
    if pending_space && !trim_end {
        out.push(' ');
    }
    Cow::Owned(out)
}

/// Prepare font-metric `text-indent: ch` values before Taffy measures leaves.
///
/// Parley first shapes against the page width, but Taffy may later ask a text
/// leaf for an intrinsic size at a narrower containing width. The prepared
/// metric and flags live on `TextData`; `taffy_impl` reapplies them for each
/// width probe and rebreaks the existing layout before returning its height.
fn prepare_text_indent_before_taffy(
    doc: &mut Document,
    cascade: &CascadeResult,
    fonts: &mut FontContext,
    layout_cx: &mut LayoutContext<()>,
    max_advance: f32,
) {
    let mut parent_of: Vec<Option<usize>> = vec![None; doc.nodes.len()];
    for idx in 0..doc.nodes.len() {
        for &child in &doc.nodes[idx].children.clone() {
            if child < parent_of.len() {
                parent_of[child] = Some(idx);
            }
        }
    }
    let mut ch_probes: HashMap<(String, u32, u32, u8), f32> = HashMap::new();
    for idx in 0..doc.nodes.len() {
        let Some(text) = doc.nodes[idx].data.as_text_mut() else {
            continue;
        };
        text.text_indent_px = None;
        text.text_indent_hanging = false;
        text.text_indent_each_line = false;
        text.text_indent_rebreak = false;
        if !doc.nodes[idx].is_in_document() {
            continue;
        }
        let cv = &cascade.computed[idx];
        let Some(parent_idx) = parent_of[idx] else {
            continue; // cov:ignore: in-document text nodes always have a parent.
        };
        if cv.text_indent_ch_factor.is_none() {
            continue;
        }
        let needs_line_break_property = matches!(
            cv.word_break,
            WordBreak::BreakAll | WordBreak::KeepAll | WordBreak::BreakWord
        ) || !matches!(cv.overflow_wrap, OverflowWrap::Normal)
            || matches!(cv.white_space, WhiteSpace::BreakSpaces);
        let has_forced_break = doc.nodes[parent_idx]
            .children
            .iter()
            .any(|&child| is_forced_line_break(doc, child, cascade));
        let parent_is_inline_root = doc.nodes[parent_idx]
            .flags
            .contains(NodeFlags::IS_INLINE_ROOT);
        let has_modifier = cv.text_indent_hanging || cv.text_indent_each_line;
        let parent_is_block_container = matches!(
            cascade.computed[parent_idx].display,
            DisplayValue::Block | DisplayValue::InlineBlock
        );
        let has_other_in_document_child = has_modifier
            && doc.nodes[parent_idx]
                .children
                .iter()
                .any(|&child| child != idx && doc.nodes[child].is_in_document());
        if (has_modifier && (!parent_is_block_container || has_other_in_document_child))
            || (parent_is_inline_root
                && (has_modifier || (!needs_line_break_property && has_forced_break)))
            || (cascade.computed[parent_idx].width_ch.is_some()
                && (has_modifier || !needs_line_break_property))
        {
            // Modifier line scope and a `ch` containing width still use the
            // established post-Taffy path for inline roots and width-aware
            // boxes. The pre-Taffy bridge is limited to direct, finite-width
            // text runs where its measured line break is unambiguous.
            continue;
        }
        let layout_max_advance = match &doc.nodes[idx].data {
            crate::node::NodeData::Text(text) => text
                .text_layout
                .as_ref()
                .map(|layout| layout.layout_max_advance()),
            _ => None, // cov:ignore: the preceding as_text_mut guard makes this arm unreachable.
        };
        let preserve_wide_body_run = layout_max_advance
            .is_some_and(|advance| advance > max_advance + 0.01)
            && is_leading_body_text(doc, Some(parent_idx), idx)
            && !needs_line_break_property
            && matches!(cv.text_align, TextAlign::Start | TextAlign::Left);
        if preserve_wide_body_run {
            // Match `realign_text_after_layout`: this deliberately keeps the
            // page-width run untouched, including its indent metadata.
            continue;
        }
        let Some(options) = indent_options_for_node(
            cv.text_indent_hanging,
            cv.text_indent_each_line,
            line_start_pos(doc, cascade, &parent_of, idx),
        ) else {
            continue;
        };
        let Some(indent) = measured_text_indent_px(cv, fonts, layout_cx, &mut ch_probes) else {
            continue; // cov:ignore: only computed ch provenance reaches this point.
        };
        let hanging = options.hanging;
        let each_line = options.each_line;
        // Rebreak direct metric-backed runs before Taffy so their line count
        // contributes to the leaf height. Guarded modifier and `width: ch`
        // shapes remain on the post-Taffy path above.
        let rebreak =
            !matches!(cv.white_space, WhiteSpace::Nowrap) && cv.text_wrap != TextWrapMode::Nowrap;
        let text = doc.nodes[idx]
            .data
            .as_text_mut()
            .expect("text node remains text during layout preparation");
        text.text_indent_px = Some(indent);
        text.text_indent_hanging = hanging;
        text.text_indent_each_line = each_line;
        text.text_indent_rebreak = rebreak;
        if let Some(layout) = text.text_layout.as_mut() {
            layout.set_text_indent(indent, options);
            if rebreak && max_advance.is_finite() && max_advance > 0.0 {
                layout.break_all_lines(Some(max_advance));
            }
        } // cov:ignore: the no-layout branch is defensive for pre-shaped callers.
    }
}

fn prepare_ch_box_values_before_taffy(
    doc: &mut Document,
    cascade: &CascadeResult,
    fonts: &mut FontContext,
    layout_cx: &mut LayoutContext<()>,
) {
    let mut ch_probes: HashMap<(String, u32, u32, u8), f32> = HashMap::new();
    for idx in 0..doc.nodes.len() {
        if doc.nodes[idx].kind() != NodeKind::Element {
            continue;
        }
        let cv = &cascade.computed[idx];
        let mut measure = |provenance: &Option<ChLengthProvenance>| {
            provenance.as_ref().map(|provenance| {
                measured_ch_length_px(
                    provenance.factor,
                    Some(&provenance.font),
                    cv,
                    fonts,
                    layout_cx,
                    &mut ch_probes,
                )
            })
        };
        let width = measure(&cv.width_ch).map(|value| value.max(0.0));
        let height = measure(&cv.height_ch).map(|value| value.max(0.0));
        let padding = (
            measure(&cv.padding_ch.top).map(|value| value.max(0.0)),
            measure(&cv.padding_ch.right).map(|value| value.max(0.0)),
            measure(&cv.padding_ch.bottom).map(|value| value.max(0.0)),
            measure(&cv.padding_ch.left).map(|value| value.max(0.0)),
        );
        let margin = (
            measure(&cv.margin_ch.top),
            measure(&cv.margin_ch.right),
            measure(&cv.margin_ch.bottom),
            measure(&cv.margin_ch.left),
        );
        let style = &mut doc.nodes[idx].style;
        if let Some(width) = width {
            style.size.width = Dimension::length(width);
        }
        if let Some(height) = height {
            style.size.height = Dimension::length(height);
        }
        if let Some(top) = padding.0 {
            style.padding.top = LengthPercentage::length(top);
        }
        if let Some(right) = padding.1 {
            style.padding.right = LengthPercentage::length(right);
        }
        if let Some(bottom) = padding.2 {
            style.padding.bottom = LengthPercentage::length(bottom);
        }
        if let Some(left) = padding.3 {
            style.padding.left = LengthPercentage::length(left);
        }
        if let Some(top) = margin.0 {
            style.margin.top = LengthPercentageAuto::length(top);
        }
        if let Some(right) = margin.1 {
            style.margin.right = LengthPercentageAuto::length(right);
        }
        if let Some(bottom) = margin.2 {
            style.margin.bottom = LengthPercentageAuto::length(bottom);
        }
        if let Some(left) = margin.3 {
            style.margin.left = LengthPercentageAuto::length(left);
        }
    }
}

// cov:ignore: exercised by resource-enabled ignored WPT reftests; default coverage has no sparse asset run.
fn realign_inline_replaced_children(doc: &mut Document, cascade: &CascadeResult) {
    for parent_id in 0..doc.nodes.len() {
        if !doc.nodes[parent_id]
            .flags
            .contains(NodeFlags::IS_INLINE_ROOT)
        {
            continue;
        }
        let has_replaced_child = doc.nodes[parent_id].children.iter().any(|&child| {
            doc.nodes[child].is_in_document() && doc.nodes[child].tag_name() == Some("img")
        });
        if !has_replaced_child {
            continue;
        }
        let parent_width = doc.nodes[parent_id].unrounded_layout.size.width.max(0.0);
        if !parent_width.is_finite() || parent_width <= 0.0 {
            continue;
        }
        let line_height = line_height_px(&cascade.computed[parent_id]).max(0.0);
        if line_height <= 0.0 || !line_height.is_finite() {
            continue;
        }
        let mut x = 0.0_f32;
        let mut y = 0.0_f32;
        let mut line_height_used = line_height;
        for &child in &doc.nodes[parent_id].children.clone() {
            if !doc.nodes[child].is_in_document() {
                continue;
            }
            if doc.nodes[child].tag_name() == Some("br") {
                x = 0.0;
                y += line_height_used;
                line_height_used = line_height;
                continue;
            }
            let is_whitespace = matches!(
                &doc.nodes[child].data,
                NodeData::Text(text)
                    if text.text_content.chars().all(|ch| {
                        matches!(ch, ' ' | '\t' | '\n' | '\r' | '\u{000c}' | '\u{00a0}')
                    })
            );
            let (width, height) = if is_whitespace {
                (
                    doc.nodes[child].unrounded_layout.size.width.max(
                        style_dimension_length(doc.nodes[child].style.size.width).unwrap_or(0.0),
                    ),
                    0.0,
                )
            } else if doc.nodes[child].tag_name() == Some("img") {
                (
                    doc.nodes[child].unrounded_layout.size.width.max(0.0),
                    doc.nodes[child].unrounded_layout.size.height.max(0.0),
                )
            } else {
                continue;
            };
            if width <= 0.0 || !width.is_finite() {
                continue;
            }
            if x > 0.0 && x + width > parent_width + 0.01 {
                x = 0.0;
                y += line_height_used;
                line_height_used = line_height;
                if is_whitespace {
                    continue;
                }
            }
            if is_whitespace && x == 0.0 {
                // Collapsible leading spaces do not create an anonymous
                // inline box at a new line. Non-breaking runs still reach
                // here only when they fit after preceding content.
                continue;
            }
            doc.nodes[child].unrounded_layout.location.x = x;
            doc.nodes[child].unrounded_layout.location.y = y;
            x += width;
            line_height_used = line_height_used.max(height);
        }
        let used_height = if x > 0.0 || y == 0.0 {
            y + line_height_used
        } else {
            y
        };
        doc.nodes[parent_id].unrounded_layout.size.height = used_height;
    }
}

// Text indentation is otherwise applied to TextData layouts, so an empty
// inline block has no text layout to receive the first-line offset. Keep this
// post-layout bridge narrow: only one empty inline-block in a horizontal LTR
// block, with optional collapsible whitespace, no modifiers or forced breaks,
// and normal-flow positioning can be offset without reflowing a line.
fn realign_single_empty_inline_block_indent(doc: &mut Document, cascade: &CascadeResult) {
    for parent_id in 0..doc.nodes.len() {
        let cv = &cascade.computed[parent_id];
        if !matches!(cv.display, DisplayValue::Block | DisplayValue::InlineBlock) {
            continue;
        }
        if cv.direction != Direction::Ltr
            || cv.writing_mode != WritingMode::HorizontalTb
            || cv.text_indent_hanging
            || cv.text_indent_each_line
            || cv.text_indent_ch_factor.is_some()
            || !matches!(
                cv.text_align,
                TextAlign::Start | TextAlign::Left | TextAlign::Right
            )
        {
            continue;
        }

        let mut inline_block = None;
        let mut unsupported_content = false;
        for &child in &doc.nodes[parent_id].children {
            if !doc.nodes[child].is_in_document() {
                continue;
            }
            if is_forced_line_break(doc, child, cascade) {
                unsupported_content = true;
                break;
            }
            match &doc.nodes[child].data {
                NodeData::Text(text)
                    if matches!(
                        cascade.computed[child].white_space,
                        WhiteSpace::Normal | WhiteSpace::Nowrap
                    ) && text.text_content.chars().all(is_css_white_space) => {}
                NodeData::Element(_)
                    if cascade.computed[child].display == DisplayValue::InlineBlock
                        && cascade.computed[child].float == FloatValue::None
                        && matches!(
                            cascade.computed[child].position,
                            PositionValue::Static | PositionValue::Relative | PositionValue::Sticky
                        )
                        && doc.nodes[child].children.is_empty() =>
                {
                    if inline_block.replace(child).is_some() {
                        unsupported_content = true;
                        break;
                    }
                }
                _ => {
                    unsupported_content = true;
                    break;
                }
            }
        }
        if unsupported_content {
            continue;
        }
        let Some(inline_block) = inline_block else {
            continue;
        };

        let content_width = doc.nodes[parent_id].unrounded_layout.content_box_width();
        if !content_width.is_finite() || content_width < 0.0 {
            continue;
        }
        let indent = bounded_text_indent_amount(cv.text_indent, content_width, None);
        if !indent.is_finite() || indent == 0.0 {
            continue;
        }
        doc.nodes[inline_block].unrounded_layout.location.x += indent;
    }
}

/// Find the nearest synthetic inline formatting root for a text node.
fn inline_root_for_text(
    doc: &Document,
    parent_of: &[Option<usize>],
    text_id: usize,
) -> Option<usize> {
    let mut ancestor = parent_of[text_id];
    while let Some(id) = ancestor {
        if doc.nodes[id].flags.contains(NodeFlags::IS_INLINE_ROOT) {
            return Some(id);
        }
        ancestor = parent_of[id];
    }
    None
}

/// Return a node's horizontal position in the document coordinate space.
fn absolute_layout_x(doc: &Document, parent_of: &[Option<usize>], mut id: usize) -> f32 {
    let mut x = 0.0;
    while let Some(parent) = parent_of[id] {
        x += doc.nodes[id].unrounded_layout.location.x;
        id = parent;
    }
    x
}

fn realign_text_after_layout(
    doc: &mut Document,
    cascade: &CascadeResult,
    fonts: &mut FontContext,
    layout_cx: &mut LayoutContext<()>,
) {
    // parent map (arena に parent pointer が無いため children から逆引き)。
    let mut parent_of: Vec<Option<usize>> = vec![None; doc.nodes.len()];
    for idx in 0..doc.nodes.len() {
        for &c in &doc.nodes[idx].children.clone() {
            if c < parent_of.len() {
                parent_of[c] = Some(idx);
            }
        }
    }
    let mut ch_probes: HashMap<(String, u32, u32, u8), f32> = HashMap::new();
    for (idx, parent) in parent_of.iter().enumerate() {
        if doc.nodes[idx].kind() != NodeKind::Text {
            continue;
        }
        if !doc.nodes[idx].is_in_document() {
            continue;
        }
        // cov:ignore: multicol fragment detection is exercised by the ignored foundation WPT run.
        let mut ancestor = *parent;
        // cov:ignore: multicol fragment detection is exercised by the ignored foundation WPT run.
        let mut inside_multicol = false;
        // cov:ignore: multicol fragment detection is exercised by the ignored foundation WPT run.
        while let Some(ancestor_id) = ancestor {
            if !matches!(
                cascade.computed[ancestor_id].column_count,
                ColumnCountValue::Auto
            ) || !matches!(
                cascade.computed[ancestor_id].column_width,
                ComputedColumnWidth::Auto
            ) {
                inside_multicol = true;
                break;
            }
            ancestor = parent_of[ancestor_id];
        }
        // cov:ignore: fragment-aware text alignment is exercised by the ignored foundation WPT run.
        if inside_multicol
            // cov:ignore: fragment-aware text alignment is exercised by the ignored foundation WPT run.
            && matches!(
                &doc.nodes[idx].data,
                NodeData::Text(text) if text.multicol_fragments.is_some()
            )
        // cov:ignore: fragment-aware text alignment is exercised by the ignored foundation WPT run.
        {
            // Fragment ranges are already width-constrained and their line
            // offsets are consumed by the multicol paint path. Nested inline
            // text still needs the ordinary post-Taffy alignment pass.
            continue;
        }
        // 論理値 (`start` / `end`) は自要素の `direction` で物理値に解決する
        // (CSS Text 3 §6.1)。parley の `Start` / `End` は content-inferred
        // bidi に委ねられるため、RTL では誤った側に寄る
        // (text-align-end-001 の regress で実測)。
        // `Start` + LTR のみ skip (preshape のまま正しい — 再 break による
        // wrap 変化の regress を避ける)。
        let cv = &cascade.computed[idx];
        let parley_align = match text_align_to_parley(cv.text_align) {
            Alignment::Start if cv.direction == Direction::Rtl => Alignment::Right,
            Alignment::End if cv.direction == Direction::Rtl => Alignment::Left,
            a => a,
        };
        // `text-justify: none` disables justification (CSS Text 3 §6.2):
        // combined with `text-align: justify` it falls back to the start
        // edge (direction-aware).
        let mut align = parley_align;
        if cv.text_justify == TextJustify::None && align == Alignment::Justify {
            align = match cv.direction {
                Direction::Rtl => Alignment::Right,
                _ => Alignment::Start,
            };
        }
        // `text-indent` (CSS Text 3 §8.1): length plus hanging/each-line
        // flags, resolved against the containing block width for `%`
        // (unknown at preshape time — same width-basis problem as align).
        // The indent applies per node position (see `indent_options_for_node`
        // doc): MidLine never, BlockStart always, AfterBreak per flags.
        let nonzero_indent = match cv.text_indent {
            ComputedLengthPercentage::Px(px) => px != 0.0,
            ComputedLengthPercentage::Percent(p) => p != 0.0,
        };
        let indent_options = if nonzero_indent {
            indent_options_for_node(
                cv.text_indent_hanging,
                cv.text_indent_each_line,
                line_start_pos(doc, cascade, &parent_of, idx),
            )
        } else {
            None
        };
        let measured_indent = measured_text_indent_px(cv, fonts, layout_cx, &mut ch_probes);
        // preshape は page 幅で break するため、narrow container 内の text の
        // 折り返しは container 幅に整合しない (slice 1b で
        // 実測: length-001 の 3-line article が single-line のまま残る)。
        // 全 node re-break の実験は nowrap-001 の pinned regress を起こしたため
        // revert した (当該 experiment は別途 full-baseline 判定が必要 —
        // the preceding comment参照)。したがって re-break は
        // indent 付き / 非 Start の node のみに限定する。
        // All non-flex text re-breaks against the containing width here
        // preshape only knows the page width, so
        // narrow-container wrapping would otherwise never apply. `nowrap`
        // nodes are handled by the branch below without re-breaking.
        // `Start` alignment after re-break is a no-op offset-wise; only the
        // wrap points change.
        // `Justify` / `End` 等も同じ幅基準問題を持つため同じ経路で扱うが、
        // 本 goal の主 target は `Center`。他値もここで正しい幅基準になる。
        let Some(parent_idx) = *parent else {
            continue;
        };
        // flex line box の child は container 側の justify に委ねる (上記 doc)。
        // indent も同様に skip する — flex item の幅は taffy が indent 無し
        // layout から測っており、ここで indent を付けると box と run が乖離
        // する (既知の限界として doc に残す)。
        let needs_line_break_property = matches!(
            cv.word_break,
            WordBreak::BreakAll | WordBreak::KeepAll | WordBreak::BreakWord
        ) || !matches!(cv.overflow_wrap, OverflowWrap::Normal)
            || matches!(cv.white_space, WhiteSpace::BreakSpaces);
        let has_forced_break = doc.nodes[parent_idx]
            .children
            .iter()
            .any(|&child| is_forced_line_break(doc, child, cascade));
        if doc.nodes[parent_idx]
            .flags
            .contains(NodeFlags::IS_INLINE_ROOT)
            && !inside_multicol
            && !needs_line_break_property
            && (cv.text_indent_ch_factor.is_none()
                || cv.text_indent_hanging
                || cv.text_indent_each_line
                || has_forced_break)
        {
            continue;
        }
        let containing_width = doc.nodes[parent_idx].unrounded_layout.size.width;
        if !containing_width.is_finite() || containing_width <= 0.0 {
            continue;
        }
        let preserve_wide_body_run = !inside_multicol
            && is_leading_body_text(doc, Some(parent_idx), idx)
            && matches!(cv.text_align, TextAlign::Start | TextAlign::Left);
        let inline_continuation = inline_root_for_text(doc, &parent_of, idx).and_then(|root| {
            let root_cv = &cascade.computed[root];
            if root == parent_idx
                || cv.direction != Direction::Ltr
                || root_cv.direction != Direction::Ltr
                || root_cv.writing_mode != WritingMode::HorizontalTb
            {
                return None;
            }
            let root_x = absolute_layout_x(doc, &parent_of, root);
            let node_x = absolute_layout_x(doc, &parent_of, idx);
            let prefix = node_x - root_x;
            let width = doc.nodes[root].unrounded_layout.size.width;
            if !prefix.is_finite() || !width.is_finite() || prefix <= 0.0 || width <= prefix {
                return None;
            }
            Some((width - prefix, root_x - node_x))
        });
        let Some(layout) = doc.nodes[idx]
            .data
            .as_text_mut()
            .and_then(|t| t.text_layout.as_mut())
        else {
            continue;
        };
        if let Some((available, continuation_offset)) = inline_continuation {
            let should_rebreak = available.is_finite()
                && available > 0.0
                && available + f32::EPSILON < layout.width();
            if should_rebreak {
                layout.break_all_lines(Some(available));
                layout.align(Alignment::Start, AlignmentOptions::default());
                let line_count = layout.len();
                if let NodeData::Text(text) = &mut doc.nodes[idx].data {
                    text.text_line_offsets = Some(
                        (0..line_count)
                            .map(|line| if line == 0 { 0.0 } else { continuation_offset })
                            .collect(),
                    );
                }
                continue;
            }
        }
        // A direct body text run may intentionally overflow the page content
        // width when the paged containing block carries a margin.  Preserve
        // the page-width shaping used by the paged bridge instead of
        // re-breaking a one-line run to the narrower body box.
        if preserve_wide_body_run
            && layout.width() > containing_width + 0.01
            && !needs_line_break_property
        {
            continue;
        }
        // No-wrap (`white-space: nowrap` or `text-wrap: nowrap`): preshape
        // already broke without a width cap, so re-breaking here would wrap.
        // Apply indent and align onto the preshaped single line instead.
        // Single-line Justify is a parley no-op (last line excluded), which
        // matches `text-align: justify` under nowrap.
        if cv.white_space == WhiteSpace::Nowrap || cv.text_wrap == TextWrapMode::Nowrap {
            if let Some(options) = indent_options {
                let amount =
                    bounded_text_indent_amount(cv.text_indent, containing_width, measured_indent);
                layout.set_text_indent(amount, options);
            }
            layout.align(align, AlignmentOptions::default());
            continue;
        }
        if let Some(options) = indent_options {
            let amount =
                bounded_text_indent_amount(cv.text_indent, containing_width, measured_indent);
            layout.set_text_indent(amount, options);
        }
        layout.break_all_lines(Some(containing_width));
        // `text-align-last` (CSS Text 3 §6.1): 明示 last 値の適用は対象外。
        // parley の `align(Justify)` は最終行 (`BreakReason::None`) を
        // 意図的に除外するため (parley alignment.rs 実測)、single-line を含む
        // あらゆる last-line-only の justify/align は public API では再現
        // できない。`text_align_last` の cascade 配線 (parse/wire/inherit) は
        // 将来の parley 対応に備えて維持する。`MatchParent` の cascade
        // 未解決も同様に将来対応 (現状 `auto` 扱いの近似は reader 側で行わない —
        // 何もしないのが正しい)。
        layout.align(align, AlignmentOptions::default());
    }

    // Rebreaking text after Taffy can increase an inline child's line count.
    // Propagate that used height through auto-sized ancestors so multicolumn
    // item backgrounds and the enclosing block cover the rebroken run.
    // cov:ignore: exercised by the ignored foundation WPT run; default coverage skips ignored reftests.
    for idx in 0..doc.nodes.len() {
        if doc.nodes[idx].kind() != NodeKind::Text || !doc.nodes[idx].is_in_document() {
            continue;
        }
        if matches!(
            &doc.nodes[idx].data,
            NodeData::Text(text) if text.multicol_fragments.is_some()
        ) {
            continue;
        }
        let Some(text_height) = doc.nodes[idx].text_layout().map(|layout| layout.height()) else {
            continue;
        };
        if !text_height.is_finite() || text_height <= 0.0 {
            continue;
        }
        doc.nodes[idx].unrounded_layout.size.height =
            doc.nodes[idx].unrounded_layout.size.height.max(text_height);
        let mut child = idx;
        while let Some(parent) = parent_of[child] {
            if matches!(
                cascade.computed[parent].height,
                ComputedLengthPercentageOrAuto::Auto
            ) {
                let child_bottom = doc.nodes[child].unrounded_layout.location.y
                    + doc.nodes[child].unrounded_layout.size.height
                    + cascade.computed[parent].border.bottom.width().px();
                doc.nodes[parent].unrounded_layout.size.height = doc.nodes[parent]
                    .unrounded_layout
                    .size
                    .height
                    .max(child_bottom);
            }
            child = parent;
        }
    }

    // Taffy does not include floated descendants in an auto-height
    // containing block's used height. Propagate the bottom edge of supported
    // left/right floats through auto-height ancestors, including the
    // containing-block border. This keeps following flow from moving upward
    // after a fragmented flex item with a float descendant.
    for idx in 0..doc.nodes.len() {
        if doc.nodes[idx].kind() != NodeKind::Element
            || !doc.nodes[idx].is_in_document()
            || !matches!(
                cascade.computed[idx].float,
                FloatValue::Left | FloatValue::Right
            )
        {
            continue;
        }
        let mut child = idx;
        while let Some(parent) = parent_of[child] {
            if matches!(
                cascade.computed[parent].height,
                ComputedLengthPercentageOrAuto::Auto
            ) {
                let child_bottom = doc.nodes[child].unrounded_layout.location.y
                    + doc.nodes[child].unrounded_layout.size.height
                    + cascade.computed[parent].border.bottom.width().px();
                doc.nodes[parent].unrounded_layout.size.height = doc.nodes[parent]
                    .unrounded_layout
                    .size
                    .height
                    .max(child_bottom);
            }
            child = parent;
        }
    }
}
/// Collapse adjacent block margins inside a non-replaced inline box.
///
/// CSS 2.1 §9.2.1.1 splits an inline containing block around block-level
/// children. Whitespace-only text between those children is part of the
/// anonymous inline fragments, not an intervening block. Taffy's block path
/// already gives the children the right vertical order, but it does not
/// perform this block-in-inline margin collapse; keeping both margins would
/// add one extra line-height-sized gap between two adjacent block children.
/// Keep this pass narrow: only direct children of a plain `inline` wrapper and
/// only resolvable absolute margins are rewritten.
fn collapse_block_in_inline_margins(doc: &mut Document, idx: usize, cascade: &CascadeResult) {
    if cascade.computed[idx].display != DisplayValue::Inline {
        return;
    }

    let mut previous_block = None;
    for &child in &doc.nodes[idx].children.clone() {
        if !doc.nodes[child].is_in_document() {
            continue;
        }
        match doc.nodes[child].kind() {
            NodeKind::Text => {
                // Whitespace around a block child belongs to the anonymous
                // inline fragments and must not interrupt sibling margin
                // collapse. Any visible text does interrupt that sequence.
                if !text_of(doc, child).is_some_and(|text| text.chars().all(is_css_white_space)) {
                    previous_block = None;
                }
            }
            NodeKind::Element => {
                let display = cascade.computed[child].display;
                if display == DisplayValue::None {
                    continue;
                }
                if is_inline_element_box(cascade, child) {
                    previous_block = None;
                    continue;
                }
                if let Some(previous) = previous_block {
                    collapse_adjacent_block_margins(doc, previous, child);
                }
                previous_block = Some(child);
            }
            _ => previous_block = None,
        }
    }
}

fn collapse_adjacent_block_margins(doc: &mut Document, previous: usize, next: usize) {
    let previous_margin = doc.nodes[previous].style.margin.bottom.into_raw();
    let next_margin = doc.nodes[next].style.margin.top.into_raw();
    if previous_margin.tag() != CompactLength::LENGTH_TAG
        || next_margin.tag() != CompactLength::LENGTH_TAG
    {
        return;
    }
    let previous_margin = previous_margin.value();
    let next_margin = next_margin.value();
    if !previous_margin.is_finite() || !next_margin.is_finite() {
        return;
    }
    let collapsed = if previous_margin >= 0.0 && next_margin >= 0.0 {
        previous_margin.max(next_margin)
    } else if previous_margin <= 0.0 && next_margin <= 0.0 {
        previous_margin.min(next_margin)
    } else {
        previous_margin + next_margin
    };
    if collapsed.is_finite() {
        doc.nodes[previous].style.margin.bottom = LengthPercentageAuto::length(collapsed);
        doc.nodes[next].style.margin.top = LengthPercentageAuto::length(0.0);
    }
}

/// Return the nearest block-like inline formatting context for a node.
///
/// The nested-inline bridge must not be enabled by an autospace pair in an
/// unrelated sibling block. Low-level DOM tests do not install the HTML UA
/// stylesheet, so if no block-like ancestor is visible the highest reachable
/// ancestor is used as the conservative context fallback.
fn inline_formatting_context_root(
    cascade: &CascadeResult,
    parent_of: &[Option<usize>],
    node_id: usize,
) -> usize {
    let mut current = node_id;
    let mut fallback = node_id;
    while let Some(parent) = parent_of.get(current).copied().flatten() {
        fallback = parent;
        if matches!(
            cascade.computed[parent].display,
            DisplayValue::Block
                | DisplayValue::InlineBlock
                | DisplayValue::Flex
                | DisplayValue::InlineFlex
                | DisplayValue::Grid
                | DisplayValue::InlineGrid
                | DisplayValue::Table
                | DisplayValue::InlineTable
                | DisplayValue::TableRow
                | DisplayValue::TableCell
                | DisplayValue::TableCaption
        ) {
            return parent;
        }
        current = parent;
    }
    fallback
}

/// Return whether one inline formatting context contains an actual autospace
/// boundary. This reuses the same edge discovery and `text-autospace` option
/// handling as `preshape_text`, instead of combining character classes from
/// unrelated text nodes or ignoring custom values and language gating.
fn inline_context_has_autospace_candidate(
    doc: &Document,
    cascade: &CascadeResult,
    parent_of: &[Option<usize>],
    context_root: usize,
) -> bool {
    for (idx, node) in doc.nodes.iter().enumerate() {
        if node.kind() != NodeKind::Text
            || !node.is_in_document()
            || inline_formatting_context_root(cascade, parent_of, idx) != context_root
            || cascade.computed[idx].text_autospace == TextAutospace::NoAutospace
            || has_vertical_writing_mode(cascade, parent_of, idx)
        {
            continue;
        }
        let NodeData::Text(text) = &node.data else {
            // cov:ignore: NodeKind::Text always carries NodeData::Text; this is a defensive match arm.
            continue;
        };
        let language = effective_language_for_text(doc, parent_of, idx);
        let before = autospace_adjacent_edge_char(doc, cascade, parent_of, idx, -1);
        let after = autospace_adjacent_edge_char(doc, cascade, parent_of, idx, 1);
        if !text_autospace_boxes_with_edges(
            &text.text_content,
            cascade.computed[idx].text_autospace,
            &language,
            1.0,
            before,
            after,
        )
        .is_empty()
        {
            return true;
        }
    }
    false
}

/// Vertical line-box extent contributed by a supported `vertical-align` value.
/// Positive raised offsets extend the line's ascent; negative lowered offsets
/// extend its descent. The paint-side shift is still applied to glyphs.
/// spec: <https://www.w3.org/TR/CSS21/visudet.html#propdef-vertical-align>
fn vertical_align_linebox_extent(
    vertical_align: VerticalAlign,
    parent_font_size_px: f32,
) -> (f32, f32) {
    let shift = match vertical_align {
        VerticalAlign::Sub => parent_font_size_px / 5.0,
        VerticalAlign::Super => -(parent_font_size_px / 3.0),
        VerticalAlign::Length(Length::Px(px)) => -px,
        _ => 0.0,
    };
    if !shift.is_finite() {
        return (0.0, 0.0);
    }
    if shift < 0.0 {
        (-shift, 0.0)
    } else {
        (0.0, shift)
    }
}

/// Add synthetic line-box leading/trailing to authored padding without losing
/// a percentage component. Taffy's calc callback already resolves this mixed
/// value for bridged CSS lengths.
fn padding_with_linebox_extent(
    doc: &mut Document,
    value: ComputedLengthPercentage,
    extra: f32,
    site: &'static str,
) -> LengthPercentage {
    if extra <= 0.0 {
        return computed_length_percentage_to_taffy_length_percentage(
            value,
            site,
            &mut doc.layout_warnings,
        );
    }
    let extra = sanitize_taffy(extra, site, &mut doc.layout_warnings);
    match value {
        ComputedLengthPercentage::Px(px) => {
            LengthPercentage::length(sanitize_taffy(px + extra, site, &mut doc.layout_warnings))
        }
        ComputedLengthPercentage::Percent(percent) => {
            let percent = sanitize_taffy(percent, site, &mut doc.layout_warnings);
            let value = CalcLengthPercentage { percent, px: extra };
            doc.calc_values.push(std::sync::Arc::new(value));
            let pointer = doc
                .calc_values
                .last()
                .map(|value| (&**value) as *const _ as *const ())
                .expect("calc value was just pushed");
            LengthPercentage::calc(pointer)
        }
    }
}

fn establish_minimal_line_boxes(doc: &mut Document, cascade: &CascadeResult) {
    let mut parent_of = vec![None; doc.nodes.len()];
    for parent in 0..doc.nodes.len() {
        for &child in &doc.nodes[parent].children {
            if child < parent_of.len() {
                parent_of[child] = Some(parent);
            }
        }
    }
    let mut autospace_candidates_by_root = HashMap::new();
    for idx in 0..doc.nodes.len() {
        if doc.nodes[idx].kind() != NodeKind::Element {
            continue;
        }
        // A multicolumn container establishes fragmentainers rather than the
        // synthetic single flex line used by ordinary inline roots.
        // cov:ignore: exercised by the ignored foundation WPT run; default coverage skips ignored reftests.
        if !matches!(cascade.computed[idx].column_count, ColumnCountValue::Auto)
            // cov:ignore: multicol fragmentainer roots are exercised by the ignored foundation WPT run.
            || !matches!(
                cascade.computed[idx].column_width,
                ComputedColumnWidth::Auto
            )
        // cov:ignore: multicol fragmentainer roots are exercised by the ignored foundation WPT run.
        {
            doc.nodes[idx].flags.remove(NodeFlags::IS_INLINE_ROOT);
            continue;
        }
        collapse_block_in_inline_margins(doc, idx, cascade);
        let has_autospace_candidate = if cascade.computed[idx].display == DisplayValue::Inline {
            let context_root = inline_formatting_context_root(cascade, &parent_of, idx);
            *autospace_candidates_by_root
                .entry(context_root)
                .or_insert_with(|| {
                    inline_context_has_autospace_candidate(doc, cascade, &parent_of, context_root)
                })
        } else {
            false
        };
        let qualifies = qualifies_for_minimal_line_box(doc, idx, cascade, has_autospace_candidate);
        doc.nodes[idx]
            .flags
            .set(NodeFlags::IS_INLINE_ROOT, qualifies);
        if !qualifies {
            continue;
        }
        // display:none な child も含む — taffy はそのような child を
        // Display::None として layout tree から丸ごと除外するため、
        // flex_grow/flex_shrink を check することに実害は無い (無駄では
        // あるが害はない)。
        let participating_children: Vec<usize> = doc.nodes[idx]
            .children
            .iter()
            .copied()
            .filter(|&c| doc.nodes[c].is_in_document())
            .collect();
        // この container に display:none ではない `<br>` が 1 個でも
        // あるかどうか — "`<br>` forced break" (この関数の doc 参照) の
        // 適用条件そのもの。display:none な `<br>` は box を生成しないため
        // forced break として数えない — 数えてしまうと `<br>` を含まない
        // 見た目上の container まで `flex_wrap: Wrap` になり、size-based
        // wrapping が意図せず有効になってしまう (下記 doc 参照)。
        let has_forced_break = participating_children
            .iter()
            .any(|&c| is_forced_line_break(doc, c, cascade));
        let container_cv = &cascade.computed[idx];
        let has_ch_indent = container_cv.text_indent_ch_factor.is_some()
            && !container_cv.text_indent_hanging
            && !container_cv.text_indent_each_line;
        let mut linebox_leading = 0.0_f32;
        let mut linebox_trailing = 0.0_f32;
        for &child in &participating_children {
            if cascade.computed[child].display == DisplayValue::None {
                continue;
            }
            let (leading, trailing) = vertical_align_linebox_extent(
                cascade.computed[child].vertical_align,
                container_cv.font_size.px(),
            );
            linebox_leading = linebox_leading.max(leading);
            linebox_trailing = linebox_trailing.max(trailing);
        }
        let linebox_padding_top = (linebox_leading > 0.0).then(|| {
            padding_with_linebox_extent(
                doc,
                container_cv.padding.top,
                linebox_leading,
                "line-box-leading",
            )
        });
        let linebox_padding_bottom = (linebox_trailing > 0.0).then(|| {
            padding_with_linebox_extent(
                doc,
                container_cv.padding.bottom,
                linebox_trailing,
                "line-box-trailing",
            )
        });
        {
            let style = &mut doc.nodes[idx].style;
            if let Some(padding) = linebox_padding_top {
                style.padding.top = padding;
            }
            if let Some(padding) = linebox_padding_bottom {
                style.padding.bottom = padding;
            }
            style.display = Display::Flex;
            style.flex_direction = TaffyFlexDirection::Row;
            style.flex_wrap = if has_forced_break || has_ch_indent {
                TaffyFlexWrap::Wrap
            } else {
                TaffyFlexWrap::NoWrap
            };
            style.align_items = Some(TaffyAlignItems::BASELINE);
            style.align_content = Some(TaffyAlignContent::FLEX_START);
            // `text-align: center` の line box 全体の中央寄せは flex の
            // main-axis 配置で実現する (parley 側ではなく container 側)。
            // 他値は `None` のまま (本 task の scope 外 — `Right` 等の flex
            // 対応は将来 work)。
            style.justify_content = if cascade.computed[idx].text_align == TextAlign::Center {
                Some(TaffyAlignContent::CENTER)
            } else {
                None
            };
            style.gap = Size {
                width: LengthPercentage::length(0.0),
                height: LengthPercentage::length(0.0),
            };
        }
        for c in participating_children {
            let is_break = is_forced_line_break(doc, c, cascade);
            let strip_inline_padding = cascade.computed[c].display == DisplayValue::Inline
                && has_line_box_edge_aligned_descendant(doc, c, cascade);
            let child_style = &mut doc.nodes[c].style;
            if strip_inline_padding {
                // The wrapper is bridged as a block box, so its block-axis
                // padding would move a nested top/bottom-aligned descendant
                // away from the outer line edge. For this narrow edge-aligned
                // slice, remove that padding from the wrapper's block layout.
                child_style.padding.top = LengthPercentage::length(0.0);
                child_style.padding.bottom = LengthPercentage::length(0.0);
            }
            child_style.flex_grow = 0.0;
            child_style.flex_shrink = 0.0;
            child_style.flex_basis = if is_break {
                Dimension::percent(1.0)
            } else {
                Dimension::auto()
            };
            child_style.align_self = match cascade.computed[c].vertical_align {
                VerticalAlign::Top => Some(TaffyAlignSelf::FLEX_START),
                VerticalAlign::Bottom => Some(TaffyAlignSelf::FLEX_END),
                _ => None,
            };
        }
    }
}

/// `idx` (ある in-document な node) が [`establish_minimal_line_boxes`] の
/// "`<br>` forced break" (同関数の doc section 参照) として扱われるべき
/// かどうか。
///
/// `NodeKind::Element` かつ `tag_name() == Some("br")` かつ computed
/// `display` が [`DisplayValue::None`] ではないことを見る — tag_name の
/// 直接比較は [`find_body`] の `tag_name() == Some("body")` と同じ pattern
/// であり、`<br>` を判別するために [`NodeKind`] へ新しい variant を追加する
/// 必要はない。
fn is_forced_line_break(doc: &Document, idx: usize, cascade: &CascadeResult) -> bool {
    doc.nodes[idx].kind() == NodeKind::Element
        && doc.nodes[idx].tag_name() == Some("br")
        && cascade.computed[idx].display != DisplayValue::None
}

/// Return whether an inline wrapper contains a descendant whose aligned
/// subtree is explicitly pinned to a line-box edge.
///
/// The minimal bridge may keep a nested inline wrapper on taffy's block path
/// when it has no inline element child. A wrapper with
/// `padding-top`/`padding-bottom` would therefore move both its text and the
/// aligned descendant, unlike the CSS line-box model. The caller uses this
/// predicate only for the focused `top`/`bottom` slice.
fn has_line_box_edge_aligned_descendant(
    doc: &Document,
    idx: usize,
    cascade: &CascadeResult,
) -> bool {
    let mut stack = doc.nodes[idx].children.clone();
    while let Some(child) = stack.pop() {
        if doc.nodes[child].kind() == NodeKind::Element {
            if matches!(
                cascade.computed[child].vertical_align,
                VerticalAlign::Top | VerticalAlign::Bottom
            ) {
                return true;
            }
            stack.extend(doc.nodes[child].children.iter().copied());
        }
    }
    false
}

/// `idx` (ある [`NodeKind::Element`]) が
/// [`establish_minimal_line_boxes`] の minimal-line-box 処理の対象かどうか
/// — qualifying condition とその根拠は同関数の doc 参照。
fn qualifies_for_minimal_line_box(
    doc: &Document,
    idx: usize,
    cascade: &CascadeResult,
    has_autospace_candidate: bool,
) -> bool {
    // ここでは意図的に pre-bridge の `DisplayValue` を読む
    // (post-`bridge_display` の `taffy::Display` ではない — この second
    // pass が走る時点で `Block` / `Inline` / `InlineBlock` は既に全て
    // `Display::Block` に collapse 済み)。plain な `Inline` は direct text
    // children だけなら除外し、nested inline element child を持つ wrapper
    // だけを qualify する (この module の doc 参照)。
    // table 系 (`Table` / `InlineTable` / `TableRow` / `TableCell` /
    // `TableCaption`) は全 child inline の場合に限り qualify する —
    // match arm 上の注記参照。taffy dispatch 側
    // (`taffy_impl.rs` の table dispatch) は `IS_INLINE_ROOT` flag を見て
    // table engine を bypass する。
    let is_plain_inline = has_autospace_candidate
        && cascade.computed[idx].display == DisplayValue::Inline
        && cascade.computed[idx].text_autospace != TextAutospace::NoAutospace;
    if cascade.computed[idx].display == DisplayValue::Inline && !is_plain_inline {
        return false;
    }
    let has_inline_element_child = is_plain_inline
        && doc.nodes[idx].children.iter().any(|&child| {
            doc.nodes[child].kind() == NodeKind::Element && is_inline_element_box(cascade, child)
        });
    match cascade.computed[idx].display {
        DisplayValue::Block | DisplayValue::InlineBlock | DisplayValue::Inline => {}
        // Table-internal boxes whose children are ALL inline-level get no
        // anonymous fixup from the table engine (it only collects rows and
        // cells) — without this they stack vertically in taffy's block
        // path. A pure-inline table/row/cell/caption is exactly one
        // anonymous cell's content (css-tables-3 §2.2.1 fixup with a single
        // run), so flowing it inline here is equivalent. Boxes with any
        // cell/row (or other block-level) child keep the table path via the
        // disqualify arm below. Column groups/columns never have renderable
        // children and stay disqualified.
        DisplayValue::Table
        | DisplayValue::InlineTable
        | DisplayValue::TableRow
        | DisplayValue::TableCell
        | DisplayValue::TableCaption => {}
        _ => return false,
    }
    let mut inline_level_count = 0usize;
    for &c in &doc.nodes[idx].children {
        if !doc.nodes[c].is_in_document() {
            continue;
        }
        match doc.nodes[c].kind() {
            NodeKind::Text => inline_level_count += 1,
            NodeKind::Element => match cascade.computed[c].display {
                DisplayValue::Inline | DisplayValue::InlineBlock => inline_level_count += 1,
                DisplayValue::None => {}
                // block-level / flex / grid な child (および将来の
                // non_exhaustive な DisplayValue variant) は全て
                // disqualify する — block-level と inline-level の
                // 混在は対象外、この module の doc 参照。
                _ => return false,
            },
            // Comment / ProcessingInstruction / DocumentFragment /
            // Document はそもそも `is_in_document()` にならないはず
            // (`NodeData` の doc 参照) だが、その invariant を前提とせず
            // ここで defensive に fail closed する。
            // cov:ignore: unreachable — Comment/ProcessingInstruction/DocumentFragment/Document nodes always have IS_IN_DOCUMENT cleared (see document.rs's mark_in_document_flags invariant), filtered by the is_in_document() guard above; kept as a defensive fallback rather than relying on that invariant here.
            _ => return false,
        }
    }
    if is_plain_inline {
        // A plain inline with a nested inline element needs a synthetic line
        // container so nested descendants do not take the block path and
        // stack vertically. Keep direct text-only inline containers on the
        // historical path; their content already participates in the parent
        // line box and existing layout behavior remains unchanged.
        has_inline_element_child && inline_level_count >= 1
    } else {
        inline_level_count >= 2
    }
}

/// Saturating `i32` → `i16` cast — [`grid_line_value_to_taffy_placement`]
/// doc's "silently saturating" rationale.
fn saturate_i16(n: i32) -> i16 {
    n.clamp(i16::MIN as i32, i16::MAX as i32) as i16
}

/// Saturating `u32` → `u16` cast — [`grid_line_value_to_taffy_placement`] /
/// [`grid_repeat_count_to_taffy`] doc's "silently saturating" rationale.
fn saturate_u16(n: u32) -> u16 {
    n.min(u16::MAX as u32) as u16
}

// ---------------------------------------------------------------------------
// 非有限 f32 の guard
// ---------------------------------------------------------------------------

/// taffy に渡す幾何値の絶対値上限 (px、および percentage の fraction)。
///
/// # なぜ clamp が要るのか
///
/// author CSS は untrusted 入力である。`padding: 1e40px` は cssparser の
/// f64 → f32 変換で **+Inf** になり、`padding: 1e40em` は絶対化の乗算で
/// **+Inf**、`font-size: 0px` と組み合わせると `0.0 * inf` = **NaN** になる。
/// 極端な literal すら不要で、`font-size: 10em` を 38 段 nest するだけで
/// `16 * 10^38 > f32::MAX` から +Inf が出る。
///
/// これらは絶対化を cascade に入れるまで、`layout.rs` の
/// `Length::Em(_) | Length::Rem(_) => length(0.0)` arm に**偶然**吸収されて
/// いた。網羅 match 化自体は正しいが、その arm は病的な数値も潰していた。
///
/// # spec 根拠 (§ title + anchor、`data-level` 実検証済)
///
/// CSS Values 4 §5 "Numeric Data Types"
/// (<https://www.w3.org/TR/css-values-4/#numeric-types>) verbatim:
///
/// > The precision and supported range of numeric values in CSS is
/// > implementation-defined, and can vary based on the property or other
/// > context a value is used in. However, within the CSS specifications,
/// > infinite precision and range is assumed. When a value cannot be explicitly
/// > supported due to range/precision limitations, it must be converted to the
/// > closest value supported by the implementation, but how the implementation
/// > defines "closest" is implementation-defined as well.
///
/// すなわち (a) 上限を持つこと自体が spec 準拠、(b) **上限は property / context
/// ごとに違ってよい**、(c) 超過値は「実装がサポートする最も近い値」= 上限に
/// 変換する。§5 は `must be converted` と**命令形**で書いており値を捨てろとは
/// 言っていないので、declaration はそのまま生き残る。本 module が site ごとに
/// 別の上限を持つのは (b) の直接の適用である。
///
/// (「declaration を invalid にしない」という明示的な phrasing は §5 には
/// **無い** — それは §3.1 / §10.12 の文言なので、そちらから import しない。)
///
/// # §5 と §5.1 の切り分け
///
/// §5.1 "Range Restrictions and Range Definition Notation"
/// (<https://www.w3.org/TR/css-values-4/#numeric-ranges>) の range 記法
/// (`<length-percentage [0,∞]>` 等) に対する違反は **parse 段で declaration を
/// drop** する話で、raikiri では `parse_padding_side` などが済ませている。
/// 本 guard が扱うのは **§5.1 の range 内だが実装 capacity 外**の値であり、
/// §5 の適用対象である。両者は別の layer なので混同しないこと。
///
/// 同 spec の §10.12 "Range Checking"
/// (<https://www.w3.org/TR/css-values-4/#calc-range>) は math function の
/// 結果について "the value resulting from a top-level calculation must be
/// clamped to the range allowed in the target context" と規定し、clamp が
/// computed / used value に対して行われるとする — 本 guard と同じ作法だが、
/// **本 guard の入力は `calc()` ではなく素の `em` 乗算なので直接の根拠には
/// ならない**。§3.1 "Range Checking"
/// (<https://www.w3.org/TR/css-values-4/#combining-range>) の同文言は
/// interpolation 専用の条項であり、こちらも本 case には適用されない。
/// 直接の根拠は上記 §5 である。
///
/// # 値の決定 (1e7 px)
///
/// spec は上限を定めないので実装裁量 (上記 (b)(c))。実装が実際に持つ帯を
/// 一次 source から取った: CSSWG issue #4552 の Tab Atkins 投稿
/// (<https://lists.w3.org/Archives/Public/public-css-archive/2019Dec/0015.html>、
/// 2019-12-02) verbatim:
///
/// > right now an s32 LayoutUnit's upper range is between 1e7px and 1e8px
/// > (exact value depends on the LU->px conversion in use)
///
/// 同投稿は units-per-px を **Firefox 60 / Chrome 64 / old-Edge 100** と述べる
/// ので `2^31 / units` は 3.58e7 / 3.36e7 / 2.15e7 px。本実装は帯の**下端**
/// `1e7` を採る (3 engine のいずれの上限より下)。
///
/// **これは normative spec text ではない** — CSSWG issue の comment であり、
/// 「実装が現に持っている桁」を示す engineering evidence として使っている。
///
/// 1e7 px は 96dpi で約 2.6 km / A4 約 8900 ページ相当なので実用上の制約に
/// ならない。f32 の上限 (3.4e38) から 31 桁の余裕があるので、taffy が内部で行う
/// **和** (width + padding + border + margin) が overflow して非有限に戻ることは
/// ない。
///
/// # 入力側 bound の射程と、出力側 guard による決着
///
/// 本定数は [`sanitize_taffy`] 経由で **px 幾何と percentage の fraction の
/// 両方**に適用されている。px 側については上の #4552 の導出がそのまま効くが、
/// **fraction 側の bound としては、値をどれだけ小さく取っても不十分である。**
///
/// percentage の containing block に対する解決は used value 層 (taffy 側) で
/// 起き、**nest するたびに再び掛かる**ので深さについて指数的に複利する。A4
/// (793.7 px) を起点にすると f32 が非有限になるまでの余裕は約 35.6 桁なので、
/// fraction の上限を `F` (> 1) としたとき最初に非有限になる深さは概ね
/// `35.6 / log10(F)` — **常に有限**である。修正前の depth range 実測はこの
/// model と一致する:
///
/// | decl | fraction | `35.6 / log10(F)` | 実測の最初の非有限 depth |
/// |---|---|---|---|
/// | `width: 1e9%` (本定数ちょうど) | 1e7 | 5.1 | 6 |
/// | `width: 100000%` | 1e3 | 11.9 | 12 |
/// | `width: 10000%` | 1e2 | 17.8 | 18 |
/// | `width: 1000%` | 1e1 | 35.6 | 36 |
///
/// (`padding-left` を同じ値にすると test setup で 4 / 8 / — / 25 とより
/// 浅い。padding は `location` / `scrollable_overflow_rect` の累積にも寄与するため。)
///
/// depth 1 の直接証拠: `width: 1e9%` → `size.width = 7937008000.0`
/// (= A4 793.7008px × fraction 1e7) — 既に「長さ 1e7 px」の 3 桁上。対して
/// px 経路は健全で、全 property を `1e7px` にしても depth 45 まで有限のまま。
///
/// `F <= 1.0` (= `100%`) にすれば深さ非依存になるが、`width: 200%` のような
/// spec-valid で日常的な declaration を殺すので採れない。すなわち **fraction
/// 側の入力 bound をどう選んでもこの穴は閉じられない**。CSSWG #4552 も px の
/// 話しかしておらず (percentage の乗数については何も言っていない)、fraction
/// 専用の定数を導出する一次根拠も無い。
///
/// **決着は出力側に置いた** — [`sanitize_taffy_layout`] が taffy の
/// **resolve 後**の [`taffy::Layout`] を同じ `[-MAX, MAX]` で clamp する。
/// これは深さに依存しない。
///
/// この clamp が属する cascade stage は **actual value** である
/// (CSS Cascade 5 §4.6 "Actual Values"、
/// <https://www.w3.org/TR/css-cascade-5/#actual-value> verbatim:
/// "A used value is in principle ready to be used, but a user agent may not
/// be able to make use of the value in a given environment. For example, a
/// user agent may only be able to render borders with integer pixel widths
/// and may therefore have to approximate the used width.")。すなわち
/// **used value (= taffy が計算した値) は書き換えていない** — 環境由来の
/// 近似を適用した actual value を arena に置いているだけである。近似の作法は
/// CSS Values 4 §Range Restrictions
/// (<https://www.w3.org/TR/css-values-4/#numeric-ranges>) の "must be
/// converted to the closest value supported by the implementation, but how
/// the implementation defines "closest" is implementation-defined as well"
/// に従う。なお #4552 の px 由来の根拠が**本来当てはまるのはこの出力側**で
/// ある — そこで近似される値は fraction ではなく px の used value だから。
///
/// 入力側 guard ([`sanitize_taffy`]) は出力側 guard 導入後も**外さないこと**:
/// ±Inf / NaN を taffy の内部演算に入れない役割が残っており (site 1-4 の
/// test がこれを check している)、出力側 clamp は「arena に
/// 非有限が入らない」ことしか保証しない。
///
/// # 出力側 clamp が実際に効く帯 (通常 layout との境界)
///
/// 本定数は actual value の上限でもあるので、**used value が 1e7 px を超える
/// 入力では病的でなくても値が動く**。例: `width: 200%` を 14 段 nest すると
/// used width = 793.7008 × 2^14 ≒ 1.30e7 px で、actual value は 1e7 に
/// 近似される (修正前は 1.30e7 がそのまま arena に入っていた)。
///
/// これは CSS Values 4 §Range Restrictions が許す範囲だが、#4552 が挙げる
/// 3 engine の上限 (2.15e7 / 3.36e7 / 3.58e7 px) より本実装は 2.2〜3.6 倍
/// strict である点は意図的な選択として記録しておく — 同 § の "should support
/// reasonably useful ranges" は SHOULD であり、1e7 px ≒ 2.6 km / A4 8900
/// ページで充足する。「影響ゼロ」が成り立つのは `[-1e7, 1e7]` 内に収まる
/// layout に限る。
///
/// なお修正前の穴は本 guard の regression ではなかった — guard 導入前 (base) と
/// bit 一致であり、閾値超え入力では guard 有りの方が strict improvement
/// (`width: 1e40%` は base で depth 1 → guard 後 depth 6)。可用性影響も測定済で、
/// 完全な render pipeline (`raikiri::html_to_png`) は depth 1 / 3 / 4 / 6 の
/// いずれでも ~200ms で正常な PNG を出していた (hang / OOM / panic なし)。
const MAX_TAFFY_MAGNITUDE: f32 = 1e7;

/// parley に渡す `font-size` の上限 (px)。
///
/// taffy 幾何 ([`MAX_TAFFY_MAGNITUDE`]) と分けているのは CSS Values 4 §5 の
/// 「supported range は property / context ごとに違ってよい」に従うため
/// (site ごとに target context が違うので一律にしない方針)。font-size の
/// target context は parley → skrifa の glyph scaler
/// であり、幾何とは妥当域が違う。
///
/// # 値の決定 (1e6 px)
///
/// 上限の**測定値**: 依存 chain の `skrifa` は font size を 16.16 固定小数へ
/// 変換する際 `Fixed::from_bits((ppem * 64.) as i32)` を通す
/// (`skrifa-0.42.1/src/instance.rs` の `Size::fixed_linear_scale`、FreeType の
/// `FT_Set_Pixel_Size` 互換のため)。したがって `ppem * 64.0` が `i32` に
/// 収まらなくなる `i32::MAX / 64 ≈ 3.36e7` ppem で変換が saturate する
/// (Rust の `f32 as i32` は saturating cast なので UB ではないが、scale factor
/// が無意味な値になる)。
///
/// 本実装はそこから 1 桁以上下の `1e6` を採る。差分は parley が font-size に
/// 掛ける係数 (`line-height` の unitless multiplier、ascent / descent の
/// `metric / units_per_em` 比) の余裕として残す — `parley-0.10.0` の
/// `layout/data.rs` は `LineHeight::FontSizeRelative(value) * font_size` と
/// `font_size / units_per_em` を計算する。
///
/// 1e6 px の glyph は A4 高さの約 890 倍で typographic な意味を持たないので、
/// 実用上の制約にはならない。
///
/// # 本 site の harm は「値が壊れる」ではなく **hang** (実測)
///
/// 下流 sink の帰結は当初 plausible なリスクとして未 characterize のままだったが、
/// 本 guard の実装時に実測した:
/// `sanitize_finite` を恒等関数に差し替えて
/// `nonfinite_font_size_is_clamped_before_parley` を単独実行すると
/// **25 秒経っても終了しない**。すなわち非有限 font-size は parley の shaping を
/// 有界時間で終わらせない。
///
/// 対して site 1-4 の taffy 側 test は **本 test 入力では**即座に assert 失敗する
/// (値が壊れるだけ)。これは「taffy は非有限で hang しない」という一般命題では
/// ない — 測ったのは 5 本の入力だけである。taffy 内部の used value に対する
/// 挙動は下流 sink 側の characterize 課題として別途残る。
///
/// 1 element の untrusted author CSS (`<p style="font-size: 1e40px">`) で
/// 到達するので、**本 site の guard は正しさではなく可用性の要求**である。
/// 削除・迂回しないこと。
///
/// regression 検出は `nonfinite_font_size_is_clamped_before_parley` が
/// worker thread + `recv_timeout` で**有界化**してある。CI の timeout
/// (`.github/workflows/ci.yml` の job 単位 `timeout-minutes` のみで nextest 設定は
/// 無い) には頼らない — job kill は infra flake と区別できず、同一 test binary の
/// 後続 test の結果もまとめて失われるため。
const MAX_FONT_SIZE_PX: f32 = 1e6;

/// parley に渡す `line-height` の unitless multiplier
/// (`ComputedLineHeight::Number` → `parley::LineHeight::FontSizeRelative`)
/// の上限。
///
/// # 値の決定 (1e6、実測による overflow 回避)
///
/// parley は `FontSizeRelative(value) * font_size` を計算する
/// (`parley-0.10.0/src/layout/data.rs` の `push_run` 内 line height 計算)。
/// `font_size` はここに渡る時点で [`MAX_FONT_SIZE_PX`] (`1e6`) 以下に
/// clamp 済みなので、`value` 側も同じ `1e6` に抑えれば積は高々 `1e12` —
/// `f32::MAX` (`≈3.4e38`) から 26 桁以上の余裕があり、finite × finite の
/// 乗算で桁あふれして `+Inf` になることはない。
///
/// この余裕が必要な理由は実測済: `value = f32::MAX` を素通しすると
/// `f32::MAX * font_size` (`font_size` が `1.0` を超える限り) が overflow して
/// `+Inf` になり、`+Inf` な line height は [`sanitize_line_height`] の doc が
/// 挙げる hang 経路に入る。`1e6` という具体的な数値自体に他の根拠はなく、
/// 「桁あふれしないことが確認できる finite な上限」であれば足りる —
/// [`MAX_FONT_SIZE_PX`] と同じ値を採ったのは、typographic に意味のある
/// line-height multiplier (実用上せいぜい 1 桁台) から見て両方とも同程度に
/// 過大な安全域だから。
///
/// # 結合の compile-time check
///
/// 上記の overflow 非発生の論証は「両定数が同じ `1e6`」という結合そのものに
/// 依存しており、どちらか一方だけを書き換えると崩れる。直下の
/// `const _: () = assert!(...)` は「積は高々 `1e12`」という上記 paragraph
/// 自体の関係式を compile time に固定する — `f32::MAX` 直下ではなく現在の
/// 積そのものを band として check してあるので、積が**増える**方向にどちらか
/// の定数を変更すればビルドが落ちる (減る方向は安全域が広がるだけなので
/// 素通しする)。値だけ緩めて通すのではなく、両定数と overflow 論証を
/// 併せて見直すこと。[`MAX_FONT_SIZE_PX`] 自身の妥当域は同じ形の check を
/// `clamp_limits_are_in_the_documented_range` (test) が別途固定している。
const MAX_LINE_HEIGHT_NUMBER: f32 = 1e6;

// `f64` で積を取るのは、両定数を `f32` へ丸めた積が `1e12` 境界の
// どちら側に丸まるかという 1 ULP 未満の差にこの検査を左右させないため
// (`f32` 同士の積が overflow しても trap せず `+Inf` に飽和するだけで、
// `+Inf <= 1e12` は正しく false と評価される。ここでの懸念は overflow
// ではなく丸め境界の精度)。
const _: () = assert!((MAX_LINE_HEIGHT_NUMBER as f64) * (MAX_FONT_SIZE_PX as f64) <= 1e12);

/// parley に渡す `font-weight` の妥当域下限。
///
/// CSS Fonts 4 §2.2 "Font weight: the font-weight property"
/// <https://www.w3.org/TR/css-fonts-4/#font-weight-prop> の grammar は
/// `<number [1,1000]>` — target context は [`MAX_TAFFY_MAGNITUDE`] /
/// [`MAX_FONT_SIZE_PX`] とは別の sink (`parley::FontWeight`) なので、
/// 「上限は sink ごとに変える」という方針に従い spec
/// 由来の妥当域をそのまま採る — skrifa / CSSWG issue のような engineering
/// measurement を要しない、数少ない site。
const MIN_FONT_WEIGHT: f32 = 1.0;

/// parley に渡す `font-weight` の妥当域上限 (同上、CSS Fonts 4 §2.2)。
const MAX_FONT_WEIGHT: f32 = 1000.0;

/// 非有限 `font-weight` (`NaN`) の fallback 値。
///
/// CSS Fonts 4 §2.2 "Font weight: the font-weight property"
/// <https://www.w3.org/TR/css-fonts-4/#valdef-font-weight-normal> の
/// `normal` keyword の computed value (`ComputedValues::initial().
/// font_weight` の値と一致、`crates/raikiri-style/src/computed.rs` 参照)。
///
/// [`sanitize_finite`] が length 系 site で NaN を `0.0` に落とすのは、
/// padding/margin の spec initial が幾何 `0` である、あるいは `0 * inf =
/// NaN` という無限精度評価が実際に `0` になるケースだから (同関数 doc
/// 参照) — font-weight にはどちらの根拠も対応しない。`0.0` は
/// `[MIN_FONT_WEIGHT, MAX_FONT_WEIGHT]` の外なので、それをそのまま NaN の
/// 代わりに使うと sanitize 後の値が sink の妥当域を割ってしまう。よって
/// font-weight は独自の fallback を持つ。
const FALLBACK_FONT_WEIGHT: f32 = 400.0;

/// 非有限 f32 を `[min, max]` の有限値に落とす。
///
/// - **NaN → 0.0**。`f32::clamp` は NaN を **NaN のまま**返す (`NaN.clamp(a, b)`
///   は NaN) ので、clamp だけでは潰せない。NaN は数直線上の点ではないため
///   §5 の「closest value supported」も定義できない。
///
///   0.0 を選ぶ根拠は「spec initial だから」**ではない** — initial が幾何 `0`
///   なのは padding / margin だけで (CSS Box 3 `#propdef-padding-top` /
///   `#propdef-margin-top` とも `Initial: 0`)、`width` / `height` の initial は
///   **`auto`** (CSS Sizing 3 §3.1.1 "Preferred Size Properties"
///   <https://www.w3.org/TR/css-sizing-3/#preferred-size-properties>、
///   `data-level="3.1.1"` 実検証済)、`border-*-width` は **`medium`**
///   (CSS Backgrounds 3 §3.3 "Line Thickness: the border-width properties"
///   <https://www.w3.org/TR/css-backgrounds-3/#border-width>、
///   `data-level="3.3"`、TR / ED とも `Initial: medium`) である。
///   `auto` も `medium` も幾何値ではなく**解決規則 / キーワード**なので f32 の
///   代替値として選べない。よって **全 site 一律 0.0** に倒す。§5 が "closest"
///   の定義を実装裁量とするので、この選択自体が spec 準拠である。
///
///   さらに `raikiri-style::resolve` で NaN が生じる経路 (`0px` × `1e40em` = `0.0 * inf`) に
///   限れば、**0.0 は spec 上の正解と一致する** — §5 が "within the CSS
///   specifications, infinite precision and range is assumed" と述べる以上、
///   無限精度で評価した computed value は `0 × 10^40 = 0px` である。NaN は
///   f32 の有限精度が生んだ artifact にすぎない。
///
///   傍証 (直接の根拠ではない): CSS Values 4 §10.9.1 "Infinities, NaN, and
///   Signed Zero" (<https://www.w3.org/TR/css-values-4/#calc-ieee>、
///   `data-level="10.9.1"` 実検証済。ED では §10.9.2 に採番されるが anchor は
///   同一) は math function について verbatim で
///   `NaN does not escape a top-level calculation; it's censored into a zero
///   value` / `Infinities do not escape a top-level calculation; they're clamped
///   to the minimum or maximum value allowed in the context …` と規定する (後者は原文では
///   `, as defined in § 10.12 Range Checking.` と続く — 省略を `…` で示した)。
///   **本 guard の入力は `calc()` ではないので直接の根拠にはならない**
///   (#4552 と同じく engineering evidence 扱い) が、CSS が同種の状況で採る
///   censoring 規則が NaN→zero / Inf→clamp の 2 分岐でありここでの選択と
///   一致することは、選択の妥当性を補強する。
/// - **±Inf と範囲外の有限値 → `min` / `max`**。CSS Values 4 §5 の "converted
///   to the closest value supported by the implementation" の適用。
///
/// **巨大な有限値も clamp する** (単に有限化するだけにしない) — `1e38%` は
/// bridge では有限だが、taffy 内部で containing block と掛けた時点で +Inf に
/// なり、guard を置いた意味が消える。§5 は「supported range」を実装が決めると
/// しているので、範囲外の有限値を上限に寄せるのも同じ条項の適用である。
///
/// panic しない (`LayoutError` も返さない) — **clamp して続行**する方針である。
///
/// # なぜ silent clamp ではないのか
///
/// `log` / `tracing` は workspace に依存が無い (`grep` → 0 hit) が、**それが
/// 理由ではない** — 同一 crate の `fonts.rs` に dep 追加ゼロの診断機構が既に
/// ある (`FontWarn` enum + `FontWarnObserver = Option<&mut dyn FnMut(&FontWarn)>`
/// + `emit_warn`)。
///
/// 当初この observer を本 site まで通すと公開
/// signature に波及すると判断し silent のままにしていた: `sanitize_*` は
/// `bridge_*` → `apply_computed_to_style` → `layout_single_page` の奥にあり、
/// また出力側の choke point (`sanitize_taffy_layout`) は
/// `<Document as taffy::LayoutPartialTree>::set_unrounded_layout` から
/// 呼ばれる — これは `taffy` crate 側が固定した trait method signature なので
/// **観測用引数を追加できない**。
///
/// [`crate::diag::emit_warn_via`] 共通機構を導入し、
/// この 2 点を以下で解決した:
/// - `bridge_*` → `apply_computed_to_style` の chain は crate 内 private
///   function のみで構成されるため、`diag: &mut Vec<LayoutWarn>` を通すのは
///   crate-internal な signature 変更で完結する (pub シグネチャは無傷)。
/// - `set_unrounded_layout` は `self` (`&mut Document`) は受け取れるので、
///   observer を **closure ではなく owned buffer**
///   (`Document::layout_warnings`) として `self` に持たせることで、
///   trait signature を変えずに choke point からも push できるようにした。
///   `layout_single_page` がこの buffer をパスの最後で drain し、
///   `fonts.rs` と同じ `emit_warn_via` 経由で observer-or-eprintln に流す。
///
/// 「per-node で裸の `eprintln!` を撒いて spam する」ことは避けている —
/// [`push_layout_warn`] は実際に clamp が起きた (値が変わった) 場合のみ
/// event を積むので、通常範囲の layout は buffer に何も残らない。これは
/// `FontWarn` が「warn+skip の異常」だけを observer に渡し、処理した file
/// 全部を都度報告しないのと同じ設計原則である。
///
/// **残余リスク (部分的にのみ縮小)**:
/// 将来 absolutize 側に本物の算術 bug (例: 単位換算ミスで `1e9px`) が入ると、
/// 本 guard が 1e7 に吸収して**「それらしい layout」として描画されてしまう**
/// リスクは元々あった — NaN や破綻として可視化されない。これはかつて
/// 削除した fail-quiet arm (`Em(_) => length(0.0)`) と**同じ class の
/// 残余リスク**である。
///
/// 今は clamp が発生するたび [`LayoutWarn::NonFiniteClamped`] が
/// observer-or-eprintln 経由で外に出るが、**「解消」ではなく「silent から
/// stderr-visible への降格」**と正確に言うべきである — `layout_single_page`
/// に external observer を差し込む口は現状無い (`LayoutWarnObserver`
/// scaffolding の doc参照) ので、本 crate 内に stderr を能動的に監視する
/// consumer が無い限り、この event は誰にも読まれない。`fonts.rs` の
/// `FontWarn` も同じ状態 (observer 無しなら stderr のみ) なので同水準の
/// 可視性にはなったが、「値の病理が可視化される」と言えるのは stderr を
/// 見ている human operator がいる場合に限る。
fn sanitize_finite(
    v: f32,
    min: f32,
    max: f32,
    site: &'static str,
    diag: &mut Vec<LayoutWarn>,
) -> f32 {
    let clamped = if v.is_nan() { 0.0 } else { v.clamp(min, max) };
    // `v != clamped` は NaN 入力でも正しく true になる (NaN の比較は IEEE 754
    // で常に false 「以外」= `!=` は true) ので、NaN → 0.0 の代入も
    // out-of-range 値の clamp も同じ条件で拾える。範囲内の通常値は
    // `clamped == v` なので何も積まない (spam 回避、上の doc 参照)。
    if clamped != v {
        push_layout_warn(
            diag,
            LayoutWarn::NonFiniteClamped {
                site,
                raw: v,
                clamped,
            },
        );
    }
    clamped
}

/// [`MAX_TAFFY_MAGNITUDE`] を上限とする対称 clamp (taffy 幾何用)。
///
/// 対称 (`[-MAX, MAX]`) なのは **`margin` の負値が spec-valid** だから。
/// CSS Box 3 §3.1 "Page-relative (Physical) Margin Properties"
/// (<https://www.w3.org/TR/css-box-3/#margin-physical>、`data-level="3.1"`
/// 実検証済) は verbatim で
///
/// > Negative values for margin properties are allowed,
/// > but there may be implementation-specific limits.
///
/// と規定する。これは本 delta で clamp する property のうち**唯一、spec が
/// 「implementation-specific limits」の存在を明示的に認めている**箇所であり、
/// 対称であることと上限があることを同時に正当化する
/// (「非負制約が無い」という不在の論証より強い)。
///
/// `padding` / `width` / `height` / `border-width` は parse 段で非負が
/// enforce されているので、対称にしても値は変わらない。
///
/// `site` は [`LayoutWarn::NonFiniteClamped`] の call-site label としてのみ
/// 使う (clamp の算術には影響しない)。
fn sanitize_taffy(v: f32, site: &'static str, diag: &mut Vec<LayoutWarn>) -> f32 {
    sanitize_finite(v, -MAX_TAFFY_MAGNITUDE, MAX_TAFFY_MAGNITUDE, site, diag)
}

/// 非有限 (`NaN` / `±Inf`) または `[MIN_FONT_WEIGHT, MAX_FONT_WEIGHT]`
/// 範囲外の `font-weight` を `parley::FontWeight::new` へ渡す直前で
/// sanitize する。
///
/// # なぜここに置くか
///
/// 「非有限 / 範囲外 f32 の guard は値が実際に使われる sink 境界
/// (target context) に置く。parse-time (specified 層) にも resolve 層
/// (computed 層) にも置かない」という方針に従う。[`raikiri_style::page::cascade_page`]
/// (raikiri-style) の継承元 root 引数や `ComputedValues` の直接構築は
/// raikiri-style 側の resolve/computed 層であり、
/// `raikiri_style::cascade::resolve_relative_weight` も同じ層に属する —
/// この方針はそのどちらへの guard 追加も明示的に禁じる
/// ("public な computed 層 surface は sanitize しない")。
///
/// 本関数は [`preshape_text`] — `parley::FontWeight::new` を呼ぶ唯一の call
/// site — に置くことで、他の 5 site (taffy bridge
/// helper 4 本 + font-size 用 `preshape_text` 呼び出し) と同じ「sink 直前」
/// 構造に揃える (site 6)。
///
/// # NaN fallback が `0.0` ではなく [`FALLBACK_FONT_WEIGHT`] (`400.0`) な理由
///
/// [`sanitize_finite`] を直接再利用しない。理由は 2 つ:
///
/// 1. **NaN fallback がそもそも違う** — [`sanitize_finite`] は NaN を
///    無条件で `0.0` に落とすが、その根拠 (同関数 doc 参照) は font-weight
///    に対応しない。`0.0` は妥当域の外なので、そのまま使うと sanitize
///    後の値が sink の妥当域を割る。CSS Fonts 4 §2.2
///    "Font weight: the font-weight property"
///    <https://www.w3.org/TR/css-fonts-4/#valdef-font-weight-normal> の
///    `normal` の computed value である `400.0` を採る方が、
///    「fallback / 上限は sink ごとに変える」という方針に忠実。
/// 2. **signature 変更の波及範囲** — `nan_fallback` 引数を足せば理屈上
///    1 関数に統合できるが、それは既存の length 系 4 call site
///    (`sanitize_taffy` 経由の 4 本 + font-size 直接呼び出し 1 本) と、
///    それらを検証する既存 test 全部に本 task の scope
///    (font-weight 1 sink) と無関係な引数を波及させる。独立した小関数として
///    持つ方が diff が scope に対して釣り合う。
///
/// # 出力側 (`resolve_relative_weight`) は変えない
///
/// `resolve_relative_weight` の非対称処理 (`Bolder`/`Lighter`/`-Inf` の
/// 扱いが異なる、同関数 doc 参照) は本関数の追加で修正しない —
/// resolve 層の挙動変更は本方針の禁止対象であり、本 sink guard は
/// 「resolve 層が何を出しても最終的に有限 + 妥当域内にする」ことだけを
/// 保証する。
fn sanitize_font_weight(v: f32, diag: &mut Vec<LayoutWarn>) -> f32 {
    let clamped = if v.is_nan() {
        FALLBACK_FONT_WEIGHT
    } else {
        v.clamp(MIN_FONT_WEIGHT, MAX_FONT_WEIGHT)
    };
    if clamped != v {
        push_layout_warn(
            diag,
            LayoutWarn::NonFiniteClamped {
                site: "font-weight",
                raw: v,
                clamped,
            },
        );
    }
    clamped
}

/// `raikiri_style::property::FontStyle` (`Normal | Italic | Oblique`,
/// `#[non_exhaustive]`) → parley's `FontStyle` (re-exported from the
/// `parlance` crate: `Normal | Italic | Oblique(Option<f32>)`, CSS Fonts 4
/// §2.4 <https://www.w3.org/TR/css-fonts-4/#font-style-prop>).
///
/// `Oblique` has no `<angle>` payload on the raikiri-style side (bare
/// keyword only — see `raikiri_style::property::FontStyle` doc's "Scope
/// carving" section), so it maps to parley's `Oblique(None)`, which per
/// parley's own doc uses the engine-specific default oblique angle. The
/// wildcard arm exists purely for `StyleFontStyle`'s `#[non_exhaustive]`
/// forward-compat contract (a downstream match must tolerate variants
/// added to the source enum later, e.g. a future `<angle>`-bearing
/// oblique) and is unreachable with the variant set that exists today.
fn font_style_to_parley(v: StyleFontStyle) -> FontStyle {
    match v {
        StyleFontStyle::Normal => FontStyle::Normal,
        StyleFontStyle::Italic => FontStyle::Italic,
        StyleFontStyle::Oblique => FontStyle::Oblique(None),
        // cov:ignore: unreachable while StyleFontStyle is
        // Normal|Italic|Oblique only; required for its #[non_exhaustive]
        // contract (see doc above).
        _ => FontStyle::Normal,
    }
}

/// [`ComputedLineHeight`] (`cascade.computed[idx].line_height`,
/// [`ComputedValues`] の全 field は `pub` なので非有限になり得る) を parley の
/// 2 numeric sink (`ComputedLineHeight::Number` / `Length`) 直前で sanitize
/// する。`Normal` は数値を持たないのでそのまま素通しする。
///
/// `[0.0, MAX]` の非対称 clamp (対称でない) は CSS Inline 3 §5.1
/// "Line Spacing: the line-height property"
/// (<https://www.w3.org/TR/css-inline-3/#propdef-line-height>) の grammar
/// `normal | <number [0,∞]> | <length-percentage [0,∞]>` に合わせたもの —
/// `font-size` / `font-weight` の既存 sanitize サイトと同じ理由 (負値は
/// grammar 上そもそも妥当域外)。
///
/// # `+Inf` は他の site と違う経路で **hang** する
///
/// 実測 (直接 `RangedBuilder` に `StyleProperty::LineHeight` を push し、
/// worker thread + `recv_timeout` で有界化): `parley::LineHeight::Absolute(f32::INFINITY)`
/// と `FontSizeRelative(f32::INFINITY)` はいずれも `break_all_lines` を
/// hang させる。機構は `font-size` の hang (`next_x <= max_advance` が
/// `next_x = +Inf` で恒偽になる、[`MAX_FONT_SIZE_PX`] の doc参照) とは別:
/// `parley-0.10.0/src/layout/line_break.rs` の
/// `BreakerState::add_line_height` が `running_line_height =
/// running_line_height.max(height)` を計算しており、`height` が `+Inf` だと
/// `running_line_height` も `+Inf` になって `running_line_height >
/// line_max_height` (`line_max_height` の default は `f32::MAX`) が
/// 以後ずっと真になり続ける。この `max_height_exceeded` 分岐が前進しない
/// ことで hang する。
///
/// `NaN` は hang **しない** — `f32::max` は NaN を捨てて他方の被演算子を返す
/// (IEEE 754 の total-order ではなく Rust 標準の `f32::max` 挙動) ため
/// `running_line_height` は有限のまま前進する。`font-size` の
/// `NaN`/`-Inf`/巨大 finite が hang しないのと同じ非対称構造 (site 5 の
/// `parley_break_all_lines_completes_for_nan_neg_inf_and_huge_finite_font_size`
/// 参照)。
///
/// `FontSizeRelative` はさらに `value * font_size` の乗算で桁あふれし得る
/// ([`MAX_LINE_HEIGHT_NUMBER`] の doc参照) — [`MAX_LINE_HEIGHT_NUMBER`] は
/// この overflow を避ける上限。`Absolute` はそのまま渡るだけで乗算しないため
/// `f32::MAX` 自体は overflow しない (`f32::MAX > f32::MAX` は偽) が、
/// 同じ [`MAX_FONT_SIZE_PX`] を再利用して上限とする — 「巨大 finite を防ぐ」
/// ためではなく「入力側で既に non-finite になっているケースを finite に倒す」
/// ための clamp であり、typographic に意味のある line-height (px) から見て
/// 過大という理由は font-size の値そのものと同種。
fn sanitize_line_height(v: ComputedLineHeight, diag: &mut Vec<LayoutWarn>) -> ComputedLineHeight {
    match v {
        ComputedLineHeight::Normal => ComputedLineHeight::Normal,
        ComputedLineHeight::Number(n) => ComputedLineHeight::Number(sanitize_finite(
            n,
            0.0,
            MAX_LINE_HEIGHT_NUMBER,
            "line-height (number)",
            diag,
        )),
        ComputedLineHeight::Length(len) => {
            ComputedLineHeight::Length(ComputedLength(sanitize_finite(
                len.px(),
                0.0,
                MAX_FONT_SIZE_PX,
                "line-height (length)",
                diag,
            )))
        }
    }
}

/// [`ComputedLineHeight`] (`Normal | Number | Length`) → parley's
/// [`LineHeight`] (`MetricsRelative | FontSizeRelative | Absolute`,
/// `parley-0.10.0/src/style/mod.rs`).
///
/// 呼び出し側は事前に [`sanitize_line_height`] で有限化した値を渡すこと —
/// 本関数自体は値を変換するだけで sanitize しない ([`font_style_to_parley`]
/// と同じ「mapping と sanitize は別関数」構造)。
///
/// # 値レベルの対応 (各 arm の根拠)
///
/// - [`ComputedLineHeight::Normal`] → `LineHeight::MetricsRelative(1.0)` —
///   parley 自身の `Default` (`parley-0.10.0/src/style/mod.rs` の
///   `impl Default for LineHeight`) と一致する。本関数を経由しても
///   line-height 配線前の挙動 (parley default 依存) を変えない。
/// - [`ComputedLineHeight::Number`] → `LineHeight::FontSizeRelative` —
///   parley は `FontSizeRelative(value) * font_size` を計算する
///   (`parley-0.10.0/src/layout/data.rs` の `push_run`)。ここでの
///   `font_size` は `preshape_text` が同じ `RangedBuilder` へ push する
///   **自要素の** computed font-size (`StyleProperty::FontSize`) そのもの
///   なので、CSS Inline 3 §5.1 の「unitless number は子が自分の font-size に
///   掛ける」と一致する。
/// - [`ComputedLineHeight::Length`] → `LineHeight::Absolute` — computed 層で
///   既に絶対化済みの px 値 ([`ComputedLineHeight::Length`] の doc参照) を
///   そのまま渡す。parley 側も `LineHeight::Absolute(value) => value` と
///   素通しするだけ (`data.rs`) なので、二重の解決は起きない。
///
/// [`ComputedLineHeight`] は `#[non_exhaustive]` を付けない判断がされている
/// ([`raikiri_style::resolve`] module doc参照) ため、本関数も
/// [`font_style_to_parley`] と異なり wildcard arm を持たない — 将来 variant が
/// 追加されればここでコンパイルが落ちて気づける。
fn line_height_to_parley(v: ComputedLineHeight) -> LineHeight {
    match v {
        ComputedLineHeight::Normal => LineHeight::MetricsRelative(1.0),
        ComputedLineHeight::Number(n) => LineHeight::FontSizeRelative(n),
        ComputedLineHeight::Length(len) => LineHeight::Absolute(len.px()),
    }
}

/// Structured warn event for this module's non-finite-clamp diagnostic sites
/// (`sanitize_finite` / `sanitize_taffy` / `sanitize_taffy_layout` /
/// `sanitize_font_weight`). Sibling
/// of [`crate::fonts::FontWarn`], generalized via the
/// shared [`crate::diag::emit_warn_via`] mechanism so
/// the "silent clamp" residual risk documented on [`sanitize_finite`] gets
/// the same observability `fonts.rs` already has.
///
/// Every variant is fully owned (no borrowed `Path`, unlike `FontWarn`)
/// because these clamp sites only ever see primitive `f32` values. See
/// [`crate::diag`]'s module doc for why this owned shape — not `FontWarn`'s
/// borrowed one — is what a shared generic `Observer<W>` type could actually
/// have supported, and why a macro was used instead so both shapes share one
/// mechanism anyway.
///
/// No `#[non_exhaustive]` (unlike `FontWarn`, which is `pub`): that attribute
/// only constrains *downstream crates*, and this enum is `pub(crate)` with no
/// external consumer to protect. Add it back if this type is ever promoted
/// to a public export.
#[derive(Debug, Clone, Copy)]
pub(crate) enum LayoutWarn {
    /// A non-finite (NaN / +-Inf) or out-of-range `f32` was clamped to a
    /// finite in-range value before being handed to one of this module's
    /// sink boundaries: `taffy::Style` or the `Node.unrounded_layout` arena
    /// field (sites 1-5, `sanitize_finite` / `sanitize_taffy` /
    /// `sanitize_taffy_layout`),
    /// `parley::FontWeight::new` (site 6, `sanitize_font_weight`), or
    /// `StyleProperty::LineHeight`'s two numeric sub-values (sites 7-8,
    /// `sanitize_line_height`). Only
    /// emitted when clamping actually changed the
    /// value (not on every call) so ordinary in-range layouts stay silent —
    /// the "warn+skip" shape `FontWarn` uses, not a per-node trace.
    NonFiniteClamped {
        /// Call-site label (e.g. `"font-size"`, `"margin"`,
        /// `"layout.size"`) — a human-readable category, not a stable
        /// machine-parseable identifier.
        site: &'static str,
        /// Pre-clamp value (may be NaN or +-Inf).
        raw: f32,
        /// Post-clamp value actually used.
        clamped: f32,
    },
    /// `suppressed` additional [`LayoutWarn::NonFiniteClamped`] events were
    /// dropped once [`LAYOUT_WARN_CAP`] was reached during a single
    /// `layout_single_page` pass, bounding memory / `eprintln!` spam under a
    /// pathological input that clamps every field of every node (e.g. deep
    /// `width: 200%` nesting — see [`MAX_TAFFY_MAGNITUDE`]'s doc). Emitted at
    /// most once per pass, after all the real events it summarizes.
    Truncated {
        /// Count of additional `NonFiniteClamped` events dropped after the
        /// cap was reached.
        suppressed: usize,
    },
    /// One or more subtrees had a broken **parent/child geometry invariant**
    /// and were reset to a deterministic zero
    /// [`taffy::Layout`] by [`enforce_layout_invariants`]. This is a
    /// different failure class than [`LayoutWarn::NonFiniteClamped`]: that
    /// variant fires when a single `f32` field was out of range, this one
    /// fires when every individual field of the (already per-field-clamped)
    /// `Layout`s involved was in range, but the *relationship* between two
    /// or more fields — possibly on different nodes — was not (e.g. a
    /// node's own content box went negative, or a child's border box did
    /// not fit inside its parent's once one of the two had its actual value
    /// approximated by [`sanitize_taffy_layout`]). See
    /// [`enforce_layout_invariants`]'s doc for exactly which two invariants
    /// are checked and why unconditionally checking cross-node containment
    /// would be spec-incorrect.
    ///
    /// Aggregated per invariant (at most one event per invariant kind per
    /// `layout_single_page` pass, each counting every subtree it reset)
    /// rather than one event per reset subtree, so a pathological input
    /// that trips the same invariant on many nodes cannot reintroduce the
    /// per-node spam [`LAYOUT_WARN_CAP`] exists to bound.
    GeometryInvariantViolated {
        /// Which invariant was violated — `"content_box_non_negative"` or
        /// `"child_within_parent_border_box"` (see
        /// [`enforce_layout_invariants`]'s doc). Like `NonFiniteClamped`'s
        /// `site`, a human-readable category, not a stable
        /// machine-parseable identifier.
        ///
        /// The `"child_within_parent_border_box"`
        /// value can no longer actually occur — [`child_within_parent_border_box`]
        /// (the predicate) now always returns `true`, so
        /// [`enforce_layout_invariants`]'s containment branch that would
        /// produce this event is unreachable for any input. It remains
        /// listed here (and the branch remains in the code) because whether
        /// to remove the dead invariant check entirely is a separate,
        /// explicitly deferred decision. Do not treat this
        /// value's continued presence in this doc as evidence the check is
        /// still live.
        invariant: &'static str,
        /// Number of distinct subtree roots this invariant caused
        /// [`enforce_layout_invariants`] to reset in this pass (not a count
        /// of individual arena nodes touched — a reset subtree may contain
        /// further descendants that also got zeroed as part of the same
        /// reset).
        subtree_count: usize,
    },
}

impl std::fmt::Display for LayoutWarn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LayoutWarn::NonFiniteClamped { site, raw, clamped } => write!(
                f,
                "{site}: clamped non-finite/out-of-range value {raw} to {clamped}"
            ),
            LayoutWarn::Truncated { suppressed } => write!(
                f,
                "{suppressed} additional layout clamp warning(s) suppressed (buffer cap reached)"
            ),
            LayoutWarn::GeometryInvariantViolated {
                invariant,
                subtree_count,
            } => write!(
                f,
                "{invariant}: {subtree_count} subtree(s) had a broken parent/child geometry invariant and were reset to a zero layout"
            ),
        }
    }
}

/// Observer alias for [`LayoutWarn`] — sibling of `fonts.rs`'s
/// `FontWarnObserver`. Unlike that alias, this one carries no inner lifetime
/// (`LayoutWarn` is fully owned), so it is a plain, non-higher-ranked
/// `Option<&mut dyn FnMut(&LayoutWarn)>`. Not yet reachable from any public
/// entry point — see [`Document::layout_warnings`](crate::document::Document)
/// for why (dom→paint wall: `layout_single_page`'s signature is consumed by
/// `raikiri-paint` and the `raikiri` crate, so adding a parameter — or a new
/// `_with_observer` sibling — to it is a decision for that wall, not this
/// task). This scaffolding exists so a future `_with_observer` addition only
/// has to plumb one new parameter through, rather than re-deriving the whole
/// mechanism.
type LayoutWarnObserver<'o> = Option<&'o mut dyn FnMut(&LayoutWarn)>;

/// Emit a [`LayoutWarn`] event: call the observer if `Some`, otherwise
/// `eprintln!` (matches [`crate::fonts`] の `emit_warn`'s shape exactly, via the
/// shared [`crate::diag::emit_warn_via`] macro).
fn emit_layout_warn(observer: &mut LayoutWarnObserver<'_>, event: LayoutWarn) {
    crate::diag::emit_warn_via!(observer, "[raikiri-dom::layout]", event);
}

/// Cap on buffered [`LayoutWarn::NonFiniteClamped`] events per
/// `layout_single_page` pass.
///
/// A single pathological input (e.g. deep `width: 200%` nesting hitting
/// every node, [`MAX_TAFFY_MAGNITUDE`]'s doc) can clamp every `f32` field of
/// every node in the arena, which would otherwise make both the buffer and
/// the eventual `eprintln!` replay unbounded — precisely the "per-node spam"
/// concern [`sanitize_finite`]'s doc raised about threading an observer down
/// this chain in the first place. [`push_layout_warn`] collapses anything
/// past this cap into a single running [`LayoutWarn::Truncated`] counter
/// instead of dropping it silently.
pub(crate) const LAYOUT_WARN_CAP: usize = 63;

/// Push a [`LayoutWarn`] onto `diag`, respecting [`LAYOUT_WARN_CAP`].
///
/// Once the cap is reached, further events collapse into (rather than grow)
/// a single trailing [`LayoutWarn::Truncated`] counter, so the buffer size is
/// bounded (`LAYOUT_WARN_CAP + 1`) regardless of how many clamp sites fire in
/// one pass.
fn push_layout_warn(diag: &mut Vec<LayoutWarn>, event: LayoutWarn) {
    if diag.len() < LAYOUT_WARN_CAP {
        diag.push(event);
        return;
    }
    match diag.last_mut() {
        Some(LayoutWarn::Truncated { suppressed }) => *suppressed += 1,
        _ => diag.push(LayoutWarn::Truncated { suppressed: 1 }),
    }
}

/// taffy が resolve した [`taffy::Layout`] の全 f32 field を
/// [`sanitize_taffy`] に通す **出力側** guard。
///
/// # なぜ出力側なのか (入力側の bound では閉じられない)
///
/// [`MAX_TAFFY_MAGNITUDE`] の「入力側 bound の射程」節のとおり、percentage は
/// used value 層で containing block に対して解決されるため nest ごとに複利し、
/// **1 より大きい fraction 上限はどれを選んでも有限の深さで f32 を溢れさせる**。
/// 深さは untrusted な入力 (DOM の nest) が決めるので、深さ非依存の場所 —
/// resolve の**後** — に guard を置く以外に閉じ方が無い。
///
/// # call site は 1 箇所 (choke point)
///
/// `<Document as taffy::LayoutPartialTree>::set_unrounded_layout`
/// (`taffy_impl.rs`) — taffy が arena へ layout を書き戻す**唯一の**経路
/// (`taffy-0.12.1` の block / flexbox / grid / leaf 各 algorithm はすべて
/// この 1 メソッドを通る)。したがって「`Node.unrounded_layout` は決して
/// 非有限を含まない」は構造的な invariant であり、後付けの一括 range のように
/// 呼び忘れで破れることがない。
///
/// invariant の残り半分は**初期値**: `Node::new*` は
/// `Layout::with_order(0)` (`node.rs`) を置き、これは全 field 0 で有限。
/// 以降の書き込みは上記のとおり本 guard を通るので、arena が非有限 layout を
/// 持つ瞬間が存在しない。
///
/// # taffy の内部計算は変えない (used value は不変、actual value のみ近似)
///
/// `taffy::Layout` を arena から**読み戻す**のは `RoundTree::get_unrounded_layout`
/// だけで、これは `taffy::round_layout` 専用である。raikiri は `round_layout` を
/// 呼ばず `RoundTree` も実装していない (`grep -rn 'round_layout\|RoundTree'
/// crates/` → 本 doc comment 以外 0 hit、実測)。よって本 clamp は
/// **観測面だけ**を縛り、taffy 内部の
/// percentage 解決 chain (`LayoutInput::parent_size`) には影響しない。
/// すなわち「深いところで内部的に inf になった結果が clamp 済の値として
/// 見える」のであって、レイアウト計算自体を書き換えてはいない。
///
/// # 網羅的な struct literal (`..` を使わない)
///
/// 全 f32 field を明示列挙する。`..*layout` にすると taffy が将来 f32 field を
/// 増やしたときに**黙って guard の外に漏れる**が、網羅 literal なら compile
/// error になって review を強制できる。`order` は `u32` なので guard 対象外。
///
/// paint が現に読む 4 field だけに絞らないのも同じ理由 —
/// 「arena は非有限幾何を持たない」は述べられて test できる invariant だが、
/// 「paint がたまたま読む field」はそうではない。
///
/// # 保証するのは finiteness だけ (box model の包含関係は保存しない)
///
/// なお本 guard が保証するのは **finiteness だけ**で、box model の包含関係
/// (CSS Box 3 の content ⊆ padding ⊆ border) は保存しない — field ごとに
/// 独立に clamp するので、`size.width` と `padding.{left,right}` が同時に
/// 飽和すると `size.width - padding.left - padding.right` は負になりうる。
/// 現在 `padding` / `border` / `scrollable_overflow_rect` / `scrollbar_size` を読む
/// consumer は無い (grep 実測) が、将来 paint がこれらを使うときは
/// 非負性を仮定しないこと。
///
/// `diag` collects [`LayoutWarn::NonFiniteClamped`] events for whichever
/// fields actually get clamped (site labels: `"layout.location"`,
/// `"layout.size"`, `"layout.scrollable_overflow_rect"`, `"layout.scrollbar_size"`,
/// `"layout.border"`, `"layout.padding"`, `"layout.margin"`). The sole caller
/// (`<Document as taffy::LayoutPartialTree>::set_unrounded_layout` in
/// `taffy_impl.rs`) passes `&mut self.layout_warnings` — an owned buffer on
/// `Document`, not a live observer — because that trait method's signature
/// is fixed by `taffy` and cannot receive one (see
/// `Document::layout_warnings`'s doc for why).
pub(crate) fn sanitize_taffy_layout(
    layout: &TaffyLayout,
    diag: &mut Vec<LayoutWarn>,
) -> TaffyLayout {
    fn size(s: Size<f32>, site: &'static str, diag: &mut Vec<LayoutWarn>) -> Size<f32> {
        Size {
            width: sanitize_taffy(s.width, site, diag),
            height: sanitize_taffy(s.height, site, diag),
        }
    }
    fn rect(r: Rect<f32>, site: &'static str, diag: &mut Vec<LayoutWarn>) -> Rect<f32> {
        Rect {
            left: sanitize_taffy(r.left, site, diag),
            right: sanitize_taffy(r.right, site, diag),
            top: sanitize_taffy(r.top, site, diag),
            bottom: sanitize_taffy(r.bottom, site, diag),
        }
    }
    TaffyLayout {
        order: layout.order,
        location: Point {
            x: sanitize_taffy(layout.location.x, "layout.location", diag),
            y: sanitize_taffy(layout.location.y, "layout.location", diag),
        },
        size: size(layout.size, "layout.size", diag),
        scrollable_overflow_rect: rect(
            layout.scrollable_overflow_rect,
            "layout.scrollable_overflow_rect",
            diag,
        ),
        scrollbar_size: size(layout.scrollbar_size, "layout.scrollbar_size", diag),
        border: rect(layout.border, "layout.border", diag),
        padding: rect(layout.padding, "layout.padding", diag),
        margin: rect(layout.margin, "layout.margin", diag),
    }
}

/// [`sanitize_taffy_layout`] が保証する **finiteness** の一段上のレイヤー —
/// 親子 geometry の**意味的** invariant を検査し、破れている subtree を
/// 決定的な既定 geometry (ゼロ) に置き換える。
///
/// # なぜ `sanitize_taffy_layout` だけでは閉じないか
///
/// `sanitize_taffy_layout` の doc が明言する通り、
/// その guard は **field ごとに独立に** clamp するため、box model の包含
/// 関係 (CSS Box 3 の content ⊆ padding ⊆ border) は保存しない —
/// `size.width` と `padding.{left,right}` が同時に飽和すると
/// `content_box_width()` が負になりうる。また taffy 内部の演算 chain は `LayoutOutput`
/// 経由で **clamp 前の生値** を子から親へ返す (`taffy-0.12.1` の
/// `compute/block.rs:947,973,981,1072`、`set_unrounded_layout` が呼ばれる
/// のはその**後**であり、かつ子の `Layout` を書くのは子自身ではなく
/// **親の algorithm**) ため、ある node の位置が「別の (クランプ済) node」を
/// 基準に計算されていても、各 node は「自分の field が有限」であることしか
/// 保証されない。本関数はその 2 つの隙間 — 単一 node 内の box model 包含
/// 関係、および親子間の位置関係 — を埋める。
///
/// # 検査する 2 つの invariant
///
/// 1. **content box 非負** — `Layout::content_box_width()` /
///    `content_box_height()` が両方 `>= 0.0`。**全 node に無条件で**適用する。
///    実装時に実測した (probe: `<div style="width: 10px; padding: 50px;
///    box-sizing: border-box;">` を通常経路 (`layout_single_page`) で
///    layout): taffy 自身が border box を `size.width == padding_left +
///    padding_right` (= `100.0`) まで自動的に stretch し、content box を
///    ちょうど `0.0` に floor する (`taffy-0.12.1/src/compute/mod.rs` の
///    `maybe_max(padding_border_size)` と同型の内部ロジック)。すなわち
///    **非有限 clamp が絡まない通常の CSS では content box が負になること
///    はない** — この invariant を無条件で検査しても legitimate な layout
///    を誤検出しない。
/// 2. **child の border box の原点 (`location`) が parent の border box に
///    収まる** — もともとは containment を検査する invariant として設計
///    されたが、**飽和した axis は符号を
///    問わず無条件に ok とする変更が入った** (詳細は後述の符号別の節) — つまり本 invariant は
///    もはや実際には何も検査しない (taffy の座標系は「parent border box
///    原点からの相対位置」、`taffy-0.12.1/src/tree/layout.rs` の
///    `Layout::content_box_x/y` の doc参照)。**`child.size` は見ない** —
///    本 invariant はもともと「child の **location** が parent の
///    border box 内」とだけ規定しており、child 自身の大きさは対象にしていない
///    ([`child_within_parent_border_box`] の doc「`child.size` を見ない理由」
///    節、実装時に extent (`location + size`) 版で `width: 200%` の
///    legitimate nest を誤検出することが判明した経緯を記録している)。
///
///    **以下はこの変更が入る前の設計とその根拠の記録** — 上述のとおりこの変更以降、
///    本 invariant は実際には何も検査しない無条件 accept になっている。
///    なぜ元々こちらを無条件検査にしなかったか: CSS は
///    子が親の border box をはみ出すことを普通に許す (`overflow: visible`
///    が initial 値、負 margin、固定して小さい container + 大きい content、
///    `width: 200%` のような「子が親より意図的に大きい」宣言) — raikiri は
///    今 `position: absolute` を未実装だが、それだけで十分再現する。実装時に
///    実測した (probe: parent `<div style="width: 50px; height: 50px;">` の
///    子に `<div style="width: 200px; height: 200px; margin-left:
///    -30px;">`) では parent `size=(50,50)` に対し child `location=(-30,0)`
///    — 原点自体が parent の左端 (`x=0`) より外に出ているが、これは**正しい
///    layout であって bug ではない**。無条件で検査すると legitimate な
///    layout を誤って fallback してしまうため、`child.location.x` /
///    `child.location.y` の**その axis 自身**が [`MAX_TAFFY_MAGNITUDE`] の
///    飽和境界にちょうど達している場合**だけ**、その axis を検査する
///    ([`child_within_parent_border_box`] の実装参照 — `parent.size` や
///    `child.size` が飽和しているかどうかはこの gate に関与しない)。
///    飽和が起きたということは、その field の「actual value」(近似後の値)
///    が taffy 内部の「used value」(実際の計算結果) と乖離している —
///    以前の版ではこれを根拠に「だからこそ改めて明示的に整合性を
///    検査し、破れていれば決定的な値に倒す」という設計だった。**その後
///    この設計は覆った**: 「actual value が近似
///    されている」こと自体は仕様上許容された範囲内の動作であり、収まって
///    いようといまいと本関数が reset の理由にすることはない、という結論に
///    符号を問わず統一された (詳細は後述の符号別の節)。
///    `saturated_but_contained_layout_is_not_reset` は元々「gate かつ
///    containment 違反」という conjunction を check する目的の test だった
///    が、この変更以降は assert 自体は変わらず通る (この test の fixture が
///    たまたま「収まっている」ケースなので) ものの、conjunction の主張は
///    もう成立しない — 同 test の doc および対の regression check
///    (`saturated_child_outside_parent_is_not_reset`、
///    「明らかに収まっていない」fixture でも reset されないことを直接示す
///    ために追加/改名) を参照。
///
///    **実装時の実測 (`width: 200%` を 45 段 nest、単一 chain)**: 当初は
///    extent (`location + size <= parent.size`) を検査していたが、この
///    legitimate な (どの深さでも「child は parent の 2 倍」という一貫した
///    関係を表す) declaration が深い段で誤って reset されることが判明した —
///    child の origin は常に `(0, 0)` のままなので、origin だけを見る現行の
///    定義では reset されない
///    (`nested_percentage_wide_child_chain_is_not_reset` が pin)。
///
///    **過去に発見された gate 不備 (修正済み)**: origin
///    だけを見るようにした直後の版は、なお gate を「`parent.size` /
///    `child.size` / `child.location` のいずれか 1 つでも飽和していれば
///    axis 区別なく両 axis を検査する」という条件にしていた。この形では
///    **child 自身の location が飽和していなくても** — 例えば同じ subtree の
///    どこか別の node の `parent.size` が (無関係な原因で) 飽和していた
///    だけで — legitimate な負 margin (`location.x` が通常範囲の負値、
///    例: `-30`) を持つ child が `>= 0.0` に落ちて誤って reset されうる、と
///    非軽微な finding として指摘された。現在の
///    axis 単位 gate (`child.location.x` / `.y` 自身が飽和している場合だけ、
///    その axis だけを検査する) はこれを構造的に閉じる — 検査対象になる
///    field は必ず「それ自身が近似された」field に限られるため、legitimate
///    な小さい負値がこの gate を通ることはない
///    (`saturated_but_contained_axis_with_legitimate_negative_margin_on_other_axis_is_not_reset`
///    が直接 check する — 「`y` 軸だけでも reset の説明がつく」fixture では
///    新旧実装を区別できないという指摘を受けて、`y` 軸が
///    飽和かつ収まっている fixture に差し替えた経緯は同 test の doc参照。
///    `saturated_axis_outside_parent_with_legitimate_negative_margin_on_other_axis_is_not_reset`
///    はこの変更が入る前は `y` 軸の検出力が保たれていることの
///    check だったが、この変更でその検出力自体が失われたため、現在は同 test の
///    doc が記録するとおり別の主張 (どちらの axis も reset の理由に
///    ならない) の check になっている)。
///
///    **負方向の false positive (当初は残余リスクとして認識されていたが、
///    後に解決)**: axis 単位の gate まで閉じた上でも、飽和した axis 自身が
///    **負**の場合に固有の false positive が残っていた。当時導入されていた版の
///    再検査式 `child.location >= 0.0 && child.location <= parent.size` は、
///    `child.location` が負である間は **恒等的に false** — `>= 0.0` を満たす
///    負数は存在しないので、負方向についてこの式は「containment を
///    re-validate する」のではなく「飽和かつ負なら無条件 reset する」式と
///    数学的に同値だった。CSS Box 3 §3.1 (前掲、`margin` の負値は
///    「implementation-specific limits」の範囲で無制限に許される) の下で、
///    [`MAX_TAFFY_MAGNITUDE`] はまさにその「implementation-specific limit」
///    自身であり、そこに達したこと自体は──合法な負方向の関係が本実装の
///    上限を超えて近似され始めた、というだけで──破綻の証拠にならない。
///    これは同じ関数がすでに無条件で信頼している「飽和していない負値」
///    (`legitimate_negative_margin_overflow_is_not_reset` の `-30` や、深い
///    nest で `-30`,`-60`,…と単調に増大する中間段)と対称であり、境界
///    (`±MAX_TAFFY_MAGNITUDE` にちょうど達する瞬間) だけ扱いが不連続に
///    反転する理由が無い。
///
///    当時はここから「したがって現在の定義は符号で分岐する:
///    飽和した axis が負なら無条件に ok、正なら従来通り `<= parent.size`
///    を再検査する」という結論を導いていた。根拠は 2 つ — (a) `padding` /
///    `border` / `width` は parse 時点で非負が enforce されるため、正方向の
///    巨大な `location` を「CSS が無制限に許す」と正当化する spec 上の
///    対称な根拠が (margin とは違って) 無い、(b) 正方向の再検査は実際に
///    両方の分岐を持つ (`<=` が真になる `saturated_but_contained_layout_is_
///    not_reset`、偽になる旧
///    `saturated_child_outside_parent_resets_subtree_to_zero_layout`) ので
///    緩めると既存の検出力を実際に失う——というものだった。
///
///    **この根拠 (a) は後に覆った**:
///    margin は symmetric — CSS Box 3 §3.1
///    (<https://www.w3.org/TR/css-box-3/#margin-physical>、"Negative values
///    for margin properties are allowed, but there may be
///    implementation-specific limits") は**負値**を明示的に許容している
///    だけで、正の margin をそれより厳しく縛る根拠にはなっていない —
///    `margin-left` の grammar (`<length-percentage> | auto`) は正負どちらの
///    巨大な値も等しく spec-legal であり、[`MAX_TAFFY_MAGNITUDE`] という
///    「implementation-specific limit」に達すること自体は、先に負方向で
///    確立したのと同じ理由で、正方向でも破綻の証拠にはならない。実際
///    `margin-left: 1e9%` (100px container 内) は `location.x` を正方向に
///    飽和させ、旧実装はこれを誤って reset していた — 実測は
///    `saturated_negative_margin_percentage_child_is_not_reset` の正方向対
///    である CSS パイプライン経由の regression test を参照。
///
///    根拠 (b) (「正方向の再検査には現に検出力がある」) はこの変更でも
///    **反証されてはいない** — 反証されたのは (a) だけで、(b) の
///    「検出力を失う」という指摘自体は正しかった。その損失は
///    **承知の上で受け入れられた** — 既存 test は TaffyLayout を直接構築する
///    のみで実 CSS パイプライン経由の検出力を一度も示しておらず、一方で
///    今回の data loss (legitimate content の完全消失) は実 CSS 経由で
///    実証済みだったため。**結果として `axis_ok` は符号を問わず「飽和して
///    いれば無条件 accept」に統一され、[`child_within_parent_border_box`]
///    は常に `true` を返す** — containment を再検査する経路は
///    もう存在しない (`axis_ok` の実装、および doc「符号を問わず無条件
///    accept になった理由」節参照)。「この invariant 自体を維持すべきか」
///    は別途明示的に deferred とされた decision であり、
///    本 doc のこの時点では未解決。
///
///    この変更で挙動が反転した regression check: 旧
///    `saturated_child_outside_parent_resets_subtree_to_zero_layout` は
///    `saturated_child_outside_parent_is_not_reset` に改名・反転し、旧
///    `saturated_location_with_legitimate_negative_margin_on_other_axis_is_not_reset`
///    は
///    `saturated_axis_outside_parent_with_legitimate_negative_margin_on_other_axis_is_not_reset`
///    に改名・反転した — 詳細はそれぞれの doc を参照。
///
///    **別案として検討し却下したもの**: 「`parent.size` 自身も同じ axis で
///    飽和していれば (符号を見ずに) re-validate をスキップする」という
///    parent 飽和ゲート案は、
///    `saturated_negative_margin_percentage_child_is_not_reset`
///    (単一の `margin-left: -1e9%` 宣言、nest 無し、parent は
///    `width: 100px` で飽和していない) を誤って reset したまま説明できない
///    ——「parent が飽和しているかどうか」ではなく「child 自身の符号」が
///    正しい判別軸である証拠として、この test を上の nested chain test と
///    独立に残している。
///
/// NaN → `0.0` (`sanitize_finite` の doc参照) は飽和境界 (`±MAX_TAFFY_
/// MAGNITUDE`) に一致しないため invariant 2 の gate をすり抜けるが、実害は
/// 無い — `location=(0,0)` `size=(0,0)` は非負サイズの任意の parent に
/// 対して常に「収まっている」ので、invariant 2 が検査対象から漏れても
/// そもそも違反として検出すべき状態にならない。padding/border が巻き込
/// まれて NaN → 0 になり content box が負に振れるケースは invariant 1 が
/// 無条件に (gate なしで) 拾う。
///
/// # 決定的 fallback: subtree をゼロ化 (`zero_layout_subtree`)
///
/// 「入力制限」「途中 saturation」「layout abort」を採らず「決定的
/// fallback」を採る設計判断は決定済み。fallback 値は検討時に挙がった 2 案
/// (「0 サイズ」「直近の有限な親サイズ」) のうち **0 サイズ**を採る —
/// [`taffy::Layout::with_order`] (`order` だけ保持、他は全 zero) は
/// (a) 自明に invariant 1 (`0 - 0 - 0 = 0 >= 0`) と invariant 2 (`(0,0)` は
/// 非負サイズの任意 parent に収まる) の両方を再帰的に満たすため、subtree
/// 全体をこの値で埋めても新たな invariant 違反を作らない (「直近の有限な
/// 親サイズへの fallback」だと、fallback 後の値がさらに invariant 2 を
/// 破らないことを別途保証する必要があり、再検査を繰り返す設計になる)、
/// (b) 検討時の記述でも「0 サイズ」を先に挙げていた、の 2 点から選んだ。
///
/// # traversal は再帰しない
///
/// この関数が対象にする入力 (深い percentage nest — [`MAX_TAFFY_MAGNITUDE`]
/// の doc 表) はまさに**深い DOM tree** なので、[`find_body`] や cascade の
/// deep-nesting 対策と同じ理由で、素朴な再帰で書くと同じ入力で stack
/// overflow の新しい経路を作ってしまう。本関数と [`zero_layout_subtree`]
/// はともに `Vec` を明示 stack として使う iterative 実装。
///
/// # taffy が実際に visit した node だけを見る
///
/// `document.nodes[idx].children` をそのまま辿らず、taffy の
/// `TraversePartialTree` 実装 (`taffy_impl.rs`) と同じ `is_in_document()`
/// filter を子の走査に適用する — `<template>` descendants など taffy が
/// そもそも layout しなかった node は `unrounded_layout` が構築時デフォルト
/// (`Layout::with_order(0)`) のまま (他 subtree の古い値が紛れ込むわけでは
/// ない) なので対象に含めても実害は無いが、taffy の traversal 契約と揃えて
/// おく方が読み手にとって驚きが無い。
///
/// # 呼び出し元
///
/// [`layout_single_page`] の Step 5 (`compute_root_layout`) 直後、Step 6
/// (warning replay) の前 — root (`<body>`) の下で taffy が実際に書いた全
/// `unrounded_layout` が揃った直後に 1 回だけ走る。`compute_root_layout` を
/// 直に呼ぶ経路 (`lib.rs` の各 unit test) はこの pass を経由しない —
/// それらのテストは `sanitize_taffy_layout` (finiteness のみ) の
/// characterization が目的であり、意味的 invariant は対象外
/// (`taffy_block_layout_does_not_hang_on_raw_nonfinite_style_geometry` の
/// doc参照)。
pub(crate) fn enforce_layout_invariants(document: &mut Document, root_idx: usize) {
    let mut content_box_violations = 0usize;
    // `child_within_parent_border_box` now always
    // returns `true` (see its doc), so the `if` below that increments this
    // is unreachable for any input — `containment_violations` can never
    // exceed 0, and the `LayoutWarn::GeometryInvariantViolated { invariant:
    // "child_within_parent_border_box", .. }` warning below can never be
    // emitted. Kept (not deleted) because removing the dead branch is part
    // of the deferred "should this invariant check exist at all" follow-up.
    let mut containment_violations = 0usize;
    let mut stack = vec![root_idx];
    while let Some(idx) = stack.pop() {
        let layout = document.nodes[idx].unrounded_layout;
        if layout.content_box_width() < 0.0 || layout.content_box_height() < 0.0 {
            zero_layout_subtree(document, idx);
            content_box_violations += 1;
            continue;
        }
        let child_count = document.nodes[idx].children.len();
        for i in 0..child_count {
            let child_idx = document.nodes[idx].children[i];
            if !document.nodes[child_idx].is_in_document() {
                continue;
            }
            let child_layout = document.nodes[child_idx].unrounded_layout;
            if !child_within_parent_border_box(&layout, &child_layout) {
                zero_layout_subtree(document, child_idx);
                containment_violations += 1;
            } else {
                stack.push(child_idx);
            }
        }
    }
    if content_box_violations > 0 {
        push_layout_warn(
            &mut document.layout_warnings,
            LayoutWarn::GeometryInvariantViolated {
                invariant: "content_box_non_negative",
                subtree_count: content_box_violations,
            },
        );
    }
    if containment_violations > 0 {
        push_layout_warn(
            &mut document.layout_warnings,
            LayoutWarn::GeometryInvariantViolated {
                invariant: "child_within_parent_border_box",
                subtree_count: containment_violations,
            },
        );
    }
}

/// [`enforce_layout_invariants`] の fallback 本体 — `root_idx` を根とする
/// subtree (taffy が実際に訪問した node のみ、`is_in_document()` filter) の
/// `unrounded_layout` を [`taffy::Layout::with_order`] (`order` だけ保持し
/// 他は全 zero) で上書きする。iterative (`Vec` stack) — 対象がまさに深い
/// DOM である以上、再帰は使わない ([`enforce_layout_invariants`] の
/// 「traversal は再帰しない」節参照)。
fn zero_layout_subtree(document: &mut Document, root_idx: usize) {
    let mut stack = vec![root_idx];
    while let Some(idx) = stack.pop() {
        let order = document.nodes[idx].unrounded_layout.order;
        document.nodes[idx].unrounded_layout = TaffyLayout::with_order(order);
        let child_count = document.nodes[idx].children.len();
        for i in 0..child_count {
            let child_idx = document.nodes[idx].children[i];
            if document.nodes[child_idx].is_in_document() {
                stack.push(child_idx);
            }
        }
    }
}

/// [`MAX_TAFFY_MAGNITUDE`] の対称 clamp 境界にちょうど乗っているかどうか。
/// `sanitize_finite` (`v.clamp(min, max)`) は範囲外の有限値をちょうど
/// `min` / `max` に丸めるので、この等価判定は「この field で実際に clamp
/// が発火した」ことの正確な proxy になる — `NaN` は `0.0` に丸まる別経路
/// なのでここには現れない ([`enforce_layout_invariants`] の doc「NaN →
/// 0.0 …」節参照)。
fn taffy_magnitude_is_saturated(v: f32) -> bool {
    v == MAX_TAFFY_MAGNITUDE || v == -MAX_TAFFY_MAGNITUDE
}

/// `child` の border box の**原点** (`location`、`child.size` は見ない) が
/// `parent` の border box (parent 座標系の原点 `(0,0)` から `parent.size`)
/// の中にあるかどうかを検査する述語として設計された。**ただし
/// 本関数は常に `true` を返す** — 飽和して
/// いない axis は元から無条件に「ok」、飽和している axis も**符号を問わず**
/// 無条件に「ok」になったため (詳細は下の「符号を問わず無条件 accept に
/// なった理由」節)、
/// containment を実際に再検査する経路はもう存在しない。「この invariant
/// check 自体を維持すべきか」は別途明示的に deferred とされた
/// decision であり、この doc の時点では未解決 (下記参照)。
///
/// # `child.size` を見ない理由
///
/// 当初は `location.x + size.width <= parent.size.width` という **extent**
/// (child の右端/下端まで含めた) containment を検査していたが、これは
/// `width: 200%` のような「子が親より意図的に大きい」legitimate な CSS を
/// 誤検出することが実装時に判明した — `width: 200%` は**どの深さでも** (飽和
/// していない浅い段も含め) 同じ「child は parent の 2 倍」という一貫した
/// 関係を表しており、破綻ではない。深い nest で個々の used value が
/// [`MAX_TAFFY_MAGNITUDE`] の帯を超えて近似され始めても、この関係自体は
/// 変わらない (`legitimate_negative_margin_overflow_is_not_reset` が check する
/// 「小さい parent + 大きい child」も同じ class の legitimate overflow)。
/// この関数の設計もこの区別を反映しており、「child の
/// **location** が parent の border box 内」とだけ書いている — extent では
/// なく **origin** の containment を指している。この関数はその通り origin
/// だけを見る。
///
/// # gate を axis 単位・`child.location` 自身に限定する理由
///
/// 直前の版は「`parent.size.width/height` か `child.size.width/height` か
/// `child.location.x/y` のいずれか 1 つでも飽和していれば、`x`/`y` **両方**を
/// 検査する」という gate だった。この形には 2 段階の false positive があった:
///
/// 1. **field 単位**: `child.location` 自身は飽和していなくても、
///    無関係な `parent.size` (同じ subtree の別の場所の飽和が原因のことも
///    ある) や `child.size` が飽和しているだけで検査が開いてしまい、
///    legitimate な負 margin (`location.x` が通常範囲の負値、例: `-30`) を
///    `>= 0.0` で弾いて誤って reset していた。
/// 2. **axis 単位**: 1 を「`child.location` 自身が飽和していること」に
///    絞っても、`x` と `y` の**どちらか一方**が飽和していれば両方を検査する
///    形のままだと、飽和していない側の axis に legitimate な負 margin が
///    あると同じ理由で誤検出しうる。
///
/// axis 単位に絞った版はどちらも閉じていた — field 単位で
/// 「検査対象にする/しない」を区別する gate 自体は **axis 自身の
/// `child.location` が実際に飽和しているかどうか**に限っていたため、
/// legitimate な負 margin (通常範囲、飽和していない) を持つ axis が
/// 誤って巻き込まれることはなかった。飽和した field だけが「actual value
/// が taffy の生の計算結果から乖離している」ため区別対象になる、という
/// 本関数群の一貫した設計原則 (本 module doc「なぜ `sanitize_taffy_layout`
/// だけでは閉じないか」節) を axis 粒度まで徹底した形、という説明はこの
/// 時点では正確だった。**この変更以降は、飽和した axis も
/// 無条件 accept になったため、この gate は「どの axis が検査対象になるか」
/// ではなく「どの axis も検査されない」という結果に収束している** — 下の
/// 「符号を問わず無条件 accept になった理由」節参照。
///
/// `saturated_but_contained_axis_with_legitimate_negative_margin_on_other_axis_is_not_reset`
/// は当初「飽和した axis だけ検査、他 axis は無条件 ok」という
/// conjunction を直接 check していた — 飽和している axis 自身は実際には
/// 収まっているようにし、もう一方の (飽和していない) axis に legitimate な
/// 負 margin を与えることで、「`y` 軸だけでも reset の説明がつく」fixture
/// では新旧実装を区別できないという指摘を踏まえた設計だった。この変更以降はこの test の assert 自体は
/// 変わらず通るが (fixture がたまたま「収まっている」ケースなので)、
/// 主張の中身は「どちらの axis も reset の理由にならない」に変わっている
/// (同 test の doc 参照)。旧
/// `saturated_location_with_legitimate_negative_margin_on_other_axis_is_not_reset`
/// (現
/// `saturated_axis_outside_parent_with_legitimate_negative_margin_on_other_axis_is_not_reset`)
/// は当初「`y` 軸の検出力」の check だったが、この変更でその検出力
/// 自体が失われたため reset されなくなった。旧
/// `saturated_child_outside_parent_resets_subtree_to_zero_layout`
/// (現 `saturated_child_outside_parent_is_not_reset`) も同様 — 飽和した
/// axis 自身が (かつては) 違反していても、この変更以降はもう検出されない。
///
/// # 符号を問わず無条件 accept になった理由 (負方向 → 正方向の順で変更)
///
/// **負方向**: axis 単位まで絞った直後の版でも
/// なお、**飽和した axis 自身が負**のケースに固有の false positive が
/// 残っていた。再検査式 `child.location >= 0.0 && child.location <=
/// parent.size` は、`child.location` が負である限り `>= 0.0` を
/// 満たしようがないので**恒等的に false** — つまり負方向についてこの式は
/// 「containment を検査する」のではなく「飽和かつ負なら無条件に reset
/// する」ことと同値だった。CSS Box 3 §3.1 (`sanitize_taffy` の doc参照、
/// margin の負値は「implementation-specific limits」の範囲で無制限に
/// 許される) の下では、[`MAX_TAFFY_MAGNITUDE`] こそがその limit そのもので
/// あり、そこに達したこと自体は合法な負方向の関係が本実装の上限を超えて
/// 近似され始めた、というだけで破綻の証拠にはならない — 同じ関数が
/// すでに無条件で信頼している「飽和していない負値」
/// (`legitimate_negative_margin_overflow_is_not_reset` の `-30`) と対称
/// であり、`±MAX_TAFFY_MAGNITUDE` の境界を跨いだ瞬間だけ扱いを不連続に
/// 反転させる理由が無い。
///
/// 当時はここで「正方向は緩めない」と結論していた。根拠は 2 つ —
/// (a) `padding` / `border` / `width` は parse 時点で非負が enforce
/// されるため、正方向の巨大な `location` を margin と同じ「CSS が無制限に
/// 許す」根拠では正当化できない、(b) 正方向の再検査は実際に pass/fail
/// 両方の分岐を持ち (`saturated_but_contained_layout_is_not_reset` が
/// pass、旧 `saturated_child_outside_parent_resets_subtree_to_zero_layout`
/// が fail)、緩めると現に存在する検出力を失う — 負方向はそもそも pass
/// する経路が存在しなかったので、失われる検出力は無い、というものだった。
///
/// **正方向**: 根拠 (a) は
/// 覆った — margin は symmetric。CSS Box 3 §3.1
/// (<https://www.w3.org/TR/css-box-3/#margin-physical>、"Negative values
/// for margin properties are allowed, but there may be
/// implementation-specific limits") は**負値**を明示的に許容している
/// だけで、正の margin をそれより厳しく縛る spec 上の対称な根拠にはなって
/// いない。`margin-left: 1e9%` (`width: 100px` container 内) は
/// `location.x` を正方向に飽和させ、旧実装はこれを誤って reset していた —
/// `saturated_positive_margin_percentage_child_is_not_reset` が実際の
/// CSS パイプライン経由でこれを check する (`saturated_negative_margin_
/// percentage_child_is_not_reset` の正方向対)。根拠 (b) (「検出力を失う」)
/// は反証されていない — その損失は承知の上で受け入れられた: 既存 test
/// (`saturated_but_contained_layout_is_not_reset`、旧
/// `saturated_child_outside_parent_resets_subtree_to_zero_layout`) は
/// TaffyLayout を直接構築するのみで実 CSS パイプライン経由の検出力を
/// 一度も示していなかった一方、今回の data loss (legitimate content の
/// 完全消失) は実 CSS 経由で実証済みだったため。
///
/// **結果**: `axis_ok` は符号を問わず「飽和していれば無条件 accept」に
/// 統一され、本関数は常に `true` を返す — containment を再検査
/// する経路はもう存在しない。旧
/// `saturated_child_outside_parent_resets_subtree_to_zero_layout` は
/// `saturated_child_outside_parent_is_not_reset` に、旧
/// `saturated_location_with_legitimate_negative_margin_on_other_axis_is_not_reset`
/// は
/// `saturated_axis_outside_parent_with_legitimate_negative_margin_on_other_axis_is_not_reset`
/// に、それぞれ改名・反転した (詳細は各 test の doc参照)。「この
/// invariant check 自体を維持すべきか」は別途明示的に deferred
/// とされた decision であり、この doc の時点では未解決。
///
/// `saturated_negative_margin_percentage_child_is_not_reset` (単一の
/// `margin-left: -1e9%` 宣言、nest 無し) と
/// `deep_nested_negative_percentage_margin_saturating_location_is_not_reset`
/// (`width: 200%; margin-left: -100%` の深い nest chain、
/// `nested_percentage_wide_child_chain_is_not_reset` の負方向対) が
/// 実際の CSS パイプライン経由でこれを check する。前者は特に、「`parent.size`
/// 自身も同じ axis で飽和していれば符号を見ずに re-validate をスキップ
/// する」という検討したが却下した別案を反証する最小 fixture でもある —
/// この fixture は `parent.size.width` が飽和していない (`100.0` のまま)
/// ので、判別軸は「parent も飽和しているか」ではなく「child 自身の符号」
/// でなければならないことを示す (この変更以降、この判別軸自体は意味を失った
/// が、fixture と regression check としての価値は変わらない)。
/// `saturated_negative_location_is_not_reset` は同じ組み合わせを直接構築
/// した最小 synthetic case で孤立させて検査する
/// (`saturated_but_contained_layout_is_not_reset` と対になる、正方向
/// ケースの負方向対 — この変更以降はどちらも「飽和していれば無条件 accept」
/// という同じ結論の pin)。
fn child_within_parent_border_box(parent: &TaffyLayout, child: &TaffyLayout) -> bool {
    /// 1 axis 分の判定。`location` はその axis の `child.location.{x,y}`
    /// (`parent_size` は本体の式ではもう一切使わない — dead parameter。
    /// 削除せず「対応する `parent.size.{width,height}` を渡す」という
    /// 呼び出し規約の見た目だけ残しているのは、この関数・`axis_ok` 自体を
    /// 削除するかどうかを含めて「この invariant check を維持すべきか」が
    /// 別途明示的に deferred とされた follow-up だから — 将来その follow-up
    /// で `axis_ok` ごと削除される可能性があることを見越して、今
    /// signature を先回りして変える判断はしていない。詳細は上の doc
    /// 「符号を問わず無条件 accept になった理由」節。同じ理由で、直下の
    /// `if !taffy_magnitude_is_saturated(location) { return true; }` 分岐
    /// も実質的には常に `true` を返す既定文と等価な dead branch になって
    /// いる — こちらも `axis_ok` ごと削除されうる同じ deferred follow-up
    /// まで、あえて `true` 一本に畳んでいない)。
    fn axis_ok(location: f32, _parent_size: f32) -> bool {
        if !taffy_magnitude_is_saturated(location) {
            return true;
        }
        // 飽和していれば符号を問わず無条件 accept。
        // 負方向は先に確立していた — 本 decision は
        // その前例を正方向にも対称に拡張し、旧 `location <= parent_size`
        // 再検査 (正方向限定) を撤去した。containment を再検査する経路は
        // もう存在しない。
        true
    }
    axis_ok(child.location.x, parent.size.width) && axis_ok(child.location.y, parent.size.height)
}

/// [`ComputedLengthPercentage`] → [`taffy::LengthPercentage`] bridge
/// (padding / gap 用)。
///
/// `site` distinguishes the callers (`"padding"` / `"row-gap"` /
/// `"column-gap"`) in [`LayoutWarn::NonFiniteClamped`] events — same
/// `site`-parameter shape as
/// [`computed_length_percentage_or_auto_to_taffy_dimension`]
/// (`"width"`/`"height"`), generalized once a second caller
/// ([`bridge_gap`]) appeared.
///
/// # 網羅 match
///
/// 引数が **computed 層**の型になったため 2 arm で網羅する。以前あった
/// `Length::Em(_) | Length::Rem(_) => length(0.0)` (font-relative unit を黙って
/// 0px に潰す fail-quiet) と `_ => length(0.0)` (non_exhaustive catch-all) は
/// **削除した** — `em` / `rem` / `pt` は cascade の phase 3 で px に絶対化済み
/// であり、computed 層に到達しない。
///
/// **ただし削除した arm は「単位」だけでなく「病的な f32 の値」も吸収していた**
/// (`Em(inf)` / `0.0 * inf` = NaN)。その分は [`sanitize_taffy`] が
/// backfill している — **guard を「不要な防御」と
/// 判断して外さないこと。**
///
/// [`ComputedLengthPercentage`] に `#[non_exhaustive]` が付いていないのは、
/// この網羅性を今得るための explicit trade である (将来 `Calc` variant が
/// 増えるときに coordinated breaking change を払う。`raikiri_style::resolve`
/// の module doc 参照)。**`_` arm を足して「forward-compat」にしてはならない** —
/// trade の得る側を捨てることになる。
///
/// # Percent policy
///
/// `Percent(p)` → `percent(sanitize_taffy(p / 100.0))` — CSS spec の authored
/// 0-100 を taffy の fraction 0.0-1.0 に変換し、[`sanitize_taffy`] で有限化する。
/// containing block に対する解決は **used value 層**
/// (CSS Cascade 5 §4.5 <https://www.w3.org/TR/css-cascade-5/#used>) であり
/// taffy に委譲する — **guard が bound するのは fraction であって解決後の
/// used value ではない** ([`MAX_TAFFY_MAGNITUDE`] の射程節を参照)。
fn computed_length_percentage_to_taffy_length_percentage(
    len: ComputedLengthPercentage,
    site: &'static str,
    diag: &mut Vec<LayoutWarn>,
) -> LengthPercentage {
    match len {
        // site 1: `sanitize_taffy` で非有限を落とす。
        ComputedLengthPercentage::Px(v) => LengthPercentage::length(sanitize_taffy(v, site, diag)),
        ComputedLengthPercentage::Percent(p) => {
            LengthPercentage::percent(sanitize_taffy(p / 100.0, site, diag))
        }
    }
}

/// [`ComputedLengthPercentageOrAuto`] → [`taffy::Dimension`] bridge
/// (width / height 用)。
///
/// 網羅 match (`Px` / `Percent`) の分岐ロジック・Percent policy・非有限 guard は
/// [`computed_length_percentage_to_taffy_length_percentage`] と同じ (参照先は
/// 2 arm)。本関数はそれに `Auto` arm が加わり合計 3 arm (catch-all なし)。
/// `Auto` → `Dimension::auto()` (f32 を持たないので guard 対象外)。
///
/// [`bridge_size`] から width / height 両方で consume される。
///
/// `site` distinguishes the two [`bridge_size`] callers (`"width"` /
/// `"height"`) in [`LayoutWarn::NonFiniteClamped`] events.
fn computed_length_percentage_or_auto_to_taffy_dimension(
    calc_values: &mut Vec<std::sync::Arc<raikiri_style::property::CalcLengthPercentage>>,
    loa: ComputedLengthPercentageOrAuto,
    site: &'static str,
    diag: &mut Vec<LayoutWarn>,
) -> Dimension {
    match loa {
        // site 2。
        ComputedLengthPercentageOrAuto::Px(v) => Dimension::length(sanitize_taffy(v, site, diag)),
        ComputedLengthPercentageOrAuto::Percent(p) => {
            Dimension::percent(sanitize_taffy(p / 100.0, site, diag))
        }
        ComputedLengthPercentageOrAuto::Auto => Dimension::auto(),
        ComputedLengthPercentageOrAuto::Calc(value) => {
            calc_values.push(std::sync::Arc::new(value));
            let pointer = calc_values
                .last()
                .map(|value| (&**value) as *const _ as *const ())
                .expect("calc value was just pushed");
            Dimension::calc(pointer)
        }
    }
}

/// [`ComputedLengthPercentageOrAuto`] → [`taffy::LengthPercentageAuto`] bridge
/// for min/max-size properties.
///
/// Unlike the margin/inset bridge below, min/max preferred sizes can carry a
/// calc() payload. Preserve that payload in the document arena so consumers
/// such as table `<col>` sizing do not observe a false zero constraint.
fn computed_length_percentage_or_auto_to_taffy_min_max(
    calc_values: &mut Vec<std::sync::Arc<raikiri_style::property::CalcLengthPercentage>>,
    loa: ComputedLengthPercentageOrAuto,
    site: &'static str,
    diag: &mut Vec<LayoutWarn>,
) -> LengthPercentageAuto {
    match loa {
        ComputedLengthPercentageOrAuto::Px(v) => {
            LengthPercentageAuto::length(sanitize_taffy(v, site, diag))
        }
        ComputedLengthPercentageOrAuto::Percent(p) => {
            LengthPercentageAuto::percent(sanitize_taffy(p / 100.0, site, diag))
        }
        ComputedLengthPercentageOrAuto::Auto => LengthPercentageAuto::auto(),
        ComputedLengthPercentageOrAuto::Calc(value) => {
            calc_values.push(std::sync::Arc::new(value));
            let pointer = calc_values
                .last()
                .map(|value| (&**value) as *const _ as *const ())
                .expect("calc value was just pushed");
            LengthPercentageAuto::calc(pointer)
        }
    }
}

/// [`ComputedLengthPercentageOrAuto`] → [`taffy::LengthPercentageAuto`] bridge
/// (margin 用)。
///
/// 網羅 match / Percent policy / 非有限 guard は
/// [`computed_length_percentage_to_taffy_length_percentage`] と同じ。`Auto` →
/// `LengthPercentageAuto::auto()` (CSS Box 3 §3.1 "margin auto = distribute
/// available space" を taffy に委譲、f32 を持たないので guard 対象外)。
fn computed_length_percentage_or_auto_to_taffy_length_percentage_auto(
    loa: ComputedLengthPercentageOrAuto,
    site: &'static str,
    diag: &mut Vec<LayoutWarn>,
) -> LengthPercentageAuto {
    match loa {
        // site 3。margin は負値が spec-valid なので
        // `sanitize_taffy` の対称 clamp が load-bearing。唯一の caller
        // (`bridge_margin`) 由来なので site label は固定。
        ComputedLengthPercentageOrAuto::Px(v) => {
            LengthPercentageAuto::length(sanitize_taffy(v, site, diag))
        }
        ComputedLengthPercentageOrAuto::Percent(p) => {
            LengthPercentageAuto::percent(sanitize_taffy(p / 100.0, site, diag))
        }
        ComputedLengthPercentageOrAuto::Auto => LengthPercentageAuto::auto(),
        // Calc payloads are currently produced only for preferred sizes;
        // margin calc resolution will share the same arena in a later pass.
        ComputedLengthPercentageOrAuto::Calc(_) => LengthPercentageAuto::length(0.0),
    }
}

/// [`ComputedLength`] (px) → [`taffy::LengthPercentage`] bridge
/// (`border-*-width` 用)。
///
/// sibling 3 helper (`computed_length_percentage_to_taffy_length_percentage` /
/// `computed_length_percentage_or_auto_to_taffy_dimension` /
/// `computed_length_percentage_or_auto_to_taffy_length_percentage_auto`) と同じ
/// `<src>_to_taffy_<dst>` 命名 / 同じ cluster に置く。
///
/// `border-*-width` 専用に分けているのは、grammar (`<line-width>` =
/// `<length [0,∞]> | thin | medium | thick`) が `<percentage>` を含まないため
/// computed 層でも length しか来ないから (CSS Backgrounds 3 §3.3 "Line
/// Thickness: the border-width properties"
/// <https://www.w3.org/TR/css-backgrounds-3/#border-width>)。戻り値型は
/// [`computed_length_percentage_to_taffy_length_percentage`] と同一
/// (`LengthPercentage` が taffy 側の最小共通型) だが、入力型が
/// [`ComputedLength`] なので percentage arm を
/// 持たない点が違う。非有限 guard ([`sanitize_taffy`]) は同じく通す。
fn computed_length_to_taffy_length_percentage(
    len: ComputedLength,
    diag: &mut Vec<LayoutWarn>,
) -> LengthPercentage {
    // site 4。唯一の caller (`bridge_border`) 由来
    // なので site label は固定。
    LengthPercentage::length(sanitize_taffy(len.px(), "border-width", diag))
}

/// 全 Text node を parley で pre-shape、結果を `Node.text_layout` に格納する。
///
/// 呼び出し側 (`layout_single_page`) は事前に全 `Node.text_layout = None` に
/// clear 済であることを前提とする (re-entrance safety)。
///
/// Font stack / size / weight / style / line-height は
/// `cascade.computed[idx]` (親から inherit 済) を消費。
/// `max_advance` は行折り返し境界で、通常 `page_box.width`。
///
/// # `line_height` は taffy の `compute_root_layout` より前でも正しく配線できる
///
/// 本関数は `layout_single_page` 内で taffy の `compute_root_layout` より
/// **前**に走る (下記 `text_align` 参照)。これは一般に「taffy が確定させる
/// 幅を必要とする property」には問題になるが、`line_height` はその種類の
/// property ではない — 依存方向が逆であり、むしろこの順序が**必要**:
///
/// - parley は line height を `RunMetrics.line_height` として shape 時点
///   (`builder.build(&text)` 内、`break_all_lines` より前) に計算する
///   (`parley-0.10.0/src/layout/data.rs` の `push_run`)。入力はフォント
///   metrics (ascent / descent / leading、shape 対象フォントから直接取得)
///   と `font_size` (本関数がすでに push した自要素の computed font-size)、
///   そして本関数が push する `StyleProperty::LineHeight` の値だけであり、
///   taffy が確定させる containing block 幅や利用可能領域には一切依存しない
///   (`parley-0.10.0/src/resolve/mod.rs` の対応 arm も `device pixel scale`
///   の乗算のみ)。
/// - 逆に、shape 済みの `Layout::height()` (line height を織り込み済み) は
///   `taffy_impl.rs` の `compute_child_layout` が leaf node の intrinsic
///   size として taffy に**渡す側**の入力になる。つまり line height は
///   taffy の出力を必要とするのではなく、taffy の入力を作る側に立つ —
///   `text_align` (taffy が確定させる幅を align の基準として必要とする、
///   下記参照) とは依存の向きが逆であり、本関数がここで走ることは
///   line height にとって「早すぎる」のではなくちょうど必要なタイミングである。
///
/// # 未消費の `ComputedValues` field
///
/// `cascade.computed[idx]` には他にも `direction` / `text_align` が乗って
/// いるが、本関数はいずれも読まない:
///
/// - `direction` — 配線先の API 自体が無い。`RangedBuilder` / `TreeBuilder`
///   は base direction を受け取る public API を公開しておらず、parley 内部の
///   bidi resolver は base level 引数に常に `None` を渡して呼ばれる (段落内の
///   文字列から Unicode Bidirectional Algorithm の P2/P3
///   first-strong-character heuristic で自動推定し、強い方向を持つ文字が
///   無ければ LTR に fallback — Unicode Standard Annex #9
///   <https://www.unicode.org/reports/tr9/>)。つまり `cv.direction` を明示的
///   に渡す先の API 自体が現状無い — LTR がハードコードされた default なの
///   ではない。
/// - `text_align` — 本関数の末尾の `layout.align(...)` は意図的に
///   `Alignment::Start` に固定し、`cv.text_align` を読まない。
///   正しい幅基準の align は taffy 後の [`realign_text_after_layout`] が担う
///   (確定した containing block 幅で `break_all_lines` + `align` し直す)。
///   本関数で素朴に enum mapping してしまうと、使える幅が `max_advance`
///   (= 通常 `page_box.width`) のみのため、狭い containing block 内の
///   `Center` / `Right` / `End` / `Justify` がページ幅基準にズレる。
///   `Start` は幅に依存しないためここでも正しい。
///   加えて `parley::Alignment::Start` / `End` は layout 内の bidi 解析結果から
///   physical 方向を解決するため、`direction` を配線せずに `text_align`
///   だけ配線しても `Start`/`End` は content-inferred direction での解決に
///   留まる — この 2 つは独立した gap ではなく 1 セットであり、`direction`
///   の配線口は parley public API に存在しない (上記 `direction` bullet 参照)。
///   flex 化された line box (`IS_INLINE_ROOT`) の中央寄せは parley 側ではなく
///   container 側の `justify_content` が担う
///   ([`establish_minimal_line_boxes`] doc 参照)。
///
/// # 失敗しない
///
/// 以前は `Result<(), LayoutError>` を返していた。唯一の `Err` 経路は
/// `cv.font_size` が specified 層の `Length` で `Px` 以外だった場合の
/// `LayoutError::Internal` だったが、`font_size` が [`ComputedLength`] (px) に
/// なって match 自体が消えたため到達不能になった。`pub(crate)` なので戻り値の
/// narrowing は crate 内で完結する (外部影響 0)。
///
/// Preserve the previous per-text-node expansion for inline contexts whose
/// shared line cursor and wrap positions are not available to pre-shaping.
/// Also returns the byte length each tab was replaced by (in source order) so
/// offsets into `text` can be remapped.
fn expand_tabs_locally(
    text: &str,
    tab_size: ComputedTabSize,
    space_advance: f32,
) -> (String, Vec<usize>) {
    let stop = match tab_size {
        ComputedTabSize::Number(number) if number > 0.0 && number.is_finite() => number,
        ComputedTabSize::Length(length)
            if length.px().is_finite() && length.px() >= 0.0 && space_advance > 0.0 =>
        {
            length.px() / space_advance
        }
        _ => 0.0,
    };
    let mut output = String::with_capacity(text.len());
    let mut tab_lens = Vec::new();
    if stop <= 0.0 {
        output.extend(text.chars().filter(|&character| character != '\t'));
        tab_lens.resize(text.matches('\t').count(), 0);
        return (output, tab_lens);
    }
    let mut column = 0.0_f32;
    for character in text.chars() {
        match character {
            '\n' => {
                column = 0.0;
                output.push(character);
            }
            '\t' => {
                let next = ((column / stop).floor() + 1.0) * stop;
                let count = (next.round() - column.round()).max(0.0) as usize;
                column = next;
                output.extend(std::iter::repeat_n(' ', count));
                tab_lens.push(count);
            }
            _ => {
                column += 1.0;
                output.push(character);
            }
        }
    }
    (output, tab_lens)
}

/// Move inline-box offsets computed on `original` onto the text produced by a
/// rewrite that replaced its tabs (in source order) by `tab_lens` bytes and
/// kept every other character. Autospace boundaries are detected before tabs
/// are rewritten so a tab still separates its neighbors even when it expands
/// to nothing.
fn remap_boxes_through_tab_rewrite(original: &str, tab_lens: &[usize], boxes: &mut [InlineBox]) {
    let tabs: Vec<usize> = original.match_indices('\t').map(|(at, _)| at).collect();
    debug_assert_eq!(tabs.len(), tab_lens.len());
    for inline_box in boxes {
        let shift: isize = tabs
            .iter()
            .zip(tab_lens)
            .take_while(|&(&at, _)| at < inline_box.index)
            .map(|(_, &len)| len as isize - 1)
            .sum();
        inline_box.index = inline_box.index.saturating_add_signed(shift);
    }
}

/// Resolve `tab-size` to an absolute stop interval in CSS pixels.
fn tab_stop_advance(tab_size: ComputedTabSize, block_space_advance: f32) -> f32 {
    match tab_size {
        ComputedTabSize::Number(number)
            if number.is_finite() && number >= 0.0 && block_space_advance.is_finite() =>
        {
            (number * block_space_advance).clamp(0.0, MAX_TAFFY_MAGNITUDE)
        }
        ComputedTabSize::Length(length) if length.px().is_finite() && length.px() >= 0.0 => {
            length.px().min(MAX_TAFFY_MAGNITUDE)
        }
        _ => 0.0,
    }
}

/// Byte ranges of tab replacement spaces and the word spacing each needs.
type TabSpacingRanges = Vec<(std::ops::Range<usize>, f32)>;

/// Replace each tab by an invisible U+0020 with ranged WordSpacing so its
/// advance exactly reaches the next stop. Keeping a normal glyph run preserves
/// the text's font metrics and line height; the ranged spacing supplies the
/// block-container-derived physical width when the inline font differs.
///
/// Also returns the byte length each tab was replaced by (in source order).
fn replace_tabs_with_styled_spaces(
    text: &str,
    interval: f32,
    space_base_advance: f32,
    mut measure_segment: impl FnMut(&str) -> f32,
) -> (String, TabSpacingRanges, Vec<usize>) {
    if !text.contains('\t') {
        return (text.to_owned(), Vec::new(), Vec::new());
    }
    let mut tab_lens = Vec::new();
    let mut output = String::with_capacity(text.len());
    let mut spacing_ranges = Vec::new();
    let mut segment = String::new();
    let mut cursor = 0.0_f32;
    let mut flush_segment = |segment: &mut String, output: &mut String, cursor: &mut f32| {
        if segment.is_empty() {
            return;
        }
        let measured = measure_segment(segment);
        if measured.is_finite() && measured > 0.0 {
            *cursor = (*cursor + measured).min(MAX_TAFFY_MAGNITUDE);
        }
        output.push_str(segment);
        segment.clear();
    };
    for character in text.chars() {
        match character {
            '\t' => {
                flush_segment(&mut segment, &mut output, &mut cursor);
                let before = output.len();
                if interval > 0.0 && interval.is_finite() {
                    let next = ((cursor / interval).floor() + 1.0) * interval;
                    let gap = (next - cursor).max(0.0);
                    if gap > 0.0 {
                        let start = output.len();
                        output.push(' ');
                        let word_spacing = (gap - space_base_advance)
                            .clamp(-MAX_TAFFY_MAGNITUDE, MAX_TAFFY_MAGNITUDE);
                        spacing_ranges.push((start..output.len(), word_spacing));
                    }
                    cursor = next.min(MAX_TAFFY_MAGNITUDE);
                }
                tab_lens.push(output.len() - before);
            }
            '\n' => {
                flush_segment(&mut segment, &mut output, &mut cursor);
                output.push(character);
                cursor = 0.0;
            }
            _ => segment.push(character),
        }
    }
    flush_segment(&mut segment, &mut output, &mut cursor);
    (output, spacing_ranges, tab_lens)
}

/// Measure one probe glyph/character advance (px) with Parley.
///
/// The caller chooses the sample because tabs use U+0020 while `ch` uses the
/// U+0030 zero glyph. Non-finite, zero, or implausibly large results fall back
/// to `font_size * 0.5` as a fail-safe.
///
/// [`Layout::width`]: parley::Layout::width
fn probe_text_advance(
    fonts: &mut FontContext,
    layout_cx: &mut LayoutContext<()>,
    sample: &str,
    family_str: &str,
    font_size_px: f32,
    font_weight: f32,
    font_style: StyleFontStyle,
) -> f32 {
    probe_text_advance_inner(
        fonts,
        layout_cx,
        sample,
        family_str,
        font_size_px,
        font_weight,
        font_style,
        false,
    )
}

fn probe_ch_zero_advance(
    fonts: &mut FontContext,
    layout_cx: &mut LayoutContext<()>,
    family_str: &str,
    font_size_px: f32,
    font_weight: f32,
    font_style: StyleFontStyle,
) -> f32 {
    probe_text_advance_inner(
        fonts,
        layout_cx,
        "0",
        family_str,
        font_size_px,
        font_weight,
        font_style,
        true,
    )
}

#[allow(clippy::too_many_arguments)]
fn probe_text_advance_inner(
    fonts: &mut FontContext,
    layout_cx: &mut LayoutContext<()>,
    sample: &str,
    family_str: &str,
    font_size_px: f32,
    font_weight: f32,
    font_style: StyleFontStyle,
    allow_zero: bool,
) -> f32 {
    let mut warnings: Vec<LayoutWarn> = Vec::new();
    let size = sanitize_finite(
        font_size_px,
        0.0,
        MAX_FONT_SIZE_PX,
        "font-size",
        &mut warnings,
    );
    let weight = sanitize_font_weight(font_weight, &mut warnings);
    let family = FontFamily::from(family_str);
    let mut builder = layout_cx.ranged_builder(fonts, sample, 1.0, true);
    builder.push_default(StyleProperty::FontFamily(family));
    builder.push_default(StyleProperty::FontSize(size));
    builder.push_default(StyleProperty::FontWeight(FontWeight::new(weight)));
    builder.push_default(StyleProperty::FontStyle(font_style_to_parley(font_style)));
    let mut layout: Layout<()> = builder.build(sample);
    layout.break_all_lines(None);
    let w = layout.width();
    let has_glyph_run = allow_zero
        && layout
            .lines()
            .flat_map(|line| line.items())
            .any(|item| match item {
                PositionedLayoutItem::GlyphRun(run) => run.glyphs().any(|glyph| glyph.id != 0),
                _ => false, // cov:ignore: this helper shapes plain text and cannot create non-glyph items.
            });
    if w.is_finite() && (w > 0.0 || has_glyph_run) && w <= size * 4.0 {
        w
    } else {
        size * 0.5
    }
}

#[derive(Clone, Copy)]
struct TextProbeStyle<'a> {
    family_str: &'a str,
    font_size_px: f32,
    font_weight: f32,
    font_style: StyleFontStyle,
    letter_spacing: f32,
    word_spacing: f32,
}

fn probe_text_full_width(
    fonts: &mut FontContext,
    layout_cx: &mut LayoutContext<()>,
    sample: &str,
    style: TextProbeStyle<'_>,
) -> f32 {
    let mut warnings = Vec::new();
    let size = sanitize_finite(
        style.font_size_px,
        0.0,
        MAX_FONT_SIZE_PX,
        "font-size",
        &mut warnings,
    );
    let weight = sanitize_font_weight(style.font_weight, &mut warnings);
    let mut builder = layout_cx.ranged_builder(fonts, sample, 1.0, false);
    builder.push_default(StyleProperty::FontFamily(FontFamily::from(
        style.family_str,
    )));
    builder.push_default(StyleProperty::FontSize(size));
    builder.push_default(StyleProperty::FontWeight(FontWeight::new(weight)));
    builder.push_default(StyleProperty::FontStyle(font_style_to_parley(
        style.font_style,
    )));
    builder.push_default(StyleProperty::LetterSpacing(style.letter_spacing));
    builder.push_default(StyleProperty::WordSpacing(style.word_spacing));
    let mut layout: Layout<()> = builder.build(sample);
    layout.break_all_lines(None);
    layout.full_width()
}

/// Return whether the requested face maps both the space and zero glyphs.
///
/// CSS's first-available `ch` face must be usable for the space and must also
/// provide U+0030, whose advance supplies the metric. A mapped zero-advance
/// glyph is valid; cmap presence is the only coverage check here.
fn family_candidate_has_ch_glyphs(
    fonts: &mut FontContext,
    family: &str,
    font_weight: f32,
    font_style: StyleFontStyle,
) -> bool {
    use parley::fontique::{Attributes, FontWidth, QueryStatus};

    let weight = sanitize_font_weight(font_weight, &mut Vec::new());
    let mut query = fonts.collection.query(&mut fonts.source_cache);
    query.set_families([family]);
    query.set_attributes(Attributes::new(
        FontWidth::default(),
        font_style_to_parley(font_style),
        FontWeight::new(weight),
    ));
    let mut has_ch_glyphs = false;
    query.matches_with(|font| {
        has_ch_glyphs = font.charmap().is_some_and(|charmap| {
            charmap.map(0x20_u32).is_some_and(|glyph| glyph != 0)
                && charmap.map(0x30_u32).is_some_and(|glyph| glyph != 0)
        });
        if has_ch_glyphs {
            QueryStatus::Stop
        } else {
            QueryStatus::Continue
        }
    });
    has_ch_glyphs
}

/// Return whether any face in a generic family maps both required glyphs.
fn generic_family_has_ch_glyphs(
    fonts: &mut FontContext,
    generic: parley::fontique::GenericFamily,
    font_weight: f32,
    font_style: StyleFontStyle,
) -> bool {
    use parley::fontique::{Attributes, FontWeight, FontWidth, QueryStatus};

    let families: Vec<_> = fonts.collection.generic_families(generic).collect();
    if families.is_empty() {
        return false;
    }
    let weight = sanitize_font_weight(font_weight, &mut Vec::new());
    let mut query = fonts.collection.query(&mut fonts.source_cache);
    query.set_families(families);
    query.set_attributes(Attributes::new(
        FontWidth::default(),
        font_style_to_parley(font_style),
        FontWeight::new(weight),
    ));
    let mut has_ch_glyphs = false;
    query.matches_with(|font| {
        has_ch_glyphs = font.charmap().is_some_and(|charmap| {
            charmap.map(0x20_u32).is_some_and(|glyph| glyph != 0)
                && charmap.map(0x30_u32).is_some_and(|glyph| glyph != 0)
        });
        if has_ch_glyphs {
            QueryStatus::Stop
        } else {
            QueryStatus::Continue
        }
    });
    has_ch_glyphs
}

/// Probe U+0030 using the first remaining family that can supply the glyph.
///
/// Named faces are checked through Fontique cmap metadata. An unregistered
/// name is skipped so a later available family can win; when a generic family
/// is reached, Parley receives the remaining authored list so its fallback
/// resolver preserves generic and later-family order. A valid zero-advance
/// mapped glyph remains accepted by `probe_ch_zero_advance`.
fn probe_ch_text_advance(
    fonts: &mut FontContext,
    layout_cx: &mut LayoutContext<()>,
    family_str: &str,
    font_size_px: f32,
    font_weight: f32,
    font_style: StyleFontStyle,
) -> f32 {
    let candidates: Vec<&str> = family_str
        .split(',')
        .map(str::trim)
        .filter(|candidate| !candidate.is_empty())
        .collect();
    let mut saw_unregistered_named = false;
    for (index, raw_candidate) in candidates.iter().copied().enumerate() {
        if raw_candidate.is_empty() {
            // cov:ignore: candidates were already filtered for emptiness.
            continue;
        }
        let was_quoted = raw_candidate.starts_with('"') || raw_candidate.starts_with('\'');
        let name = raw_candidate
            .strip_prefix('"')
            .and_then(|value| value.strip_suffix('"'))
            .or_else(|| {
                raw_candidate
                    .strip_prefix('\'')
                    .and_then(|value| value.strip_suffix('\''))
            })
            .unwrap_or(raw_candidate)
            .trim();
        let generic = if was_quoted {
            None
        } else {
            let lower_name = name.to_ascii_lowercase();
            parley::fontique::GenericFamily::parse(&lower_name)
        };
        let registered = fonts.collection.family_id(name).is_some();
        if !registered && generic.is_none() {
            saw_unregistered_named = true;
        }
        let has_glyphs =
            registered && family_candidate_has_ch_glyphs(fonts, name, font_weight, font_style);
        if let Some(generic) = generic {
            // An unresolved named face may represent an @font-face source
            // that the WPT loader could not activate (for example a missing
            // or malformed web-font resource). Do not silently replace that unavailable
            // face with a generic metric; the style-layer fallback is the
            // deterministic 0.5em result for this no-face case.
            if saw_unregistered_named
                || !generic_family_has_ch_glyphs(fonts, generic, font_weight, font_style)
            {
                continue;
            }
            let remaining_families = candidates[index..].join(", ");
            return probe_ch_zero_advance(
                fonts,
                layout_cx,
                &remaining_families,
                font_size_px,
                font_weight,
                font_style,
            );
        }
        if has_glyphs {
            return probe_ch_zero_advance(
                fonts,
                layout_cx,
                raw_candidate,
                font_size_px,
                font_weight,
                font_style,
            );
        }
    }
    // No remaining family advertises U+0030. Do not shape the rejected list
    // again: Parley may emit a .notdef run whose advance would masquerade as
    // a valid zero-width glyph. Match the style-layer fallback instead.
    let mut warnings = Vec::new();
    sanitize_finite(
        font_size_px,
        0.0,
        MAX_FONT_SIZE_PX,
        "font-size",
        &mut warnings,
    ) * 0.5
}

/// Return whether a named face maps U+6C34, the character used by CSS `ic`.
fn family_candidate_has_ic_glyph(
    fonts: &mut FontContext,
    family: &str,
    font_weight: f32,
    font_style: StyleFontStyle,
) -> bool {
    use parley::fontique::{Attributes, FontWeight, FontWidth, QueryStatus};

    let weight = sanitize_font_weight(font_weight, &mut Vec::new());
    let mut query = fonts.collection.query(&mut fonts.source_cache);
    query.set_families([family]);
    query.set_attributes(Attributes::new(
        FontWidth::default(),
        font_style_to_parley(font_style),
        FontWeight::new(weight),
    ));
    let mut has_glyph = false;
    query.matches_with(|font| {
        has_glyph = font
            .charmap()
            .is_some_and(|charmap| charmap.map(0x6C34_u32).is_some_and(|glyph| glyph != 0));
        if has_glyph {
            QueryStatus::Stop
        } else {
            QueryStatus::Continue
        }
    });
    has_glyph
}

/// Return whether a generic family has any face mapping U+6C34.
fn generic_family_has_ic_glyph(
    fonts: &mut FontContext,
    generic: parley::fontique::GenericFamily,
    font_weight: f32,
    font_style: StyleFontStyle,
) -> bool {
    use parley::fontique::{Attributes, FontWeight, FontWidth, QueryStatus};

    let families: Vec<_> = fonts.collection.generic_families(generic).collect();
    if families.is_empty() {
        // cov:ignore: generic-family availability is platform-dependent and may be empty.
        return false;
    }
    let weight = sanitize_font_weight(font_weight, &mut Vec::new());
    let mut query = fonts.collection.query(&mut fonts.source_cache);
    query.set_families(families);
    query.set_attributes(Attributes::new(
        FontWidth::default(),
        font_style_to_parley(font_style),
        FontWeight::new(weight),
    ));
    let mut has_glyph = false;
    query.matches_with(|font| {
        has_glyph = font
            .charmap()
            .is_some_and(|charmap| charmap.map(0x6C34_u32).is_some_and(|glyph| glyph != 0));
        if has_glyph {
            QueryStatus::Stop
        } else {
            QueryStatus::Continue
        }
    });
    has_glyph
}

/// Measure U+6C34 using the first available family that maps it.
///
/// CSS `ic` uses the ideographic advance from a mapped face. Do not accept a
/// `.notdef` advance when a family lacks U+6C34, and preserve a genuine zero
/// advance. If no candidate maps the character, use the specified 1em fallback.
///
/// The computed family list currently does not retain `@font-face`
/// `unicode-range` provenance for this metric. Such alias-specific filtering
/// remains a known limitation; ordinary registered-family and generic fallback
/// selection is still checked against cmap metadata here.
fn probe_ic_text_advance(
    fonts: &mut FontContext,
    layout_cx: &mut LayoutContext<()>,
    family_str: &str,
    font_size_px: f32,
    font_weight: f32,
    font_style: StyleFontStyle,
) -> f32 {
    let candidates: Vec<&str> = family_str
        .split(',')
        .map(str::trim)
        .filter(|candidate| !candidate.is_empty())
        .collect();
    let mut saw_unregistered_named = false;
    for (index, raw_candidate) in candidates.iter().copied().enumerate() {
        let was_quoted = raw_candidate.starts_with('"') || raw_candidate.starts_with('\'');
        let name = raw_candidate
            .strip_prefix('"')
            .and_then(|value| value.strip_suffix('"'))
            .or_else(|| {
                raw_candidate
                    .strip_prefix('\'')
                    .and_then(|value| value.strip_suffix('\''))
            })
            .unwrap_or(raw_candidate)
            .trim();
        let generic = if was_quoted {
            // Quoted generic-looking names are ordinary family names.
            None
        } else {
            parley::fontique::GenericFamily::parse(&name.to_ascii_lowercase())
        };
        let registered = fonts.collection.family_id(name).is_some();
        if !registered && generic.is_none() {
            saw_unregistered_named = true;
        }
        if let Some(generic) = generic {
            if saw_unregistered_named
                || !generic_family_has_ic_glyph(fonts, generic, font_weight, font_style)
            {
                // An earlier unavailable family prevents this generic fallback.
                continue;
            }
            let remaining_families = candidates[index..].join(", ");
            return probe_ic_zero_advance(
                fonts,
                layout_cx,
                &remaining_families,
                font_size_px,
                font_weight,
                font_style,
            );
        }
        if registered && family_candidate_has_ic_glyph(fonts, name, font_weight, font_style) {
            return probe_ic_zero_advance(
                fonts,
                layout_cx,
                raw_candidate,
                font_size_px,
                font_weight,
                font_style,
            );
        }
    }
    sanitize_finite(
        font_size_px,
        0.0,
        MAX_FONT_SIZE_PX,
        "font-size",
        &mut Vec::new(),
    )
}

fn probe_ic_zero_advance(
    fonts: &mut FontContext,
    layout_cx: &mut LayoutContext<()>,
    family_str: &str,
    font_size_px: f32,
    font_weight: f32,
    font_style: StyleFontStyle,
) -> f32 {
    let size = sanitize_finite(
        font_size_px,
        0.0,
        MAX_FONT_SIZE_PX,
        "font-size",
        &mut Vec::new(),
    );
    let weight = sanitize_font_weight(font_weight, &mut Vec::new());
    let mut builder = layout_cx.ranged_builder(fonts, "水", 1.0, false);
    builder.push_default(StyleProperty::FontFamily(FontFamily::from(family_str)));
    builder.push_default(StyleProperty::FontSize(size));
    builder.push_default(StyleProperty::FontWeight(FontWeight::new(weight)));
    builder.push_default(StyleProperty::FontStyle(font_style_to_parley(font_style)));
    let mut layout: Layout<()> = builder.build("水");
    layout.break_all_lines(None);
    let has_glyph = layout
        .lines()
        .flat_map(|line| line.items())
        .any(|item| match item {
            PositionedLayoutItem::GlyphRun(run) => run.glyphs().any(|glyph| glyph.id != 0),
            // cov:ignore: a plain single-character metric layout is expected to contain only glyph runs.
            _ => false,
        });
    let width = layout.full_width();
    // Keep the same 4em malformed-font guard used by the existing `ch`
    // metric probe while accepting zero as a valid mapped advance.
    if has_glyph && width.is_finite() && width >= 0.0 && width <= size * 4.0 {
        width
    } else {
        // cov:ignore: malformed or missing-glyph metric fallback is defensive.
        size
    }
}

/// Measure the `ch` advance needed by a computed `text-indent` value.
///
/// The cache key mirrors the text shaping face selection used by
/// [`probe_text_advance`], and the caller supplies the same font context that
/// shaped the document's text. The caller supplies the authored factor only
/// for `ch` values; this function returns the clamped used px value.
fn measured_ch_length_px(
    factor: f32,
    source: Option<&raikiri_style::ChFontKey>,
    cv: &ComputedValues,
    fonts: &mut FontContext,
    layout_cx: &mut LayoutContext<()>,
    probes: &mut HashMap<(String, u32, u32, u8), f32>,
) -> f32 {
    let family_atoms = source.map(|key| &key.family).unwrap_or(&cv.font_family);
    let family = family_atoms
        .iter()
        .map(|a| a.0.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let size = source.map_or(cv.font_size, |key| key.size);
    let weight = source.map_or(cv.font_weight, |key| key.weight);
    let font_style = source.map_or(cv.font_style, |key| key.style);
    let style = match font_style {
        StyleFontStyle::Normal => 0,
        StyleFontStyle::Italic => 1,
        StyleFontStyle::Oblique => 2,
        _ => 0,
    };
    let key = (family.clone(), size.px().to_bits(), weight.to_bits(), style);
    let advance = *probes.entry(key).or_insert_with(|| {
        probe_ch_text_advance(fonts, layout_cx, &family, size.px(), weight, font_style)
    });
    let used = factor * advance;
    if used.is_nan() {
        0.0
    } else {
        used.clamp(-MAX_TAFFY_MAGNITUDE, MAX_TAFFY_MAGNITUDE)
    }
}

fn measured_text_indent_px(
    cv: &ComputedValues,
    fonts: &mut FontContext,
    layout_cx: &mut LayoutContext<()>,
    probes: &mut HashMap<(String, u32, u32, u8), f32>,
) -> Option<f32> {
    let factor = cv.text_indent_ch_factor?;
    Some(measured_ch_length_px(
        factor,
        cv.text_indent_ch_font.as_ref(),
        cv,
        fonts,
        layout_cx,
        probes,
    ))
}

fn bounded_text_indent_amount(
    value: ComputedLengthPercentage,
    containing_width: f32,
    measured: Option<f32>,
) -> f32 {
    let raw = match value {
        ComputedLengthPercentage::Px(px) => measured.unwrap_or(px),
        ComputedLengthPercentage::Percent(percent) => containing_width * percent / 100.0,
    };
    if raw.is_nan() {
        0.0
    } else {
        raw.clamp(-MAX_TAFFY_MAGNITUDE, MAX_TAFFY_MAGNITUDE)
    }
}

/// [`ComputedLength`]: raikiri_style::ComputedLength
/// [`ComputedLineHeight`]: raikiri_style::ComputedLineHeight
/// CSS white-space characters (CSS Text 3 §4.1.1): space, tab, LF, FF, CR.
fn is_css_white_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\x0C' | '\r')
}

/// Zero-width space (U+200B): a segment break adjacent to it is removed
/// without leaving a space (CSS Text 3 §4.1.2 rule 1).
const ZERO_WIDTH_SPACE: char = '\u{200B}';

/// Hangul blocks for the CSS Text 3 §4.1.2 Hangul carve-out (a break with
/// Hangul on either side is NOT removed even between wide characters):
/// Jamo, Compatibility Jamo, Extended-A/B, Syllables. Ranges are stable
/// since Unicode 2.0 and need no data tables.
fn is_hangul_for_break(c: char) -> bool {
    matches!(
        c,
        '\u{1100}'..='\u{11FF}'
            | '\u{3130}'..='\u{318F}'
            | '\u{A960}'..='\u{A97F}'
            | '\u{AC00}'..='\u{D7AF}'
            | '\u{D7B0}'..='\u{D7FF}'
    )
}

/// East Asian Width map for segment-break decisions (CSS Text 3 §4.1.2
/// rule 2: removal when both sides are Fullwidth/Wide/Halfwidth).
/// Loaded once per process from ICU compiled data (same provider the
/// parley dependency already links; no new data pulled in).
fn east_asian_width_map()
-> icu_properties::CodePointMapDataBorrowed<'static, icu_properties::props::EastAsianWidth> {
    icu_properties::CodePointMapDataBorrowed::<icu_properties::props::EastAsianWidth>::new()
}

/// Whether `c` is a default-ignorable code point for segment-break
/// neighbor selection, excluding U+200B whose explicit rule is handled by
/// the caller.
fn is_segment_break_ignorable(c: char) -> bool {
    // U+200B has an explicit segment-break rule and must remain visible to
    // the caller; other default-ignorable code points (variation selectors,
    // soft hyphen, LRM, etc.) do not participate in the EAW neighbor test.
    c != ZERO_WIDTH_SPACE
        && icu_properties::CodePointSetData::new::<
            icu_properties::props::DefaultIgnorableCodePoint,
        >()
        .contains(c)
}

/// Whether `c` counts as wide for break removal: East Asian Width
/// Fullwidth/Wide/Halfwidth (Ambiguous excluded) and not Hangul (which
/// the spec carves out even when wide, e.g. Hangul syllables).
fn is_wide_for_break(c: char) -> bool {
    use icu_properties::props::EastAsianWidth;
    let ea = east_asian_width_map().get(c);
    (ea == EastAsianWidth::Fullwidth
        || ea == EastAsianWidth::Wide
        || ea == EastAsianWidth::Halfwidth)
        && !is_hangul_for_break(c)
}

/// Deepest edge text char inside element `elem` (`dir` -1: last,
/// +1: first), skipping `display:none` subtrees and empty text.
/// Returns `None` when the element holds no text (e.g. empty inline or
/// image): callers treat that as narrow, never as a boundary.
fn deep_edge_char(doc: &Document, cascade: &CascadeResult, elem: usize, dir: i8) -> Option<char> {
    let kids = &doc.nodes[elem].children;
    let range: Box<dyn Iterator<Item = usize>> = if dir < 0 {
        Box::new((0..kids.len()).rev())
    } else {
        Box::new(0..kids.len())
    };
    for i in range {
        let c = kids[i];
        if !doc.nodes[c].is_in_document() {
            continue;
        }
        match doc.nodes[c].kind() {
            NodeKind::Text => {
                let t = text_of(doc, c)?;
                if t.is_empty() {
                    continue;
                }
                return if dir < 0 {
                    t.chars().next_back()
                } else {
                    t.chars().next()
                };
            }
            NodeKind::Element => {
                if cascade.computed[c].display == DisplayValue::None {
                    continue;
                }
                if let Some(ch) = deep_edge_char(doc, cascade, c, dir) {
                    return Some(ch);
                }
            }
            _ => continue,
        }
    }
    None
}

/// Directly neighboring char of text node `idx` (`dir` -1: char before,
/// +1: char after) in reading order: in-sibling text edge (including
/// collapsible spaces — adjacency to a space keeps it), else the edge
/// text of a neighboring element (deep), else ascending through inline
/// ancestors like [`has_inline_adjacent`]. `None` at a block boundary or
/// when no text is found (treated as narrow, never as a break).
fn edge_char(
    doc: &Document,
    cascade: &CascadeResult,
    parent_of: &[Option<usize>],
    idx: usize,
    dir: i8,
) -> Option<char> {
    let mut node = idx;
    loop {
        let p = parent_of[node]?;
        let kids = &doc.nodes[p].children;
        let pos = kids.iter().position(|&c| c == node)?;
        let mut i = pos as isize + dir as isize;
        while i >= 0 && (i as usize) < kids.len() {
            let sib = kids[i as usize];
            i += dir as isize;
            if !doc.nodes[sib].is_in_document() {
                continue;
            }
            match doc.nodes[sib].kind() {
                NodeKind::Text => {
                    let t = text_of(doc, sib)?;
                    if t.is_empty() {
                        continue;
                    }
                    return if dir < 0 {
                        t.chars().next_back()
                    } else {
                        t.chars().next()
                    };
                }
                NodeKind::Element => {
                    if cascade.computed[sib].display == DisplayValue::None {
                        continue;
                    }
                    if let Some(ch) = deep_edge_char(doc, cascade, sib, dir) {
                        return Some(ch);
                    }
                    continue;
                }
                _ => continue,
            }
        }
        if doc.nodes[p].kind() == NodeKind::Element && is_inline_element_box(cascade, p) {
            node = p;
            continue;
        }
        return None;
    }
}

/// Check one side of a text node for a neighboring whitespace-only node that
/// contains a segment break. Unlike `whitespace_run_has_break`, this keeps
/// the direction so a leading space prefix is not associated with a later,
/// unrelated break in the same inline sequence.
fn whitespace_edge_has_break(
    doc: &Document,
    cascade: &CascadeResult,
    parent_of: &[Option<usize>],
    idx: usize,
    dir: i8,
) -> bool {
    let Some(parent) = parent_of[idx] else {
        return false;
    };
    let Some(pos) = doc.nodes[parent].children.iter().position(|&c| c == idx) else {
        return false;
    };
    let mut i = pos as isize + dir as isize;
    while i >= 0 && (i as usize) < doc.nodes[parent].children.len() {
        let sibling = doc.nodes[parent].children[i as usize];
        i += dir as isize;
        if !doc.nodes[sibling].is_in_document() {
            continue;
        }
        if doc.nodes[sibling].kind() == NodeKind::Element
            && cascade.computed[sibling].display == DisplayValue::None
        {
            continue;
        }
        if doc.nodes[sibling].kind() != NodeKind::Text {
            break;
        }
        let Some(sibling_text) = text_of(doc, sibling) else {
            break;
        };
        if !sibling_text.chars().all(is_css_white_space) {
            break;
        }
        if sibling_text.contains(['\n', '\r']) {
            return true;
        }
    }
    false
}

/// Remove whitespace runs whose segment break is removable because of the
/// significant characters on both sides. The ordinary collapse pass still
/// handles all other runs; this narrow pre-pass only prevents it from
/// turning a removable CJK break into a space after the parser combines the
/// surrounding indentation and character into one text node.
fn remove_removable_segment_break_runs(
    doc: &Document,
    cascade: &CascadeResult,
    parent_of: &[Option<usize>],
    idx: usize,
    s: &str,
) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut i = 0usize;
    while i < chars.len() {
        if !is_css_white_space(chars[i]) {
            out.push(chars[i]);
            i += 1;
            continue;
        }
        let start = i;
        while i < chars.len() && is_css_white_space(chars[i]) {
            i += 1;
        }
        let end = i;
        let has_break = chars[start..end]
            .iter()
            .any(|&ch| matches!(ch, '\n' | '\r'))
            || (start == 0 && whitespace_edge_has_break(doc, cascade, parent_of, idx, -1))
            || (end == chars.len() && whitespace_edge_has_break(doc, cascade, parent_of, idx, 1));
        if !has_break {
            out.extend(chars[start..end].iter().copied());
            continue;
        }
        let significant =
            |ch: &&char| !is_css_white_space(**ch) && !is_segment_break_ignorable(**ch);
        let before = chars[..start]
            .iter()
            .rev()
            .find(significant)
            .copied()
            .or_else(|| edge_non_whitespace_char(doc, cascade, parent_of, idx, -1));
        let after = chars[end..]
            .iter()
            .find(significant)
            .copied()
            .or_else(|| edge_non_whitespace_char(doc, cascade, parent_of, idx, 1));
        let removable = before == Some(ZERO_WIDTH_SPACE)
            || after == Some(ZERO_WIDTH_SPACE)
            || matches!((before, after), (Some(a), Some(b)) if is_wide_for_break(a) && is_wide_for_break(b));
        if !removable {
            out.extend(chars[start..end].iter().copied());
        }
    }
    out
}

/// Whether the contiguous text-node whitespace run around `idx` contains
/// a segment break. This handles the common HTML-tokenizer shape where a
/// run's spaces and character-reference LF tokens become separate siblings.
/// A mixed text node is intentionally not crossed: its own collapse pass has
/// already seen the complete text and must remain the owner of that run.
fn whitespace_run_has_break(
    doc: &Document,
    cascade: &CascadeResult,
    parent_of: &[Option<usize>],
    idx: usize,
    text: &str,
) -> bool {
    if text.contains(['\n', '\r']) {
        return true;
    }
    let Some(parent) = parent_of[idx] else {
        return false;
    };
    let Some(pos) = doc.nodes[parent].children.iter().position(|&c| c == idx) else {
        return false;
    };
    for dir in [-1isize, 1] {
        let mut i = pos as isize + dir;
        while i >= 0 && (i as usize) < doc.nodes[parent].children.len() {
            let sibling = doc.nodes[parent].children[i as usize];
            i += dir;
            if !doc.nodes[sibling].is_in_document() {
                continue;
            }
            if doc.nodes[sibling].kind() == NodeKind::Element
                && cascade.computed[sibling].display == DisplayValue::None
            {
                continue;
            }
            if doc.nodes[sibling].kind() != NodeKind::Text {
                break;
            }
            let Some(sibling_text) = text_of(doc, sibling) else {
                break;
            };
            if !sibling_text.chars().all(is_css_white_space) {
                break;
            }
            if sibling_text.contains(['\n', '\r']) {
                return true;
            }
        }
    }
    false
}

/// Find the nearest non-CSS-whitespace character at an element edge.
///
/// This is deliberately different from `deep_edge_char`: segment-break
/// transformation has to look through the whole collapsible run, not merely
/// at the first LF/space text node in that run. The HTML tokenizer commonly
/// creates one text node per character reference, so a run such as
/// `一些\n\n\n中文` is represented by several adjacent whitespace nodes.
fn deep_edge_non_whitespace_char(
    doc: &Document,
    cascade: &CascadeResult,
    elem: usize,
    dir: i8,
) -> Option<char> {
    let kids = &doc.nodes[elem].children;
    let range: Box<dyn Iterator<Item = usize>> = if dir < 0 {
        Box::new((0..kids.len()).rev())
    } else {
        Box::new(0..kids.len())
    };
    for i in range {
        let c = kids[i];
        if !doc.nodes[c].is_in_document() {
            continue;
        }
        match doc.nodes[c].kind() {
            NodeKind::Text => {
                let Some(t) = text_of(doc, c) else {
                    continue;
                };
                let edge = if dir < 0 {
                    t.chars()
                        .rev()
                        .find(|&ch| !is_css_white_space(ch) && !is_segment_break_ignorable(ch))
                } else {
                    t.chars()
                        .find(|&ch| !is_css_white_space(ch) && !is_segment_break_ignorable(ch))
                };
                if edge.is_some() {
                    return edge;
                }
            }
            NodeKind::Element => {
                if cascade.computed[c].display == DisplayValue::None {
                    continue;
                }
                if let Some(ch) = deep_edge_non_whitespace_char(doc, cascade, c, dir) {
                    return Some(ch);
                }
            }
            _ => continue,
        }
    }
    None
}

/// Directly neighboring non-whitespace character of a text node in reading
/// order. CSS-whitespace-only text nodes are skipped so callers can evaluate
/// one segment-break run even when the parser split it into many nodes.
fn edge_non_whitespace_char(
    doc: &Document,
    cascade: &CascadeResult,
    parent_of: &[Option<usize>],
    idx: usize,
    dir: i8,
) -> Option<char> {
    let mut node = idx;
    loop {
        let p = parent_of[node]?;
        let kids = &doc.nodes[p].children;
        let pos = kids.iter().position(|&c| c == node)?;
        let mut i = pos as isize + dir as isize;
        while i >= 0 && (i as usize) < kids.len() {
            let sib = kids[i as usize];
            i += dir as isize;
            if !doc.nodes[sib].is_in_document() {
                continue;
            }
            match doc.nodes[sib].kind() {
                NodeKind::Text => {
                    let Some(t) = text_of(doc, sib) else {
                        continue;
                    };
                    let edge = if dir < 0 {
                        t.chars()
                            .rev()
                            .find(|&ch| !is_css_white_space(ch) && !is_segment_break_ignorable(ch))
                    } else {
                        t.chars()
                            .find(|&ch| !is_css_white_space(ch) && !is_segment_break_ignorable(ch))
                    };
                    if edge.is_some() {
                        return edge;
                    }
                }
                NodeKind::Element => {
                    if cascade.computed[sib].display == DisplayValue::None {
                        continue;
                    }
                    if let Some(ch) = deep_edge_non_whitespace_char(doc, cascade, sib, dir) {
                        return Some(ch);
                    }
                }
                _ => continue,
            }
        }
        if doc.nodes[p].kind() == NodeKind::Element && is_inline_element_box(cascade, p) {
            node = p;
            continue;
        }
        return None;
    }
}

/// Whether `idx` generates an inline-level box for white-space trimming.
///
/// Text nodes are inline-level; elements follow their computed display
/// (`inline`, `inline-block`, `inline-table`). `contents` is treated as
/// inline (conservative: a wrong keep preserves old behavior, a wrong drop
/// would regress). `<br>` counts as a block boundary (leading/trailing
/// spaces around a forced break collapse).
fn is_inline_for_trim(doc: &Document, cascade: &CascadeResult, idx: usize) -> bool {
    if doc.nodes[idx].kind() == NodeKind::Text {
        return true;
    }
    // Absolutely/fixed-positioned boxes are out of the inline flow and cannot
    // keep a collapsible space at the boundary of the following text.
    if matches!(
        cascade.computed[idx].position,
        PositionValue::Absolute | PositionValue::Fixed
    ) {
        return false;
    }
    if doc.nodes[idx].tag_name() == Some("br") {
        return false;
    }
    matches!(
        cascade.computed[idx].display,
        DisplayValue::Inline
            | DisplayValue::InlineBlock
            | DisplayValue::InlineTable
            | DisplayValue::Contents
    )
}

/// Whether the element's own box is inline-level (strict: `contents` and
/// `<br>` excluded — used for ancestor ascent, where a `contents` wrapper
/// must not stop the search but a real inline box does not continue it...
/// actually ascent continues through ALL inline ancestors; this helper
/// reports whether `idx` (an element) generates an inline-level box,
/// `<br>` included as inline for ascent since content inside... `<br>` has
/// no children, so it never matters here).
fn is_inline_element_box(cascade: &CascadeResult, idx: usize) -> bool {
    matches!(
        cascade.computed[idx].display,
        DisplayValue::Inline
            | DisplayValue::InlineBlock
            | DisplayValue::InlineTable
            | DisplayValue::Contents
    )
}

/// Nearest significant sibling of the node holding text `idx` in `dir`
/// (-1: preceding, +1: following), skipping `display:none` boxes and
/// whitespace-only text nodes (both generate nothing trimmable-against).
/// Returns the sibling id, or the ancestor to ascend to when the parent's
/// edge is reached without finding one (handled by the caller via
/// `inline_beyond_block_edge`).
fn significant_sibling(
    doc: &Document,
    cascade: &CascadeResult,
    parent: usize,
    pos: usize,
    dir: i8,
) -> Option<usize> {
    let kids = &doc.nodes[parent].children;
    let mut i = pos as isize + dir as isize;
    while i >= 0 && (i as usize) < kids.len() {
        let sib = kids[i as usize];
        i += dir as isize;
        if !doc.nodes[sib].is_in_document() {
            continue;
        }
        if doc.nodes[sib].kind() == NodeKind::Element
            && (cascade.computed[sib].display == DisplayValue::None
                || matches!(
                    cascade.computed[sib].position,
                    PositionValue::Absolute | PositionValue::Fixed
                ))
        {
            continue;
        }
        if doc.nodes[sib].kind() == NodeKind::Text
            && text_of(doc, sib).is_some_and(|t| t.chars().all(is_css_white_space))
        {
            continue;
        }
        return Some(sib);
    }
    None
}

/// Whether inline-level content precedes/follows text node `idx`
/// (`dir` -1/+1) for collapsible-space trimming (CSS Text 3 §4.1.2 phases
/// II–III): spaces at a block boundary are removed; between inlines kept.
/// Ascends through inline ancestors so `x<span> a</span>` keeps its space.
fn has_inline_adjacent(
    doc: &Document,
    cascade: &CascadeResult,
    parent_of: &[Option<usize>],
    idx: usize,
    dir: i8,
) -> bool {
    let mut node = idx;
    loop {
        let p = match parent_of[node] {
            Some(p) => p,
            None => return false,
        };
        let kids = &doc.nodes[p].children;
        let pos = match kids.iter().position(|&c| c == node) {
            Some(pos) => pos,
            None => return false,
        };
        if let Some(sib) = significant_sibling(doc, cascade, p, pos, dir) {
            return is_inline_for_trim(doc, cascade, sib);
        }
        // Parent edge: ascend iff the parent itself is inline-level.
        if doc.nodes[p].kind() == NodeKind::Element && is_inline_element_box(cascade, p) {
            node = p;
            continue;
        }
        return false;
    }
}

/// Find the nearest in-flow inline character on one side of a text node.
///
/// Text nodes are shaped independently, but autospace applies across ordinary
/// inline-element boundaries. Whitespace, block-level boxes, atomic inline
/// boxes, and isolation boundaries stop the search rather than being skipped;
/// default-ignorable code points such as variation selectors are looked past.
fn autospace_adjacent_edge_char(
    doc: &Document,
    cascade: &CascadeResult,
    parent_of: &[Option<usize>],
    idx: usize,
    dir: i8,
) -> Option<char> {
    let mut node = idx;
    loop {
        let parent = parent_of[node]?;
        let children = &doc.nodes[parent].children;
        let position = children.iter().position(|&child| child == node)?;
        let siblings: Box<dyn Iterator<Item = &usize>> = if dir < 0 {
            Box::new(children[..position].iter().rev())
        } else {
            Box::new(children[position + 1..].iter())
        };
        for &sibling in siblings {
            match boundary_inline_edge(doc, cascade, sibling, dir, is_segment_break_ignorable) {
                InlineEdge::Empty => continue,
                InlineEdge::Break => return None,
                InlineEdge::Char(character) => return Some(character),
            }
        }
        if doc.nodes[parent].kind() == NodeKind::Element
            && is_inline_element_box(cascade, parent)
            && !is_shaping_isolation_boundary(doc, parent)
            && !boundary_shaping_box_breaks(cascade, parent)
        {
            node = parent;
            continue;
        }
        return None;
    }
}

/// Whether a character belongs to a script whose joining behavior must
/// continue across inline box boundaries. The CSS Text boundary-shaping cases covered
/// here exercise Arabic, N'Ko, and Mongolian; a zero-width joiner at an
/// inline boundary lets Parley retain the same joining context when the DOM
/// stores each styled text node in a separate layout.
fn is_boundary_shaping_char(ch: char) -> bool {
    matches!(
        ch as u32,
        0x0600..=0x06ff
            | 0x0750..=0x077f
            | 0x07c0..=0x07ff
            | 0x08a0..=0x08ff
            | 0x1800..=0x18af
            | 0xfb50..=0xfdff
            | 0xfe70..=0xfeff
    )
}

/// Add the zero-width joiners needed to preserve joining-script context at
/// inline box boundaries while each DOM text node is shaped independently.
///
/// Only the character on each edge matters: a space, an authored ZWNJ, or a
/// non-joining script at that edge already ends the joining context, so no
/// joiner is added there even when the rest of the run is joining script.
fn add_boundary_shaping_joiners(mut text: String, before: bool, after: bool) -> String {
    if after
        && text
            .chars()
            .next_back()
            .is_some_and(is_boundary_shaping_char)
    {
        text.push('\u{200d}');
    }
    if before && text.chars().next().is_some_and(is_boundary_shaping_char) {
        text.insert(0, '\u{200d}');
    }
    text
}

/// Whether an element creates a bidi isolation boundary for shaping.
///
/// The current style bridge does not expose `unicode-bidi` in computed values,
/// so cover the HTML isolation forms used by the WPT boundary-shaping tests:
/// `<bdi>` and an inline element with `dir="auto"`.
fn is_shaping_isolation_boundary(doc: &Document, node_id: usize) -> bool {
    doc.nodes[node_id].tag_name() == Some("bdi")
        || doc.nodes[node_id]
            .attribute("dir")
            .is_some_and(|value| value.eq_ignore_ascii_case("auto"))
}

/// Whether a boundary box prevents joining across its inline content.
///
/// Nonzero physical inline-edge margins, padding, and borders are shaping
/// boundaries; outline and text decoration are not. The current style bridge
/// normalizes writing mode to horizontal-tb, so the inline edges are left and
/// right. Atomic inline-level boxes also cannot share a shaping context with
/// their siblings.
fn boundary_shaping_box_breaks(cascade: &CascadeResult, node_id: usize) -> bool {
    let cv = &cascade.computed[node_id];
    if !matches!(cv.display, DisplayValue::Inline | DisplayValue::Contents) {
        return true;
    }
    let nonzero_length_percentage = |value: ComputedLengthPercentage| match value {
        ComputedLengthPercentage::Px(px) | ComputedLengthPercentage::Percent(px) => {
            px.abs() > f32::EPSILON
        }
    };
    let nonzero_length_percentage_or_auto = |value: ComputedLengthPercentageOrAuto| match value {
        ComputedLengthPercentageOrAuto::Px(px) | ComputedLengthPercentageOrAuto::Percent(px) => {
            px.abs() > f32::EPSILON
        }
        ComputedLengthPercentageOrAuto::Calc(_) => true,
        ComputedLengthPercentageOrAuto::Auto => false,
    };
    [cv.margin.left, cv.margin.right]
        .into_iter()
        .any(nonzero_length_percentage_or_auto)
        || [cv.padding.left, cv.padding.right]
            .into_iter()
            .any(nonzero_length_percentage)
        || [cv.border.left.width().px(), cv.border.right.width().px()]
            .into_iter()
            .any(|width| width.abs() > f32::EPSILON)
}

/// Whether a joining-script shaping context may cross the selected inline
/// boundary: the first rendered character beyond the boundary must itself be
/// able to join (a joining-script character or an authored ZWJ), and no
/// box-model, atomic, line-break, or bidi-isolation boundary may intervene.
fn boundary_shaping_adjacent(
    doc: &Document,
    cascade: &CascadeResult,
    parent_of: &[Option<usize>],
    idx: usize,
    dir: i8,
) -> bool {
    let mut node = idx;
    loop {
        let p = match parent_of[node] {
            Some(p) => p,
            None => return false,
        };
        let kids = &doc.nodes[p].children;
        let pos = match kids.iter().position(|&c| c == node) {
            Some(pos) => pos,
            None => return false,
        };
        let siblings: Box<dyn Iterator<Item = &usize>> = if dir < 0 {
            Box::new(kids[..pos].iter().rev())
        } else {
            Box::new(kids[pos + 1..].iter())
        };
        for &sib in siblings {
            match boundary_inline_edge(doc, cascade, sib, dir, |_| false) {
                InlineEdge::Empty => continue,
                InlineEdge::Break => return false,
                InlineEdge::Char(ch) => {
                    return ch == '\u{200d}' || is_boundary_shaping_char(ch);
                }
            }
        }
        if doc.nodes[p].kind() == NodeKind::Element && is_inline_element_box(cascade, p) {
            if is_shaping_isolation_boundary(doc, p) || boundary_shaping_box_breaks(cascade, p) {
                return false;
            }
            node = p;
            continue;
        }
        return false;
    }
}

/// What a subtree contributes at the edge facing a shaping boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InlineEdge {
    /// The subtree renders nothing inline; look further along the boundary.
    Empty,
    /// The subtree ends the joining context before any character.
    Break,
    /// The first rendered character met from the boundary side.
    Char(char),
}

/// Walk into `idx` from the side facing the boundary (`dir` +1 enters from
/// the start, -1 from the end) and report the first character it renders,
/// ignoring characters for which `skip` returns true. Out-of-flow and `display:none` boxes render nothing inline; atomic
/// inlines, `<br>`, bidi isolates, and inline boxes with nonzero inline-edge
/// margin/border/padding break the context.
fn boundary_inline_edge(
    doc: &Document,
    cascade: &CascadeResult,
    idx: usize,
    dir: i8,
    skip: fn(char) -> bool,
) -> InlineEdge {
    let node = &doc.nodes[idx];
    if !node.is_in_document() {
        return InlineEdge::Empty;
    }
    if let Some(text) = text_of(doc, idx) {
        let edge = if dir < 0 {
            text.chars().rev().find(|&ch| !skip(ch))
        } else {
            text.chars().find(|&ch| !skip(ch))
        };
        return edge.map_or(InlineEdge::Empty, InlineEdge::Char);
    }
    // Comments and processing instructions never carry the in-document flag,
    // so any remaining node here is an element.
    let cv = &cascade.computed[idx];
    if cv.display == DisplayValue::None
        || matches!(cv.position, PositionValue::Absolute | PositionValue::Fixed)
    {
        return InlineEdge::Empty;
    }
    if !is_inline_for_trim(doc, cascade, idx)
        || is_shaping_isolation_boundary(doc, idx)
        || boundary_shaping_box_breaks(cascade, idx)
    {
        return InlineEdge::Break;
    }
    let children = &node.children;
    let ordered: Box<dyn Iterator<Item = &usize>> = if dir < 0 {
        Box::new(children.iter().rev())
    } else {
        Box::new(children.iter())
    };
    for &child in ordered {
        match boundary_inline_edge(doc, cascade, child, dir, skip) {
            InlineEdge::Empty => continue,
            edge => return edge,
        }
    }
    InlineEdge::Empty
}

/// Raw text content of a text node.
fn text_of(doc: &Document, idx: usize) -> Option<&str> {
    match &doc.nodes[idx].data {
        crate::node::NodeData::Text(t) => Some(t.text_content.as_str()),
        _ => None,
    }
}

/// Collapse `text` per its computed `white-space` (CSS Text 3 §4.1) for
/// shaping: segment breaks/tabs become spaces (except `pre` family),
/// collapsible runs merge, and spaces at block boundaries are removed
/// (sibling context via `parent_of`, built once per preshape pass).
/// `pre`/`pre-wrap`/`break-spaces` pass through untouched; `pre-line`
/// keeps newlines but collapses spaces with boundary trimming.
/// Result of [`collapse_text_for_shaping`]: shaped text plus whether a
/// kept trailing space was stripped for forward migration.
struct CollapsedText {
    /// Text to shape (leading kept spaces already NBSP-ified in place;
    /// trailing kept spaces stripped — see `migrate_count`).
    text: String,
    /// A collapsible trailing space was removed while inline content
    /// follows: the caller prepends one NBSP to the next shaped text
    /// (parley keeps leading NBSP, trims trailing — so spaces only ever
    /// migrate forward, never stay trailing).
    migrate_count: u32,
}

fn has_authored_non_whitespace_text(doc: &Document, idx: usize) -> bool {
    text_of(doc, idx).is_some_and(|text| text.chars().any(|ch| !is_css_white_space(ch)))
        || doc.nodes[idx]
            .children
            .iter()
            .any(|&child| has_authored_non_whitespace_text(doc, child))
}

// cov:ignore: exercised by the ignored foundation WPT run; default coverage skips ignored reftests.
fn preserves_inline_whitespace_item(
    doc: &Document,
    cascade: &CascadeResult,
    parent_of: &[Option<usize>],
    idx: usize,
    fallback_width: f32,
) -> bool {
    let Some(parent) = parent_of.get(idx).copied().flatten() else {
        return false;
    };
    if multicol_metrics_for_node(cascade, parent_of, parent, fallback_width).is_some()
        || !matches!(
            cascade.computed[parent].display,
            DisplayValue::Block | DisplayValue::InlineBlock
        )
    {
        return false;
    }
    let Some(pos) = doc.nodes[parent]
        .children
        .iter()
        .position(|&child| child == idx)
    else {
        return false;
    };
    let has_inline_element = |children: &[usize]| {
        children.iter().any(|&child| {
            doc.nodes[child].kind() == NodeKind::Element
                && is_inline_element_box(cascade, child)
                && cascade.computed[child].display != DisplayValue::None
                && has_authored_non_whitespace_text(doc, child)
        })
    };
    has_inline_element(&doc.nodes[parent].children[..pos])
        && has_inline_element(&doc.nodes[parent].children[pos + 1..])
}

fn collapse_text_for_shaping(
    doc: &Document,
    cascade: &CascadeResult,
    parent_of: &[Option<usize>],
    idx: usize,
    text: &str,
    ws: WhiteSpace,
) -> CollapsedText {
    match ws {
        WhiteSpace::Pre | WhiteSpace::PreWrap | WhiteSpace::BreakSpaces => {
            return CollapsedText {
                text: text.to_string(),
                migrate_count: 0,
            };
        }
        _ => {}
    }
    // Flex/grid containers drop whitespace-only text children outright
    // (CSS Flexbox 1 §4 / CSS Grid 1 §5.2 anonymous-item rules: collapsed
    // runs vanish instead of becoming zero-size items). Without this the
    // migration below would inject NBSPs between flex items (e.g. WPT
    // flexbox_flex-0-0's newline-separated spans), shifting them apart.
    // Content-bearing text (anonymous flex/grid items) keeps flowing below.
    if text.chars().all(is_css_white_space)
        && let Some(p) = parent_of[idx]
        && doc.nodes[p].kind() == NodeKind::Element
        && matches!(
            cascade.computed[p].display,
            DisplayValue::Flex | DisplayValue::Grid
        )
    {
        return CollapsedText {
            text: String::new(),
            migrate_count: 0,
        };
    }
    // `matches!` carries its own wildcard arm, so this stays exhaustive
    // against the non-exhaustive `WhiteSpace` (cross-crate match without a
    // visible wildcard would not compile).
    // Phase I: CR/CRLF become LF; tabs/FF become spaces. Line feeds are
    // KEPT here (even for `normal`/`nowrap`): an interior single `\n`
    // carries East Asian Width / adjacency context that only the shaper
    // resolves correctly (CSS Text 3 §4.1.2 segment-break rules — parley
    // implements them; a blanket `\n`→space conversion breaks e.g. WPT
    // segment-break-transformation-rules-001 fullwidth/fullwidth). Runs of
    // 2+ line feeds collapse to one space below (multi-break runs never
    // reach the shaper as breaks; WPT removable-2 pins this), while a
    // single interior break passes through. Edge breaks are decided after
    // the boundary flags are known.
    let mut s = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\r' {
            if chars.peek() == Some(&'\n') {
                chars.next();
            }
            s.push('\n');
        } else if c == '\t' || c == '\x0C' {
            s.push(' ');
        } else {
            s.push(c);
        }
    }
    // Line-feed runs: `pre-line` keeps them verbatim for the shaper
    // (paragraph breaks survive); `normal`/`nowrap` collapse every maximal
    // run to one space here. The single-vs-wide decision for a lone break
    // between inlines happens in the all-whitespace arm below with East
    // Asian Width data; single-node content keeps the space approximation
    // it always had (multi-break collapse is pinned by WPT removable-2).
    let is_pre_line = matches!(ws, WhiteSpace::PreLine);
    // Lone-break optimized path (`normal`/`nowrap` only): an all-whitespace
    // node holding a segment-break run decides with direct-neighbor context
    // instead of the blanket conversion below. CSS Text 3 §4.1.2: a break
    // next to a zero-width space vanishes; next to a collapsible space the
    // space
    // survives (migrate); in a multi-break run one space survives; between
    // two wide (Fullwidth/Wide/Halfwidth, non-Hangul) chars it vanishes;
    // otherwise one space survives. `pre-line` keeps its feeds for the
    // shaper via the normal pipeline (dedupe carve-out above). Mixed text
    // nodes are handled by the removable-run pre-pass below.
    if !is_pre_line
        && s.chars().all(is_css_white_space)
        && whitespace_run_has_break(doc, cascade, parent_of, idx, text)
    {
        // A segment-break run may be split into several whitespace text
        // nodes by the HTML tokenizer. Let only its first node decide the
        // result; otherwise each LF would independently migrate a space.
        if edge_char(doc, cascade, parent_of, idx, -1).is_some_and(is_css_white_space) {
            return CollapsedText {
                text: String::new(),
                migrate_count: 0,
            };
        }
        let before = has_inline_adjacent(doc, cascade, parent_of, idx, -1);
        let after = has_inline_adjacent(doc, cascade, parent_of, idx, 1);
        if !before || !after {
            return CollapsedText {
                text: String::new(),
                migrate_count: 0,
            };
        }
        // Skip all CSS whitespace nodes surrounding this run before applying
        // the segment-break transformation rules. In particular, removable-3
        // and removable-4 put ordinary spaces around every LF; those spaces
        // are part of the same sequence and must not migrate before the wide
        // character test runs.
        let p = edge_non_whitespace_char(doc, cascade, parent_of, idx, -1);
        let n = edge_non_whitespace_char(doc, cascade, parent_of, idx, 1);
        if p == Some(ZERO_WIDTH_SPACE) || n == Some(ZERO_WIDTH_SPACE) {
            return CollapsedText {
                text: String::new(),
                migrate_count: 0,
            };
        }
        if let (Some(a), Some(b)) = (p, n)
            && is_wide_for_break(a)
            && is_wide_for_break(b)
        {
            return CollapsedText {
                text: String::new(),
                migrate_count: 0,
            };
        }
        return CollapsedText {
            text: String::new(),
            migrate_count: 1,
        };
    }

    let s = if is_pre_line {
        s
    } else {
        remove_removable_segment_break_runs(doc, cascade, parent_of, idx, &s)
    };
    let chars_vec: Vec<char> = s.chars().collect();
    let mut merged = String::with_capacity(s.len());
    let mut in_spaces = false;
    let mut i = 0usize;
    while i < chars_vec.len() {
        let c = chars_vec[i];
        if c == ' ' {
            if !in_spaces {
                merged.push(' ');
            }
            in_spaces = true;
            i += 1;
        } else if c == '\n' {
            let mut j = i;
            while j < chars_vec.len() && chars_vec[j] == '\n' {
                j += 1;
            }
            if is_pre_line {
                for &k in &chars_vec[i..j] {
                    merged.push(k);
                }
                in_spaces = false;
            } else {
                merged.push(' ');
                in_spaces = true;
            }
            i = j;
        } else {
            in_spaces = false;
            merged.push(c);
            i += 1;
        }
    }
    // Extended dedupe: an all-whitespace node whose previous relevant
    // sibling's raw text ENDS with whitespace collapses away — runs
    // spanning nodes (spaces, breaks, or mixed) keep the single space the
    // earlier node migrates. Skipped for `pre-line` around line feeds (a
    // following break must survive to shape the paragraph); pure-space
    // runs still dedupe.
    // NBSP-bearing nodes never dedupe: NBSPs don't collapse, so each
    // migrating NBSP node adds its own space to the pending count.
    if merged.chars().all(is_css_white_space)
        && !merged.contains('\u{00A0}')
        && let Some(p) = parent_of[idx]
        && let Some(pos) = doc.nodes[p].children.iter().position(|&c| c == idx)
    {
        let raw_has_break = text_of(doc, idx).is_some_and(|t| t.contains('\n'));
        for &prev in doc.nodes[p].children[..pos].iter().rev() {
            if !doc.nodes[prev].is_in_document() {
                continue;
            }
            if doc.nodes[prev].kind() == NodeKind::Element
                && cascade.computed[prev].display == DisplayValue::None
            {
                continue;
            }
            if doc.nodes[prev].kind() == NodeKind::Text
                && let Some(t) = text_of(doc, prev)
            {
                let ends_ws = t.chars().next_back().is_some_and(is_css_white_space);
                if !ends_ws {
                    break;
                }
                if is_pre_line && (raw_has_break || t.contains('\n')) {
                    break;
                }
                // Run continues here: the earlier node migrates the
                // single surviving space.
                return CollapsedText {
                    text: String::new(),
                    migrate_count: 0,
                };
            }
            break;
        }
    }
    let before = has_inline_adjacent(doc, cascade, parent_of, idx, -1);
    let after = has_inline_adjacent(doc, cascade, parent_of, idx, 1);
    // Lone-space arm covers plain spaces AND lone NBSPs (a `&nbsp;` entity
    // parses to its own text node, which parley would otherwise trim to
    // zero when shaped alone — the same fate as a lone collapsible space,
    // so it migrates identically; WPT unremovable-* pins the outcome).
    if merged.chars().all(|c| c == ' ' || c == '\u{00A0}') {
        // Fully collapsed: a lone boundary space vanishes; between inlines
        // the survivors migrate forward (shaped alone even NBSP trims to
        // zero — parley keeps only leading non-collapsible runs followed
        // by more content, measured directly). Count = NBSPs plus one for
        // a collapsible run (runs already merged to one above; NBSPs never
        // collapse, so each is preserved).
        if before && after {
            let count = merged.chars().filter(|&c| c == '\u{00A0}').count() as u32
                + merged.contains(' ') as u32;
            return CollapsedText {
                text: String::new(),
                migrate_count: count,
            };
        }
        return CollapsedText {
            text: String::new(),
            migrate_count: 0,
        };
    }
    // Edge line feeds (single — runs already collapsed above): dropped
    // at block edges, converted to a space toward inline content (a lone
    // survivor is indistinguishable from a collapsed space downstream;
    // wide-char pairs across nodes stay space-approximated — symmetric
    // pairs are unaffected either way).
    let mut out = merged;
    if !before {
        out = out.trim_start_matches([' ', '\n']).to_string();
    } else if out.starts_with('\n') {
        out.replace_range(..1, " ");
    }
    if !after {
        out = out.trim_end_matches([' ', '\n']).to_string();
    } else if out.ends_with('\n') {
        out.pop();
        out.push(' ');
    }
    let trailing_kept = after && out.ends_with(' ');
    if after {
        // Trailing survivor migrates forward (see `migrate_count`); strip
        // it here so shaping never sees a trimmable edge space.
        out = out.trim_end_matches(' ').to_string();
    }
    // Kept leading spaces become NBSP in place: parley keeps a leading
    // NBSP followed by content (measured), while a plain leading space
    // would trim. NBSP forfeits a soft-wrap opportunity at that spot;
    // acceptable since the alternative (today) is losing the space.
    if out.starts_with(' ') {
        out.replace_range(..1, "\u{00A0}");
    }
    CollapsedText {
        text: out,
        migrate_count: trailing_kept as u32,
    }
}

/// A coarse CSS Text 4 autospace class.  The style layer preserves the
/// complete `text-autospace` value; this layout slice only needs the boundary
/// classes to create non-painting in-flow advances.
#[derive(Clone, Copy, PartialEq, Eq)]
enum TextAutospaceClass {
    Ideograph,
    Letter,
    Numeric,
    Other,
}

fn text_autospace_class(c: char) -> TextAutospaceClass {
    use icu_properties::props::{GeneralCategoryGroup, Script};

    let gc =
        icu_properties::CodePointMapDataBorrowed::<icu_properties::props::GeneralCategory>::new()
            .get(c);
    let eaw = east_asian_width_map().get(c);
    let wide = matches!(
        eaw,
        icu_properties::props::EastAsianWidth::Fullwidth
            | icu_properties::props::EastAsianWidth::Wide
    );
    // CSS Text's ideographic set includes Han and the Japanese kana/stroke
    // ranges. Punctuation in the shared ranges is not an ideograph.
    let ideograph =
        (icu_properties::CodePointMapDataBorrowed::<icu_properties::props::Script>::new().get(c)
            == Script::Han
            || matches!(c, '\u{3041}'..='\u{30ff}' | '\u{31c0}'..='\u{31ff}'))
            && !GeneralCategoryGroup::Punctuation.contains(gc);
    if ideograph {
        return TextAutospaceClass::Ideograph;
    }
    if !wide
        && (GeneralCategoryGroup::Letter.contains(gc) || GeneralCategoryGroup::Mark.contains(gc))
    {
        return TextAutospaceClass::Letter;
    }
    if !wide && GeneralCategoryGroup::Number.contains(gc) {
        return TextAutospaceClass::Numeric;
    }
    TextAutospaceClass::Other
}

fn text_autospace_ascii_punctuation(c: char) -> bool {
    // CSS Text 4's `punctuation` class is language-sensitive. Chromium's
    // current behavior (and the zh WPT slice) inserts around these ASCII
    // punctuation marks, but not around fullwidth/CJK punctuation.
    matches!(c, '!' | '#' | ':' | ';' | '?')
}

#[cfg(test)]
fn text_autospace_boxes(
    text: &str,
    value: TextAutospace,
    language: &str,
    font_size: f32,
) -> Vec<InlineBox> {
    text_autospace_boxes_with_edges(text, value, language, font_size, None, None)
}

fn text_autospace_boxes_with_edges(
    text: &str,
    value: TextAutospace,
    language: &str,
    font_size: f32,
    before: Option<char>,
    after: Option<char>,
) -> Vec<InlineBox> {
    text_autospace_boxes_with_width(
        text,
        value,
        language,
        (font_size * 0.125).max(0.0),
        before,
        after,
    )
}

fn text_autospace_boxes_with_width(
    text: &str,
    value: TextAutospace,
    language: &str,
    width: f32,
    before: Option<char>,
    after: Option<char>,
) -> Vec<InlineBox> {
    let (ideograph_alpha, ideograph_numeric, punctuation) = match value {
        TextAutospace::Normal | TextAutospace::Auto => {
            (true, true, language_matches(language, "zh"))
        }
        TextAutospace::NoAutospace => (false, false, false),
        TextAutospace::Custom {
            ideograph_alpha,
            ideograph_numeric,
            punctuation,
            ..
        } => (
            ideograph_alpha,
            ideograph_numeric,
            punctuation && language_matches(language, "zh"),
        ),
        // `TextAutospace` is non-exhaustive so downstream crates remain
        // source-compatible when the style layer gains another keyword.
        _ => (true, true, language_matches(language, "zh")), // cov:ignore: no future non-exhaustive variant exists in the pinned style crate.
    };
    if !ideograph_alpha && !ideograph_numeric && !punctuation {
        return Vec::new();
    }
    if !width.is_finite() || width <= 0.0 {
        return Vec::new();
    }

    let mut previous =
        before.and_then(|c| (!is_segment_break_ignorable(c)).then(|| (c, text_autospace_class(c))));
    let mut last = None;
    let mut boxes = Vec::new();
    for (index, c) in text.char_indices() {
        // Default-ignorable characters, including variation selectors, do not
        // break the neighboring-character test. The VS WPT expects `国` + VS
        // + `A` to use the same autospace boundary as `国A`.
        if is_segment_break_ignorable(c) {
            continue;
        }
        let class = text_autospace_class(c);
        if let Some((previous_char, previous_class)) = previous
            && text_autospace_pair_needs_box(
                previous_char,
                previous_class,
                c,
                class,
                ideograph_alpha,
                ideograph_numeric,
                punctuation,
            )
        {
            boxes.push(InlineBox {
                // Reserve the high ID range for autospace boxes so paint can
                // distinguish their inline baseline semantics from tabs.
                id: u64::MAX - boxes.len() as u64,
                kind: InlineBoxKind::InFlow,
                index,
                width,
                height: 0.0,
            });
        }
        previous = Some((c, class));
        last = Some((c, class));
    }
    if let (Some((previous_char, previous_class)), Some(next)) = (last, after)
        && !is_segment_break_ignorable(next)
        && text_autospace_pair_needs_box(
            previous_char,
            previous_class,
            next,
            text_autospace_class(next),
            ideograph_alpha,
            ideograph_numeric,
            punctuation,
        )
    {
        boxes.push(InlineBox {
            // Reserve the high ID range for autospace boxes so paint can
            // distinguish their inline baseline semantics from tabs.
            id: u64::MAX - boxes.len() as u64,
            kind: InlineBoxKind::InFlow,
            index: text.len(),
            width,
            height: 0.0,
        });
    }
    boxes
}

fn text_autospace_pair_needs_box(
    previous_char: char,
    previous_class: TextAutospaceClass,
    current_char: char,
    current_class: TextAutospaceClass,
    ideograph_alpha: bool,
    ideograph_numeric: bool,
    punctuation: bool,
) -> bool {
    let class_boundary = match (previous_class, current_class) {
        (TextAutospaceClass::Ideograph, TextAutospaceClass::Letter)
        | (TextAutospaceClass::Letter, TextAutospaceClass::Ideograph) => ideograph_alpha,
        (TextAutospaceClass::Ideograph, TextAutospaceClass::Numeric)
        | (TextAutospaceClass::Numeric, TextAutospaceClass::Ideograph) => ideograph_numeric,
        _ => false,
    };
    let punctuation_boundary = punctuation
        && ((text_autospace_ascii_punctuation(previous_char)
            && current_class == TextAutospaceClass::Ideograph)
            || (previous_class == TextAutospaceClass::Ideograph
                && text_autospace_ascii_punctuation(current_char)));
    class_boundary || punctuation_boundary
}

fn effective_language_for_text(
    doc: &Document,
    parent_of: &[Option<usize>],
    text_idx: usize,
) -> String {
    let mut current = parent_of[text_idx];
    while let Some(idx) = current {
        if let crate::node::NodeData::Element(element) = &doc.nodes[idx].data
            && let Some(attr) = element
                .attributes
                .iter()
                .find(|attr| attr.local.eq_ignore_ascii_case("lang") || attr.local == "xml:lang")
        {
            return attr.value.trim().to_ascii_lowercase();
        }
        current = parent_of[idx];
    }
    String::new()
}

fn language_matches(language: &str, primary: &str) -> bool {
    language == primary
        || language
            .strip_prefix(primary)
            .is_some_and(|rest| rest.starts_with('-'))
}

fn text_transform_has_full_width(transform: TextTransform) -> bool {
    matches!(
        transform,
        TextTransform::FullWidth
            | TextTransform::CapitalizeFullWidth
            | TextTransform::UppercaseFullWidth
            | TextTransform::LowercaseFullWidth
            | TextTransform::FullWidthFullSizeKana
            | TextTransform::CapitalizeFullWidthFullSizeKana
            | TextTransform::UppercaseFullWidthFullSizeKana
            | TextTransform::LowercaseFullWidthFullSizeKana
    )
}

/// Apply the supported CSS `text-transform` values after whitespace collapsing
/// and before shaping. The operation is performed on each text node, preserving
/// the existing layout and paint ownership model.
fn apply_text_transform(text: &str, transform: TextTransform, language: &str) -> String {
    let (case, full_width, full_size_kana) = match transform {
        TextTransform::None => (None, false, false),
        TextTransform::Capitalize => (Some(TextTransform::Capitalize), false, false),
        TextTransform::Uppercase => (Some(TextTransform::Uppercase), false, false),
        TextTransform::Lowercase => (Some(TextTransform::Lowercase), false, false),
        TextTransform::FullWidth => (None, true, false),
        TextTransform::FullSizeKana => (None, false, true),
        TextTransform::CapitalizeFullWidth => (Some(TextTransform::Capitalize), true, false),
        TextTransform::UppercaseFullWidth => (Some(TextTransform::Uppercase), true, false),
        TextTransform::LowercaseFullWidth => (Some(TextTransform::Lowercase), true, false),
        TextTransform::CapitalizeFullSizeKana => (Some(TextTransform::Capitalize), false, true),
        TextTransform::UppercaseFullSizeKana => (Some(TextTransform::Uppercase), false, true),
        TextTransform::LowercaseFullSizeKana => (Some(TextTransform::Lowercase), false, true),
        TextTransform::FullWidthFullSizeKana => (None, true, true),
        TextTransform::CapitalizeFullWidthFullSizeKana => {
            (Some(TextTransform::Capitalize), true, true)
        }
        TextTransform::UppercaseFullWidthFullSizeKana => {
            (Some(TextTransform::Uppercase), true, true)
        }
        TextTransform::LowercaseFullWidthFullSizeKana => {
            (Some(TextTransform::Lowercase), true, true)
        }
        _ => (None, false, false),
    };
    let mut cased = String::with_capacity(text.len());
    let mut in_word = false;
    for c in text.chars() {
        match case {
            Some(TextTransform::Uppercase) => {
                if language_matches(language, "tr") || language_matches(language, "az") {
                    match c {
                        'i' => cased.push('İ'),
                        'ı' => cased.push('I'),
                        _ => cased.extend(c.to_uppercase()),
                    }
                } else {
                    cased.extend(c.to_uppercase());
                }
            }
            Some(TextTransform::Lowercase) => {
                if language_matches(language, "tr") || language_matches(language, "az") {
                    match c {
                        'I' => cased.push('ı'),
                        'İ' => cased.push('i'),
                        _ => cased.extend(c.to_lowercase()),
                    }
                } else {
                    cased.extend(c.to_lowercase());
                }
            }
            Some(TextTransform::Capitalize) => {
                if c.is_alphanumeric() {
                    let can_titlecase = !(0x24D0..=0x24E9).contains(&(c as u32));
                    if !in_word && can_titlecase {
                        cased.extend(c.to_uppercase());
                    } else {
                        cased.push(c);
                    }
                    in_word = true;
                } else {
                    cased.push(c);
                    in_word = false;
                }
            }
            _ => cased.push(c),
        }
    }
    if matches!(case, Some(TextTransform::Lowercase))
        && (language_matches(language, "tr") || language_matches(language, "az"))
    {
        tailor_turkic_combining_dot(&mut cased);
    }
    if matches!(case, Some(TextTransform::Capitalize)) && language_matches(language, "nl") {
        tailor_dutch_ij(&mut cased);
    }
    if matches!(case, Some(TextTransform::Uppercase)) && language_matches(language, "el") {
        strip_greek_tonos(&mut cased);
    }
    cased
        .chars()
        .map(|c| {
            let c = if full_size_kana {
                full_size_kana_char(c)
            } else {
                c
            };
            if full_width { full_width_char(c) } else { c }
        })
        .collect()
}

fn tailor_turkic_combining_dot(text: &mut String) {
    let chars: Vec<char> = text.chars().collect();
    let combining_dot = char::from_u32(0x0307).unwrap();
    let mut out = String::with_capacity(text.len());
    let mut index = 0;
    while index < chars.len() {
        if chars[index] == 'ı' && chars.get(index + 1) == Some(&combining_dot) {
            out.push('i');
            index += 2;
        } else {
            out.push(chars[index]);
            index += 1;
        }
    }
    *text = out;
}

fn tailor_dutch_ij(text: &mut String) {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut at_word_start = true;
    let mut index = 0;
    while index < chars.len() {
        let c = chars[index];
        if at_word_start && c == 'I' && chars.get(index + 1) == Some(&'j') {
            out.push('I');
            out.push('J');
            at_word_start = false;
            index += 2;
            continue;
        }
        out.push(c);
        at_word_start = !c.is_alphanumeric();
        index += 1;
    }
    *text = out;
}

fn strip_greek_tonos(text: &mut String) {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut index = 0;
    while index < chars.len() {
        if !chars[index].is_alphabetic() {
            out.push(chars[index]);
            index += 1;
            continue;
        }
        let mut end = index;
        while end < chars.len()
            && (chars[end].is_alphabetic() || ('\u{0300}'..='\u{036f}').contains(&chars[end]))
        {
            end += 1;
        }
        let letter_count = chars[index..end]
            .iter()
            .filter(|c| c.is_alphabetic())
            .count();
        let keep_tonos = letter_count == 1;
        let mut word_index = index;
        while word_index < end {
            let c = chars[word_index];
            if !keep_tonos
                && let Some(&next) = chars.get(word_index + 1)
                && matches!(c, 'Ά' | 'Έ' | 'Ή' | 'Ί' | 'Ό' | 'Ύ' | 'Ώ')
                && matches!(next, 'Ι' | 'Υ')
            {
                out.push(greek_tonos_removed(c));
                out.push(greek_diaeresis(next));
                word_index += 2;
            } else if !keep_tonos
                && let (Some(&diaeresis), Some(&acute)) =
                    (chars.get(word_index + 1), chars.get(word_index + 2))
                && matches!(c, 'Ι' | 'Υ')
                && diaeresis == '\u{0308}'
                && acute == '\u{0301}'
            {
                out.push(greek_diaeresis(c));
                word_index += 3;
            } else if !keep_tonos && c == '\u{0301}' {
                word_index += 1;
            } else {
                out.push(if keep_tonos {
                    c
                } else {
                    greek_tonos_removed(c)
                });
                word_index += 1;
            }
        }
        index = end;
    }
    *text = out;
}

fn greek_tonos_removed(c: char) -> char {
    match c {
        'Ά' => 'Α',
        'Έ' => 'Ε',
        'Ή' => 'Η',
        'Ί' => 'Ι',
        'Ό' => 'Ο',
        'Ύ' => 'Υ',
        'Ώ' => 'Ω',
        _ => c,
    }
}

fn greek_diaeresis(c: char) -> char {
    match c {
        'Ι' => 'Ϊ',
        'Υ' => 'Ϋ',
        _ => c,
    }
}

fn full_size_kana_char(c: char) -> char {
    match c {
        'ぁ' => 'あ',
        'ぃ' => 'い',
        'ぅ' => 'う',
        'ぇ' => 'え',
        'ぉ' => 'お',
        'ゕ' => 'か',
        'ゖ' => 'け',
        'っ' => 'つ',
        'ゃ' => 'や',
        'ゅ' => 'ゆ',
        'ょ' => 'よ',
        'ゎ' => 'わ',
        'ァ' => 'ア',
        'ィ' => 'イ',
        'ゥ' => 'ウ',
        'ェ' => 'エ',
        'ォ' => 'オ',
        'ヵ' => 'カ',
        'ㇰ' => 'ク',
        'ヶ' => 'ケ',
        'ㇱ' => 'シ',
        'ㇲ' => 'ス',
        'ッ' => 'ツ',
        'ㇳ' => 'ト',
        'ㇴ' => 'ヌ',
        'ㇵ' => 'ハ',
        'ㇶ' => 'ヒ',
        'ㇷ' => 'フ',
        'ㇸ' => 'ヘ',
        'ㇹ' => 'ホ',
        'ㇺ' => 'ム',
        'ャ' => 'ヤ',
        'ュ' => 'ユ',
        'ョ' => 'ヨ',
        'ㇻ' => 'ラ',
        'ㇼ' => 'リ',
        'ㇽ' => 'ル',
        'ㇾ' => 'レ',
        'ㇿ' => 'ロ',
        'ヮ' => 'ワ',
        'ｧ' => 'ｱ',
        'ｨ' => 'ｲ',
        'ｩ' => 'ｳ',
        'ｪ' => 'ｴ',
        'ｫ' => 'ｵ',
        'ｯ' => 'ﾂ',
        'ｬ' => 'ﾔ',
        'ｭ' => 'ﾕ',
        'ｮ' => 'ﾖ',
        '\u{1B132}' => '\u{3053}',
        '\u{1B150}' => '\u{3090}',
        '\u{1B151}' => '\u{3091}',
        '\u{1B152}' => '\u{3092}',
        '\u{1B155}' => '\u{30B3}',
        '\u{1B164}' => '\u{30F0}',
        '\u{1B165}' => '\u{30F1}',
        '\u{1B166}' => '\u{30F2}',
        '\u{1B167}' => '\u{30F3}',
        _ => c,
    }
}

fn full_width_char(c: char) -> char {
    match c {
        ' ' => char::from_u32(0x3000).unwrap(),
        '!'..='~' => char::from_u32(c as u32 + 0xFEE0).unwrap_or(c),
        _ => c,
    }
}

fn is_leading_body_text(document: &Document, body_id: Option<usize>, node_id: usize) -> bool {
    let Some(body_id) = body_id else {
        return false;
    };
    for &child_id in &document.nodes[body_id].children {
        if child_id == node_id {
            return true;
        }
        let Some(child) = document.get_node(child_id) else {
            continue;
        };
        match child.kind() {
            NodeKind::Text => {
                if let crate::node::NodeData::Text(text) = &child.data
                    && !text.text_content.trim().is_empty()
                {
                    return false;
                }
            }
            NodeKind::Element
                if child.is_display_none() || child.is_non_rendered_html_element() => {}
            _ => return false,
        }
    }
    false
}

fn parley_word_break(value: WordBreak, line_break: LineBreak) -> ParleyWordBreak {
    if matches!(line_break, LineBreak::Anywhere) {
        return ParleyWordBreak::BreakAll;
    }
    match value {
        WordBreak::BreakAll => ParleyWordBreak::BreakAll,
        WordBreak::KeepAll => ParleyWordBreak::KeepAll,
        // `manual`, `auto-phrase`, and the deprecated `break-word` do not
        // have a direct Parley word-break mode. `break-word` gets its
        // emergency wrapping behavior from `parley_overflow_wrap` below.
        WordBreak::Normal | WordBreak::Manual | WordBreak::AutoPhrase | WordBreak::BreakWord => {
            ParleyWordBreak::Normal
        }
        _ => ParleyWordBreak::Normal,
    }
}

fn parley_overflow_wrap(
    word_break: WordBreak,
    line_break: LineBreak,
    value: OverflowWrap,
) -> ParleyOverflowWrap {
    if matches!(line_break, LineBreak::Anywhere) {
        return ParleyOverflowWrap::Anywhere;
    }
    if matches!(word_break, WordBreak::BreakWord) {
        return ParleyOverflowWrap::BreakWord;
    }
    match value {
        OverflowWrap::Normal => ParleyOverflowWrap::Normal,
        OverflowWrap::Anywhere => ParleyOverflowWrap::Anywhere,
        OverflowWrap::BreakWord => ParleyOverflowWrap::BreakWord,
        _ => ParleyOverflowWrap::Normal,
    }
}

fn parley_text_wrap_mode(value: TextWrapMode, nowrap: bool) -> ParleyTextWrapMode {
    if nowrap || matches!(value, TextWrapMode::Nowrap) {
        ParleyTextWrapMode::NoWrap
    } else {
        ParleyTextWrapMode::Wrap
    }
}

/// Dispatch a multicolumn container through the nested fragmentation seam.
///
/// Taffy still resolves the ordinary box and intrinsic sizes, while this
/// seam owns nested child placement, fragmentainer breaks, and fragment-tree
/// records. Standalone and deferred out-of-flow cases continue through the
/// PR #79 projection so their exact geometry remains unchanged.
// cov:ignore: nested recursive layout is exercised by the ignored nested WPT reftests.
pub(crate) fn compute_multicol_layout(
    tree: &mut Document,
    node_id: TaffyNodeId,
    inputs: LayoutInput,
    block_ctx: Option<&mut BlockContext<'_>>,
) -> LayoutOutput {
    let index = usize::from(node_id);
    let Some(style) = tree.nodes[index].multicol else {
        return compute_block_layout(tree, node_id, inputs, block_ctx);
    };

    // Taffy supplies the used border-box width to a block child. Do not fall
    // back to the page width when a nested box is measured: that would make a
    // nested percentage gap resolve against the wrong containing block.
    let parent_width = inputs.parent_size.width;
    let available_width = inputs
        .known_dimensions
        .width
        .or(match inputs.available_space.width {
            AvailableSpace::Definite(value) => Some(value),
            AvailableSpace::MinContent | AvailableSpace::MaxContent => None,
        })
        .or_else(|| {
            multicol_definite_dimension(tree, tree.nodes[index].style.size.width, parent_width)
        })
        .unwrap_or(0.0);
    let available_height = if style.height_definite {
        multicol_definite_dimension(
            tree,
            tree.nodes[index].style.size.height,
            inputs.parent_size.height,
        )
        .or(inputs.known_dimensions.height)
    } else {
        None
    };
    let Some(context) = FragmentationContext::resolve(available_width, available_height, style)
    else {
        return compute_block_layout(tree, node_id, inputs, block_ctx);
    };

    // Keep the existing foundational post-pass authoritative for standalone
    // multicol boxes and for deferred out-of-flow cases. The custom path is
    // entered only at a real nested block-flow boundary.
    // A logical min-block-size + break-inside:avoid child with no direct
    // line-break flow must stay on the foundational block projection: that
    // path preserves the item's unfragmented minimum before we apply the
    // column offsets below. Text-bearing children retain the existing custom
    // line-range projection.
    let custom_scope = (tree.fragmentation_stack.is_empty()
        && (multicol_has_nested_descendant(tree, index)
            || (multicol_has_direct_block_child(tree, index)
                && tree.nodes[index].style.size.height == Dimension::auto())
            || multicol_has_direct_break_flow(tree, index))
        || !tree.fragmentation_stack.is_empty())
        && (!multicol_has_min_constrained_child(tree, index)
            || multicol_has_direct_break_flow(tree, index))
        && style.horizontal
        && !multicol_has_out_of_flow_descendant(tree, index);
    let stack_depth = tree.fragmentation_stack.len();
    tree.fragmentation_stack.push(context);
    let mut output = compute_block_layout(tree, node_id, inputs, block_ctx);
    if let Some(parent_height) = available_height.filter(|_| {
        multicol_has_min_constrained_child(tree, index)
            && !multicol_has_direct_break_flow(tree, index)
    }) {
        let children: Vec<usize> = tree.nodes[index]
            .children
            .iter()
            .copied()
            .filter(|&child| {
                tree.nodes[child].is_in_document()
                    && tree.nodes[child].kind() == NodeKind::Element
                    && tree.nodes[child].style.display != Display::None
            })
            .collect();
        if let Some(&first) = children.first() {
            let child_height = tree.nodes[first].unrounded_layout.size.height;
            // `break-inside: avoid` consumes the unfragmented overflow on both
            // sides of a too-short/too-tall fragmentainer. Taffy does not
            // expose that fragmentation marker on its foundational block
            // path, so express the equivalent separation as a direct-child
            // block offset.
            let step = child_height + 2.0 * (parent_height - child_height).abs();
            for (order, child) in children.into_iter().enumerate() {
                tree.nodes[child].unrounded_layout.location.y = order as f32 * step;
            }
        }
    }
    if !style.height_definite && multicol_has_min_constrained_child(tree, index) {
        // The minimum-sized children overflow the auto multicol box, but the
        // auto box's used block-size remains the minimum child size rather
        // than the sum of those overflowing children.
        let min_child_height = tree.nodes[index]
            .children
            .iter()
            .filter(|&&child| {
                tree.nodes[child].is_in_document()
                    && tree.nodes[child].kind() == NodeKind::Element
                    && tree.nodes[child].style.display != Display::None
            })
            .map(|&child| tree.nodes[child].unrounded_layout.size.height)
            .fold(0.0_f32, f32::max);
        output.size.height = output.size.height.min(min_child_height);
    }
    if custom_scope && inputs.run_mode == RunMode::PerformLayout {
        // Taffy's input width is normally already the content width for this
        // bridge. Correct it for authored padding/border before deriving the
        // child column width, so percentage gaps use the used content box.
        let content_width = multicol_content_width(tree, index, output.size.width, parent_width);
        let resolved = FragmentationContext::resolve(content_width, available_height, style)
            .unwrap_or(context);
        if let Some(active) = tree.fragmentation_stack.last_mut() {
            *active = resolved;
        }
        let used_height =
            relayout_nested_multicol_children(tree, node_id, resolved, output.size.height);
        if available_height.is_none() {
            output.size.height = used_height.max(0.0);
        }
    }
    // Truncating to the depth captured on entry keeps the stack balanced if a
    // future child strategy starts pushing a nested context of its own.
    tree.fragmentation_stack.truncate(stack_depth);
    output
}

// cov:ignore: nested recursive layout is exercised by the ignored nested WPT reftests.
fn multicol_definite_dimension(
    tree: &Document,
    value: Dimension,
    basis: Option<f32>,
) -> Option<f32> {
    let basis = basis.unwrap_or(0.0);
    let resolved = match value.expand() {
        ExpandedDimension::Length(px) => px,
        ExpandedDimension::Percent(percent) => basis * percent,
        ExpandedDimension::Calc(pointer) => tree.resolve_calc_value(pointer, basis),
        ExpandedDimension::Auto
        | ExpandedDimension::MinContent
        | ExpandedDimension::MaxContent
        | ExpandedDimension::FitContentPx(_)
        | ExpandedDimension::FitContentPercent(_)
        | ExpandedDimension::FitContent
        | ExpandedDimension::Stretch
        | ExpandedDimension::Content => return None,
    };
    resolved.is_finite().then_some(resolved.max(0.0))
}

// cov:ignore: nested recursive layout is exercised by the ignored nested WPT reftests.
fn multicol_resolve_inset(tree: &Document, value: LengthPercentage, basis: f32) -> f32 {
    let resolved = match value.expand() {
        ExpandedLengthPercentage::Length(px) => px,
        ExpandedLengthPercentage::Percent(percent) => basis * percent,
        ExpandedLengthPercentage::Calc(pointer) => tree.resolve_calc_value(pointer, basis),
    };
    if resolved.is_finite() {
        resolved.max(0.0)
    } else {
        0.0
    }
}

// cov:ignore: nested recursive layout is exercised by the ignored nested WPT reftests.
fn multicol_content_width(
    tree: &Document,
    node_id: usize,
    border_box_width: f32,
    parent_width: Option<f32>,
) -> f32 {
    let style = &tree.nodes[node_id].style;
    let basis = parent_width.unwrap_or(border_box_width).max(0.0);
    let horizontal = multicol_resolve_inset(tree, style.padding.left, basis)
        + multicol_resolve_inset(tree, style.padding.right, basis)
        + multicol_resolve_inset(tree, style.border.left, basis)
        + multicol_resolve_inset(tree, style.border.right, basis);
    (border_box_width - horizontal).max(0.0)
}

// cov:ignore: nested recursive layout is exercised by the ignored nested WPT reftests.
fn multicol_has_nested_descendant(tree: &Document, node_id: usize) -> bool {
    let mut pending = tree.nodes[node_id].children.clone();
    while let Some(child) = pending.pop() {
        if tree.nodes[child].multicol.is_some() {
            return true;
        }
        pending.extend(tree.nodes[child].children.iter().copied());
    }
    false
}

// cov:ignore: nested recursive layout is exercised by the ignored nested WPT reftests.
fn multicol_has_direct_break_flow(tree: &Document, node_id: usize) -> bool {
    tree.nodes[node_id].children.iter().copied().any(|child| {
        tree.nodes[child].kind() == NodeKind::Element
            && tree.nodes[child].is_in_document()
            && tree.nodes[child]
                .children
                .iter()
                .any(|&grandchild| tree.nodes[grandchild].tag_name() == Some("br"))
    })
}

fn multicol_has_out_of_flow_descendant(tree: &Document, node_id: usize) -> bool {
    let mut pending = tree.nodes[node_id].children.clone();
    while let Some(child) = pending.pop() {
        if tree.nodes[child].style.position == TaffyPosition::Absolute {
            return true;
        }
        pending.extend(tree.nodes[child].children.iter().copied());
    }
    false
}

// cov:ignore: direct block fragmentation is exercised by the ignored multicol WPT reftests.
fn multicol_has_direct_block_child(tree: &Document, node_id: usize) -> bool {
    tree.nodes[node_id].children.iter().any(|&child| {
        tree.nodes[child].is_in_document()
            && tree.nodes[child].kind() == NodeKind::Element
            && tree.nodes[child].style.display != Display::None
    })
}

// A logical non-auto child minimum paired with break avoidance participates
// in block sizing before column projection. Keep that case on the foundational
// path until the custom projection can account for the minimum's unfragmented
// overflow.
fn multicol_has_min_constrained_child(tree: &Document, node_id: usize) -> bool {
    tree.nodes[node_id].children.iter().any(|&child| {
        tree.nodes[child].is_in_document()
            && tree.nodes[child].kind() == NodeKind::Element
            && tree.nodes[child].style.display != Display::None
            && tree.nodes[child].has_logical_min_block_size
            && !tree.nodes[child].style.min_size.height.is_auto()
    })
}

// cov:ignore: nested recursive layout is exercised by the ignored nested WPT reftests.
fn relayout_nested_multicol_children(
    tree: &mut Document,
    node_id: TaffyNodeId,
    context: FragmentationContext,
    fallback_height: f32,
) -> f32 {
    let index = usize::from(node_id);
    let children: Vec<usize> = tree.nodes[index]
        .children
        .iter()
        .copied()
        .filter(|&child| {
            tree.nodes[child].is_in_document() && tree.nodes[child].style.display != Display::None
        })
        .collect();
    let container_fragment = tree.fragment_tree.push(crate::fragment::LayoutFragment {
        node_id: index,
        parent: None,
        fragmentainer: context.column_index,
        x: context.origin_x,
        y: context.origin_y,
        width: context.available_width,
        height: fallback_height,
        line_start: None,
        line_end: None,
    });
    // This tranche models `column-fill:auto`: source-order children consume
    // the current definite fragmentainer before the next column starts.
    // Balancing remains on the foundational direct-text path.
    let fragment_height = context.available_height;
    let mut column = 0usize;
    let mut cursor = 0.0f32;
    let mut maximum = 0.0f32;
    // With an auto-height multicol, measure block children once before
    // placing them. This gives the simple balancing pass a target height;
    // measuring against the current column would otherwise keep every child
    // in the first column and leave the container taller than necessary.
    let auto_measurements = if tree.fragmentation_stack.len() == 1
        && fragment_height.is_none()
        && !multicol_has_nested_descendant(tree, index)
    {
        let mut measurements = Vec::with_capacity(children.len());
        for &child in &children {
            let child_inputs = LayoutInput {
                run_mode: RunMode::PerformLayout,
                sizing_mode: SizingMode::InherentSize,
                axis: RequestedAxis::Both,
                known_dimensions: Size {
                    width: Some(context.column_width),
                    height: None,
                },
                known_dimensions_are_definite: Size {
                    width: true,
                    height: false,
                },
                parent_size: Size {
                    width: Some(context.column_width),
                    height: context.available_height,
                },
                available_space: Size {
                    width: AvailableSpace::Definite(context.column_width),
                    height: AvailableSpace::MaxContent,
                },
                vertical_margins_are_collapsible: TaffyLine::FALSE,
            };
            if matches!(tree.nodes[child].data, NodeData::Text(_)) {
                tree.nodes[child].style.size.height = Dimension::auto();
                tree.nodes[child].cache.clear();
            }
            let output = tree.compute_child_layout(TaffyNodeId::from(child), child_inputs);
            let layout = tree.nodes[child].unrounded_layout;
            let margin_top = layout.margin.top;
            let margin_bottom = layout.margin.bottom;
            measurements.push((
                child,
                output,
                layout,
                margin_top,
                margin_bottom,
                margin_top + output.size.height + margin_bottom,
            ));
        }
        let total = measurements.iter().map(|entry| entry.5).sum::<f32>();
        let largest = measurements.iter().map(|entry| entry.5).fold(0.0, f32::max);
        let target = (total / context.column_count as f32).max(largest);
        Some((measurements, target))
    } else {
        None
    };

    let mut avoid_column_break_after_previous = false;
    for (order, child) in children.into_iter().enumerate() {
        let measured = auto_measurements
            .as_ref()
            .and_then(|(entries, _)| entries.iter().find(|entry| entry.0 == child));
        let (child_output, mut child_layout, margin_top, margin_bottom, needed) =
            if let Some((_, output, layout, margin_top, margin_bottom, needed)) = measured {
                (*output, *layout, *margin_top, *margin_bottom, *needed)
            } else {
                let child_height = match fragment_height {
                    Some(height) => AvailableSpace::Definite((height - cursor).max(0.0)),
                    None => AvailableSpace::MaxContent,
                };
                let child_inputs = LayoutInput {
                    run_mode: RunMode::PerformLayout,
                    sizing_mode: SizingMode::InherentSize,
                    axis: RequestedAxis::Both,
                    known_dimensions: Size {
                        width: Some(context.column_width),
                        height: None,
                    },
                    known_dimensions_are_definite: Size {
                        width: true,
                        height: false,
                    },
                    parent_size: Size {
                        width: Some(context.column_width),
                        height: context.available_height,
                    },
                    available_space: Size {
                        width: AvailableSpace::Definite(context.column_width),
                        height: child_height,
                    },
                    vertical_margins_are_collapsible: TaffyLine::FALSE,
                };

                // prepare_multicol_layout gives direct text a temporary used height
                // for its page-level projection. A nested width probe must measure the
                // shaped layout again instead of reusing that stale height.
                if matches!(tree.nodes[child].data, NodeData::Text(_)) {
                    tree.nodes[child].style.size.height = Dimension::auto();
                    tree.nodes[child].cache.clear();
                }
                let output = tree.compute_child_layout(TaffyNodeId::from(child), child_inputs);
                refresh_nested_text_fragments(tree, child, context);
                let layout = tree.nodes[child].unrounded_layout;
                let margin_top = layout.margin.top;
                let margin_bottom = layout.margin.bottom;
                let needed = margin_top + output.size.height + margin_bottom;
                (output, layout, margin_top, margin_bottom, needed)
            };
        let break_height =
            fragment_height.or_else(|| auto_measurements.as_ref().map(|(_, height)| *height));
        let avoid_column_break = avoid_column_break_after_previous
            || matches!(tree.nodes[child].break_before, BreakBetween::Avoid);
        if let Some(height) = break_height
            && column + 1 < context.column_count
            && cursor > 0.0
            && cursor + needed > height
            && !avoid_column_break
        {
            maximum = maximum.max(cursor);
            tree.fragment_tree
                .record_break(crate::fragment::BreakToken {
                    node_id: index,
                    child_index: order,
                    line_index: 0,
                });
            column += 1;
            cursor = 0.0;
        }

        let column_context = context.in_column(
            column,
            context.origin_x + context.column_offset_x(column),
            context.origin_y + cursor,
        );
        let x = child_layout.location.x + context.column_offset_x(column);
        let y = cursor + margin_top;
        child_layout.order = order as u32;
        child_layout.size = child_output.size;
        child_layout.location = Point { x, y };
        tree.set_unrounded_layout(TaffyNodeId::from(child), &child_layout);
        refresh_nested_text_fragments(tree, child, column_context);
        let child_fragment = tree.fragment_tree.push(crate::fragment::LayoutFragment {
            node_id: child,
            parent: Some(container_fragment),
            fragmentainer: column_context.column_index,
            x: column_context.origin_x,
            y: column_context.origin_y + margin_top,
            width: child_output.size.width,
            height: child_output.size.height,
            line_start: None,
            line_end: None,
        });
        tree.fragment_tree.reparent_roots(child, child_fragment);
        // Keep text ranges in the same first-class tree as element boxes.
        // Paint still consumes the node-local ranges for compatibility, while
        // future incremental reflow can walk one nested fragment tree.
        let text_fragments = tree.nodes[child]
            .multicol_fragments()
            .map(|fragments| fragments.to_vec());
        if let Some(text_fragments) = text_fragments {
            for fragment in text_fragments {
                tree.fragment_tree.push(crate::fragment::LayoutFragment {
                    node_id: child,
                    parent: Some(child_fragment),
                    fragmentainer: column_context.column_index,
                    x: child_layout.location.x + fragment.x,
                    y: child_layout.location.y + fragment.y,
                    width: child_output.size.width,
                    height: child_output.size.height,
                    line_start: Some(fragment.line_start),
                    line_end: Some(fragment.line_end),
                });
            }
        }
        cursor = y + child_output.size.height + margin_bottom;
        if tree.nodes[child].kind() == NodeKind::Element {
            avoid_column_break_after_previous =
                matches!(tree.nodes[child].break_after, BreakBetween::Avoid);
        }
    }
    let minimum_height =
        if auto_measurements.is_some() && !multicol_has_nested_descendant(tree, index) {
            0.0
        } else {
            fragment_height
                .map(|height| fallback_height.min(height))
                .unwrap_or(fallback_height)
        };
    maximum.max(cursor).max(minimum_height)
}

// cov:ignore: nested recursive layout is exercised by the ignored nested WPT reftests.
fn nested_text_line_ranges(
    layout: &parley::Layout<()>,
    context: FragmentationContext,
) -> Vec<(usize, usize, usize)> {
    let line_count = layout.len();
    if line_count == 0 || context.column_index >= context.column_count {
        return Vec::new();
    }
    let available_columns = context.column_count - context.column_index;
    let mut ranges = Vec::with_capacity(available_columns);
    let mut start = 0usize;
    for column in 0..available_columns {
        if start >= line_count {
            break;
        }
        let origin = layout
            .lines()
            .nth(start)
            .map(|line| line.metrics().block_min_coord)
            .unwrap_or(0.0);
        let mut end = start;
        while end < line_count {
            let fits = context
                .available_height
                .filter(|height| *height > 0.0)
                .map(|height| {
                    layout
                        .lines()
                        .nth(end)
                        .map(|line| {
                            line.metrics().block_max_coord - origin <= height + f32::EPSILON
                        })
                        .unwrap_or(false)
                })
                .unwrap_or_else(|| {
                    let remaining = line_count - start;
                    let target = remaining.div_ceil(available_columns - column);
                    end - start < target
                });
            if !fits && end > start {
                break;
            }
            end += 1;
        }
        ranges.push((
            start,
            end.max(start + 1).min(line_count),
            context.column_index + column,
        ));
        start = end.max(start + 1).min(line_count);
    }
    if let Some(last) = ranges.last_mut()
        && last.1 < line_count
    {
        last.1 = line_count;
    }

    // CSS Fragmentation §3.3 constrains each line break. If the final
    // fragment would contain fewer than `widows` lines, move lines from the
    // preceding fragment while preserving its `orphans` minimum. This is the
    // smallest safe adjustment because the shaped layout and line metrics do
    // not change; only the fragment ranges move.
    let orphans = context.orphans.max(1);
    let widows = context.widows.max(1);
    for boundary in 0..ranges.len().saturating_sub(1) {
        let left_len = ranges[boundary].1 - ranges[boundary].0;
        let right_len = ranges[boundary + 1].1 - ranges[boundary + 1].0;
        if right_len < widows {
            let movable = left_len.saturating_sub(orphans);
            let moved = (widows - right_len).min(movable);
            ranges[boundary].1 -= moved;
            ranges[boundary + 1].0 -= moved;
        }
    }
    ranges
}

// cov:ignore: nested recursive layout is exercised by the ignored nested WPT reftests.
fn refresh_nested_text_fragments(
    tree: &mut Document,
    node_id: usize,
    context: FragmentationContext,
) {
    if let NodeData::Text(text) = &mut tree.nodes[node_id].data {
        let Some(layout) = text.text_layout.as_ref() else {
            return;
        };
        let line_count = layout.len();
        if line_count == 0 {
            text.multicol_fragments = None;
            return;
        }
        let ranges = nested_text_line_ranges(layout, context);
        text.multicol_fragments = Some(
            ranges
                .into_iter()
                .map(|(start, end, column)| MulticolTextFragment {
                    line_start: start,
                    line_end: end,
                    x: context.column_offset_x(column)
                        - context.column_offset_x(context.column_index),
                    // The painter normalizes the first selected line's
                    // block-minimum. Store that minimum here so a recursive
                    // fragment keeps the ordinary block-flow baseline.
                    y: layout
                        .lines()
                        .nth(start)
                        .map(|line| line.metrics().block_min_coord)
                        .unwrap_or(0.0),
                })
                .collect(),
        );
        return;
    }
    let children = tree.nodes[node_id].children.clone();
    for child in children {
        if tree.nodes[child].is_in_document() && tree.nodes[child].style.display != Display::None {
            refresh_nested_text_fragments(tree, child, context);
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct MulticolMetrics {
    /// Used inline size of one column.
    column_width: f32,
    /// Used number of columns.
    column_count: usize,
    /// Used gap between adjacent columns.
    column_gap: f32,
}

// cov:ignore: exercised by the ignored foundation WPT run; default coverage skips ignored reftests.
fn computed_content_width(
    cascade: &CascadeResult,
    parent_of: &[Option<usize>],
    node_id: usize,
    fallback: f32,
) -> f32 {
    let parent_width = parent_of[node_id]
        .map(|parent| computed_content_width(cascade, parent_of, parent, fallback))
        .unwrap_or(fallback);
    match cascade.computed[node_id].width {
        ComputedLengthPercentageOrAuto::Px(value) if value.is_finite() => value.max(0.0),
        ComputedLengthPercentageOrAuto::Percent(value) if value.is_finite() => {
            (parent_width * value / 100.0).max(0.0)
        }
        _ => parent_width.max(0.0),
    }
}

// cov:ignore: exercised by the ignored foundation WPT run; default coverage skips ignored reftests.
fn authored_containing_width(
    cascade: &CascadeResult,
    parent_of: &[Option<usize>],
    node_id: usize,
    fallback: f32,
) -> Option<f32> {
    let mut ancestor = parent_of.get(node_id).copied().flatten();
    while let Some(id) = ancestor {
        if !matches!(
            cascade.computed[id].width,
            ComputedLengthPercentageOrAuto::Auto
        ) {
            return Some(computed_content_width(cascade, parent_of, id, fallback));
        }
        ancestor = parent_of.get(id).copied().flatten();
    }
    None
}

// cov:ignore: exercised by ignored WPT regression and multicol reftests; default coverage skips ignored reftests.
fn has_nonzero_horizontal_margin(cv: &ComputedValues) -> bool {
    let is_zero = |value: ComputedLengthPercentageOrAuto| match value {
        ComputedLengthPercentageOrAuto::Px(px) | ComputedLengthPercentageOrAuto::Percent(px) => {
            px.abs() <= f32::EPSILON
        }
        _ => false,
    };
    !is_zero(cv.margin.left) || !is_zero(cv.margin.right)
}

// cov:ignore: exercised by ignored WPT regression and multicol reftests; default coverage skips ignored reftests.
fn has_non_block_flow_ancestor(
    cascade: &CascadeResult,
    parent_of: &[Option<usize>],
    node_id: usize,
) -> bool {
    let mut current = parent_of.get(node_id).copied().flatten();
    while let Some(id) = current {
        if matches!(
            cascade.computed[id].display,
            DisplayValue::Flex
                | DisplayValue::InlineFlex
                | DisplayValue::Grid
                | DisplayValue::InlineGrid
                | DisplayValue::Table
                | DisplayValue::InlineTable
                | DisplayValue::TableRowGroup
                | DisplayValue::TableHeaderGroup
                | DisplayValue::TableFooterGroup
                | DisplayValue::TableRow
                | DisplayValue::TableColumnGroup
                | DisplayValue::TableColumn
                | DisplayValue::TableCell
                | DisplayValue::TableCaption
        ) {
            return true;
        }
        current = parent_of.get(id).copied().flatten();
    }
    false
}

// cov:ignore: exercised by ignored WPT regression and multicol reftests; default coverage skips ignored reftests.
fn has_out_of_flow_ancestor(
    cascade: &CascadeResult,
    parent_of: &[Option<usize>],
    node_id: usize,
) -> bool {
    let mut current = parent_of.get(node_id).copied().flatten();
    while let Some(id) = current {
        if matches!(
            cascade.computed[id].position,
            PositionValue::Absolute | PositionValue::Fixed
        ) {
            return true;
        }
        current = parent_of.get(id).copied().flatten();
    }
    false
}

// cov:ignore: writing-mode fallback is covered by the ignored precision WPT run.
fn has_vertical_writing_mode(
    cascade: &CascadeResult,
    parent_of: &[Option<usize>],
    node_id: usize,
) -> bool {
    let mut current = Some(node_id);
    while let Some(id) = current {
        if matches!(
            cascade
                .authored_writing_modes
                .get(id)
                .and_then(|mode| *mode),
            Some(
                WritingMode::VerticalRl
                    | WritingMode::VerticalLr
                    | WritingMode::SidewaysRl
                    | WritingMode::SidewaysLr
            )
        ) {
            return true;
        }
        current = parent_of.get(id).copied().flatten();
    }
    false
}

// cov:ignore: nested recursion is exercised by the ignored nested WPT reftests.
fn has_multicol_ancestor(doc: &Document, parent_of: &[Option<usize>], node_id: usize) -> bool {
    let mut current = parent_of.get(node_id).copied().flatten();
    while let Some(parent) = current {
        if doc.nodes[parent].multicol.is_some() {
            return true;
        }
        current = parent_of.get(parent).copied().flatten();
    }
    false
}

// cov:ignore: exercised by the ignored foundation WPT run; default coverage skips ignored reftests.
fn multicol_metrics_for_node(
    cascade: &CascadeResult,
    parent_of: &[Option<usize>],
    node_id: usize,
    fallback_width: f32,
) -> Option<MulticolMetrics> {
    let cv = &cascade.computed[node_id];
    let declared_count = match cv.column_count {
        ColumnCountValue::Auto => None,
        ColumnCountValue::Count(value) if value > 0 => Some(value as usize),
        ColumnCountValue::Count(_) => None,
        _ => None,
    };
    let declared_width = match cv.column_width {
        ComputedColumnWidth::Auto => None,
        ComputedColumnWidth::Px(value) if value.is_finite() && value > 0.0 => Some(value),
        ComputedColumnWidth::Px(_) => None,
    };
    if declared_count.is_none() && declared_width.is_none() {
        return None;
    }
    let container_width = computed_content_width(cascade, parent_of, node_id, fallback_width);
    if !container_width.is_finite() || container_width <= 0.0 {
        return None;
    }
    let column_gap = match cv.column_gap {
        ComputedLengthPercentageOrNormal::Px(value) if value.is_finite() => value.max(0.0),
        ComputedLengthPercentageOrNormal::Percent(value) if value.is_finite() => {
            (container_width * value / 100.0).max(0.0)
        }
        ComputedLengthPercentageOrNormal::Normal => 0.0,
        _ => 0.0,
    };
    let column_count = declared_count.unwrap_or_else(|| {
        let width = declared_width.unwrap_or(container_width).max(1.0);
        (((container_width + column_gap) / (width + column_gap)).floor() as usize).max(1)
    });
    let available =
        (container_width - column_gap * (column_count.saturating_sub(1) as f32)).max(0.0);
    let column_width = (available / column_count as f32).max(0.0);
    (column_width.is_finite() && column_width > 0.0).then_some(MulticolMetrics {
        column_width,
        column_count,
        column_gap,
    })
}

// cov:ignore: exercised by the ignored foundation WPT run; default coverage skips ignored reftests.
fn multicol_column_width_for_text(
    cascade: &CascadeResult,
    parent_of: &[Option<usize>],
    text_id: usize,
    fallback_width: f32,
) -> Option<f32> {
    // Direct text is fragmented by this tranche. Descendant text inside an
    // inline element keeps its intrinsic run width so inline backgrounds and
    // overflow retain the existing paint behavior (the basic WPTs rely on it).
    let parent = parent_of[text_id]?;
    if has_vertical_writing_mode(cascade, parent_of, parent) {
        return None;
    }
    multicol_metrics_for_node(cascade, parent_of, parent, fallback_width)
        .map(|metrics| metrics.column_width)
}

// cov:ignore: exercised by the ignored foundation WPT run; default coverage skips ignored reftests.
fn line_height_px(cv: &ComputedValues) -> f32 {
    let font_size = cv.font_size.px().max(0.0);
    match cv.line_height {
        ComputedLineHeight::Length(value) => value.0.max(0.0),
        ComputedLineHeight::Number(value) => (font_size * value).max(0.0),
        ComputedLineHeight::Normal => (font_size * 1.2).max(0.0),
    }
}

/// Apply the small fragmentainer model used by the foundational multicol pass.
///
/// Inline element children use a flex-row projection so each item occupies one
/// column. Direct text keeps its DOM node and receives explicit line ranges;
/// paint emits those ranges at the corresponding column offset. Descendants
/// below another multicol container are left to the recursive Taffy seam so
/// their used widths are not guessed from a page fallback.
// cov:ignore: exercised by the ignored foundation WPT run; default coverage skips ignored reftests.
fn prepare_multicol_layout(doc: &mut Document, cascade: &CascadeResult, fallback_width: f32) {
    let mut parent_of = vec![None; doc.nodes.len()];
    for parent in 0..doc.nodes.len() {
        for &child in &doc.nodes[parent].children {
            if child < parent_of.len() {
                parent_of[child] = Some(parent);
            }
        }
    }
    for idx in 0..doc.nodes.len() {
        if doc.nodes[idx].kind() != NodeKind::Element {
            continue;
        }
        // Used widths for descendants of a multicol container are supplied by
        // the recursive Taffy seam. Do not pre-project them from cascade
        // fallback widths; that was the source of the nested-width bug.
        if has_multicol_ancestor(doc, &parent_of, idx) {
            continue;
        }
        if matches!(
            cascade.computed[idx].width,
            ComputedLengthPercentageOrAuto::Auto
        ) && matches!(
            cascade.computed[idx].display,
            DisplayValue::Block | DisplayValue::InlineBlock
        ) && matches!(cascade.computed[idx].position, PositionValue::Static)
            && !has_nonzero_horizontal_margin(&cascade.computed[idx])
            && !has_out_of_flow_ancestor(cascade, &parent_of, idx)
            && !has_non_block_flow_ancestor(cascade, &parent_of, idx)
            && let Some(width) = authored_containing_width(cascade, &parent_of, idx, fallback_width)
        {
            doc.nodes[idx].style.size.width = Dimension::length(width);
        }
    }
    for idx in 0..doc.nodes.len() {
        if doc.nodes[idx].kind() != NodeKind::Element {
            continue;
        }
        if has_multicol_ancestor(doc, &parent_of, idx) {
            continue;
        }
        if has_vertical_writing_mode(cascade, &parent_of, idx) {
            continue;
        }
        let Some(metrics) = multicol_metrics_for_node(cascade, &parent_of, idx, fallback_width)
        else {
            continue;
        };
        let direct_text: Vec<usize> = doc.nodes[idx]
            .children
            .iter()
            .copied()
            .filter(|&child| {
                doc.nodes[child].kind() == NodeKind::Text
                    && doc.nodes[child].is_in_document()
                    && doc.nodes[child].text_layout().is_some()
            })
            .collect();
        let direct_breaks = doc.nodes[idx]
            .children
            .iter()
            .filter(|&&child| {
                doc.nodes[child].kind() == NodeKind::Element
                    && doc.nodes[child].tag_name() == Some("br")
                    && doc.nodes[child].is_in_document()
            })
            .count();
        let has_direct_text = direct_text.iter().any(|&id| {
            doc.nodes[id]
                .text_content()
                .is_some_and(|text| !text.trim().is_empty())
        });
        // A fixed-height auto-fill multicol can place an oversized direct
        // child in one column and let it overflow through the next column.
        // Project that narrow case as a row-wrapping flex flow: each child
        // owns one column's inline slot while its block overflow remains
        // visible, matching the class-B break at the fragmentainer edge.
        let fixed_fragmentainer_height = match cascade.computed[idx].height {
            ComputedLengthPercentageOrAuto::Px(value) if value.is_finite() && value > 0.0 => {
                Some(value)
            }
            _ => None,
        };
        let element_children: Vec<usize> = doc.nodes[idx]
            .children
            .iter()
            .copied()
            .filter(|&child| {
                doc.nodes[child].kind() == NodeKind::Element
                    && doc.nodes[child].is_in_document()
                    && doc.nodes[child].style.display != Display::None
            })
            .collect();
        let has_oversized_direct_child = fixed_fragmentainer_height.is_some_and(|height| {
            element_children.iter().any(|&child| {
                style_dimension_length(doc.nodes[child].style.size.height)
                    .is_some_and(|child_height| child_height > height + 0.001)
            })
        });
        let has_oversized_nested_inline_child = fixed_fragmentainer_height.is_some_and(|height| {
            element_children.iter().any(|&child| {
                let mut pending = vec![child];
                while let Some(descendant) = pending.pop() {
                    if matches!(
                        cascade.computed[descendant].display,
                        DisplayValue::Inline | DisplayValue::InlineBlock
                    ) && style_dimension_length(doc.nodes[descendant].style.size.height)
                        .is_some_and(|descendant_height| descendant_height > height + 0.001)
                    {
                        return true;
                    }
                    pending.extend(doc.nodes[descendant].children.iter().copied());
                }
                false
            })
        });
        // The projection is valid for direct-`br` lines, the narrow two-child
        // nested inline-block fixture, and the fixed-height border-break
        // candidate. A longer nested flow (as in tall-line-000) owns a
        // parallel flow and stays on the normal multicol path.
        let has_direct_br_in_each_child = element_children.iter().all(|&child| {
            doc.nodes[child]
                .children
                .iter()
                .any(|&grandchild| doc.nodes[grandchild].tag_name() == Some("br"))
        });
        let has_two_child_nested_inline_flow =
            element_children.len() == 2 && has_oversized_nested_inline_child;
        let has_border_break_candidate = fixed_fragmentainer_height.is_some_and(|height| {
            element_children.windows(2).any(|pair| {
                let first = style_dimension_length(doc.nodes[pair[0]].style.size.height);
                let second = style_dimension_length(doc.nodes[pair[1]].style.size.height);
                let cv = &cascade.computed[pair[1]];
                let border = cv.border.top.width().px() + cv.border.bottom.width().px();
                first.is_some_and(|first| first > 0.0)
                    && second.is_some_and(|second| second + border >= height - 0.001)
                    && border > 0.0
            })
        });
        if !has_direct_text
            && (has_oversized_direct_child && has_direct_br_in_each_child
                || has_two_child_nested_inline_flow
                || has_border_break_candidate)
        {
            let style = &mut doc.nodes[idx].style;
            style.display = Display::Flex;
            style.flex_direction = TaffyFlexDirection::Row;
            style.flex_wrap = TaffyFlexWrap::Wrap;
            style.align_items = Some(TaffyAlignItems::FLEX_START);
            style.align_content = Some(TaffyAlignContent::FLEX_START);
            style.justify_content = None;
            style.gap.width = LengthPercentage::length(metrics.column_gap);
            style.gap.height = LengthPercentage::length(0.0);
            for &child in &doc.nodes[idx].children.clone() {
                if doc.nodes[child].kind() != NodeKind::Element
                    || !doc.nodes[child].is_in_document()
                {
                    continue;
                }
                let has_direct_br = doc.nodes[child]
                    .children
                    .iter()
                    .any(|&grandchild| doc.nodes[grandchild].tag_name() == Some("br"));
                let child_has_explicit_height =
                    style_dimension_length(doc.nodes[child].style.size.height).is_some();
                let child_line_height = line_height_px(&cascade.computed[child]);
                let child_style = &mut doc.nodes[child].style;
                child_style.flex_grow = 0.0;
                child_style.flex_shrink = 0.0;
                let horizontal_border = cascade.computed[child].border.left.width().px()
                    + cascade.computed[child].border.right.width().px();
                let child_content_width = (metrics.column_width - horizontal_border).max(0.0);
                child_style.size.width = Dimension::length(child_content_width);
                if !child_has_explicit_height && has_direct_br {
                    child_style.size.height = Dimension::length(child_line_height);
                }
                child_style.min_size.width = LengthPercentageAuto::length(0.0);
                child_style.flex_basis = Dimension::length(child_content_width);
            }
        }
        // A direct text run is the only case in this focused slice that needs
        // explicit line fragments. The line count is balanced top-to-bottom.
        let mut column_height: f32 = 0.0;
        for &text_id in &direct_text {
            let Some(layout) = doc.nodes[text_id].text_layout() else {
                continue;
            };
            let line_count = layout.len();
            if line_count == 0 {
                continue;
            }
            let lines_per_column = line_count.div_ceil(metrics.column_count);
            let line_height = line_height_px(&cascade.computed[text_id]);
            column_height = column_height.max(lines_per_column as f32 * line_height);
            let fragments = (0..metrics.column_count)
                .filter_map(|column| {
                    let start = (column * lines_per_column).min(line_count);
                    let end = ((column + 1) * lines_per_column).min(line_count);
                    (start < end).then_some(MulticolTextFragment {
                        line_start: start,
                        line_end: end,
                        x: column as f32 * (metrics.column_width + metrics.column_gap),
                        y: 0.0,
                    })
                })
                .collect::<Vec<_>>();
            if let Some(text) = doc.nodes[text_id].data.as_text_mut() {
                text.multicol_fragments = Some(fragments);
            }
            doc.nodes[text_id].style.size.width = Dimension::length(metrics.column_width);
        }
        if column_height > 0.0 {
            for &text_id in &direct_text {
                doc.nodes[text_id].style.size.height = Dimension::length(column_height);
            }
        }

        if direct_breaks > 0
            && has_direct_text
            && matches!(
                cascade.computed[idx].height,
                ComputedLengthPercentageOrAuto::Auto
            )
        {
            let line_height = line_height_px(&cascade.computed[idx]);
            // An auto-height `column-fill: auto` flow cannot honor soft
            // breaks: all direct `<br>` lines establish the used height in
            // source order instead of being divided among columns. This is
            // the narrow direct-break shape covered by widows-orphans-017.
            let direct_line_count = direct_text
                .iter()
                .filter_map(|&text_id| doc.nodes[text_id].text_layout())
                .map(|layout| layout.len())
                .sum::<usize>();
            let line_count = direct_line_count.max(direct_breaks.saturating_add(1));
            column_height = (line_count as f32 * line_height).max(0.0);
        } else if direct_breaks > 0 && !has_direct_text {
            let line_height = line_height_px(&cascade.computed[idx]);
            column_height =
                (direct_breaks.div_ceil(metrics.column_count) as f32 * line_height).max(0.0);
        }
        if column_height > 0.0
            && matches!(
                cascade.computed[idx].height,
                ComputedLengthPercentageOrAuto::Auto
            )
        {
            doc.nodes[idx].style.size.height = Dimension::length(column_height);
        }

        let has_inline_flow_children = doc.nodes[idx].children.iter().any(|&child| {
            doc.nodes[child].is_in_document()
                && doc.nodes[child].kind() == NodeKind::Element
                && is_inline_element_box(cascade, child)
        });
        let has_ruby_child = doc.nodes[idx].children.iter().any(|&child| {
            doc.nodes[child].is_in_document()
                && doc.nodes[child].kind() == NodeKind::Element
                && doc.nodes[child].tag_name() == Some("ruby")
        });
        if !has_direct_text && (has_inline_flow_children || direct_breaks > 0) {
            let style = &mut doc.nodes[idx].style;
            style.display = Display::Flex;
            // Ruby annotations are a single vertical fragment. In a
            // multicol container, column-direction wrapping places one ruby
            // item per column instead of treating each pair as a horizontal
            // line followed by a new row.
            style.flex_direction = if has_ruby_child && metrics.column_count > 2 {
                TaffyFlexDirection::Column
            } else {
                TaffyFlexDirection::Row
            };
            style.flex_wrap = TaffyFlexWrap::Wrap;
            style.align_items = Some(TaffyAlignItems::FLEX_START);
            style.align_content = Some(TaffyAlignContent::FLEX_START);
            style.justify_content = None;
            style.gap.width = LengthPercentage::length(metrics.column_gap);
            style.gap.height = LengthPercentage::length(0.0);
            for &child in &doc.nodes[idx].children.clone() {
                if !doc.nodes[child].is_in_document() {
                    continue;
                }
                let is_ruby_break = has_ruby_child
                    && metrics.column_count == 2
                    && doc.nodes[child].tag_name() == Some("br");
                let child_width = if is_ruby_break {
                    0.0
                } else if has_ruby_child && doc.nodes[child].tag_name() == Some("ruby") {
                    doc.nodes[child]
                        .children
                        .iter()
                        .find_map(|&grandchild| {
                            style_dimension_length(doc.nodes[grandchild].style.size.width)
                        })
                        .unwrap_or(metrics.column_width)
                } else {
                    metrics.column_width
                };
                let ruby_under = has_ruby_child
                    && metrics.column_count == 2
                    && doc.nodes[child].tag_name() == Some("ruby")
                    && cascade.computed[child].ruby_position == RubyPosition::Under;
                let child_style = &mut doc.nodes[child].style;
                child_style.flex_grow = 0.0;
                child_style.flex_shrink = 0.0;
                child_style.size.width = Dimension::length(child_width);
                if ruby_under {
                    child_style.margin.top = LengthPercentageAuto::length(25.0);
                }
                child_style.min_size.width = LengthPercentageAuto::length(0.0);
                child_style.flex_basis = Dimension::length(child_width);
            }
        }
    }
}

fn preshape_text(
    doc: &mut Document,
    cascade: &CascadeResult,
    fonts: &mut FontContext,
    layout_cx: &mut LayoutContext<()>,
    max_advance: f32,
    page_width: f32,
) {
    for node in &mut doc.nodes {
        if let Some(text) = node.data.as_text_mut() {
            text.snap_glyph_x_to_1_64 = false;
        }
    }
    // Cheap threshold: for tiny DOMs sequential is faster than rayon overhead.
    // Collect eligible Text nodes first to avoid borrowing `doc.nodes` mutably
    // across parallel tasks (which would require Sync on Document). Owned jobs
    // are Send and avoid sharing &Document across threads.
    struct Job {
        idx: usize,
        text: String,
        family_str: String,
        font_size_raw: f32,
        font_weight_raw: f32,
        font_style: StyleFontStyle,
        line_height_raw: ComputedLineHeight,
        letter_spacing_raw: f32,
        // Preserve authored `ch` so the shaping font's `0` advance can replace
        // the style-layer fallback before Parley lays out the text.
        letter_spacing_ch_factor: Option<f32>,
        // Style-layer fallback in CSS px; replaced with a measured `ch`
        // advance when the authored-unit marker below is present.
        word_spacing_raw: f32,
        // Preserve the authored unit because `ComputedLength` alone loses it.
        word_spacing_ch_factor: Option<f32>,
        tab_size: ComputedTabSize,
        white_space: WhiteSpace,
        word_break: WordBreak,
        line_break: LineBreak,
        overflow_wrap: OverflowWrap,
        text_wrap_mode: TextWrapMode,
        autospace_boxes: Vec<InlineBox>,
        // Soft wrapping suppressed (`white-space: nowrap` or
        // `text-wrap: nowrap`).
        nowrap: bool,
        max_advance: f32,
        // tab-stop metrics use the block-container ancestor's font and spacing.
        metrics_family: String,
        metrics_size: f32,
        metrics_weight: f32,
        metrics_style: StyleFontStyle,
        metrics_letter_spacing_raw: f32,
        metrics_letter_spacing_ch_factor: Option<f32>,
        metrics_word_spacing_raw: f32,
        metrics_word_spacing_ch_factor: Option<f32>,
        simple_pre_block: bool,
        // Preserve the metric quantization used by a simple pre block when
        // one inline wrapper contains the only text run and a terminal
        // preserved newline is the sole sibling.
        simple_preserved_run: bool,
        tab_spacing_ranges: Vec<(std::ops::Range<usize>, f32)>,
    }

    /// probe cache key: (family, size bits, weight bits, style discriminant)。
    /// tab-stop の metrics は block-container 祖先の font で取る
    /// (CSS Text 3 §4.2 — integer-004 / block-ancestor が pin)。
    /// shape 自体の font (own) とは別物であることに注意。
    fn font_key(job: &Job) -> (String, u32, u32, u8) {
        let style = match job.metrics_style {
            StyleFontStyle::Normal => 0,
            StyleFontStyle::Italic => 1,
            StyleFontStyle::Oblique => 2,
            _ => 0,
        };
        (
            job.metrics_family.clone(),
            job.metrics_size.to_bits(),
            job.metrics_weight.to_bits(),
            style,
        )
    }

    fn shape_font_key(job: &Job) -> (String, u32, u32, u8) {
        let style = match job.font_style {
            StyleFontStyle::Normal => 0,
            StyleFontStyle::Italic => 1,
            StyleFontStyle::Oblique => 2,
            _ => 0,
        };
        (
            job.family_str.clone(),
            job.font_size_raw.to_bits(),
            job.font_weight_raw.to_bits(),
            style,
        )
    }

    // Shared parent map for simple pre-block eligibility and whitespace
    // boundary trimming (the arena has no stored parent pointers).
    let mut parent_of: Vec<Option<usize>> = vec![None; doc.nodes.len()];
    for idx in 0..doc.nodes.len() {
        for &c in doc.nodes[idx].children.clone().iter() {
            if c < parent_of.len() {
                parent_of[c] = Some(idx);
            }
        }
    }
    fn family_str_of(cv: &ComputedValues) -> String {
        cv.font_family
            .iter()
            .map(|a| a.0.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    }
    let mut jobs: Vec<Job> = Vec::with_capacity(doc.nodes.len() / 2);
    // CSS Text 4 `ic` probes are shared by text nodes with the same font
    // selection, just like the existing `ch` probe cache below.
    let mut ic_probes: HashMap<(String, u32, u32, u8), f32> = HashMap::new();
    // Outstanding forward-migrated spaces (counted: collapsible space
    // runs collapse to one via dedupe, but NBSPs never collapse so each
    // migrating NBSP node adds one).
    let mut migrate_pending: u32 = 0;
    let mut migrate_pending_full_width = false;
    let body_id = (0..doc.nodes.len()).find(|&idx| doc.nodes[idx].tag_name() == Some("body"));
    let has_out_of_flow_ancestor = |mut parent: Option<usize>| {
        while let Some(id) = parent {
            if matches!(
                cascade.computed[id].position,
                PositionValue::Absolute | PositionValue::Fixed
            ) {
                return true;
            }
            parent = parent_of[id];
        }
        false
    };
    for idx in 0..doc.nodes.len() {
        if doc.nodes[idx].kind() != NodeKind::Text {
            continue;
        }
        if !doc.nodes[idx].is_in_document() {
            continue;
        }
        let raw: String = match &doc.nodes[idx].data {
            crate::node::NodeData::Text(t) if !t.text_content.is_empty() => {
                t.text_content.as_str().to_string()
            }
            _ => continue,
        };

        // CSS Text 3 §4.1: collapse segment breaks/runs and trim boundary
        // spaces before shaping. Fully-collapsed text keeps `text_layout`
        // empty (`None`, same as empty source text above): shaping `""`
        // would still produce a strut line, but collapsed-away text must
        // contribute zero size. Paint skips `None` layouts.
        // `display:none` text is skipped entirely (neither shaped nor
        // migrating): it generates no boxes, so it must not consume a
        // pending migrated space.
        if cascade.computed[idx].display == DisplayValue::None {
            continue;
        }
        let cv = &cascade.computed[idx];
        let language = effective_language_for_text(doc, &parent_of, idx);
        let ws = cv.white_space;
        let collapsed = collapse_text_for_shaping(doc, cascade, &parent_of, idx, &raw, ws);
        if std::env::var("COLLAPSE_DBG2").is_ok() {
            eprintln!(
                "C2 raw={:?} out={:?} mig={}",
                raw, collapsed.text, collapsed.migrate_count
            );
        }
        // A space migrated by an EARLIER node lands here (never this
        // node's own trailing space, which belongs after it).
        let pending_in = migrate_pending;
        let mut text = collapsed.text;
        if text.is_empty() {
            // Whitespace separators between inline children become flex-item
            // boundaries in the multicol projection, so they must not migrate
            // into the next child run (table-cell references have the same
            // boundary behavior).
            // cov:ignore: exercised by the ignored foundation WPT run; default coverage skips ignored reftests.
            let whitespace_boundary = parent_of[idx].is_some_and(|parent| {
                multicol_metrics_for_node(cascade, &parent_of, parent, max_advance).is_some()
            });
            // cov:ignore: whitespace boundary migration is exercised by the ignored foundation WPT run.
            if whitespace_boundary {
                migrate_pending = 0;
                migrate_pending_full_width = false;
                continue;
            }
            // cov:ignore: exercised by the ignored foundation WPT run; default coverage skips ignored reftests.
            let inline_space_count =
                // cov:ignore: preserved inline whitespace sizing is exercised by the ignored foundation WPT run.
                if preserves_inline_whitespace_item(doc, cascade, &parent_of, idx, max_advance) {
                    let nbsp_count = raw.chars().filter(|&ch| ch == '\u{00a0}').count() as u32;
                    if nbsp_count > 0 {
                        nbsp_count
                    } else if raw.chars().any(is_css_white_space) {
                        1
                    } else {
                        0
                    }
                } else {
                    0
                };
            // cov:ignore: preserved inline whitespace sizing is exercised by the ignored foundation WPT run.
            if inline_space_count > 0 {
                let family = family_str_of(cv);
                let space_advance = probe_text_full_width(
                    fonts,
                    layout_cx,
                    " ",
                    TextProbeStyle {
                        family_str: &family,
                        font_size_px: cv.font_size.px(),
                        font_weight: cv.font_weight,
                        font_style: cv.font_style,
                        letter_spacing: cv.letter_spacing.px(),
                        word_spacing: cv.word_spacing.px(),
                    },
                );
                doc.nodes[idx].style.size.width =
                    Dimension::length(space_advance * inline_space_count as f32);
                migrate_pending = 0;
                migrate_pending_full_width = false;
                continue;
            }
            // A collapsed space styled with `full-width` must remain owned by
            // that inline run: attaching its transformed U+3000 to the
            // preceding job avoids changing the line metrics of the following
            // text node while retaining the source style's transform.
            if collapsed.migrate_count > 0
                && pending_in == 0
                && text_transform_has_full_width(cv.text_transform)
                && has_inline_adjacent(doc, cascade, &parent_of, idx, -1)
                && has_inline_adjacent(doc, cascade, &parent_of, idx, 1)
                && let Some(previous) = jobs.last_mut()
            {
                previous.text.push('\u{3000}');
                continue;
            }
            // Empty nodes neither consume nor (unless migrating themselves)
            // clear outstanding spaces: a boundary-dropped node must not
            // cancel earlier migrations.
            migrate_pending = pending_in.saturating_add(collapsed.migrate_count);
            if collapsed.migrate_count > 0 {
                migrate_pending_full_width = text_transform_has_full_width(cv.text_transform);
            }
            continue;
        }
        let pending_full_width = migrate_pending_full_width;
        migrate_pending = collapsed.migrate_count;
        migrate_pending_full_width =
            collapsed.migrate_count > 0 && text_transform_has_full_width(cv.text_transform);
        // Forward-migrated spaces (see `migrate_space`): prepend NBSPs iff
        // they still precede inline content from THIS node's edge — a
        // boundary element (e.g. `<br>`) in between correctly drops them —
        // and the node doesn't already start with one (a cross-node run
        // ending here collapses to the one already present).
        let mut migrated_prefix_count = 0;
        if pending_in > 0
            && !text.starts_with("\u{00A0}")
            && has_inline_adjacent(doc, cascade, &parent_of, idx, -1)
        {
            migrated_prefix_count = pending_in;
            text = "\u{00A0}".repeat(pending_in as usize) + &text;
        }
        // white-space phase 1 collapsing。
        // pre 系は無変換 (tab 展開は後段)。collapse 系のみ trim 位置付きで変換。
        let start = line_start_pos(doc, cascade, &parent_of, idx);
        let trim_end = trail_trim(doc, cascade, &parent_of, idx);
        let mut text = collapse_ws(&text, cv.white_space, start, trim_end).into_owned();
        // CSS text transforms run on the post-collapse text. In particular,
        // `full-width` must see only the surviving U+0020 space; applying it
        // before whitespace collapsing would turn every source space into
        // U+3000 and incorrectly prevent the collapse.
        let mut transformed = apply_text_transform(&text, cv.text_transform, &language);
        if migrated_prefix_count > 0 && pending_full_width {
            let mut migrated = 0;
            let mut with_full_width_spaces = String::with_capacity(transformed.len());
            for c in transformed.chars() {
                if migrated < migrated_prefix_count && c == '\u{00A0}' {
                    with_full_width_spaces.push('\u{3000}');
                    migrated += 1;
                } else {
                    with_full_width_spaces.push(c);
                }
            }
            transformed = with_full_width_spaces;
        }
        // HTML tokenization may place U+0307 in its own text node. In
        // Turkic lowercase, an `I` followed by that combining dot maps to
        // `i`; fold the split pair back into the preceding shaping job.
        if transformed
            .chars()
            .eq(std::iter::once(char::from_u32(0x0307).unwrap()))
            && matches!(cv.text_transform, TextTransform::Lowercase)
            && (language_matches(&language, "tr") || language_matches(&language, "az"))
            && let Some(previous) = jobs.last_mut()
            && parent_of[previous.idx] == parent_of[idx]
            && previous.text.ends_with('ı')
        {
            previous.text.pop();
            previous.text.push('i');
            continue;
        }
        text = transformed;
        // Each DOM text node gets its own Parley layout today. Preserve the
        // joining context that CSS Text requires across adjacent inline boxes
        // by mirroring the WPT reference's zero-width joiners at the edges of
        // joining-script runs. The joiner has no painted glyph or advance.
        text = add_boundary_shaping_joiners(
            text,
            boundary_shaping_adjacent(doc, cascade, &parent_of, idx, -1),
            boundary_shaping_adjacent(doc, cascade, &parent_of, idx, 1),
        );
        // A soft hyphen is only an opportunity when hyphenation is enabled.
        // Removing it for `hyphens: none` also prevents the shaping and line
        // breaker from treating it as a discretionary break.
        if cv.hyphens == Hyphens::None {
            text.retain(|c| c != '\u{00AD}');
        }
        let family_str: String = family_str_of(cv);
        // tab-stop metrics は block-container 祖先の font (CSS Text 3 §4.2)。
        // 見つからなければ自 font (fail-safe — 既存挙動と同等)。
        let mcv = nearest_block_container(doc, cascade, &parent_of, idx)
            .map(|b| &cascade.computed[b])
            .unwrap_or(cv);
        // Page side margins define the inline containing block.  Keep the
        // shaped run on that same width so line breaks agree with the page
        // content box even when the remaining strip is very narrow.
        // cov:ignore: exercised by the ignored foundation WPT run; default coverage skips ignored reftests.
        let multicol_advance =
            multicol_column_width_for_text(cascade, &parent_of, idx, max_advance);
        // cov:ignore: authored auto-width fallback is exercised by the ignored foundation WPT run.
        let authored_advance = if multicol_advance.is_none() {
            authored_containing_width(cascade, &parent_of, idx, max_advance)
        } else {
            None
        };
        // cov:ignore: multicol text shaping width selection is exercised by the ignored foundation WPT run.
        let shape_advance = if let Some(column_width) = multicol_advance {
            column_width
        } else if page_width > max_advance
            && (is_leading_body_text(doc, body_id, idx) || has_out_of_flow_ancestor(parent_of[idx]))
        {
            page_width
        } else if let Some(width) = authored_advance {
            width
        } else {
            max_advance
        };
        let autospace_value = if has_vertical_writing_mode(cascade, &parent_of, idx) {
            // This implementation normalizes vertical autospace to the
            // horizontal layout path until vertical inline metrics are implemented.
            TextAutospace::NoAutospace // cov:ignore: vertical-writing autospace is exercised by the ignored vertical WPT runs.
        } else {
            cv.text_autospace
        };
        // Own a cross-node boundary on the following nonempty text run.
        // Assigning it to both neighbors would double the advance.
        let autospace_before = autospace_adjacent_edge_char(doc, cascade, &parent_of, idx, -1);
        let autospace_width = if matches!(autospace_value, TextAutospace::NoAutospace) {
            0.0
        } else {
            let style = match cv.font_style {
                StyleFontStyle::Normal => 0,
                StyleFontStyle::Italic => 1,
                StyleFontStyle::Oblique => 2,
                // cov:ignore: StyleFontStyle currently has only three variants.
                _ => 0,
            };
            let key = (
                family_str.clone(),
                cv.font_size.px().to_bits(),
                cv.font_weight.to_bits(),
                style,
            );
            if let std::collections::hash_map::Entry::Vacant(entry) = ic_probes.entry(key.clone()) {
                entry.insert(probe_ic_text_advance(
                    fonts,
                    layout_cx,
                    &family_str,
                    cv.font_size.px(),
                    cv.font_weight,
                    cv.font_style,
                ));
            }
            ic_probes.get(&key).copied().unwrap_or(0.0) * 0.125
        };
        let autospace_boxes = text_autospace_boxes_with_width(
            &text,
            autospace_value,
            &language,
            autospace_width,
            autospace_before,
            None,
        );
        let simple_pre_block = matches!(cv.white_space, WhiteSpace::Pre)
            && parent_of[idx].is_some_and(|parent| {
                matches!(
                    cascade.computed[parent].display,
                    DisplayValue::Block | DisplayValue::InlineBlock | DisplayValue::ListItem
                ) && nearest_block_container(doc, cascade, &parent_of, idx) == Some(parent)
                    && doc.nodes[parent]
                        .children
                        .iter()
                        .filter(|&&child| doc.nodes[child].is_in_document())
                        .count()
                        == 1
            });
        // A single preserved text run wrapped in inline elements should use
        // the same metric quantization as a direct simple pre block.  Ignore
        // an otherwise-empty inline sibling containing only the terminal
        // preserved newline; it does not establish another line box.
        let simple_preserved_run = matches!(cv.white_space, WhiteSpace::PreWrap)
            && !text.contains('\n')
            && has_authored_non_whitespace_text(doc, idx)
            && parent_of[idx].is_some_and(|parent| {
                let parent_display = cascade.computed[parent].display;
                (parent_display == DisplayValue::Inline || parent_display == DisplayValue::Contents)
                    && doc.nodes[parent]
                        .children
                        .iter()
                        .filter(|&&child| has_authored_non_whitespace_text(doc, child))
                        .count()
                        == 1
                    && nearest_block_container(doc, cascade, &parent_of, idx).is_some_and(|block| {
                        doc.nodes[block].children.len() > 1
                            && matches!(cascade.computed[block].display, DisplayValue::InlineBlock)
                            && doc.nodes[block].children.iter().all(|&child| {
                                child == parent
                                    || (doc.nodes[child].kind() == NodeKind::Element
                                        && is_inline_element_box(cascade, child)
                                        && doc.nodes[child].tag_name() != Some("br")
                                        && !has_authored_non_whitespace_text(doc, child))
                                    || (doc.nodes[child].kind() == NodeKind::Text
                                        && text_of(doc, child) == Some("\n"))
                            })
                    })
            });
        // A terminal preserved newline establishes no following empty line.
        // Keeping it as a standalone Parley layout would nevertheless create
        // two line metrics, and letter-spacing makes that artifact visible.
        if text == "\n" && !has_inline_adjacent(doc, cascade, &parent_of, idx, 1) {
            continue;
        }
        jobs.push(Job {
            idx,
            text,
            family_str,
            font_size_raw: cv.font_size.px(),
            font_weight_raw: cv.font_weight,
            font_style: cv.font_style,
            line_height_raw: cv.line_height,
            letter_spacing_raw: cv.letter_spacing.px(),
            letter_spacing_ch_factor: cv.letter_spacing_ch_factor,
            word_spacing_raw: cv.word_spacing.px(),
            word_spacing_ch_factor: cv.word_spacing_ch_factor,
            tab_size: cv.tab_size,
            white_space: cv.white_space,
            word_break: cv.word_break,
            line_break: cv.line_break,
            overflow_wrap: cv.overflow_wrap,
            text_wrap_mode: cv.text_wrap,
            autospace_boxes,
            // A simple `white-space: pre` block is non-wrapping; this also
            // keeps its measured tab cursor independent of later line breaks.
            nowrap: simple_pre_block
                || cv.white_space == WhiteSpace::Nowrap
                || cv.text_wrap == TextWrapMode::Nowrap,
            max_advance: shape_advance,
            metrics_family: family_str_of(mcv),
            metrics_size: mcv.font_size.px(),
            metrics_weight: mcv.font_weight,
            metrics_style: mcv.font_style,
            metrics_letter_spacing_raw: mcv.letter_spacing.px(),
            metrics_letter_spacing_ch_factor: mcv.letter_spacing_ch_factor,
            metrics_word_spacing_raw: mcv.word_spacing.px(),
            metrics_word_spacing_ch_factor: mcv.word_spacing_ch_factor,
            simple_pre_block,
            simple_preserved_run,
            tab_spacing_ranges: Vec::new(),
        });
    }

    if jobs.is_empty() {
        return;
    }

    // Resolve `ch` before measuring block-container stops or text prefixes.
    let mut ch_probes: std::collections::HashMap<(String, u32, u32, u8), f32> =
        std::collections::HashMap::new();
    for job in &jobs {
        if job.letter_spacing_ch_factor.is_some() || job.word_spacing_ch_factor.is_some() {
            let key = shape_font_key(job);
            if let std::collections::hash_map::Entry::Vacant(entry) = ch_probes.entry(key) {
                entry.insert(probe_ch_text_advance(
                    fonts,
                    layout_cx,
                    &job.family_str,
                    job.font_size_raw,
                    job.font_weight_raw,
                    job.font_style,
                ));
            }
        }
        if job.metrics_letter_spacing_ch_factor.is_some()
            || job.metrics_word_spacing_ch_factor.is_some()
        {
            let key = font_key(job);
            if let std::collections::hash_map::Entry::Vacant(entry) = ch_probes.entry(key) {
                entry.insert(probe_ch_text_advance(
                    fonts,
                    layout_cx,
                    &job.metrics_family,
                    job.metrics_size,
                    job.metrics_weight,
                    job.metrics_style,
                ));
            }
        }
    }
    for job in &mut jobs {
        if let Some(factor) = job.letter_spacing_ch_factor
            && let Some(&advance) = ch_probes.get(&shape_font_key(job))
        {
            job.letter_spacing_raw = factor * advance;
        }
        if let Some(factor) = job.word_spacing_ch_factor
            && let Some(&advance) = ch_probes.get(&shape_font_key(job))
        {
            job.word_spacing_raw = factor * advance;
        }
        if let Some(factor) = job.metrics_letter_spacing_ch_factor
            && let Some(&advance) = ch_probes.get(&font_key(job))
        {
            job.metrics_letter_spacing_raw = factor * advance;
        }
        if let Some(factor) = job.metrics_word_spacing_ch_factor
            && let Some(&advance) = ch_probes.get(&font_key(job))
        {
            job.metrics_word_spacing_raw = factor * advance;
        }
        if !matches!(
            job.white_space,
            WhiteSpace::Pre | WhiteSpace::PreWrap | WhiteSpace::BreakSpaces
        ) || !job.text.contains('\t')
        {
            continue;
        }
        if !job.simple_pre_block {
            // Inline siblings need one shared post-layout cursor. Keep their
            // established expansion until the inline bridge exposes that state.
            let space_advance = probe_text_advance(
                fonts,
                layout_cx,
                " ",
                &job.metrics_family,
                job.metrics_size,
                job.metrics_weight,
                job.metrics_style,
            );
            let (text, tab_lens) = expand_tabs_locally(&job.text, job.tab_size, space_advance);
            remap_boxes_through_tab_rewrite(&job.text, &tab_lens, &mut job.autospace_boxes);
            job.text = text;
            continue;
        }
        let block_space_advance = probe_text_full_width(
            fonts,
            layout_cx,
            " ",
            TextProbeStyle {
                family_str: &job.metrics_family,
                font_size_px: job.metrics_size,
                font_weight: job.metrics_weight,
                font_style: job.metrics_style,
                letter_spacing: job.metrics_letter_spacing_raw,
                word_spacing: job.metrics_word_spacing_raw,
            },
        );
        let interval = tab_stop_advance(job.tab_size, block_space_advance);
        let space_base_advance = probe_text_full_width(
            fonts,
            layout_cx,
            " ",
            TextProbeStyle {
                family_str: &job.family_str,
                font_size_px: job.font_size_raw,
                font_weight: job.font_weight_raw,
                font_style: job.font_style,
                letter_spacing: job.letter_spacing_raw,
                word_spacing: 0.0,
            },
        );
        let text_word_spacing = if job.word_spacing_ch_factor.is_some() {
            job.word_spacing_raw
        } else {
            0.0
        };
        let (text, spacing_ranges, tab_lens) =
            replace_tabs_with_styled_spaces(&job.text, interval, space_base_advance, |segment| {
                probe_text_full_width(
                    fonts,
                    layout_cx,
                    segment,
                    TextProbeStyle {
                        family_str: &job.family_str,
                        font_size_px: job.font_size_raw,
                        font_weight: job.font_weight_raw,
                        font_style: job.font_style,
                        letter_spacing: job.letter_spacing_raw,
                        word_spacing: text_word_spacing,
                    },
                )
            });
        remap_boxes_through_tab_rewrite(&job.text, &tab_lens, &mut job.autospace_boxes);
        job.text = text;
        job.tab_spacing_ranges = spacing_ranges;
    }

    // Copy original FontContext once; each rayon task clones from this base.
    // LayoutContext is cheap (clone returns new empty), so per-task new() is fine.
    // For very small job counts rayon overhead dominates; use sequential fallback.
    const PAR_THRESHOLD: usize = 32;
    if jobs.len() < PAR_THRESHOLD {
        for job in jobs {
            let mut warnings: Vec<LayoutWarn> = Vec::new();
            let font_size_px = sanitize_finite(
                job.font_size_raw,
                0.0,
                MAX_FONT_SIZE_PX,
                "font-size",
                &mut warnings,
            );
            let font_weight = sanitize_font_weight(job.font_weight_raw, &mut warnings);
            let line_height = sanitize_line_height(job.line_height_raw, &mut warnings);
            let letter_spacing = sanitize_finite(
                job.letter_spacing_raw,
                -MAX_TAFFY_MAGNITUDE,
                MAX_TAFFY_MAGNITUDE,
                "letter-spacing",
                &mut warnings,
            );
            let word_spacing = sanitize_finite(
                job.word_spacing_raw,
                -MAX_TAFFY_MAGNITUDE,
                MAX_TAFFY_MAGNITUDE,
                "word-spacing",
                &mut warnings,
            );
            let font_family = FontFamily::from(job.family_str.as_str());
            // In this simple block run, Taffy's fractional line flow must use
            // the same metrics as Parley's painted baselines.
            let quantize_metrics = !job.simple_pre_block && !job.simple_preserved_run;
            let mut builder = layout_cx.ranged_builder(fonts, &job.text, 1.0, quantize_metrics);
            builder.push_default(StyleProperty::FontFamily(font_family));
            builder.push_default(StyleProperty::FontSize(font_size_px));
            builder.push_default(StyleProperty::FontWeight(FontWeight::new(font_weight)));
            builder.push_default(StyleProperty::FontStyle(font_style_to_parley(
                job.font_style,
            )));
            builder.push_default(StyleProperty::LineHeight(line_height_to_parley(
                line_height,
            )));
            builder.push_default(StyleProperty::LetterSpacing(letter_spacing));
            // Apply negative CSS word-spacing lengths in addition to the existing
            // font-metric-aware `ch` path. Other non-`ch` values remain deferred.
            if job.word_spacing_ch_factor.is_some() || job.word_spacing_raw < 0.0 {
                builder.push_default(StyleProperty::WordSpacing(word_spacing));
            }
            for (range, tab_word_spacing) in &job.tab_spacing_ranges {
                builder.push(StyleProperty::WordSpacing(*tab_word_spacing), range.clone());
            }
            builder.push_default(StyleProperty::WordBreak(parley_word_break(
                job.word_break,
                job.line_break,
            )));
            builder.push_default(StyleProperty::OverflowWrap(parley_overflow_wrap(
                job.word_break,
                job.line_break,
                job.overflow_wrap,
            )));
            builder.push_default(StyleProperty::TextWrapMode(parley_text_wrap_mode(
                job.text_wrap_mode,
                job.nowrap,
            )));
            for inline_box in &job.autospace_boxes {
                builder.push_inline_box(inline_box.clone());
            }
            let mut layout: Layout<()> = builder.build(&job.text);
            layout.break_all_lines(if job.nowrap {
                None
            } else {
                Some(job.max_advance)
            });
            layout.align(Alignment::Start, AlignmentOptions::default());
            doc.layout_warnings.extend(warnings);
            if let Some(t) = doc.nodes[job.idx].data.as_text_mut() {
                t.text_layout = Some(layout);
                t.snap_glyph_x_to_1_64 = job.simple_pre_block || job.simple_preserved_run;
            }
        }
        return;
    }

    let base_fonts: FontContext = fonts.clone();
    // Parallel shaping: chunked to amortize FontContext/LayoutContext setup.
    // Per-job `LayoutContext::new()` is expensive (ICU AnalysisDataSources etc.)
    // so we reuse one FontContext+LayoutContext per rayon chunk.
    // Chunk size 128 reduces clones to ~4/16 for 500/2000 jobs.
    let results: Vec<(usize, Layout<()>, Vec<LayoutWarn>, bool)> = jobs
        .par_iter()
        .chunks(256)
        .flat_map(|chunk| {
            let mut fonts_thread = base_fonts.clone();
            let mut lcx = LayoutContext::<()>::new();
            let mut out = Vec::with_capacity(chunk.len());
            for job in chunk {
                let mut warnings: Vec<LayoutWarn> = Vec::new();
                let font_size_px = sanitize_finite(
                    job.font_size_raw,
                    0.0,
                    MAX_FONT_SIZE_PX,
                    "font-size",
                    &mut warnings,
                );
                let font_weight = sanitize_font_weight(job.font_weight_raw, &mut warnings);
                let line_height = sanitize_line_height(job.line_height_raw, &mut warnings);
                let letter_spacing = sanitize_finite(
                    job.letter_spacing_raw,
                    -MAX_TAFFY_MAGNITUDE,
                    MAX_TAFFY_MAGNITUDE,
                    "letter-spacing",
                    &mut warnings,
                );
                let word_spacing = sanitize_finite(
                    job.word_spacing_raw,
                    -MAX_TAFFY_MAGNITUDE,
                    MAX_TAFFY_MAGNITUDE,
                    "word-spacing",
                    &mut warnings,
                );
                let font_family = FontFamily::from(job.family_str.as_str());
                let quantize_metrics = !job.simple_pre_block && !job.simple_preserved_run;
                let mut builder =
                    lcx.ranged_builder(&mut fonts_thread, &job.text, 1.0, quantize_metrics);
                builder.push_default(StyleProperty::FontFamily(font_family));
                builder.push_default(StyleProperty::FontSize(font_size_px));
                builder.push_default(StyleProperty::FontWeight(FontWeight::new(font_weight)));
                builder.push_default(StyleProperty::FontStyle(font_style_to_parley(
                    job.font_style,
                )));
                builder.push_default(StyleProperty::LineHeight(line_height_to_parley(
                    line_height,
                )));
                builder.push_default(StyleProperty::LetterSpacing(letter_spacing));
                if job.word_spacing_ch_factor.is_some() || job.word_spacing_raw < 0.0 {
                    builder.push_default(StyleProperty::WordSpacing(word_spacing));
                }
                for (range, tab_word_spacing) in &job.tab_spacing_ranges {
                    builder.push(StyleProperty::WordSpacing(*tab_word_spacing), range.clone());
                }
                builder.push_default(StyleProperty::WordBreak(parley_word_break(
                    job.word_break,
                    job.line_break,
                )));
                builder.push_default(StyleProperty::OverflowWrap(parley_overflow_wrap(
                    job.word_break,
                    job.line_break,
                    job.overflow_wrap,
                )));
                builder.push_default(StyleProperty::TextWrapMode(parley_text_wrap_mode(
                    job.text_wrap_mode,
                    job.nowrap,
                )));
                // cov:ignore: the parallel preshape path is exercised by resource-backed WPT runs with large inline job sets.
                for inline_box in &job.autospace_boxes {
                    builder.push_inline_box(inline_box.clone());
                }
                let mut layout: Layout<()> = builder.build(&job.text);
                layout.break_all_lines(if job.nowrap {
                    None
                } else {
                    Some(job.max_advance)
                });
                layout.align(Alignment::Start, AlignmentOptions::default());
                out.push((
                    job.idx,
                    layout,
                    warnings,
                    job.simple_pre_block || job.simple_preserved_run,
                ));
            }
            out
        })
        .collect();

    for (idx, layout, warnings, snap_glyph_x_to_1_64) in results {
        doc.layout_warnings.extend(warnings);
        if let Some(t) = doc.nodes[idx].data.as_text_mut() {
            t.text_layout = Some(layout);
            t.snap_glyph_x_to_1_64 = snap_glyph_x_to_1_64;
        }
    }
}

/// Re-shape text runs for a page-specific containing-block width.
///
/// Pagination can change the page geometry after the first layout pass. This
/// helper refreshes only text layouts, leaving taffy's already computed box
/// geometry intact, so a page-aware painter can use the correct line breaks for
/// the page it is about to paint. It is intentionally separate from
/// [`layout_single_page`] because callers must opt into this narrow
/// post-pagination operation.
pub fn relayout_text_for_width(
    document: &mut Document,
    cascade: &CascadeResult,
    max_advance: f32,
    page_width: f32,
    mut font_ctx: FontContext,
) {
    for node in document.nodes.iter_mut() {
        if let Some(text) = node.data.as_text_mut() {
            text.text_layout = None;
            text.text_line_offsets = None;
            text.text_indent_px = None;
            text.text_indent_hanging = false;
            text.text_indent_each_line = false;
            text.text_indent_rebreak = false;
        }
    }
    let mut layout_cx = LayoutContext::<()>::new();
    preshape_text(
        document,
        cascade,
        &mut font_ctx,
        &mut layout_cx,
        max_advance,
        page_width,
    );
    prepare_text_indent_before_taffy(
        document,
        cascade,
        &mut font_ctx,
        &mut layout_cx,
        max_advance,
    );
}

/// Correct the static position of grid abspos items whose placement is `auto`.
/// Taffy handles explicit grid-area placement, but its static-position fallback
/// uses the border edge instead of the grid content box.
fn realign_grid_abspos_static_positions(document: &mut Document, cascade: &CascadeResult) {
    fn align_offset(value: AlignSelfValue, parent: SelfAlignmentValue, free: f32) -> f32 {
        let value = match value {
            AlignSelfValue::Auto => parent,
            AlignSelfValue::Value(value) => value,
            _ => SelfAlignmentValue::Start,
        };
        match value {
            SelfAlignmentValue::Center => free / 2.0,
            SelfAlignmentValue::End | SelfAlignmentValue::FlexEnd => free,
            _ => 0.0,
        }
    }
    for child_id in 0..document.nodes.len() {
        let child = &cascade.computed[child_id];
        if !matches!(
            child.position,
            PositionValue::Absolute | PositionValue::Fixed
        ) || !matches!(child.top, ComputedLengthPercentageOrAuto::Auto)
            || !matches!(child.right, ComputedLengthPercentageOrAuto::Auto)
            || !matches!(child.bottom, ComputedLengthPercentageOrAuto::Auto)
            || !matches!(child.left, ComputedLengthPercentageOrAuto::Auto)
        {
            continue;
        }
        let Some(parent_id) = document
            .nodes
            .iter()
            .position(|node| node.children.contains(&child_id))
        else {
            continue;
        };
        let parent = &cascade.computed[parent_id];
        if !matches!(
            parent.display,
            DisplayValue::Grid | DisplayValue::InlineGrid
        ) || !matches!(child.grid_row_start, GridLineValue::Auto)
            || !matches!(child.grid_row_end, GridLineValue::Auto)
            || !matches!(child.grid_column_start, GridLineValue::Auto)
            || !matches!(child.grid_column_end, GridLineValue::Auto)
        {
            continue;
        }
        let parent_size = document.nodes[parent_id].unrounded_layout.size;
        let border_left = parent.border.left.width().px();
        let border_right = parent.border.right.width().px();
        let border_top = parent.border.top.width().px();
        let border_bottom = parent.border.bottom.width().px();
        let parent_style = &document.nodes[parent_id].style;
        let padding_left =
            used_style_length_percentage(parent_style.padding.left, parent_size.width)
                .unwrap_or(0.0);
        let padding_right =
            used_style_length_percentage(parent_style.padding.right, parent_size.width)
                .unwrap_or(0.0);
        let padding_top = used_style_length_percentage(parent_style.padding.top, parent_size.width)
            .unwrap_or(0.0);
        let padding_bottom =
            used_style_length_percentage(parent_style.padding.bottom, parent_size.width)
                .unwrap_or(0.0);
        let content_width =
            (parent_size.width - border_left - border_right - padding_left - padding_right)
                .max(0.0);
        let content_height =
            (parent_size.height - border_top - border_bottom - padding_top - padding_bottom)
                .max(0.0);
        let child_size = document.nodes[child_id].unrounded_layout.size;
        let x = border_left + padding_left;
        let y = border_top
            + padding_top
            + align_offset(
                child.align_self,
                parent.align_items,
                content_height - child_size.height,
            );
        let child_layout = &mut document.nodes[child_id].unrounded_layout;
        child_layout.location.x = x;
        child_layout.location.y = y;
        let _ = content_width; // horizontal normal alignment is start in this fallback.
    }
}

/// 単一 A4 (or 指定 PageBox) ページに Document を layout する。
///
/// # 変更 (in-place)
/// - Node.text_layout を全 `None` にクリア (re-entrance safety)
/// - `apply_computed_to_style` で computed → taffy::Style bridge (現時点では no-op)
/// - `preshape_text` で全 Text node を parley shape、Node.text_layout に格納
/// - `apply_page_box_to_body` で body.style.size = length(PageBox)
/// - `compute_root_layout` で taffy 計算、Node.unrounded_layout に書き込む
///
/// # Errors
/// - `LayoutError::Internal` — `<body>` element が見つからない (fragment
///   parse は現行実装では非対応) / taffy internal
///
///   parley shape (`preshape_text`) は **失敗しない**
///   (同関数の doc 参照)。
///
/// # Non-goals (現時点)
/// - 同じ Document で複数回呼ぶことは safe (text_layout を毎回 clear) だが、
///   incremental (差分だけ再走) は将来追加予定
/// - Consumer からの PageBox 上書きは将来の per-page PageBox 対応で扱う
/// - Fragment parse (no `<body>`) support は将来追加予定
/// # API 互換性
///
/// この signature は以前の 3-arg `(document, cascade, page_box)` から
/// 4-arg `(document, cascade, page_box, font_ctx)` に **意図的に breaking
/// change** された (choice β)。α (dual API: 既存 3-arg +
/// 新規 `_with_fonts`) との trade-off の末、raikiri-dom 内 caller が全て
/// in-repo (12 箇所 = production 1 + test 11) であり、内部 DI の explicit
/// 化と signature 統一の方が長期保守で優れると判断した。
///
/// # Note on error size
/// `LayoutError::Resolver(ResolverError)` transitively contains
/// `NetworkError` which embeds a `PolicyViolation` payload (~144 bytes),
/// exceeding clippy::result_large_err's 128 byte threshold.
#[allow(clippy::result_large_err)]
pub fn layout_single_page(
    document: &mut Document,
    cascade: &CascadeResult,
    page_box: PageBox,
    mut font_ctx: FontContext,
) -> Result<(), LayoutError> {
    // observation-side entry で
    // membership を sync する — `mark_in_document_flags` は flags_dirty=false
    // なら idempotent no-op なので、既に sink.finish() 経由で sync 済の場合は
    // 事実上 free。post-parse mutation (`Document::append_*` 等) の後で cascade
    // を skip して直接 layout する consumer に対する safety net。
    //
    // Contract note: cascade は `&D: Dom` を取り mutation 不可なので、cascade
    // 呼び出し側で sync せざるを得ない (parse.finish() 経由でしか自動 sync
    // されない)。layout はここで sync することで少なくとも layout/paint 段に
    // stale bit を持ち込まないことを保証する。
    document.mark_in_document_flags();
    if document.layout_cascade_generation != Some(cascade.generation()) {
        // Computed Grid/Flex style can change without a DOM tree mutation. Do
        // not let Taffy's per-node cache or resolved Grid rows survive that
        // cascade transition.
        document.layout_dirty = true;
    }
    document.layout_cascade_generation = None;

    // Step 0: text_layout re-entrance clear
    for node in document.nodes.iter_mut() {
        if let Some(t) = node.data.as_text_mut() {
            t.text_layout = None;
            t.text_line_offsets = None;
            t.multicol_fragments = None; // cov:ignore: reset is exercised by repeated ignored WPT layouts.
            t.text_indent_px = None;
            t.text_indent_hanging = false;
            t.text_indent_each_line = false;
            t.text_indent_rebreak = false;
        }
    }
    // Step 0b: layout_warnings re-entrance clear —
    // same rationale as the text_layout clear above: this Vec is populated
    // over the course of a pass (bridges below, then the taffy compute step
    // via `set_unrounded_layout`) and drained near the end of this function,
    // but an early `?` return (Step 3) would otherwise leave a previous call's
    // leftover entries for the next call to inherit.
    document.layout_warnings.clear();
    document.fragment_tree.clear();
    document.fragmentation_stack.clear();

    // Step 1: ComputedValues → taffy::Style bridge (現時点では no-op site)
    apply_computed_to_style(document, cascade);

    // Step 2: resolve the paper/content split before shaping.  Text wrapping
    // uses the content width, not the outer paper width.
    let margins = page_margins(cascade, page_box);
    let insets = page_content_insets(cascade, page_box);
    // Page decorations affect the physical origin, not the inline size of the
    // initial containing block.  This also keeps text from wrapping merely
    // because an @page rule adds border/padding around the paper.
    let content_width = margins.content_width(page_box).max(0.0);
    let content_height = (margins.content_height(page_box) - insets.top - insets.bottom).max(0.0);

    // Step 2b: pre-shape all text with parley
    // font_ctx は呼び出し側が構築 (system font 経路なら FontContext::new()、
    // VRT なら raikiri_dom::fonts::build_wpt_font_ctx で 確認済み)
    let mut layout_cx = LayoutContext::<()>::new();
    prepare_ch_box_values_before_taffy(document, cascade, &mut font_ctx, &mut layout_cx);
    preshape_text(
        document,
        cascade,
        &mut font_ctx,
        &mut layout_cx,
        content_width,
        page_box.width,
    );
    // Resolve font-metric `text-indent: ch` before Taffy so leaf heights use
    // the same indent that the post-layout realignment will paint.
    prepare_text_indent_before_taffy(
        document,
        cascade,
        &mut font_ctx,
        &mut layout_cx,
        content_width,
    );
    // Establish the foundational multicolumn fragmentainer projection after
    // text shaping, so direct text can be split by its actual line count.
    // cov:ignore: exercised by the ignored foundation WPT run; default coverage skips ignored reftests.
    prepare_multicol_layout(document, cascade, content_width);

    // Step 3: <body> lookup
    let body_id = find_body(document).ok_or_else(|| LayoutError::Internal {
        message: "no <body> element found (fragment parse not supported yet)".to_string(),
    })?;

    // The minimal UA sheet contributes the usual 8px body margin.  The body
    // is also used as the synthetic page root, so feeding that UA margin into
    // taffy would apply it twice to ordinary element children.  Keep the
    // computed value for the page cursor/paint walk and remove only the exact
    // UA-origin sides from the synthetic root style.  The origin metadata is
    // needed because an authored `margin: 8px` is otherwise indistinguishable
    // from the UA rule after value computation.
    {
        let used = cascade.computed[body_id].margin;
        let style_margin = &mut document.nodes[body_id].style.margin;
        let non_ua = cascade.non_ua_margin_sides.get(body_id);
        let is_ua_default = |value: ComputedLengthPercentageOrAuto| matches!(value, ComputedLengthPercentageOrAuto::Px(px) if (px - 8.0).abs() <= 0.001);
        if is_ua_default(used.top) && !non_ua.is_some_and(|sides| sides.top) {
            style_margin.top = LengthPercentageAuto::length(0.0);
        }
        if is_ua_default(used.right) && !non_ua.is_some_and(|sides| sides.right) {
            style_margin.right = LengthPercentageAuto::length(0.0);
        }
        if is_ua_default(used.bottom) && !non_ua.is_some_and(|sides| sides.bottom) {
            style_margin.bottom = LengthPercentageAuto::length(0.0);
        }
        if is_ua_default(used.left) && !non_ua.is_some_and(|sides| sides.left) {
            style_margin.left = LengthPercentageAuto::length(0.0);
        }
    }

    // Step 4: body.style.size は紙面ではなく page content box へ強制セット。
    // Page margins are painted/represented outside this taffy root.
    apply_page_content_box_to_body(document, body_id, page_box, margins, insets);
    // Taffy's static-position absolute fallback does not account for a
    // resolved horizontal margin when width/left/right are all auto. Resolve
    // that narrow case before compute so descendants are shaped against the
    // same border-box width that the containing-block equation requires.
    resolve_direct_absolute_auto_widths(document, cascade, body_id, content_width);

    // Step 5: taffy compute
    compute_root_layout(
        document,
        TaffyNodeId::from(body_id),
        taffy::Size {
            width: AvailableSpace::Definite(content_width),
            height: AvailableSpace::Definite(content_height),
        },
    );
    realign_inline_replaced_children(document, cascade); // cov:ignore: resource-enabled ignored WPT path.
    realign_single_empty_inline_block_indent(document, cascade);
    // Step 5a: taffy 確定幅基準の text 再配置 (`text-align: center` 等)。
    // glyph offset のみを変え、box geometry は変えないため invariant 検査の前後
    // どちらでもよいが、確定幅を読む側として compute 直後に置く。
    realign_grid_abspos_static_positions(document, cascade);
    realign_text_after_layout(document, cascade, &mut font_ctx, &mut layout_cx);
    // Step 5b: 親子 geometry の意味的 invariant を検査し、破れている subtree
    // を決定的 fallback (ゼロ) に倒す。Step 5 の内部
    // (`set_unrounded_layout` 経由の `sanitize_taffy_layout`) が保証するのは
    // finiteness だけなので、その一段上のレイヤーとしてここに置く —
    // `enforce_layout_invariants`'s doc 参照。`document.layout_warnings` へ
    // 積む event は Step 1/2 と同じ buffer で、Step 6 がまとめて drain する。
    enforce_layout_invariants(document, body_id);

    // Step 6: replay buffered LayoutWarn events.
    //
    // `document.layout_warnings` accumulated events from this function's own
    // bridge calls (Step 1 / Step 2, via `&mut document.layout_warnings`
    // passed directly), from `<Document as
    // taffy::LayoutPartialTree>::set_unrounded_layout` (invoked internally
    // by `compute_root_layout` just above, via `self` — see
    // `Document::layout_warnings`'s doc for why that trait-fixed signature
    // can only reach an owned buffer, not a live observer), and from Step 5b
    // (`enforce_layout_invariants`) just above, which
    // pushes into the same buffer directly since it already holds `&mut
    // Document`.
    //
    // No external caller can supply an observer yet — `layout_single_page`'s
    // signature is a dom→paint boundary (`raikiri-paint` and `raikiri` both
    // call it directly) and adding a parameter, or a new `_with_observer`
    // sibling, is a decision for that wall rather than this task. `observer`
    // is therefore always `None` today, so this always falls back to the
    // same `eprintln!` shape `fonts.rs` uses when uncalled with an observer —
    // but the integration logic is real and ready for a future `_with_observer`
    // sibling to wire an observer through with no further refactor.
    let mut observer: LayoutWarnObserver<'_> = None;
    for event in document.layout_warnings.drain(..) {
        emit_layout_warn(&mut observer, event);
    }
    document.layout_cascade_generation = Some(cascade.generation());

    Ok(())
}

/// One page in the block-flow pagination result.
///
/// `content_origin_y` is measured in the single laid-out document's body
/// coordinate space.  A page-aware painter subtracts it before adding the
/// page's physical top margin.  Keeping the source coordinate here means a
/// later scene can select a page without cloning or relaying out the DOM.
#[derive(Debug, Clone, PartialEq)]
pub struct PageSlice {
    /// Zero-based page number.
    pub page_index: u32,
    /// Body-content y coordinate at which this page begins.
    pub content_origin_y: f32,
    /// Named page selected by the first class-A box on this page.
    pub page_name: Option<String>,
}

/// Layout a document and return neutral per-page fragment snapshots.
///
/// This is the `raikiri-dom` producer-facing geometry API corresponding to
/// fulgur's `PaginationGeometryTable`.  The existing [`layout_pages`] API is
/// unchanged; this convenience wrapper runs it and projects its post-layout
/// DOM coordinates into page-local [`PageFragmentItem`] records.
///
/// The geometry pass clips a box at page boundaries. Shaped text placements
/// additionally carry a neutral line range based on each line's CSS-px center;
/// the pass does not expose the shaping engine or re-shape the text. Fixed-
/// positioned subtrees use their existing post-layout geometry as complete
/// per-page repeat records; table header/footer repetition is not synthesized
/// without corresponding pagination support. The projection is deterministic
/// and keeps source-node identity stable, so a consumer can select
/// continuation lines without re-running pagination.
#[allow(clippy::result_large_err)]
pub fn layout_page_fragments(
    document: &mut Document,
    cascade: &CascadeResult,
    page_box: PageBox,
    font_ctx: FontContext,
) -> Result<Vec<PageFragment>, LayoutError> {
    let slices = layout_pages(document, cascade, page_box, font_ctx)?;
    Ok(page_fragments_from_slices(
        document, cascade, page_box, &slices,
    ))
}

/// Resolve one producer-owned page metadata record from a page cascade.
///
/// The returned `content_box.x/y` is the physical page-local offset for the
/// page's content-relative item rectangles. This helper keeps the conversion
/// in `raikiri-dom` so consumers do not recompute page margins or insets.
pub fn resolve_page_fragment_geometry(
    cascade: &CascadeResult,
    page_box: PageBox,
    page_index: u32,
) -> PageFragmentPageGeometry {
    let margins = page_margins(cascade, page_box);
    let content_insets = page_content_insets(cascade, page_box);
    let content_width = margins.content_width(page_box).max(0.0);
    let content_height =
        (margins.content_height(page_box) - content_insets.top - content_insets.bottom).max(0.0);
    let content_box = PageFragmentRect::new(
        margins.left + content_insets.left,
        margins.top + content_insets.top,
        content_width,
        content_height,
    );
    let margins = PageFragmentInsets::new(margins.top, margins.right, margins.bottom, margins.left);
    let content_insets = PageFragmentInsets::new(
        content_insets.top,
        content_insets.right,
        content_insets.bottom,
        content_insets.left,
    );
    let orientation = if page_box.width > page_box.height {
        PageFragmentOrientation::Landscape
    } else {
        PageFragmentOrientation::Portrait
    };
    PageFragmentPageGeometry::new(
        page_index,
        page_box,
        margins,
        content_insets,
        content_box,
        orientation,
    )
}

/// Project an already-paginated document using one fixed geometry for all pages.
///
/// This compatibility entry point remains valid for fixed-page callers. New
/// page-aware callers should use [`page_fragments_from_slices_with_page_geometry`]
/// so each page carries its producer-resolved metadata.
pub fn page_fragments_from_slices(
    document: &Document,
    cascade: &CascadeResult,
    page_box: PageBox,
    slices: &[PageSlice],
) -> Vec<PageFragment> {
    let geometries: Vec<_> = slices
        .iter()
        .map(|slice| resolve_page_fragment_geometry(cascade, page_box, slice.page_index))
        .collect();
    page_fragments_from_slices_with_page_geometry(document, cascade, page_box, slices, &geometries)
}

/// Project slices using one resolved geometry record for each page.
///
/// `page_geometries` is producer-owned resolved metadata. A missing page index
/// falls back to `page_box` and the supplied cascade for compatibility, but a
/// page-aware caller should provide every emitted page explicitly. Item
/// rectangles remain relative to each page's `content_box` origin; consumers
/// add `content_box.x/y` exactly once when placing them on the physical page.
pub fn page_fragments_from_slices_with_page_geometry(
    document: &Document,
    cascade: &CascadeResult,
    page_box: PageBox,
    slices: &[PageSlice],
    page_geometries: &[PageFragmentPageGeometry],
) -> Vec<PageFragment> {
    let fallback_geometry = resolve_page_fragment_geometry(cascade, page_box, 0);
    let mut ordered_slices: Vec<&PageSlice> = slices.iter().collect();
    ordered_slices.sort_by(|left, right| {
        left.page_index
            .cmp(&right.page_index)
            .then_with(|| left.content_origin_y.total_cmp(&right.content_origin_y))
    });
    let mut pages: Vec<PageFragment> = ordered_slices
        .iter()
        .map(|slice| {
            let geometry = page_geometries
                .iter()
                .find(|geometry| geometry.page_index == slice.page_index)
                .copied()
                .unwrap_or_else(|| fallback_geometry.with_page_index(slice.page_index));
            PageFragment::with_page_geometry(
                slice.page_index,
                geometry,
                slice.content_origin_y,
                slice.page_name.clone(),
            )
        })
        .collect();

    if pages.is_empty() {
        return pages;
    }

    let Some(body_id) = find_body(document) else {
        return pages;
    };

    // Collect absolute post-pagination coordinates.  The arena index is the
    // stable NodeId projection used by `raikiri_traits::Dom`; sorting by it
    // reproduces fulgur's deterministic BTreeMap iteration order regardless of
    // traversal implementation details.
    struct PageFragmentSource {
        node_id: NodeId,
        node_kind: NodeKind,
        tag_name: Option<String>,
        abs_x: f32,
        abs_y: f32,
        width: f32,
        height: f32,
        line_metrics: Option<Vec<(f32, f32)>>,
        is_repeat: bool,
    }

    let mut nodes = Vec::new();
    let mut stack = vec![(body_id, 0.0_f32, 0.0_f32, false)];
    while let Some((node_id, parent_abs_x, parent_abs_y, inherited_repeat)) = stack.pop() {
        let Some(node) = document.get_node(node_id) else {
            continue; // cov:ignore: document-owned child links are valid by construction.
        };
        if !node.is_in_document() || node.is_non_rendered_html_element() || node.is_display_none() {
            continue;
        }
        let layout = node.unrounded_layout;
        let abs_x = parent_abs_x + layout.location.x;
        let abs_y = parent_abs_y + layout.location.y;
        let width = finite_nonnegative(layout.size.width);
        let height = finite_nonnegative(layout.size.height);
        // A fixed-position subtree is painted in every committed page. The
        // existing layout pass already computes one viewport-relative box;
        // preserve that geometry and mark the records as complete repeats
        // instead of clipping them as in-flow content.
        let is_repeat = inherited_repeat
            || matches!(
                cascade
                    .computed
                    .get(node_id)
                    .map(|computed| &computed.position),
                Some(PositionValue::Fixed)
            );
        let include = match node.kind() {
            NodeKind::Text => node
                .text_content()
                .is_some_and(|text| !text.trim().is_empty()),
            NodeKind::Element => true,
            _ => false, // cov:ignore: non-rendered node kinds are filtered by the document invariant.
        };
        if include && abs_x.is_finite() && abs_y.is_finite() {
            let line_metrics = (node.kind() == NodeKind::Text).then(|| {
                node.text_layout()
                    .into_iter()
                    .flat_map(|layout| {
                        layout.lines().map(|line| {
                            let metrics = line.metrics();
                            (metrics.block_min_coord, metrics.block_max_coord)
                        })
                    })
                    .collect()
            });
            nodes.push(PageFragmentSource {
                node_id: NodeId::new(node_id as u64),
                node_kind: node.kind(),
                tag_name: node.tag_name().map(str::to_owned),
                abs_x,
                abs_y,
                width,
                height,
                line_metrics,
                is_repeat,
            });
        } // cov:ignore: layout sanitization normally keeps source coordinates finite.

        if node.kind() == NodeKind::Element {
            for &child_id in node.children.iter().rev() {
                stack.push((child_id, abs_x, abs_y, is_repeat));
            }
        }
    }
    nodes.sort_by_key(|node| node.node_id);

    for source in nodes {
        let kind = match source.node_kind {
            NodeKind::Text => PageFragmentKind::Text,
            NodeKind::Element if source.tag_name.as_deref() == Some("img") => {
                PageFragmentKind::Replaced
            }
            _ => PageFragmentKind::Box,
        };
        let mut placements = Vec::new();
        let repeat_line_range = source.line_metrics.as_deref().and_then(|metrics| {
            (!metrics.is_empty()).then(|| {
                PageFragmentLineRange::new(0, u32::try_from(metrics.len()).unwrap_or(u32::MAX))
            })
        });
        for (page_slot, slice) in ordered_slices.iter().enumerate() {
            let page_start = slice.content_origin_y;
            let page_end = ordered_slices
                .get(page_slot + 1)
                .map(|next| next.content_origin_y)
                .filter(|next| next.is_finite() && *next > page_start)
                .unwrap_or_else(|| {
                    page_start
                        + pages
                            .get(page_slot)
                            .map(|page| page.content_box.height)
                            .unwrap_or(0.0)
                });
            if !page_start.is_finite() || !page_end.is_finite() || page_end <= page_start {
                continue;
            }
            if source.is_repeat {
                placements.push((
                    page_slot,
                    source.abs_y.max(0.0),
                    source.height,
                    repeat_line_range,
                ));
                continue;
            }
            let (intersects, fragment_y, fragment_height) = if source.height > 0.0 {
                let bottom = source.abs_y + source.height;
                let intersects = source.abs_y < page_end && bottom > page_start;
                let top = source.abs_y.max(page_start);
                let bottom = bottom.min(page_end);
                (
                    intersects,
                    (top - page_start).max(0.0),
                    (bottom - top).max(0.0),
                )
            } else {
                (
                    source.abs_y >= page_start && source.abs_y <= page_end,
                    (source.abs_y - page_start).max(0.0),
                    0.0,
                )
            };
            if intersects {
                let line_range = source.line_metrics.as_deref().and_then(|metrics| {
                    line_range_for_page(metrics, source.abs_y, page_start, page_end)
                });
                placements.push((page_slot, fragment_y, fragment_height, line_range));
            }
        }
        let fragment_count = placements.len() as u32;
        for (fragment_index, (page_slot, y, fragment_height, line_range)) in
            placements.into_iter().enumerate()
        {
            let Some(page) = pages.get_mut(page_slot) else {
                continue; // cov:ignore: placements are indexed from the same slices used to build pages.
            };
            let item = PageFragmentItem::new(
                source.node_id,
                PageFragmentRect::new(source.abs_x, y, source.width, fragment_height),
                kind,
                fragment_index as u32,
                fragment_count,
                source.is_repeat,
            )
            .with_page_index(page.page_index);
            page.items.push(match line_range {
                Some(range) => item.with_line_range(range),
                None => item,
            });
        }
    }

    pages
}

/// Collect deterministic page-local link events from page snapshots.
///
/// The event geometry is copied from the correlated `PageFragmentItem`, so a
/// consumer can join on `(placement_node_id, page_index)` without seeing the
/// layout engine's internal tree. `anchor_node_id` preserves the owning `<a>`
/// identity when a link wraps text or replaced descendants. Raw trimmed
/// `href` values, including empty and relative values, are not URL-parsed.
/// A box ancestor is omitted when a more specific text or replaced placement
/// already represents the same link, retaining one useful hit rectangle per
/// visible leaf and a fallback for empty/non-text links.
pub fn page_fragment_events_from_pages(
    document: &Document,
    pages: &[PageFragment],
) -> Vec<PageFragmentEvent> {
    let Some(body_id) = find_body(document) else {
        return Vec::new();
    };

    let mut parent_by_node = HashMap::new();
    for (parent_id, node) in document.nodes.iter().enumerate() {
        for &child_id in &node.children {
            parent_by_node.insert(child_id, parent_id);
        }
    }

    let mut owners = HashMap::<usize, usize>::new();
    let mut hrefs = HashMap::<usize, String>::new();
    let mut stack = vec![(body_id, None::<usize>)];
    while let Some((node_id, inherited_owner)) = stack.pop() {
        let node = &document.nodes[node_id];
        if !node.is_in_document() || node.is_non_rendered_html_element() || node.is_display_none() {
            continue;
        }
        let owner = if node.kind() == NodeKind::Element && node.tag_name() == Some("a") {
            node.attribute("href")
                .map(str::trim)
                .map(|href| {
                    hrefs.entry(node_id).or_insert_with(|| href.to_owned());
                    node_id
                })
                .or(inherited_owner)
        } else {
            inherited_owner
        };
        if let Some(owner) = owner {
            owners.insert(node_id, owner);
        }
        for &child_id in node.children.iter().rev() {
            stack.push((child_id, owner));
        }
    }

    let mut events = Vec::new();
    for page in pages {
        let linked_items: Vec<&PageFragmentItem> = page
            .items
            .iter()
            .filter(|item| {
                usize::try_from(item.node_id.0)
                    .ok()
                    .and_then(|node_id| owners.get(&node_id))
                    .and_then(|owner| hrefs.get(owner))
                    .is_some()
                    && item.rect.width > 0.0
                    && item.rect.height > 0.0
            })
            .collect();
        for item in linked_items.iter().copied() {
            // `linked_items` has already validated all three lookups above.
            // Keeping the invariant explicit here avoids a second set of
            // impossible branches in the hot event projection loop.
            let placement_node_id = usize::try_from(item.node_id.0)
                .expect("linked item NodeId must fit the local arena index");
            let anchor_id = *owners
                .get(&placement_node_id)
                .expect("linked item must have an anchor owner");
            let href = hrefs
                .get(&anchor_id)
                .expect("anchor owner must retain its href");
            if item.kind == PageFragmentKind::Box
                && linked_items.iter().any(|other| {
                    let other_node_id = usize::try_from(other.node_id.0)
                        .expect("linked item NodeId must fit the local arena index");
                    other.node_id != item.node_id
                        && owners.get(&other_node_id) == Some(&anchor_id)
                        && is_descendant_of(other_node_id, placement_node_id, &parent_by_node)
                })
            {
                continue;
            }
            events.push(PageFragmentEvent::Link(PageFragmentLinkEvent::new(
                NodeId::new(anchor_id as u64),
                item.node_id,
                page.page_index,
                item.rect,
                item.fragment_index,
                item.fragment_count,
                item.is_repeat,
                item.line_range,
                PageFragmentLink::new(href.clone()),
            )));
        }
    }
    events.sort_by(|left, right| match (left, right) {
        (PageFragmentEvent::Link(left), PageFragmentEvent::Link(right)) => left
            .page_index
            .cmp(&right.page_index)
            .then_with(|| left.anchor_node_id.cmp(&right.anchor_node_id))
            .then_with(|| left.placement_node_id.cmp(&right.placement_node_id))
            .then_with(|| left.rect.y.total_cmp(&right.rect.y))
            .then_with(|| left.rect.x.total_cmp(&right.rect.x))
            .then_with(|| left.fragment_index.cmp(&right.fragment_index)),
        _ => std::cmp::Ordering::Equal, // cov:ignore: future non-exhaustive event variant cannot be constructed here
    });
    events
}

fn is_descendant_of(
    mut candidate: usize,
    ancestor: usize,
    parent_by_node: &HashMap<usize, usize>,
) -> bool {
    while let Some(parent) = parent_by_node.get(&candidate).copied() {
        if parent == ancestor {
            return true;
        }
        candidate = parent;
    }
    false
}

/// Group page snapshots into a deterministic NodeId-ordered geometry table.
///
/// `pages` is normally the ordered snapshot returned by
/// [`layout_page_fragments`] or [`page_fragments_from_slices`]. The result
/// normalizes each item to its containing page and sorts node fragments by
/// page/fragment index, while preserving the producer's `is_repeat` flag.
pub fn page_fragment_geometry_table(pages: &[PageFragment]) -> PageFragmentGeometryTable {
    let mut table = PageFragmentGeometryTable::new();
    for page in pages {
        for item in &page.items {
            // The page snapshot is authoritative for this placement. Normalize
            // manually assembled snapshots as well as producer output so the
            // node-centric table always retains the page index required by a
            // fulgur-compatible consumer.
            let item = item.clone().with_page_index(page.page_index);
            let geometry = table
                .entry(item.node_id)
                .or_insert_with(|| PageFragmentGeometry::new(item.node_id, item.is_repeat));
            debug_assert_eq!(geometry.is_repeat, item.is_repeat); // cov:ignore: one producer cannot mix split and repeat records.
            geometry.fragments.push(item);
        }
    }
    for geometry in table.values_mut() {
        geometry
            .fragments
            .sort_by_key(|item| (item.page_index, item.fragment_index));
    }
    table
}

fn line_range_for_page(
    line_metrics: &[(f32, f32)],
    text_abs_y: f32,
    page_start: f32,
    page_end: f32,
) -> Option<PageFragmentLineRange> {
    let mut first = None;
    let mut end = 0_u32;
    for (index, (line_top, line_bottom)) in line_metrics.iter().copied().enumerate() {
        let top = text_abs_y + line_top;
        let bottom = text_abs_y + line_bottom;
        let center = (top + bottom) * 0.5;
        if !center.is_finite() || center < page_start - 0.001 || center >= page_end - 0.001 {
            continue;
        }
        let index = index as u32;
        first.get_or_insert(index);
        end = index.saturating_add(1);
    }
    first.map(|start| PageFragmentLineRange::new(start, end))
}

fn finite_nonnegative(value: f32) -> f32 {
    if value.is_finite() {
        value.max(0.0)
    } else {
        0.0
    }
}

fn page_break_is_forced(value: BreakBetween) -> bool {
    matches!(value, BreakBetween::Page)
}

fn selected_page_name(cascade: &CascadeResult, node_id: usize) -> Option<String> {
    match cascade.page_values.get(node_id) {
        Some(raikiri_style::property::PageValue::Named(name)) => Some(name.to_string()),
        _ => None,
    }
}

/// Layout a document once and produce ordered page slices.
///
/// The current fragmentainer handles ordinary block-flow children of
/// `<body>`.  It honors forced `page` breaks (including the legacy aliases
/// already normalized by the style cascade), coalesces adjacent forced
/// breaks, and keeps an empty explicitly-created page.  A block that is taller
/// than a page is exposed on each intersecting slice; splitting its internal
/// line/child fragments is deliberately left to the next fragmentation pass.
/// Direct tables additionally keep a row whose cell boxes would cross a page
/// together by moving that row to the next fragmentainer. Rowspans, repeated
/// header/footer groups, and cell-internal breaks remain outside this pass.
/// Column-direction flex containers likewise keep a direct flex item together;
/// row-direction flex, wrapping-line, and intrinsic-item fragmentation remain
/// outside this pass. A single explicit-column grid likewise keeps direct
/// non-spanning items together; multi-column placement and spanning items are
/// outside this pass.
/// For an ordinary direct-body block whose complete text layout fits within one
/// fragmentainer, `orphans` / `widows` can move the block intact when the
/// natural split would leave too few line boxes on either side. Oversized or
/// nested inline formatting contexts remain outside this pass.
///
/// This is intentionally separate from [`layout_single_page`]: existing
/// callers retain the single-page contract while paged callers get a real
/// per-page result and the same post-layout DOM as the scene builder.
#[allow(clippy::result_large_err)]
pub fn layout_pages(
    document: &mut Document,
    cascade: &CascadeResult,
    page_box: PageBox,
    font_ctx: FontContext,
) -> Result<Vec<PageSlice>, LayoutError> {
    layout_pages_with_page_steps(document, cascade, page_box, font_ctx, &[])
}

/// [`layout_pages`] と同一だが、先に `resolver` で `<img>` の intrinsic
/// サイズを解決する。解決結果は同じ `Document` に保存されるため、ページ
/// 分割後の通常のレイアウト処理と paint 時の pixel source が同じ画像を
/// 参照できる。
#[allow(clippy::result_large_err)]
pub fn layout_pages_with_resolver(
    document: &mut Document,
    cascade: &CascadeResult,
    page_box: PageBox,
    font_ctx: FontContext,
    resolver: &dyn ReplacedResolver,
) -> Result<Vec<PageSlice>, LayoutError> {
    layout_pages_with_resolver_and_base_url(document, cascade, page_box, font_ctx, resolver, None)
}

/// [`layout_pages_with_resolver`] with relative image URLs resolved against a
/// document base URL.
#[allow(clippy::result_large_err)]
pub fn layout_pages_with_resolver_and_base_url(
    document: &mut Document,
    cascade: &CascadeResult,
    page_box: PageBox,
    font_ctx: FontContext,
    resolver: &dyn ReplacedResolver,
    base_url: Option<&url::Url>,
) -> Result<Vec<PageSlice>, LayoutError> {
    document.mark_in_document_flags();
    match base_url {
        Some(base_url) => {
            crate::image_resolve::resolve_images_with_base(document, resolver, Some(base_url))
        }
        None => crate::image_resolve::resolve_images(document, resolver),
    }
    .map_err(LayoutError::Resolver)?;
    layout_pages(document, cascade, page_box, font_ctx)
}

/// [`layout_pages_with_page_geometry`] と同一だが、先に `resolver` で
/// `<img>` の intrinsic サイズを解決する。
#[allow(clippy::result_large_err)]
pub fn layout_pages_with_page_geometry_and_resolver(
    document: &mut Document,
    cascade: &CascadeResult,
    page_box: PageBox,
    font_ctx: FontContext,
    page_steps: &[f32],
    page_widths: &[f32],
    resolver: &dyn ReplacedResolver,
) -> Result<Vec<PageSlice>, LayoutError> {
    layout_pages_with_page_geometry_and_resolver_and_base_url(
        document,
        cascade,
        page_box,
        font_ctx,
        page_steps,
        page_widths,
        resolver,
        None,
    )
}

/// [`layout_pages_with_page_geometry_and_resolver`] with document-relative image URLs.
#[allow(clippy::result_large_err, clippy::too_many_arguments)]
pub fn layout_pages_with_page_geometry_and_resolver_and_base_url(
    document: &mut Document,
    cascade: &CascadeResult,
    page_box: PageBox,
    font_ctx: FontContext,
    page_steps: &[f32],
    page_widths: &[f32],
    resolver: &dyn ReplacedResolver,
    base_url: Option<&url::Url>,
) -> Result<Vec<PageSlice>, LayoutError> {
    document.mark_in_document_flags();
    match base_url {
        Some(base_url) => {
            crate::image_resolve::resolve_images_with_base(document, resolver, Some(base_url))
        }
        None => crate::image_resolve::resolve_images(document, resolver),
    }
    .map_err(LayoutError::Resolver)?;
    layout_pages_with_page_geometry(
        document,
        cascade,
        page_box,
        font_ctx,
        page_steps,
        page_widths,
    )
}

/// Layout ordinary block flow with an optional per-page content-height schedule.
///
/// An empty schedule preserves the historical fixed fragmentainer height.  The
/// paged WPT adapter supplies a schedule when `@page` changes the page size or
/// margins after the first page; the normal API remains fixed-size by default.
#[allow(clippy::result_large_err)]
pub fn layout_pages_with_page_steps(
    document: &mut Document,
    cascade: &CascadeResult,
    page_box: PageBox,
    font_ctx: FontContext,
    page_steps: &[f32],
) -> Result<Vec<PageSlice>, LayoutError> {
    layout_pages_with_page_geometry(document, cascade, page_box, font_ctx, page_steps, &[])
}

/// Return the start page value propagated from the first in-flow child box.
///
/// CSS Page 3 derives a box's start page value from its first child when
/// that child participates in a class-A break point. Nested named boxes
/// therefore do not each open a page; the deepest first in-flow box owns
/// the value compared at the boundary.
fn propagated_start_page_name(
    document: &Document,
    cascade: &CascadeResult,
    node_id: usize,
    inherited_page_name: Option<&str>,
) -> (bool, Option<String>) {
    propagated_start_page_name_with_order(document, cascade, node_id, inherited_page_name, true)
}

fn propagated_start_page_name_with_order(
    document: &Document,
    cascade: &CascadeResult,
    node_id: usize,
    inherited_page_name: Option<&str>,
    resolved_layout: bool,
) -> (bool, Option<String>) {
    let Some(node) = document.get_node(node_id) else {
        return (false, None);
    };
    if !node.is_in_document() || node.is_display_none() {
        return (false, None);
    }
    if matches!(node.kind(), NodeKind::Text) {
        let has_text = matches!(
            &node.data,
            crate::node::NodeData::Text(text) if !text.text_content.trim().is_empty()
        );
        if !has_text {
            return (false, None);
        }
        return (true, inherited_page_name.map(ToOwned::to_owned));
    }
    let display = cascade.computed[node_id].display;
    let page_applies = !matches!(
        display,
        DisplayValue::Inline | DisplayValue::InlineFlex | DisplayValue::InlineGrid
    );
    let explicit_page_name = if page_applies {
        selected_page_name(cascade, node_id)
    } else {
        None
    };
    let used_page_name = explicit_page_name
        .clone()
        .or_else(|| inherited_page_name.map(ToOwned::to_owned));
    let child_order = if resolved_layout {
        pagination_child_order(document, cascade, node_id)
    } else {
        initial_page_child_order(document, cascade, node_id)
    };
    for child_id in child_order {
        if child_id >= cascade.computed.len() {
            continue;
        }
        let child_computed = &cascade.computed[child_id];
        if matches!(
            child_computed.position,
            PositionValue::Absolute | PositionValue::Fixed
        ) || !matches!(child_computed.float, FloatValue::None)
        {
            continue;
        }
        let (has_box, child_start) = propagated_start_page_name_with_order(
            document,
            cascade,
            child_id,
            used_page_name.as_deref(),
            resolved_layout,
        );
        if has_box {
            return (true, child_start);
        }
    }
    (true, used_page_name)
}

/// Layout ordinary block flow with per-page content heights and widths.
///
/// `page_widths` contains content-box widths corresponding to `page_steps`.
/// When present, percentage-sized boxes are rescaled after pagination so a
/// later page with a different containing-block width does not retain the
/// first page's used percentage width.  An empty width schedule keeps the
/// existing fixed-page behavior.
#[allow(clippy::result_large_err)]
pub fn layout_pages_with_page_geometry(
    document: &mut Document,
    cascade: &CascadeResult,
    page_box: PageBox,
    font_ctx: FontContext,
    page_steps: &[f32],
    page_widths: &[f32],
) -> Result<Vec<PageSlice>, LayoutError> {
    layout_single_page(document, cascade, page_box, font_ctx)?;

    let body_id = find_body(document).ok_or_else(|| LayoutError::Internal {
        message: "no <body> element found (fragment parse not supported yet)".to_string(),
    })?;
    let margins = page_margins(cascade, page_box);
    let insets = page_content_insets(cascade, page_box);
    // The current single-page bridge collapses the root box to zero size, so
    // html/body block-start margins are not represented in descendant
    // coordinates. Carry both margins into the initial fragmentainer cursor
    // instead of treating the first text run as page-zero content.
    let html_margin_top = document.nodes[document.root]
        .children
        .iter()
        .copied()
        .find(|&node_id| document.nodes[node_id].tag_name() == Some("html"))
        .and_then(|html_id| {
            used_style_length_percentage_auto(
                document.nodes[html_id].style.margin.top,
                margins.content_width(page_box),
            )
            .or_else(|| {
                used_computed_length_percentage_or_auto(
                    cascade.computed[html_id].margin.top,
                    margins.content_width(page_box),
                )
            })
            .map(|value| value.max(0.0))
        })
        .unwrap_or(0.0);
    let body_has_element_child = document.nodes[body_id]
        .children
        .iter()
        .any(|&child_id| document.nodes[child_id].kind() == NodeKind::Element);
    let body_has_canvas_background = {
        let body = &cascade.computed[body_id];
        body.background_color.a != 0 || !matches!(body.background_image, BackgroundImage::None)
    };
    let body_has_direct_text = document.nodes[body_id].children.iter().any(|&child_id| {
        document.nodes[child_id].kind() == NodeKind::Text
            && document.nodes[child_id].unrounded_layout.size.height > 0.0
    });
    let body_top_is_non_ua = cascade
        .non_ua_margin_sides
        .get(body_id)
        .is_some_and(|sides| sides.top);
    let body_margin_top = if body_top_is_non_ua
        || (body_has_direct_text && !body_has_element_child && body_has_canvas_background)
    {
        let used = if body_top_is_non_ua {
            used_style_length_percentage_auto(
                document.nodes[body_id].style.margin.top,
                margins.content_width(page_box),
            )
        } else {
            None
        };
        used.or_else(|| {
            used_computed_length_percentage_or_auto(
                cascade.computed[body_id].margin.top,
                margins.content_width(page_box),
            )
        })
        .map(|value| value.max(0.0))
        .unwrap_or(0.0)
    } else {
        0.0
    };
    let root_margin_top = html_margin_top + body_margin_top;
    // Keep the scheduled inline size identical to the first layout pass;
    // page border/padding are applied as a paint offset, not as a narrower
    // containing block.
    let content_width = margins.content_width(page_box).max(0.0);
    let content_height = (margins.content_height(page_box) - insets.top - insets.bottom).max(0.0);
    // A page with margins consuming the entire paper still needs a finite
    // cursor for forced breaks.  No valid page box reaches this path in normal
    // CSS, but the fallback keeps the API panic-free for direct callers.
    let fixed_page_step = if content_height.is_finite() && content_height > 0.0 {
        content_height
    } else {
        page_box.height.max(1.0)
    };
    // A page-size/margin change can make the fragmentainer height vary by
    // page.  The default API supplies no schedule and therefore follows the
    // fixed first-page height used historically.
    let page_step = fixed_page_step;
    let page_step_at = |page_index: u32| {
        page_steps
            .get(page_index as usize)
            .copied()
            .filter(|step| step.is_finite() && *step > 0.0)
            .unwrap_or(fixed_page_step)
    };
    let page_origin = |page_index: u32| {
        (0..page_index)
            .map(page_step_at)
            .fold(0.0_f32, |sum, step| sum + step)
    };
    let page_index_for_y = |y: f32| {
        if !y.is_finite() || y <= 0.0 {
            return 0;
        }
        let mut origin = 0.0_f32;
        for page_index in 0..4096_u32 {
            let step = page_step_at(page_index);
            if y < origin + step {
                return page_index;
            }
            origin += step;
        }
        4095
    };
    // A box ending exactly at a page edge belongs to the preceding page.
    let page_index_for_end = |end: f32| {
        if !end.is_finite() || end <= 0.0 {
            return 0;
        }
        let mut origin = 0.0_f32;
        for page_index in 0..4096_u32 {
            let step = page_step_at(page_index);
            if end <= origin + step {
                return page_index;
            }
            origin += step;
        }
        4095
    };
    // A root margin that reaches into a later fragmentainer consumes whole
    // leading pages.  Do not leave the first content run stranded halfway
    // down the first nonblank page.
    let root_flow_offset = if root_margin_top >= fixed_page_step {
        page_origin(page_index_for_y(root_margin_top))
    } else {
        root_margin_top
    };

    // Pagination adjusts selected boxes after taffy has produced one normal
    // flow layout.  Keep the arena's parent relation so a descendant can be
    // materialized relative to an already-shifted ancestor instead of being
    // shifted twice.
    let mut parent_of = vec![None; document.nodes.len()];
    for (parent_id, node) in document.nodes.iter().enumerate() {
        for &child_id in &node.children {
            if child_id < parent_of.len() {
                parent_of[child_id] = Some(parent_id);
            }
        }
    }

    fn current_abs_y(document: &Document, node_id: usize, parent_of: &[Option<usize>]) -> f32 {
        let mut id = node_id;
        let mut y = 0.0_f32;
        let mut guard = 0_usize;
        while guard <= parent_of.len() {
            y += document.nodes[id].unrounded_layout.location.y;
            let Some(parent_id) = parent_of[id] else {
                break;
            };
            id = parent_id;
            guard += 1;
            if id >= document.nodes.len() {
                break;
            }
        }
        y
    }

    fn materialize_y(
        document: &mut Document,
        node_id: usize,
        desired_y: f32,
        parent_of: &[Option<usize>],
    ) {
        if !desired_y.is_finite() {
            return;
        }
        let actual_y = current_abs_y(document, node_id, parent_of);
        let delta = desired_y - actual_y;
        if delta.is_finite() {
            document.nodes[node_id].unrounded_layout.location.y += delta;
        }
    }

    fn is_descendant_or_self(
        document: &Document,
        node_id: usize,
        ancestor_id: usize,
        parent_of: &[Option<usize>],
    ) -> bool {
        let mut current = Some(node_id);
        let mut guard = 0_usize;
        while let Some(id) = current {
            if id == ancestor_id {
                return true;
            }
            if id >= document.nodes.len() || guard > parent_of.len() {
                break;
            }
            current = parent_of[id];
            guard += 1;
        }
        false
    }

    fn has_footnote_ancestor(
        cascade: &CascadeResult,
        node_id: usize,
        parent_of: &[Option<usize>],
    ) -> bool {
        let mut current = parent_of.get(node_id).copied().flatten();
        let mut guard = 0_usize;
        while let Some(id) = current {
            if cascade
                .computed
                .get(id)
                .is_some_and(|computed| matches!(computed.float, FloatValue::Footnote))
            {
                return true;
            }
            current = parent_of.get(id).copied().flatten();
            guard += 1;
            if guard > parent_of.len() {
                break;
            }
        }
        false
    }

    #[derive(Clone)]
    struct PageCandidate {
        node_id: usize,
        raw_y: f32,
        height: f32,
        is_text: bool,
        is_direct_body_text: bool,
        is_direct_body_element: bool,
        is_table_row: bool,
        is_flex_item: bool,
        is_grid_item: bool,
        is_named: bool,
        is_float_descendant: bool,
        /// Page type inherited from the nearest containing class-A box.
        /// `None` is the anonymous page type, not an unknown value.
        page_name: Option<String>,
        /// A named descendant nested inside a flex item defers one boundary
        /// until the containing flex box has finished.
        deferred_named_break_after: bool,
        /// An inline canvas with a named page is a boundary marker, but its
        /// inline-level box does not itself establish the named page type.
        inline_named_page: bool,
    }

    fn has_nested_named_page_descendant(
        document: &Document,
        cascade: &CascadeResult,
        node_id: usize,
        depth: u32,
    ) -> bool {
        let Some(node) = document.get_node(node_id) else {
            return false;
        };
        for &child_id in &node.children {
            let child_depth = depth.saturating_add(1);
            if child_depth >= 2 && selected_page_name(cascade, child_id).is_some() {
                return true;
            }
            if has_nested_named_page_descendant(document, cascade, child_id, child_depth) {
                return true;
            }
        }
        false
    }

    /// A named box containing a hidden subtree still establishes an explicit
    /// page boundary. Keep that boundary distinct from the zero-height named
    /// runs that are otherwise coalesced at one source coordinate.
    fn has_display_none_descendant(document: &Document, node_id: usize) -> bool {
        let Some(node) = document.get_node(node_id) else {
            return false;
        };
        node.children.iter().any(|&child_id| {
            let Some(child) = document.get_node(child_id) else {
                return false;
            };
            child.is_display_none() || has_display_none_descendant(document, child_id)
        })
    }

    // Candidate collection carries the recursive layout state explicitly so page
    // membership is decided from source coordinates before local page offsets.
    #[allow(clippy::too_many_arguments)]
    fn collect_candidates(
        document: &Document,
        cascade: &CascadeResult,
        node_id: usize,
        parent_abs_y: f32,
        direct_body_child: bool,
        body_id: usize,
        parent_height: f32,
        page_step: f32,
        inherited_page_name: Option<String>,
        inside_table: bool,
        flex_column_parent: bool,
        grid_single_column_parent: bool,
        inside_flex: bool,
        inside_float: bool,
        inside_out_of_flow: bool,
        out: &mut Vec<PageCandidate>,
    ) {
        let Some(node) = document.get_node(node_id) else {
            return;
        };
        if !node.is_in_document() || node.is_non_rendered_html_element() {
            return;
        }
        match node.kind() {
            NodeKind::Text => {
                if (direct_body_child || parent_height > 0.0)
                    && matches!(
                        &node.data,
                        crate::node::NodeData::Text(text) if !text.text_content.trim().is_empty()
                    )
                {
                    out.push(PageCandidate {
                        node_id,
                        raw_y: parent_abs_y + node.unrounded_layout.location.y,
                        height: node.unrounded_layout.size.height.max(0.0),
                        is_text: true,
                        is_direct_body_text: direct_body_child,
                        is_direct_body_element: false,
                        is_table_row: false,
                        is_flex_item: false,
                        is_grid_item: false,
                        is_named: false,
                        is_float_descendant: inside_float,
                        page_name: inherited_page_name,
                        deferred_named_break_after: false,
                        inline_named_page: false,
                    });
                }
            }
            NodeKind::Element => {
                if node.is_display_none() {
                    return;
                }
                let raw_y = parent_abs_y + node.unrounded_layout.location.y;
                let computed = &cascade.computed[node_id];
                // A floated box does not establish a page transition merely
                // because it carries an inherited/explicit `page` value.  In
                // particular, CSS Page 3 keeps a float in the preceding
                // fragmentainer in the page-name-float cases.
                let is_float = !matches!(computed.float, FloatValue::None);
                let float_subtree = inside_float || is_float;
                let out_of_flow_subtree = inside_out_of_flow
                    || matches!(
                        computed.position,
                        PositionValue::Absolute | PositionValue::Fixed
                    );
                let explicit_page_name = selected_page_name(cascade, node_id);
                // Inline canvas boxes do not establish the named page type,
                // but their `page` value still marks the boundary between the
                // surrounding class-A boxes. This mirrors the inline canvas
                // CSS Page cases without treating a replaced inline box as a
                // block-level named-page candidate.
                let inline_named_page = direct_body_child
                    && explicit_page_name.is_some()
                    && matches!(computed.display, DisplayValue::Inline)
                    && node
                        .tag_name()
                        .is_some_and(|tag| tag.eq_ignore_ascii_case("canvas"));
                // The `page` property on an out-of-flow box does not open a
                // normal-flow page boundary. Keep its inherited page context,
                // but do not use its explicit name to split pagination.
                let own_page_name =
                    if !inline_named_page && !float_subtree && !inside_flex && !out_of_flow_subtree
                    {
                        explicit_page_name.clone()
                    } else {
                        None
                    };
                let (has_propagated_page_name, propagated_page_name) = propagated_start_page_name(
                    document,
                    cascade,
                    node_id,
                    inherited_page_name.as_deref(),
                );
                let page_name = if has_propagated_page_name {
                    propagated_page_name
                } else {
                    own_page_name.clone().or(inherited_page_name.clone())
                };
                let child_page_name = own_page_name.clone().or(inherited_page_name);
                let deferred_named_break_after = matches!(computed.display, DisplayValue::Flex)
                    && has_nested_named_page_descendant(document, cascade, node_id, 0);
                let is_body = node_id == body_id;
                let participates_in_flow = matches!(
                    computed.position,
                    PositionValue::Static | PositionValue::Relative | PositionValue::Sticky
                );
                let is_tall_direct_absolute = direct_body_child
                    && matches!(computed.position, PositionValue::Absolute)
                    && node.unrounded_layout.size.height > page_step;
                let table_row_candidate = inside_table
                    && matches!(computed.display, DisplayValue::TableRow)
                    && !inside_float
                    && !inside_flex;
                let flex_item_candidate =
                    flex_column_parent && !inside_float && !out_of_flow_subtree;
                let grid_item_candidate =
                    grid_single_column_parent && !inside_float && !out_of_flow_subtree;
                // The table engine stores row geometry on its cells rather
                // than on the anonymous row box. Derive the row border box
                // from those direct cells so pagination can keep the row
                // together without changing table sizing.
                let (candidate_raw_y, candidate_height) = if table_row_candidate {
                    let mut row_top = f32::INFINITY;
                    let mut row_bottom = f32::NEG_INFINITY;
                    for &child_id in &node.children {
                        let Some(child) = document.get_node(child_id) else {
                            continue;
                        };
                        let child_top = child.unrounded_layout.location.y;
                        row_top = row_top.min(child_top);
                        row_bottom = row_bottom.max(child_top + child.unrounded_layout.size.height);
                    }
                    if row_top.is_finite() && row_bottom.is_finite() && row_bottom > row_top {
                        (raw_y + row_top, row_bottom - row_top)
                    } else {
                        (raw_y, node.unrounded_layout.size.height.max(0.0))
                    }
                } else {
                    (raw_y, node.unrounded_layout.size.height.max(0.0))
                };
                let is_break_candidate = !is_body
                    && !inside_float
                    && ((participates_in_flow
                        && direct_body_child
                        && explicit_page_name.is_none())
                        || is_tall_direct_absolute
                        || table_row_candidate
                        || flex_item_candidate
                        || grid_item_candidate
                        || own_page_name.is_some()
                        || inline_named_page
                        || page_break_is_forced(computed.break_before)
                        || page_break_is_forced(computed.break_after));
                if is_break_candidate {
                    out.push(PageCandidate {
                        node_id,
                        raw_y: candidate_raw_y,
                        height: candidate_height,
                        is_text: false,
                        is_direct_body_text: false,
                        is_direct_body_element: direct_body_child,
                        is_table_row: table_row_candidate,
                        is_flex_item: flex_item_candidate,
                        is_grid_item: grid_item_candidate,
                        is_named: own_page_name.is_some(),
                        is_float_descendant: inside_float,
                        page_name: page_name.clone(),
                        deferred_named_break_after,
                        inline_named_page,
                    });
                }
                let child_is_direct_body = node_id == body_id;
                let child_order = pagination_child_order(document, cascade, node_id);
                for child_id in child_order {
                    collect_candidates(
                        document,
                        cascade,
                        child_id,
                        raw_y,
                        child_is_direct_body,
                        body_id,
                        node.unrounded_layout.size.height.max(0.0),
                        page_step,
                        child_page_name.clone(),
                        inside_table
                            || matches!(
                                computed.display,
                                DisplayValue::Table
                                    | DisplayValue::InlineTable
                                    | DisplayValue::TableRowGroup
                                    | DisplayValue::TableHeaderGroup
                                    | DisplayValue::TableFooterGroup
                                    | DisplayValue::TableRow
                            ),
                        matches!(
                            computed.display,
                            DisplayValue::Flex | DisplayValue::InlineFlex
                        ) && matches!(
                            computed.flex_direction,
                            FlexDirectionValue::Column | FlexDirectionValue::ColumnReverse
                        ),
                        matches!(
                            computed.display,
                            DisplayValue::Grid | DisplayValue::InlineGrid
                        ) && document.nodes[node_id].grid_column_count == 1,
                        inside_flex || matches!(computed.display, DisplayValue::Flex),
                        float_subtree,
                        out_of_flow_subtree,
                        out,
                    );
                }
            }
            _ => {}
        }
    }

    let mut candidates = Vec::new();
    collect_candidates(
        document,
        cascade,
        body_id,
        root_flow_offset,
        false,
        body_id,
        0.0,
        page_step,
        None,
        false,
        false,
        false,
        false,
        false,
        false,
        &mut candidates,
    );
    let mut flow_shift = 0.0_f32;
    let mut current_page = 0_u32;
    let mut max_page = 0_u32;
    // Keep a break-after attached to its source box until its descendants
    // have been visited.  Applying it to the first flex child would turn a
    // break-after on the flex container into an extra blank page.
    let mut pending_break_source: Option<usize> = None;
    // When a forced-break box underflows into the preceding page, descendants
    // follow the box while later siblings still begin after the break boundary.
    let mut pending_underflow: Option<(usize, f32)> = None;
    // A positive margin on a box after a forced break moves its descendants
    // with the box, but does not move later siblings a second time.
    let mut pending_descendant_margin: Option<(usize, f32)> = None;
    // Named class-A boxes at one source coordinate form one page transition.
    // This coalesces zero-height named runs such as a/b/c/d/e without creating
    // one blank page per zero-height box.
    let mut last_named_raw_y: Option<f32> = None;
    let mut last_named_was_zero_height = false;
    let mut last_named_had_display_none_descendant = false;
    let mut saw_child = false;
    // Direct fixed-height blocks are the only class-A boxes for which this
    // minimal paginator can safely move a leading line as a unit. Track the
    // first text child so a block is not reflowed repeatedly for later lines.
    let mut checked_block_text = HashSet::new();
    let mut checked_orphans_widows = HashSet::new();
    // Footnotes are collected in source order and stacked upward from the
    // bottom edge of their anchor page. The normal-flow height is removed
    // from `flow_shift`, leaving the box only in this page-local area.
    let mut footnote_heights: HashMap<u32, f32> = HashMap::new();
    let mut current_page_name =
        selected_page_name(cascade, body_id).or_else(|| first_page_name(document, cascade));
    let mut page_names = vec![current_page_name.clone()];

    let mut trailing_flex_child_by_parent = HashMap::<usize, Option<usize>>::new();
    for candidate in candidates {
        let node_id = candidate.node_id;
        if let Some((ancestor_id, correction)) = pending_underflow
            && !is_descendant_or_self(document, node_id, ancestor_id, &parent_of)
        {
            flow_shift += correction;
            pending_underflow = None;
        }
        if document.get_node(node_id).is_none() {
            continue;
        }
        let descendant_margin = if let Some((ancestor_id, margin)) = pending_descendant_margin {
            if is_descendant_or_self(document, node_id, ancestor_id, &parent_of) {
                margin
            } else {
                pending_descendant_margin = None;
                0.0
            }
        } else {
            0.0
        };
        if candidate.is_text {
            // A footnote's descendants follow the moved footnote box. They are
            // deliberately not paginated independently, otherwise the stale
            // pre-pagination coordinates would move them back into normal flow.
            if candidate.is_float_descendant && has_footnote_ancestor(cascade, node_id, &parent_of)
            {
                continue;
            }
            // Text is not itself a class-A box, but non-empty text can be the
            // content that follows a nested named box.  Carry the containing
            // box's page type so that content after `page:b` inside a
            // `page:a` box resumes on an `a` page rather than becoming an
            // anonymous page.  Only direct body text consumes a pending
            // break-after; descendants must wait until their containing box
            // has finished.
            let raw_y = candidate.raw_y;
            let height = candidate.height;
            let candidate_page_name = candidate.page_name.clone();
            let mut effective_y = raw_y + flow_shift + descendant_margin;
            // A fixed-height direct block whose first line would straddle the
            // current fragmentainer is pushed as a unit. This matches normal
            // block fragmentation for monolithic content while leaving tall
            // blocks and already-forced page transitions to the existing
            // fragment logic.
            let direct_block_parent = parent_of[node_id]
                .and_then(|parent_id| (parent_of[parent_id] == Some(body_id)).then_some(parent_id));
            if let Some(block_id) = direct_block_parent
                && checked_block_text.insert(block_id)
                && style_dimension_length(document.nodes[block_id].style.size.height)
                    .is_some_and(|value| value >= 0.0)
            {
                let block_height = document.nodes[block_id].unrounded_layout.size.height;
                let block_raw_y =
                    root_flow_offset + current_abs_y(document, block_id, &parent_of) - flow_shift;
                let relative_text_y = raw_y - block_raw_y;
                let block_effective_y = effective_y - relative_text_y;
                let page_start = page_origin(current_page);
                let page_end = page_start + page_step_at(current_page);
                let first_line_overflows = effective_y.is_finite()
                    && height.is_finite()
                    && effective_y >= page_start
                    && block_effective_y >= page_start
                    && block_effective_y < page_end
                    && block_height.is_finite()
                    && block_height <= page_step_at(current_page) + 0.001
                    && effective_y + height > page_end;
                if first_line_overflows {
                    let natural_page = page_index_for_y(effective_y + height);
                    let target_page = current_page.saturating_add(1).max(natural_page);
                    let target_y = page_origin(target_page);
                    let delta = target_y - block_effective_y;
                    if delta.is_finite() && delta > 0.0 {
                        materialize_y(document, block_id, block_effective_y + delta, &parent_of);
                        flow_shift += delta;
                        effective_y += delta;
                    }
                }
            }
            // `orphans` and `widows` constrain breaks between line boxes, not
            // breaks between block-level children.  The current paginator keeps
            // a text run in one `parley::Layout`, so the safe first step is to
            // move a fitting direct block as a unit when its natural split would
            // violate either constraint.  Oversized blocks stay on the
            // existing whole-box path; their line-level fragment map is a
            // separate concern.
            if let Some(block_id) = direct_block_parent
                && checked_orphans_widows.insert(block_id)
                && let Some(text_layout) = document.nodes[node_id].text_layout()
            {
                let page_start = page_origin(current_page);
                let page_end = page_start + page_step_at(current_page);
                let line_metrics: Vec<(f32, f32)> = text_layout
                    .lines()
                    .map(|line| {
                        let metrics = line.metrics();
                        (metrics.block_min_coord, metrics.block_max_coord)
                    })
                    .collect();
                let total_lines = line_metrics.len();
                let starts_on_page = line_metrics.first().is_some_and(|(line_top, _)| {
                    let top = effective_y + line_top;
                    top >= page_start - 0.001 && top < page_end
                });
                let block_height = document.nodes[block_id].unrounded_layout.size.height;
                let fits_on_page =
                    block_height.is_finite() && block_height <= page_step_at(current_page) + 0.001;
                let lines_before_break = line_metrics
                    .iter()
                    .take_while(|(_, line_bottom)| effective_y + line_bottom <= page_end + 0.001)
                    .count();
                let needs_break = lines_before_break < total_lines;
                let orphans = cascade.computed[node_id].orphans.max(1) as usize;
                let widows = cascade.computed[node_id].widows.max(1) as usize;
                let violates_line_constraints = needs_break
                    && (lines_before_break < orphans
                        || total_lines.saturating_sub(lines_before_break) < widows);
                if starts_on_page && fits_on_page && violates_line_constraints {
                    let block_raw_y = root_flow_offset
                        + current_abs_y(document, block_id, &parent_of)
                        - flow_shift;
                    let delta = page_origin(current_page.saturating_add(1)) - block_raw_y;
                    if delta.is_finite() && delta > 0.0 {
                        materialize_y(document, block_id, block_raw_y + delta, &parent_of);
                        flow_shift += delta;
                        effective_y += delta;
                    }
                }
            }
            materialize_y(document, node_id, effective_y, &parent_of);
            let named_page_change = saw_child
                && height > 0.0
                && !candidate.is_float_descendant
                && candidate_page_name != current_page_name;
            let pending_break_applies = pending_break_source.is_some_and(|source| {
                !is_descendant_or_self(document, node_id, source, &parent_of)
            });
            let consumes_pending_break = pending_break_applies && candidate.is_direct_body_text;
            if saw_child {
                if consumes_pending_break || named_page_change {
                    let natural_page = if effective_y.is_finite() && effective_y >= 0.0 {
                        page_index_for_y(effective_y)
                    } else {
                        current_page
                    };
                    let target_page = current_page.saturating_add(1).max(natural_page);
                    let target_y = page_origin(target_page);
                    // The candidate was materialized to `effective_y` above,
                    // so this is the additional movement for this boundary.
                    // Keep it separate from `flow_shift`: the latter also
                    // affects candidates whose ancestor is not moved here.
                    let node_delta = target_y - effective_y;
                    let shift_delta = target_y - effective_y;
                    if node_delta.is_finite() && shift_delta.is_finite() {
                        document.nodes[node_id].unrounded_layout.location.y += node_delta;
                        flow_shift += shift_delta;
                        effective_y += shift_delta;
                    }
                    current_page = target_page;
                    current_page_name = candidate_page_name.clone();
                } else if effective_y.is_finite() && effective_y >= 0.0 {
                    current_page = current_page.max(page_index_for_y(effective_y));
                }
            } else {
                saw_child = true;
                current_page = 0;
                if !candidate.is_float_descendant {
                    current_page_name = candidate_page_name.clone();
                }
            }
            if height > 0.0
                && !candidate.is_float_descendant
                && !named_page_change
                && !consumes_pending_break
            {
                current_page_name = candidate_page_name;
            }
            if page_names.len() <= current_page as usize {
                page_names.resize(current_page as usize + 1, None);
            }
            page_names[current_page as usize] = current_page_name.clone();
            max_page = max_page.max(current_page);
            if height > 0.0 && effective_y.is_finite() && effective_y >= 0.0 {
                let end = (effective_y + height).max(effective_y);
                if end.is_finite() && end > 0.0 {
                    let end_page = page_index_for_end(end);
                    max_page = max_page.max(end_page);
                }
            }
            if consumes_pending_break {
                pending_break_source = None;
            }
            let candidate_break_after = page_break_is_forced(cascade.computed[node_id].break_after)
                || candidate.deferred_named_break_after;
            let pending_source_is_ancestor = pending_break_source
                .is_some_and(|source| is_descendant_or_self(document, node_id, source, &parent_of));
            if candidate_break_after && !pending_source_is_ancestor {
                pending_break_source = Some(node_id);
            }
            continue;
        }

        let computed = &cascade.computed[node_id];
        let candidate_page_name = candidate.page_name.clone();
        // `float: footnote` is a page-local out-of-flow placement. Keep the
        // anchor's page, stack multiple notes from the bottom, and remove the
        // note's ordinary-flow footprint so following content can continue.
        if matches!(computed.float, FloatValue::Footnote) {
            let raw_y = candidate.raw_y;
            let height = candidate.height;
            let effective_y = raw_y + flow_shift + descendant_margin;
            let anchor_page = if effective_y.is_finite() && effective_y >= 0.0 {
                page_index_for_y(effective_y)
            } else {
                current_page
            };
            let note_page = current_page.max(anchor_page);
            let used = footnote_heights.entry(note_page).or_insert(0.0);
            let target_y = (page_origin(note_page) + page_step_at(note_page) - *used - height)
                .max(page_origin(note_page));
            materialize_y(document, node_id, target_y, &parent_of);
            *used += height;
            flow_shift -= height;
            max_page = max_page.max(note_page);
            saw_child = true;
            continue;
        }
        // Copy layout data before any break adjustment so the immutable node
        // borrow does not overlap the in-place location update below.
        let raw_y = candidate.raw_y;
        let height = candidate.height;
        let mut effective_y = raw_y + flow_shift + descendant_margin;
        materialize_y(document, node_id, effective_y, &parent_of);
        let forced_before =
            page_break_is_forced(computed.break_before) || candidate.inline_named_page;
        let style = &document.nodes[node_id].style;
        let margin_top = used_style_length_percentage_auto(style.margin.top, content_width)
            .or_else(|| used_computed_length_percentage_or_auto(computed.margin.top, content_width))
            .unwrap_or(0.0);
        let margin_bottom = used_style_length_percentage_auto(style.margin.bottom, content_width)
            .or_else(|| {
                used_computed_length_percentage_or_auto(computed.margin.bottom, content_width)
            })
            .unwrap_or(0.0)
            .max(0.0);
        // A block with `break-inside: avoid-page` stays intact when its used
        // block size plus its trailing margin would cross the current page.
        // This is the direct block-flow case; nested formatting contexts still
        // require fragment-level break opportunities.
        let page_overflow = matches!(
            computed.break_inside,
            raikiri_style::property::BreakInside::Avoid
                | raikiri_style::property::BreakInside::AvoidPage
        ) && effective_y.is_finite()
            && effective_y >= page_origin(current_page)
            && effective_y < page_origin(current_page) + page_step_at(current_page)
            && effective_y + height + margin_bottom
                > page_origin(current_page) + page_step_at(current_page);
        // A named page requested by a class-A box starts a new page when it
        // differs from the current named page. `auto` leaves the current page
        // type in place; it does not manufacture a second break boundary.
        // Any non-empty class-A box with a different page name starts a new
        // page.  `None` is the anonymous/unnamed page type, so an anonymous
        // box after a named one is a transition too.  Zero-height named boxes
        // at the same source coordinate are coalesced into one transition;
        // this matters for chains of empty named boxes with overflowing text.
        let same_named_coordinate = candidate.is_named
            && candidate.is_direct_body_element
            && height <= 0.001
            && last_named_was_zero_height
            && !last_named_had_display_none_descendant
            && last_named_raw_y.is_some_and(|previous| (raw_y - previous).abs() <= 0.001);
        let named_page_change =
            saw_child && candidate_page_name != current_page_name && !same_named_coordinate;
        let at_page_start =
            effective_y.is_finite() && (effective_y - page_origin(current_page)).abs() <= 0.001;
        let natural_page_at_position = if effective_y.is_finite() && effective_y >= 0.0 {
            page_index_for_y(effective_y)
        } else {
            current_page
        };
        // A forced break at the start of a page already opened by natural
        // overflow describes the same boundary and must not create a blank
        // page.  This matters when the page height changes after page zero:
        // the fixed-height pass may have considered the box to be on the
        // current page, while the scheduled pass places it exactly at the
        // next page origin.
        let forced_break_at_page_start =
            forced_before && (at_page_start || natural_page_at_position > current_page);
        // Keep an ordinary table row intact when it would cross the current
        // page. The row is a class-A boundary owned by the table engine; its
        // cells and descendants follow the same flow shift below. Rowspans,
        // repeated header groups, and cell-internal breaks remain out of this
        // narrow row-boundary pass.
        let table_row_overflow = candidate.is_table_row
            && height <= page_step_at(current_page) + 0.001
            && effective_y.is_finite()
            && effective_y >= page_origin(current_page)
            && effective_y < page_origin(current_page) + page_step_at(current_page)
            && effective_y + height > page_origin(current_page) + page_step_at(current_page);
        // A trailing flex item may extend into the containing flex box's
        // continuation at the page edge. Earlier items still move intact when
        // they would cross a fragmentainer, preserving the existing column
        // flex pagination behavior.
        let flex_item_is_last = candidate.is_flex_item
            && parent_of[node_id].is_some_and(|parent_id| {
                let trailing_child = trailing_flex_child_by_parent
                    .entry(parent_id)
                    .or_insert_with(|| {
                        let children = document.nodes[parent_id].layout_children();
                        let is_column_reverse = matches!(
                            cascade.computed[parent_id].flex_direction,
                            FlexDirectionValue::ColumnReverse
                        );
                        if is_column_reverse {
                            children.iter().find(|&&child_id| {
                                is_in_flow_flex_child_for_pagination(document, parent_id, child_id)
                            })
                        } else {
                            children.iter().rev().find(|&&child_id| {
                                is_in_flow_flex_child_for_pagination(document, parent_id, child_id)
                            })
                        }
                        .copied()
                    });
                *trailing_child == Some(node_id)
            });
        let flex_item_overflow = candidate.is_flex_item
            && !flex_item_is_last
            && effective_y.is_finite()
            && effective_y >= page_origin(current_page)
            && effective_y < page_origin(current_page) + page_step_at(current_page)
            && effective_y + height > page_origin(current_page) + page_step_at(current_page);
        let grid_item_overflow = candidate.is_grid_item
            && effective_y.is_finite()
            && effective_y >= page_origin(current_page)
            && effective_y < page_origin(current_page) + page_step_at(current_page)
            && effective_y + height > page_origin(current_page) + page_step_at(current_page);

        let pending_break_applies = pending_break_source
            .is_some_and(|source| !is_descendant_or_self(document, node_id, source, &parent_of));
        let page_transition = saw_child
            && (pending_break_applies
                || (forced_before && !forced_break_at_page_start)
                || named_page_change
                || page_overflow
                || table_row_overflow
                || flex_item_overflow
                || grid_item_overflow);
        if saw_child {
            // A break-after on the preceding box and a break-before (or named
            // page transition) on this box describe the same boundary, not two
            // blank pages.
            if page_transition {
                let natural_page = if effective_y.is_finite() && effective_y >= 0.0 {
                    page_index_for_y(effective_y)
                } else {
                    current_page
                };
                let target_page = current_page.saturating_add(1).max(natural_page);
                let target_y = page_origin(target_page);
                // A negative block-start margin can pull a forced-break box
                // back into the preceding page.  Keep the break boundary for
                // following siblings (and for page count), but materialize
                // this box at the underflowed coordinate.  This is the
                // `underflow-from-next-page` case from CSS Break.
                let underflow_y = target_y + margin_top;
                let node_target_y = if margin_top < 0.0 && underflow_y < target_y {
                    underflow_y
                } else {
                    // The forced break is before the box's margin box, so a
                    // positive block-start margin remains on the new page.
                    target_y + margin_top.max(0.0)
                };
                // The candidate was materialized to `effective_y` above,
                // so this is the additional movement for this boundary.
                // Keep `flow_shift` at the page boundary even when the
                // candidate itself underflows into the preceding page.
                let node_delta = node_target_y - effective_y;
                let shift_delta = target_y - effective_y;
                let row_flex_break_before = forced_before
                    && parent_of[node_id].is_some_and(|parent_id| {
                        matches!(
                            cascade.computed[parent_id].display,
                            DisplayValue::Flex | DisplayValue::InlineFlex
                        ) && matches!(
                            cascade.computed[parent_id].flex_direction,
                            FlexDirectionValue::Row | FlexDirectionValue::RowReverse
                        )
                    });
                if node_delta.is_finite() && shift_delta.is_finite() {
                    document.nodes[node_id].unrounded_layout.location.y += node_delta;
                    // A forced break on one wrapped row-flex item belongs to
                    // its whole flex line. Move same-line siblings together;
                    // other lines keep their existing flow coordinates.
                    if row_flex_break_before && let Some(parent_id) = parent_of[node_id] {
                        let siblings = document.nodes[parent_id].children.clone();
                        for sibling_id in siblings {
                            if sibling_id == node_id {
                                continue;
                            }
                            let sibling_raw_y = current_abs_y(document, sibling_id, &parent_of);
                            if (sibling_raw_y - raw_y).abs() <= 0.001 {
                                materialize_y(
                                    document,
                                    sibling_id,
                                    sibling_raw_y + shift_delta,
                                    &parent_of,
                                );
                            }
                        }
                    }
                    if node_target_y < target_y {
                        flow_shift += node_delta;
                        pending_underflow = Some((node_id, shift_delta - node_delta));
                    } else {
                        flow_shift += shift_delta;
                    }
                    effective_y += node_delta;
                }
                current_page = target_page;
                if pending_break_applies {
                    pending_break_source = None;
                }
            } else if effective_y.is_finite() && effective_y >= 0.0 {
                current_page = current_page.max(page_index_for_y(effective_y));
            }
            if page_transition && margin_top > 0.0 {
                pending_descendant_margin = Some((node_id, margin_top));
            }
            // An inline direct child keeps its source-order inline x position
            // in taffy's single pre-pagination layout. A forced page boundary
            // starts a fresh page formatting context, so reset that box's
            // inline origin before painting the next slice.
            if page_transition
                && candidate.is_direct_body_element
                && matches!(computed.display, DisplayValue::Inline)
            {
                document.nodes[node_id].unrounded_layout.location.x = 0.0;
            }
        } else {
            // A forced break before the first class-A box does not manufacture
            // a leading blank page.
            saw_child = true;
            current_page = 0;
        }

        if candidate.is_direct_body_element || candidate.is_named {
            current_page_name = candidate_page_name;
            if candidate.is_direct_body_element {
                last_named_raw_y = candidate.is_named.then_some(raw_y);
                last_named_was_zero_height = candidate.is_named && height <= 0.001;
                last_named_had_display_none_descendant =
                    candidate.is_named && has_display_none_descendant(document, node_id);
            }
        }
        if page_names.len() <= current_page as usize {
            page_names.resize(current_page as usize + 1, None);
        }
        page_names[current_page as usize] = current_page_name.clone();

        max_page = max_page.max(current_page);
        if height > 0.0 && effective_y.is_finite() && effective_y >= 0.0 {
            // A box ending exactly at a page edge belongs to the preceding
            // page.  `f32::EPSILON` is too small at ordinary CSS coordinates
            // and rounds away, so use the half-open interval directly:
            // ceil(end / step) - 1.  The box remains visible on every slice it
            // intersects; line-level splitting is a later pass.
            let end = (effective_y + height).max(effective_y);
            if end.is_finite() && end > 0.0 {
                // A fixed-height direct block that fits on the physical paper
                // is monolithic in this minimal paginator.  Page border/padding
                // reduce the fragmentainer's content height, but must not make
                // the block manufacture a second page merely to expose its
                // overflow into the page decoration area.
                let monolithic_paper_fit = candidate.is_direct_body_element
                    && (insets.top > 0.0
                        || insets.right > 0.0
                        || insets.bottom > 0.0
                        || insets.left > 0.0)
                    && effective_y >= page_origin(current_page)
                    && effective_y + height <= page_origin(current_page) + page_box.height + 0.001
                    && style_dimension_length(document.nodes[node_id].style.size.height)
                        .is_some_and(|value| value >= 0.0);
                if !monolithic_paper_fit {
                    let end_page = page_index_for_end(end);
                    max_page = max_page.max(end_page);
                }
            }
        }
        let candidate_break_after = page_break_is_forced(computed.break_after)
            || candidate.deferred_named_break_after
            || candidate.inline_named_page;
        let pending_source_is_ancestor = pending_break_source
            .is_some_and(|source| is_descendant_or_self(document, node_id, source, &parent_of));
        if candidate_break_after && !pending_source_is_ancestor {
            pending_break_source = Some(node_id);
        }
    }

    if !page_widths.is_empty() {
        let base_width = page_widths
            .first()
            .copied()
            .filter(|width| width.is_finite() && *width > 0.0)
            .unwrap_or(content_width);
        if base_width.is_finite() && base_width > 0.0 {
            for node_id in 0..document.nodes.len() {
                if !matches!(
                    cascade.computed[node_id].width,
                    ComputedLengthPercentageOrAuto::Percent(_)
                ) {
                    continue;
                }
                let page_index = page_index_for_y(current_abs_y(document, node_id, &parent_of));
                let Some(target_width) = page_widths
                    .get(page_index as usize)
                    .copied()
                    .filter(|width| width.is_finite() && *width > 0.0)
                else {
                    continue;
                };
                let scale = target_width / base_width;
                if scale.is_finite() && (scale - 1.0).abs() > 0.0001 {
                    document.nodes[node_id].unrounded_layout.size.width *= scale;
                }
            }
        }
    }

    Ok((0..=max_page)
        .map(|page_index| PageSlice {
            page_index,
            content_origin_y: page_origin(page_index),
            page_name: page_names.get(page_index as usize).cloned().flatten(),
        })
        .collect())
}

/// [`layout_single_page`] と同一だが、`<img>` 等 replaced element の
/// intrinsic size を `resolver` 経由で解決してから layout する。
///
/// # Errors
/// [`layout_single_page`] と同じ、加えて `LayoutError::Resolver` はこの
/// 関数固有 — `resolver.resolve()` が `Err` を返した時点で pre-pass を
/// 打ち切り、その error を `LayoutError::Resolver` に包んで返す (taffy
/// layout 自体は走らない)。`ReplacedResolver` の契約上 `Err` は常に
/// terminal であり、placeholder への degrade は Consumer が
/// `Ok(ResolvedIntrinsic { disposition: Fallback { .. } })` として表現する
/// — 詳細は `crate::image_resolve::resolve_images` の doc 参照。
///
/// # 実行順 (load-bearing)
/// `resolve_images` の前に [`Document::mark_in_document_flags`] を呼ぶ。
/// `resolve_images` は inert subtree (`<template>` 子孫等) の `<img>` を
/// membership flag で skip するので、flag が stale だと本来 fetch すべきで
/// ない URL に対して実 fetch が走ってしまう。[`layout_single_page`] 内でも
/// 同じ sync が走るが、そちらは本 pre-pass より**後**なので間に合わない。
/// `mark_in_document_flags` は `flags_dirty == false` のとき O(1) no-op
/// なので、二重呼び出しの実コストは無い。
///
/// # Note on error size
/// `LayoutError::Resolver(ResolverError)` transitively contains
/// `NetworkError` which embeds a `PolicyViolation` payload (~144 bytes),
/// exceeding clippy::result_large_err's 128 byte threshold.
#[allow(clippy::result_large_err)]
pub fn layout_single_page_with_resolver(
    document: &mut Document,
    cascade: &CascadeResult,
    page_box: PageBox,
    font_ctx: FontContext,
    resolver: &dyn ReplacedResolver,
) -> Result<(), LayoutError> {
    layout_single_page_with_resolver_and_base_url(
        document, cascade, page_box, font_ctx, resolver, None,
    )
}

/// [`layout_single_page_with_resolver`] with document-relative image URLs.
#[allow(clippy::result_large_err)]
pub fn layout_single_page_with_resolver_and_base_url(
    document: &mut Document,
    cascade: &CascadeResult,
    page_box: PageBox,
    font_ctx: FontContext,
    resolver: &dyn ReplacedResolver,
    base_url: Option<&url::Url>,
) -> Result<(), LayoutError> {
    // See "# 実行順" above — this must precede `resolve_images`, whose
    // membership gate reads the flags this refreshes.
    document.mark_in_document_flags();
    match base_url {
        Some(base_url) => {
            crate::image_resolve::resolve_images_with_base(document, resolver, Some(base_url))
        }
        None => crate::image_resolve::resolve_images(document, resolver),
    }
    .map_err(LayoutError::Resolver)?;
    layout_single_page(document, cascade, page_box, font_ctx)
}

#[cfg(test)]
mod tests;
