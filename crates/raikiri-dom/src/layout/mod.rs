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
