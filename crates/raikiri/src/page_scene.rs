//! Per-page immutable snapshot used by the `raikiri` crate's dogfooding and
//! validation paths. It is **not** the fulgur-facing contract: that surface is
//! produced and owned primarily by `raikiri-dom`.
//!
//! # Internal dogfooding contract
//!
//! [`PageScene`] is an **immutable snapshot of one page** after it passes through
//! raikiri's internal pipeline (fragmentation + reflow + LayoutBuffer).
//! Internal validation and raster paths consume it through forward iteration and
//! batch operations; they do not handle reflow or re-fragmentation themselves.
//!
//! # Node identity
//!
//! [`raikiri_traits::NodeId`] identifies DOM nodes distributed across a page.
//! The raikiri crate's Document (umbrella re-export) and PageScene share the
//! **same NodeId space**. A consumer can therefore use one `NodeId` for Document
//! property lookups and PageScene drawable / fragment lookups **without conversion**
//! (an established design decision).
//!
//! # Landing scope: pub type surface
//!
//! Only the public type surface was initially landed (`#[non_exhaustive]` keeps
//! future field additions semver-non-breaking). The path that populates `PageScene`
//! from the internal pipeline is filled in as part of future streaming pagination.
//!
//! # Landing scope: html_to_png refactor through PageScene
//!
//! The internal `html_to_png` pipeline was refactored through PageScene.
//! [`build_page_scene`] assembles a [`PageScene`] with metadata and fragments
//! extracted from the post-layout Document. [`PageScene::rasterize`] reuses the
//! byte-identical sequence of `raikiri_paint::paint_single_page`,
//! `anyrender_vello_cpu`, and PNG encoding verbatim.
//!
//! At this stage, `drawables` was left empty on purpose. Populating entries
//! without glyph runs or position information would change emission order and
//! break byte identity; this was a hard constraint.
//!
//! # Landing scope: populate drawables (narrowed, items 1-3)
//!
//! [`build_page_scene`] now populates `drawables` (post-layout Element nodes →
//! [`crate::BlockEntry`], post-layout Text nodes → [`crate::ParagraphEntry`]; see
//! the [`crate::entries`] module docs). **The paint pipeline still does not
//! consume `PageDrawables`**: [`PageScene::rasterize`] continues to pass `dom`
//! and `cascade` to `raikiri_paint::paint_single_page` verbatim (see the
//! `rasterize` docs below). Keeping population separate from consumption makes
//! this change byte-identical in VRT. Reworking
//! `raikiri_paint::paint_single_page` to consume `PageDrawables` (the original
//! item 4) remains future work pending a crate-topology decision.

use crate::PageDrawables;
use crate::entries::{BlockEntry, ParagraphEntry};
use crate::raster_budget::RasterBufferBudget;
use anyrender::render_to_buffer;
use anyrender_vello_cpu::VelloCpuImageRenderer;
use raikiri_dom::{CounterSnapshotBudget, Document};
use raikiri_style::property::{DisplayValue, FloatValue, PositionValue};
use raikiri_style::resolve::{ComputedLengthPercentage, ComputedLengthPercentageOrAuto};
use raikiri_style::{CascadeResult, PageMarginBoxCascadeResult};
use raikiri_traits::{NodeId, NodeKind, PageBox, RenderError};
use std::collections::{BTreeMap, HashMap};

/// Type alias for coordinates within `PageScene`.
///
/// This is for a dogfooding snapshot in the `raikiri` crate, not the
/// fulgur-facing coordinate contract. It is an alias rather than a newtype,
/// so arithmetic can use Rust's primitive f32 operators directly. This was
/// the initial minimal-surface decision; stronger unit safety would require
/// a separate decision to introduce a newtype.
///
/// # Unit contract issue: `Pt` currently carries CSS px, not PDF pt
///
/// `build_page_scene` populates `PageMetadata.size` and [`Fragment`] fields
/// (of this `Pt` type) with **CSS px** values (1/96 in, from PageBox / taffy
/// `unrounded_layout`). This documentation promises "PDF point 1/72 in", so a
/// fulgur consumer that reads these values as pt gets about 33% geometry drift.
/// The byte-identical raster path is unaffected: paint walks the Node arena
/// again in CSS px. Unit conversion or a type rename is deferred to future
/// work because byte-identical maintenance was the primary scope.
///
/// The later [`crate::entries`] types (`BlockEntry`'s `layout_size`,
/// `border_widths`, etc.) reuse the same `Pt` alias and deliberately inherit
/// this CSS-px-in-Pt debt rather than introduce another kind of known unit
/// mismatch (see the [`crate::entries`] module docs).
pub type Pt = f32;

/// Page orientation.
///
/// A consumer-facing property reflecting CSS Paged Media's
/// `size: portrait | landscape`. It is the primary axis for future
/// interpretation of `size: <named-size>` (A4 / Letter, etc.) together with
/// [`PageMetadata::page_name`].
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Orientation {
    /// Portrait (width < height), the CSS default.
    #[default]
    Portrait,
    /// Landscape (width > height).
    Landscape,
}

/// Metadata for an entire page (size / name / orientation).
///
/// `size` is `(width, height)` in Pt. The consumer contract connects the
/// CSS `@page` rule's `size` descriptor to the consumer's canvas size.
#[non_exhaustive]
#[derive(Debug, Clone, Default)]
pub struct PageMetadata {
    /// Page (width, height) in Pt.
    pub size: (Pt, Pt),
    /// Zero-based position in the paginated document.
    pub page_index: u32,
    /// Name of the page selected by CSS rules such as
    /// `@page :first { size: A4 landscape; }`. `None` for an unnamed (default) page.
    pub page_name: Option<String>,
    /// [`Orientation::Portrait`] / [`Orientation::Landscape`].
    pub orientation: Orientation,
}

/// Coordinates of one fragment corresponding to a [`NodeId`] (in Pt,
/// relative to the body content area, following Fulgur's per-fragment
/// coordinate semantics in drawables.rs:430-438).
///
/// PageScene represents a multi-page block (for example, a `<div>` split into
/// two fragments across a page break) as multiple entries in
/// `fragments: BTreeMap<NodeId, Vec<Fragment>>`. `page_index` identifies the
/// fragment's page across pages; it is fixed within one PageScene.
///
/// Re-exported as `PageSceneFragment` to distinguish these scene coordinates
/// from the layout views used by drawing consumers.
#[non_exhaustive]
#[derive(Debug, Clone, Default)]
pub struct Fragment {
    /// Zero-based index of the page containing this fragment.
    pub page_index: u32,
    /// Fragment top-left x (border box, relative to the body-content-area origin, Pt).
    pub x: Pt,
    /// Fragment top-left y (border box, relative to the body-content-area origin, Pt).
    pub y: Pt,
    /// Fragment width (border box, Pt).
    pub width: Pt,
    /// Fragment height (border box, Pt).
    pub height: Pt,
}

/// Immutable snapshot of one page. Public type consumed one page at a time
/// by forward-iterating dogfooding and validation paths within `raikiri`.
///
/// Fulgur-facing page output is designed primarily in `raikiri-dom`; this
/// type is not that external contract.
///
/// # Field meanings
///
/// - [`page_metadata`](PageScene::page_metadata): page size / name / orientation
/// - [`node_ids`](PageScene::node_ids): [`NodeId`] values for every DOM node on
///   this page, in the order established by raikiri's internal fragmentation pass
/// - [`fragments`](PageScene::fragments): per-fragment coordinates for each NodeId;
///   the `Vec` has length 1 when a node occupies one fragment on the page
/// - [`drawables`](PageScene::drawables): per-attribute node map (see
///   [`PageDrawables`]), with an ECS-like shape
/// - [`root_id`](PageScene::root_id) / [`body_id`](PageScene::body_id): NodeIds
///   of the `<html>` / `<body>` nodes; entry points for consumers looking up
///   root-level styling (background / opacity)
/// - [`body_offset_pt`](PageScene::body_offset_pt): page-absolute offset after
///   html → body margin collapse (following `body_offset_pt` semantics in
///   Fulgur drawables.rs:430-438). Consumers add this to the body-content-area-
///   relative per-fragment (x, y) in [`fragments`](PageScene::fragments) to get
///   page-absolute coordinates
/// - [`margin_boxes`](PageScene::margin_boxes): matching `@page` margin-box
///   declaration bags for the downstream slot-layout pass
#[non_exhaustive]
#[derive(Debug, Clone, Default)]
pub struct PageScene {
    /// Page size / name / orientation.
    pub page_metadata: PageMetadata,
    /// All NodeIds on this page (in raikiri internal pass order).
    pub node_ids: Vec<NodeId>,
    /// Per-fragment coordinates keyed by NodeId.
    pub fragments: BTreeMap<NodeId, Vec<Fragment>>,
    /// Per-attribute node map (see [`PageDrawables`]).
    pub drawables: PageDrawables,
    /// `<html>` root NodeId, or `None` if the document has no root.
    pub root_id: Option<NodeId>,
    /// `<body>` NodeId, or `None` if the document has no body.
    pub body_id: Option<NodeId>,
    /// Page-absolute offset (Pt, Pt) accounting for html → body margin collapse.
    /// Add it to body-content-area-relative fragment coordinates to obtain
    /// page-absolute coordinates.
    pub body_offset_pt: (Pt, Pt),
    /// Cascaded margin-box declaration bags for this page, in source order.
    ///
    /// This is the page-scene consumer boundary: the future margin-box layout
    /// pass can resolve inherited values and geometry without reparsing the
    /// stylesheet or reaching back into the document rule tree.
    pub margin_boxes: Vec<PageMarginBoxCascadeResult>,
    /// Body-content y origin used to extract this page from a shared layout.
    /// Zero for the first page and for scenes built through the compatibility
    /// single-page helper.
    pub content_origin_y: Pt,
}

impl PageScene {
    /// Rasterize a one-page snapshot on an A4 (or supplied `PageBox`) canvas
    /// and return PNG bytes. Reuse `counter_budget` for every page in one
    /// document render operation.
    ///
    /// # Why does a snapshot take `dom` and `cascade` parameters?
    ///
    /// [`PageDrawables`] entries (BlockEntry / ParagraphEntry) have their
    /// minimal fields populated, but paint has not switched to consuming them.
    /// Glyph runs (their actual shapes and positions) still live on the
    /// paragraph roots of the DOM arena as shodo lines, outside the field-type
    /// policy for [`crate::entries`] (which excludes raikiri-style and shodo
    /// types). The source of truth for paint therefore remains the post-layout
    /// arena, which [`raikiri_paint::paint_single_page`] consumes by DFS. To maintain
    /// byte-identical output, rasterize still passes `dom` and `cascade` through
    /// to the existing paint pipeline verbatim. Reworking `paint_single_page`
    /// to consume `PageDrawables` (achieving actual snapshot semantics and
    /// dropping the `dom` / `cascade` parameters) is separate future work
    /// pending a crate-topology decision.
    ///
    /// # Internals
    /// 1. Validate and reserve the rounded pixel dimensions with
    ///    `RasterBufferBudget` (the same limit used by html_to_png).
    /// 2. Build `PaintScene` with `anyrender::render_to_buffer::<VelloCpuImageRenderer, _>`.
    /// 3. Call `raikiri_paint::paint_single_page(scene, dom, cascade, page_box)` verbatim.
    /// 4. Serialize RGBA8 to PNG with `encode_png` (`tiny_skia::Pixmap::encode_png`).
    ///
    /// # Errors
    /// Returns `RenderError::Configuration` for non-finite or non-positive
    /// dimensions and `RenderError::LimitExceeded` when the page exceeds the
    /// shared raster edge or byte budget.
    ///
    /// # Panics
    /// - The `anyrender_vello_cpu` output buffer length differs from
    ///   `width * height * 4` (invariant violation).
    /// - `tiny_skia::Pixmap::encode_png` fails (not expected for a well-formed pixmap).
    /// - (Debug builds only) `cascade` and `dom` did not come from the same
    ///   `cascade()` call (`cascade.computed.len() != dom.node_count()`). This
    ///   violates the caller-responsibility contract in the `raikiri_paint`
    ///   module docs under `## Contract`. The `debug_assert!` at the start of
    ///   [`raikiri_paint::paint_single_page`] checks it. The assert disappears
    ///   in release builds. There, behavior depends on the direction of the
    ///   mismatch: if `cascade.computed.len() < dom.node_count()`, a raw index
    ///   site (`cascade.computed[node_id]`) in the walk panics with "index out
    ///   of bounds". In the other direction
    ///   (`cascade.computed.len() > dom.node_count()`), the index stays in bounds;
    ///   rasterization completes while silently using computed values from
    ///   another document.
    ///
    /// Passing a pre-layout Document produces a PNG with missing glyphs because
    /// no paragraph has lines yet. Behavior is undefined; callers must wait
    /// for `layout_single_page` to complete.
    pub fn rasterize(
        &self,
        dom: &Document,
        cascade: &CascadeResult,
        page_box: PageBox,
        counter_budget: &mut CounterSnapshotBudget,
    ) -> Result<Vec<u8>, RenderError> {
        let size = RasterBufferBudget::new().reserve_page_box(page_box)?;

        let mut paint_result = Ok(());
        let rgba = render_to_buffer::<VelloCpuImageRenderer, _>(
            |scene| {
                paint_result = raikiri_paint::paint_single_page_with_origin(
                    scene,
                    dom,
                    cascade,
                    page_box,
                    self.content_origin_y,
                    counter_budget,
                )
            },
            size.width(),
            size.height(),
        );
        paint_result?;

        Ok(encode_png(&rgba, size.width(), size.height()))
    }

    /// Like [`Self::rasterize`], but retrieves decoded `<img>` pixels from
    /// `pixel_source` and actually draws them.
    ///
    /// This is named `_with_images`, not `_with_resolver`, because this stage
    /// takes an `ImagePixelSource` (an accessor for decoded pixels), not a
    /// `ReplacedResolver`. Intrinsic size resolution already happened before
    /// layout (see [`crate::html_to_png_with_resolver`]).
    ///
    /// # Errors
    /// Returns the same dimension and raster limit errors as [`Self::rasterize`].
    /// Reuse `counter_budget` for every page in one document render operation.
    pub fn rasterize_with_images(
        &self,
        dom: &Document,
        cascade: &CascadeResult,
        page_box: PageBox,
        pixel_source: &dyn raikiri_traits::ImagePixelSource,
        counter_budget: &mut CounterSnapshotBudget,
    ) -> Result<Vec<u8>, RenderError> {
        let size = RasterBufferBudget::new().reserve_page_box(page_box)?;

        let mut paint_result = Ok(());
        let rgba = render_to_buffer::<VelloCpuImageRenderer, _>(
            |scene| {
                paint_result = raikiri_paint::paint_single_page_with_images(
                    scene,
                    dom,
                    cascade,
                    page_box,
                    pixel_source,
                    counter_budget,
                )
            },
            size.width(),
            size.height(),
        );
        paint_result?;

        Ok(encode_png(&rgba, size.width(), size.height()))
    }
}

/// Resolve a computed margin to used px.
///
/// `basis` is the containing-block width (margin percentages resolve against
/// the width per CSS 2.1 section 8.3). `Auto` maps to zero, which matches the
/// block-level used-value rule for ordinary flow when `width` is also auto.
fn used_margin_px(value: ComputedLengthPercentageOrAuto, basis: f32) -> f32 {
    let resolved = match value {
        ComputedLengthPercentageOrAuto::Px(px) => px,
        ComputedLengthPercentageOrAuto::Percent(percent) => basis * percent / 100.0,
        ComputedLengthPercentageOrAuto::Calc(calc) => calc.px + basis * calc.percent / 100.0,
        ComputedLengthPercentageOrAuto::Auto | ComputedLengthPercentageOrAuto::MinContent => {
            return 0.0;
        }
    };
    if resolved.is_finite() { resolved } else { 0.0 }
}

/// Resolve computed padding to used px against the same width basis.
fn used_padding_px(value: ComputedLengthPercentage, basis: f32) -> f32 {
    let resolved = match value {
        ComputedLengthPercentage::Px(px) => px,
        ComputedLengthPercentage::Percent(percent) => basis * percent / 100.0,
    };
    if resolved.is_finite() { resolved } else { 0.0 }
}

/// Collapse adjoining vertical margins per CSS 2.1 section 8.3.1.
///
/// Positive margins combine to their maximum, negative margins to their
/// minimum (most negative), and mixed signs combine as the largest positive
/// plus the most negative. Including zero covers the all-positive and
/// all-negative cases without extra branches.
fn collapse_margins(values: &[f32]) -> f32 {
    let mut max_pos = 0.0_f32;
    let mut min_neg = 0.0_f32;
    for &value in values {
        if !value.is_finite() {
            continue;
        }
        if value >= 0.0 {
            max_pos = max_pos.max(value);
        } else {
            min_neg = min_neg.min(value);
        }
    }
    max_pos + min_neg
}

/// Whether the element has a top border or padding barrier.
///
/// A nonzero top border or padding breaks parent-first-child margin
/// collapsing (CSS 2.1 section 8.3.1). Border uses the absolutized computed
/// width; padding resolves percentages against the containing-block width.
fn has_top_barrier(cascade: &CascadeResult, node_idx: usize, content_width: f32) -> bool {
    let Some(computed) = cascade.computed.get(node_idx) else {
        return false;
    };
    let border = computed.border.top.width().px();
    if border.is_finite() && border > 0.001 {
        return true;
    }
    let padding = used_padding_px(computed.padding.top, content_width);
    padding > 0.001
}

/// Top margin of the first in-flow body child that adjoins the body top.
///
/// Returns `None` when no in-flow child adjoins (empty body, only
/// out-of-flow or `display: none` children, a leading non-whitespace text run,
/// a leading float, or a body with non-visible vertical overflow). In those
/// cases the body top does not collapse with a child margin and the caller
/// falls back to the `<html>`/`<body>` collapsed value alone.
///
/// Only the first level is considered. A deeper first-child chain (body with
/// no barrier whose first child also has no barrier and its own first child)
/// collapses all three per CSS 2.1 section 8.3.1, but that chain is a
/// follow-up; this helper stops after one level.
fn first_in_flow_top_margin(
    dom: &Document,
    cascade: &CascadeResult,
    body_idx: usize,
    content_width: f32,
) -> Option<f32> {
    use raikiri_style::property::OverflowValue;

    if let Some(computed) = cascade.computed.get(body_idx)
        && !matches!(
            computed.overflow.y,
            OverflowValue::Visible | OverflowValue::Clip
        )
    {
        return None;
    }
    let body = dom.get_node(body_idx)?;
    for &child_idx in &body.children {
        let Some(child) = dom.get_node(child_idx) else {
            continue; // cov:ignore: body children indices are always valid in a well-formed Document
        };
        if !child.is_in_document()
            || child.is_non_rendered_html_element()
            || child.is_display_none()
        {
            continue;
        }
        match child.kind() {
            NodeKind::Text => {
                let text = child.text_content().unwrap_or_default();
                if text.trim().is_empty() {
                    continue;
                }
                return None;
            }
            NodeKind::Element => {
                let Some(computed) = cascade.computed.get(child_idx) else {
                    continue;
                };
                if computed.display == DisplayValue::None {
                    continue;
                }
                if matches!(
                    computed.position,
                    PositionValue::Absolute | PositionValue::Fixed
                ) {
                    continue;
                }
                if !matches!(computed.float, FloatValue::None) {
                    return None;
                }
                return Some(used_margin_px(computed.margin.top, content_width));
            }
            _ => continue, // cov:ignore: body children via parsing are only Text or Element
        }
    }
    None
}

/// Extra page-absolute offset contributed by `<html>`/`<body>` margins.
///
/// Horizontally margins never collapse. Layout already places the body's
/// content inside the body's own left margin (the synthetic body root carries
/// its horizontal margins as inline padding), so the x extra is the `<html>`
/// left margin alone.
/// Vertically adjoining margins collapse (see [`collapse_margins`]), so the y
/// extra is the collapsed `<html>`/`<body>`/first-child value minus the first
/// child's own margin, which keeps the first child's page-absolute position
/// at the collapsed value while leaving its body-relative fragment untouched.
/// When the body has a top barrier or no adjoining first child, the y extra
/// is just the collapsed `<html>`/`<body>` value.
///
/// `content_width` is the page content width used as the percent basis.
/// An `<html>` top barrier (border/padding) disables `<html>`/`<body>`
/// collapsing; that rare case falls back to the plain sum and is documented
/// as an approximation.
fn body_margin_offsets(
    dom: &Document,
    cascade: &CascadeResult,
    body_idx: Option<usize>,
    html_idx: Option<usize>,
    content_width: f32,
) -> (Pt, Pt) {
    let Some(body_idx) = body_idx else {
        return (0.0, 0.0);
    };
    let basis = if content_width.is_finite() && content_width > 0.0 {
        content_width
    } else {
        0.0
    };
    let body_top = cascade
        .computed
        .get(body_idx)
        .map_or(0.0, |computed| used_margin_px(computed.margin.top, basis));
    let (html_left, html_top, html_has_barrier) = html_idx.map_or((0.0, 0.0, false), |idx| {
        let left = cascade
            .computed
            .get(idx)
            .map_or(0.0, |computed| used_margin_px(computed.margin.left, basis));
        let top = cascade
            .computed
            .get(idx)
            .map_or(0.0, |computed| used_margin_px(computed.margin.top, basis));
        let barrier = has_top_barrier(cascade, idx, basis);
        (left, top, barrier)
    });
    let left_extra = html_left;
    if html_has_barrier {
        return (left_extra, html_top + body_top);
    }
    let html_body = collapse_margins(&[html_top, body_top]);
    if has_top_barrier(cascade, body_idx, basis) {
        return (left_extra, html_body);
    }
    let Some(first_top) = first_in_flow_top_margin(dom, cascade, body_idx, basis) else {
        return (left_extra, html_body);
    };
    let collapsed = collapse_margins(&[html_top, body_top, first_top]);
    (left_extra, collapsed - first_top)
}

/// Extract metadata, fragments, and drawables from a post-layout Document
/// to construct a [`PageScene`].
///
/// Internal integration: called by `html_to_png_impl` after
/// `layout_single_page` completes. The `cascade` parameter is now active
/// (formerly `_cascade`): during the DFS walk, it inserts Element nodes into
/// `drawables` as [`BlockEntry`] and Text nodes as [`ParagraphEntry`] (see the
/// landing scope in the module docs). The other nine Entry types are not
/// inserted because their corresponding pipeline stages do not exist (see
/// the [`crate::entries`] module docs).
///
/// # NodeId identity mapping
///
/// Map raikiri-dom arena indices (`usize`) 1:1 to `raikiri_traits::NodeId(u64)`
/// via `NodeId::new(idx as u64)`, honoring the promise of a shared NodeId
/// space. This mapping becomes functionally important when paint switches to
/// consuming PageDrawables; for now it is metadata and does not affect paint.
///
/// # Fragment coordinate semantics
///
/// [`Fragment`] uses Pt relative to the body content area. The synthetic
/// body root spans the page content width at `(0, 0)` and carries the body's
/// horizontal margins as inline padding, so descendants already sit inside
/// the body's left margin; the body's own fragment is that root box inset
/// horizontally by the body's left and right margins, its border box.
/// Descendants use body-relative coordinates accumulated as
/// `parent_abs + node.unrounded_layout.location`, in the same DFS stack order
/// as `raikiri_paint::walk::paint_document`.
///
/// `body_offset_pt` is the body root's position relative to the page-absolute
/// origin, including `@page` margins, `@page` border/padding insets, and the
/// `<html>`/`<body>` element margins outside the body root: horizontally the
/// `<html>` left margin (margins never collapse, CSS 2.1 section 8.3.1).
/// Vertically
/// adjoining margins collapse to the largest positive plus the most negative
/// (CSS 2.1 section 8.3.1), so the y offset is the collapsed value minus the
/// first in-flow child's own top margin (single level only; deeper chains,
/// `<html>` border/padding, and non-visible `overflow` are follow-ups).
/// Consumers add this offset to body-content-area-relative fragment
/// coordinates to obtain page-absolute coordinates.
///
/// Build the first page of a document using the compatibility single-page
/// coordinates.
pub fn build_page_scene(dom: &Document, cascade: &CascadeResult, page_box: PageBox) -> PageScene {
    build_page_scene_for_page(dom, cascade, page_box, 0, 0.0)
}

/// Extract one page from a shared post-pagination layout.
///
/// Nodes whose border boxes intersect the page's body-content interval are
/// included.  The body container is included on every page so its propagated
/// background and page-local fragment remain available to consumers.  A box
/// crossing a page edge is currently represented on both scenes; the next
/// fragmentation pass can replace it with line/child-level fragments without
/// changing this scene boundary.
pub fn build_page_scene_for_page(
    dom: &Document,
    cascade: &CascadeResult,
    page_box: PageBox,
    page_index: u32,
    content_origin_y: Pt,
) -> PageScene {
    build_page_scene_for_page_named(dom, cascade, page_box, page_index, content_origin_y, None)
}

/// Extract one page and attach the page type selected by the pagination pass.
pub fn build_page_scene_for_page_named(
    dom: &Document,
    cascade: &CascadeResult,
    page_box: PageBox,
    page_index: u32,
    content_origin_y: Pt,
    page_name: Option<String>,
) -> PageScene {
    let margins = raikiri_dom::page_margins(cascade, page_box);
    let insets = raikiri_dom::page_content_insets(cascade, page_box);
    let content_height = (margins.content_height(page_box) - insets.top - insets.bottom).max(0.0);
    let page_top = content_origin_y;
    let page_bottom = content_origin_y + content_height;
    let page_metadata = PageMetadata {
        size: (page_box.width, page_box.height),
        page_index,
        page_name,
        orientation: if page_box.width > page_box.height {
            Orientation::Landscape
        } else {
            Orientation::Portrait
        },
    };

    let root_id = find_first_element_by_tag(dom, "html").map(|idx| NodeId::new(idx as u64));
    let html_arena_idx = find_first_element_by_tag(dom, "html");
    let body_arena_idx = find_first_element_by_tag(dom, "body");
    let body_id = body_arena_idx.map(|idx| NodeId::new(idx as u64));
    let content_width = margins.content_width(page_box).max(0.0);
    let (body_left_extra, body_top_extra) =
        body_margin_offsets(dom, cascade, body_arena_idx, html_arena_idx, content_width);
    // The body's border box is the synthetic root, which spans the page
    // content width, inset by the margins layout moved into its padding.
    let body_inline_margins = dom.body_inline_margins();

    let mut node_ids: Vec<NodeId> = Vec::new();
    let mut fragments: BTreeMap<NodeId, Vec<Fragment>> = BTreeMap::new();
    let mut drawables = PageDrawables::default();

    if let Some(body_idx) = body_arena_idx {
        // DFS from body, matching paint_document's traversal order.  Absolute
        // y stays in the shared document coordinate space until the fragment
        // is committed, which makes page intersection independent of nesting.
        let mut stack: Vec<(usize, Pt, Pt)> = vec![(body_idx, 0.0, 0.0)];
        // Content-box origin and inline element pieces of every paragraph
        // laid out by the inline engine, by root. Its text nodes have no
        // layout of their own, and its inline elements are recorded as one
        // rectangle per line; both are measured from the root's content box.
        // A root is visited before its descendants.
        type SceneRect = (Pt, Pt, Pt, Pt);
        type PieceRectsByNode = HashMap<usize, Vec<SceneRect>>;
        let mut ifc_origin: HashMap<usize, (Pt, Pt)> = HashMap::new();
        let mut ifc_pieces: HashMap<usize, PieceRectsByNode> = HashMap::new();
        let mut ifc_text_lines = HashMap::new();
        while let Some((idx, parent_abs_x, parent_abs_y)) = stack.pop() {
            let Some(node) = dom.get_node(idx) else {
                continue; // cov:ignore: DFS stack holds only body-subtree indices, always valid in a well-formed Document
            };
            if !node.is_in_document()
                || node.is_non_rendered_html_element()
                || node.is_display_none()
            {
                continue;
            }
            let layout = node.unrounded_layout;
            let abs_x = parent_abs_x + layout.location.x;
            let abs_y = parent_abs_y + layout.location.y;
            let is_body = idx == body_idx;
            if node.is_ifc_root() {
                let origin = (
                    abs_x + layout.border.left + layout.padding.left,
                    abs_y + layout.border.top + layout.padding.top,
                );
                ifc_origin.insert(idx, origin);
                let mut by_node: PieceRectsByNode = HashMap::new();
                for piece in node.ifc_inline_boxes().unwrap_or_default() {
                    let rect = piece.border_box;
                    by_node.entry(piece.node).or_default().push((
                        origin.0 + rect.x,
                        origin.1 + rect.y,
                        rect.width,
                        rect.height,
                    ));
                }
                ifc_pieces.insert(idx, by_node);
                ifc_text_lines.extend(dom.ifc_text_lines_by_node(idx));
            }
            let ifc_lines = if node.kind() == NodeKind::Text {
                ifc_text_lines.get(&idx)
            } else {
                None
            };
            // Border boxes of the node in document coordinates: one for most
            // nodes, one per line for an inline element of an ifc paragraph.
            let rects: Vec<(Pt, Pt, Pt, Pt)> = if let Some(owned) = ifc_lines {
                let (root_x, root_y) = ifc_origin
                    .get(&owned.root)
                    .copied()
                    .unwrap_or((abs_x, abs_y));
                let first_top = owned.lines.first().map_or(0.0, |l| l.top);
                let last_bottom = owned.lines.last().map_or(0.0, |l| l.bottom);
                vec![(
                    root_x,
                    root_y + first_top,
                    owned.width,
                    (last_bottom - first_top).max(0.0),
                )]
            } else {
                let pieces: Vec<(Pt, Pt, Pt, Pt)> = if node.in_ifc_subtree() {
                    ifc_root_of(dom, idx)
                        .and_then(|root| ifc_pieces.get(&root))
                        .and_then(|pieces| pieces.get(&idx))
                        .cloned()
                        .unwrap_or_default()
                } else {
                    Vec::new()
                };
                if is_body {
                    let (left, right) = body_inline_margins;
                    vec![(
                        abs_x + left,
                        abs_y,
                        (layout.size.width - left - right).max(0.0),
                        layout.size.height,
                    )]
                } else if pieces.is_empty() {
                    vec![(abs_x, abs_y, layout.size.width, layout.size.height)]
                } else {
                    pieces
                }
            };
            let on_page: Vec<(Pt, Pt, Pt, Pt)> = rects
                .into_iter()
                .filter(|&(_, y, _, height)| {
                    let height = height.max(0.0);
                    is_body
                        || if height == 0.0 {
                            y >= page_top && y <= page_bottom
                        } else {
                            y < page_bottom && y + height > page_top
                        }
                })
                .collect();

            if !on_page.is_empty() {
                let node_id = NodeId::new(idx as u64);
                node_ids.push(node_id);
                let entries = fragments.entry(node_id).or_default();
                for (x, y, width, height) in on_page {
                    entries.push(Fragment {
                        page_index,
                        x,
                        y: y - content_origin_y,
                        width,
                        height,
                    });
                }

                // `crate::entries` keeps primitive snapshots; the live DOM is
                // still the paint truth until drawables become the renderer
                // input.
                match node.kind() {
                    NodeKind::Element => {
                        let cv = &cascade.computed[idx];
                        let entry = BlockEntry {
                            background_color: (
                                cv.background_color.r,
                                cv.background_color.g,
                                cv.background_color.b,
                                cv.background_color.a,
                            ),
                            border_widths: (
                                cv.border.top.width().0,
                                cv.border.right.width().0,
                                cv.border.bottom.width().0,
                                cv.border.left.width().0,
                            ),
                            id: element_id(dom, node_id),
                            layout_size: Some((layout.size.width, layout.size.height)),
                            ..BlockEntry::default()
                        };
                        drawables.block_styles.insert(node_id, entry);
                    }
                    NodeKind::Text => {
                        // A text node outside every paragraph has no lines.
                        let line_count = ifc_lines.map_or(0, |owned| owned.lines.len());
                        let entry = ParagraphEntry {
                            line_count,
                            ..ParagraphEntry::default()
                        };
                        drawables.paragraphs.insert(node_id, entry);
                    }
                    _ => {} // cov:ignore: in-document body descendants via parsing are only Element or Text (Comment/PI bits cleared, template fragments detached, DFS starts at body)
                }
            }

            if node.kind() == NodeKind::Element {
                // The children of an inline element of an inline engine
                // paragraph are located from the paragraph's root.
                let (base_x, base_y) = if dom.contributes_layout_offset(idx) {
                    (abs_x, abs_y)
                } else {
                    (parent_abs_x, parent_abs_y)
                };
                for &child in node.children.iter().rev() {
                    stack.push((child, base_x, base_y));
                }
            }
        }
    } // cov:ignore: parse always synthesizes <body>, so the missing-body path is unreachable with a parsed Document

    PageScene {
        page_metadata,
        node_ids,
        fragments,
        drawables,
        root_id,
        body_id,
        body_offset_pt: (
            margins.left + insets.left + body_left_extra,
            margins.top + insets.top + body_top_extra,
        ),
        margin_boxes: cascade.page.margin_boxes().to_vec(),
        content_origin_y,
    }
}

/// The paragraph root above a node inside an ifc paragraph.
fn ifc_root_of(dom: &Document, idx: usize) -> Option<usize> {
    let mut current = dom.parent_of(idx);
    while let Some(id) = current {
        if dom.get_node(id)?.is_ifc_root() {
            return Some(id);
        }
        current = dom.parent_of(id);
    }
    None
}

/// Get an Element node's `id` attribute through the
/// [`raikiri_traits::Dom`] trait.
///
/// `Document::get_node(usize) -> &raikiri_dom::Node` (the raw arena accessor
/// used by this module's DFS) has no attribute lookup (see
/// `crates/raikiri-dom/src/node.rs`). Instead, perform a second, trait-based
/// lookup through `NodeId` (`Dom::node` → `Node::as_element` → `Element::id`;
/// see `crates/raikiri-dom/src/dom_impl.rs`). This second path is necessary
/// because the raw arena accessor and the trait-based element accessor do
/// not share element-attribute access. An empty `id=""` yields `None`, as
/// specified by the trait's default contract.
fn element_id(dom: &Document, node_id: NodeId) -> Option<String> {
    use raikiri_traits::{Dom as _, Element as _, Node as _};
    dom.node(node_id)?.as_element()?.id().map(str::to_string)
}

/// Return the first `<tag>` element found by DFS (only in-document nodes).
/// Internal helper for extracting root_id / body_id. It generalizes the
/// contract of `raikiri_paint::walk::find_body` (skip inert subtrees; accept
/// the first matching tag). This is the third copy of the find_body-shaped
/// pattern; consolidation was intentionally deferred and duplication accepted.
fn find_first_element_by_tag(dom: &Document, tag: &str) -> Option<usize> {
    let mut stack: Vec<usize> = vec![dom.root_index()];
    while let Some(idx) = stack.pop() {
        let node = dom.get_node(idx)?;
        if !node.is_in_document() {
            continue;
        }
        if node.kind() == NodeKind::Element && node.tag_name() == Some(tag) {
            return Some(idx);
        }
        for &c in node.children.iter().rev() {
            stack.push(c);
        }
    }
    None // cov:ignore: parse always synthesizes <html>/<body>; lookup fails only for hand-built Documents that cannot supply the CascadeResult this API requires
}

/// Serialize a premultiplied RGBA8 buffer to PNG bytes (via `tiny_skia::Pixmap`).
///
/// `rgba` must have exactly `width * height * 4` bytes. Both the
/// `anyrender_vello_cpu` output and `tiny_skia` use premultiplied RGBA8,
/// so wrap the buffer and serialize it.
///
/// # Panics
/// - `rgba.len() != width * height * 4`
/// - `width == 0 || height == 0` (invalid `tiny_skia::IntSize`)
/// - PNG serialization fails (not expected for a well-formed pixmap;
///   treated as an invariant violation).
fn encode_png(rgba: &[u8], width: u32, height: u32) -> Vec<u8> {
    let expected = (width as usize) * (height as usize) * 4;
    assert_eq!(
        rgba.len(),
        expected,
        "encode_png: expected {expected} bytes for {width}x{height}, got {}",
        rgba.len(),
    );
    let size =
        tiny_skia::IntSize::from_wh(width, height).expect("encode_png: width/height must be > 0");
    let pixmap = tiny_skia::Pixmap::from_vec(rgba.to_vec(), size)
        .expect("encode_png: Pixmap::from_vec rejected pre-validated buffer (tiny-skia invariant violation)");
    pixmap
        .encode_png()
        .expect("encode_png: tiny_skia::Pixmap::encode_png should not fail for a valid pixmap")
}

#[cfg(test)]
mod tests;
