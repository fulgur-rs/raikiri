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
use raikiri_dom::{Document, IfcTextLines, Node};
use raikiri_style::{CascadeResult, ComputedValues, PageMarginBoxCascadeResult};
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
/// relative to the page-area origin).
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
    /// Fragment top-left x (border box, relative to the page-area origin, Pt).
    pub x: Pt,
    /// Fragment top-left y (border box, relative to the page-area origin, Pt).
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
/// - [`body_offset_pt`](PageScene::body_offset_pt): page-area origin in
///   physical page coordinates, through the page margins and insets.
///   Add it to fragment coordinates to obtain page-absolute coordinates.
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
    /// Page-area origin in physical page coordinates (Pt, Pt). The field
    /// retains its historical name; root and body offsets are already in
    /// each fragment. Add this once to obtain page-absolute coordinates.
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
    /// and return PNG bytes.
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
    /// 4. Serialize RGBA8 to PNG with `encode_png` (demultiply via `tiny_skia`,
    ///    then `png::Encoder` with `png::Compression::Fast`).
    ///
    /// # Errors
    /// Returns `RenderError::Configuration` for non-finite or non-positive
    /// dimensions and `RenderError::LimitExceeded` when the page exceeds the
    /// shared raster edge or byte budget.
    ///
    /// # Panics
    /// - The `anyrender_vello_cpu` output buffer length differs from
    ///   `width * height * 4` (invariant violation).
    /// - PNG serialization fails (not expected for a well-formed buffer).
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
                )
            },
            size.width(),
            size.height(),
        );
        paint_result?;

        Ok(encode_png(rgba, size.width(), size.height()))
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
    pub fn rasterize_with_images(
        &self,
        dom: &Document,
        cascade: &CascadeResult,
        page_box: PageBox,
        pixel_source: &dyn raikiri_traits::ImagePixelSource,
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
                )
            },
            size.width(),
            size.height(),
        );
        paint_result?;

        Ok(encode_png(rgba, size.width(), size.height()))
    }
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
/// Fragments use the page-area origin. The walk starts at the document
/// element and accumulates each box's used location exactly once, including
/// the root and body's margins, borders and padding. The retained
/// `body_offset_pt` field supplies only the page margins and page insets;
/// consumers add it to fragment coordinates to reach the physical page.
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
    let area = PageArea::of(dom, cascade, page_box, content_origin_y);
    let page_metadata = page_metadata_for(page_box, page_index, page_name);

    let root_arena_idx = dom.document_element_index();
    let root_id = root_arena_idx.map(|idx| NodeId::new(idx as u64));
    let body_id = body_node_id(dom);

    let mut node_ids: Vec<NodeId> = Vec::new();
    let mut fragments: BTreeMap<NodeId, Vec<Fragment>> = BTreeMap::new();
    let mut drawables = PageDrawables::default();

    // A root with `display: none` generates no boxes, whatever an earlier
    // layout left on its descendants, and prints blank paper.
    let root_generates_box =
        cascade.root_element_computed().display != raikiri_style::property::DisplayValue::None;
    if let Some(root_idx) = root_arena_idx.filter(|_| root_generates_box) {
        // DFS from the document element, matching the paint walk. Absolute
        // y stays in the shared document coordinate space until the fragment
        // is committed, which makes page intersection independent of nesting.
        let mut stack = vec![(root_idx, 0.0, 0.0, false, false)];
        let mut ifc = IfcParagraphs::default();
        while let Some((idx, parent_abs_x, parent_abs_y, in_viewport_fixed, inside_fixed_cb)) =
            stack.pop()
        {
            let Some(node) = dom.get_node(idx) else {
                continue; // cov:ignore: DFS stack holds only document-subtree indices, always valid in a well-formed Document
            };
            if is_skipped_by_paint(node) {
                continue;
            }
            let cv = &cascade.computed[idx];
            let (in_viewport_fixed, child_fixed_cb) = fixed_flags(
                dom,
                idx,
                node,
                cv,
                idx == root_idx,
                (in_viewport_fixed, inside_fixed_cb),
            );
            let ((abs_x, abs_y), border_box) =
                node_border_box(dom, idx, node, (parent_abs_x, parent_abs_y));
            ifc.enter_root(dom, idx, node, (abs_x, abs_y));
            let ifc_lines = ifc.lines_of(node, idx);
            let rects = ifc.node_rects(dom, idx, node, (abs_x, abs_y), border_box, ifc_lines);
            let on_page = area.rects_on_page(rects, in_viewport_fixed);

            if !on_page.is_empty() {
                let node_id = NodeId::new(idx as u64);
                node_ids.push(node_id);
                fragments
                    .entry(node_id)
                    .or_default()
                    .extend(page_fragments(page_index, on_page));

                // `crate::entries` keeps primitive snapshots; the live DOM is
                // still the paint truth until drawables become the renderer
                // input.
                match node.kind() {
                    NodeKind::Element => {
                        let entry = block_entry(dom, cv, node_id, border_box);
                        drawables.block_styles.insert(node_id, entry);
                    }
                    NodeKind::Text => {
                        let entry = paragraph_entry(ifc_lines);
                        drawables.paragraphs.insert(node_id, entry);
                    }
                    _ => {} // cov:ignore: in-document body descendants via parsing are only Element or Text (Comment/PI bits cleared, template fragments detached, DFS starts at the document element)
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
                    stack.push((child, base_x, base_y, in_viewport_fixed, child_fixed_cb));
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
        body_offset_pt: area.body_offset,
        // Nor does it draw the page's margin boxes.
        margin_boxes: if root_generates_box {
            cascade.page.margin_boxes().to_vec()
        } else {
            Vec::new()
        },
        content_origin_y,
    }
}

/// A border box in document coordinates: `(x, y, width, height)`.
type SceneRect = (Pt, Pt, Pt, Pt);

/// The inline element pieces of one paragraph, by node.
type PieceRectsByNode = HashMap<usize, Vec<SceneRect>>;

/// Where the content of one page sits.
struct PageArea {
    /// Origin of the page area in physical page coordinates.
    body_offset: (Pt, Pt),
    /// Width of the page area.
    width: Pt,
    /// Height of the page area.
    height: Pt,
    /// Whether the pages run along the x axis, and from the right when they
    /// do; `None` when they run down the y axis.
    axis: Option<bool>,
    /// How far the document coordinates of the content move to land in the
    /// page area.
    translation: (Pt, Pt),
}

impl PageArea {
    fn of(
        dom: &Document,
        cascade: &CascadeResult,
        page_box: PageBox,
        content_origin_y: Pt,
    ) -> Self {
        let margins = raikiri_dom::page_margins(cascade, page_box);
        let insets = raikiri_dom::page_content_insets(cascade, page_box);
        let height = (margins.content_height(page_box) - insets.top - insets.bottom).max(0.0);
        let width = insets.page_area_width(margins, page_box);
        let axis = raikiri_dom::pages_follow_x_axis(dom, cascade);
        PageArea {
            body_offset: (margins.left + insets.left, margins.top + insets.top),
            width,
            height,
            axis,
            translation: scene_translation(dom, axis, content_origin_y, width),
        }
    }

    /// The rectangles that reach the page area once moved into it. A box
    /// fixed to the viewport stays where it is.
    fn rects_on_page(&self, rects: Vec<SceneRect>, in_viewport_fixed: bool) -> Vec<SceneRect> {
        let (dx, dy) = if in_viewport_fixed {
            (0.0, 0.0)
        } else {
            self.translation
        };
        rects
            .into_iter()
            .map(|(x, y, w, h)| (x + dx, y + dy, w, h))
            .filter(|&(x, y, w, h)| {
                let (start, extent, end) = if self.axis.is_some() {
                    (x, w.max(0.0), self.width)
                } else {
                    (y, h.max(0.0), self.height)
                };
                if extent == 0.0 {
                    start >= 0.0 && start <= end
                } else {
                    start < end && start + extent > 0.0
                }
            })
            .collect()
    }
}

/// How far the document coordinates of the page's content move to land at
/// the origin of the page area, along the axis the pages run on.
fn scene_translation(
    dom: &Document,
    page_axis: Option<bool>,
    content_origin_y: Pt,
    content_width: Pt,
) -> (Pt, Pt) {
    match page_axis {
        None => (0.0, -content_origin_y),
        Some(false) => (-content_origin_y, 0.0),
        Some(true) => (
            content_origin_y + content_width
                - dom.laid_out_page_area_width().unwrap_or(content_width),
            0.0,
        ),
    }
}

/// The size, index, name and orientation of a page.
fn page_metadata_for(
    page_box: PageBox,
    page_index: u32,
    page_name: Option<String>,
) -> PageMetadata {
    PageMetadata {
        size: (page_box.width, page_box.height),
        page_index,
        page_name,
        orientation: if page_box.width > page_box.height {
            Orientation::Landscape
        } else {
            Orientation::Portrait
        },
    }
}

/// The `<body>` that paint takes as the root of the page content, if any.
fn body_node_id(dom: &Document) -> Option<NodeId> {
    raikiri_dom::paint_rules::find_paint_root(dom)
        .filter(|&idx| dom.get_node(idx).and_then(|node| node.tag_name()) == Some("body"))
        .map(|idx| NodeId::new(idx as u64))
}

/// Whether paint skips the node and its subtree: outside the document, not
/// rendered, `display: none`, or removed by a `text-overflow` ellipsis.
fn is_skipped_by_paint(node: &Node) -> bool {
    !node.is_in_document()
        || node.is_non_rendered_html_element()
        || node.is_display_none()
        || node.is_hidden_by_text_overflow()
}

/// Whether a node is fixed to the viewport and whether its descendants have a
/// fixed containing block, given the same two flags of its parent.
fn fixed_flags(
    dom: &Document,
    idx: usize,
    node: &Node,
    cv: &ComputedValues,
    is_root: bool,
    (in_viewport_fixed, inside_fixed_cb): (bool, bool),
) -> (bool, bool) {
    let element_box = node.kind() == NodeKind::Element
        && (is_root || raikiri_dom::paint_rules::generates_box(cv));
    (
        in_viewport_fixed
            || (element_box
                && !inside_fixed_cb
                && cv.position == raikiri_style::property::PositionValue::Fixed),
        inside_fixed_cb
            || (element_box && raikiri_dom::establishes_fixed_containing_block(dom, idx, cv)),
    )
}

/// The origin of a node and its border box in document coordinates.
fn node_border_box(
    dom: &Document,
    idx: usize,
    node: &Node,
    (parent_abs_x, parent_abs_y): (Pt, Pt),
) -> ((Pt, Pt), SceneRect) {
    let layout = node.unrounded_layout;
    let (abs_x, abs_y) = dom
        .layout_origin_of(idx)
        .map(|origin| (origin.x, origin.y))
        .unwrap_or((
            parent_abs_x + layout.location.x,
            parent_abs_y + layout.location.y,
        ));
    // Descendants keep the wrapper origin; only the table's own
    // border box and style snapshot exclude the caption area.
    let border_box = node.table_grid_box().map_or(
        (abs_x, abs_y, layout.size.width, layout.size.height),
        |rect| (abs_x + rect.x, abs_y + rect.y, rect.width, rect.height),
    );
    ((abs_x, abs_y), border_box)
}

/// Content-box origin and inline element pieces of every paragraph laid out
/// by the inline engine, by root. Its text nodes have no layout of their own,
/// and its inline elements are recorded as one rectangle per line; both are
/// measured from the root's content box. A root is visited before its
/// descendants.
#[derive(Default)]
struct IfcParagraphs {
    /// Where the content box of each paragraph starts.
    origin: HashMap<usize, (Pt, Pt)>,
    /// The content width and whether lines stack right to left, for each
    /// vertical paragraph.
    vertical: HashMap<usize, (Pt, bool)>,
    /// The inline element pieces of each paragraph.
    pieces: HashMap<usize, PieceRectsByNode>,
    /// The lines each text node of a paragraph shows up on.
    text_lines: HashMap<usize, IfcTextLines>,
}

impl IfcParagraphs {
    /// Record the paragraph that `node` roots, if it roots one; `abs` is
    /// where the node sits.
    fn enter_root(&mut self, dom: &Document, idx: usize, node: &Node, (abs_x, abs_y): (Pt, Pt)) {
        if !node.is_ifc_root() {
            return;
        }
        let layout = node.unrounded_layout;
        let origin = (
            abs_x + layout.border.left + layout.padding.left,
            abs_y + layout.border.top + layout.padding.top,
        );
        self.origin.insert(idx, origin);
        if let Some(mode) = node.ifc_writing_mode().filter(|mode| mode.is_vertical()) {
            self.vertical.insert(
                idx,
                (
                    layout.content_box_width(),
                    matches!(
                        mode,
                        shodo::geometry::WritingMode::VerticalRl
                            | shodo::geometry::WritingMode::SidewaysRl
                    ),
                ),
            );
        }
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
        self.pieces.insert(idx, by_node);
        self.text_lines.extend(dom.ifc_text_lines_by_node(idx));
    }

    /// The lines a text node shows up on in its paragraph.
    fn lines_of(&self, node: &Node, idx: usize) -> Option<&IfcTextLines> {
        if node.kind() == NodeKind::Text {
            self.text_lines.get(&idx)
        } else {
            None
        }
    }

    /// Border boxes of the node in document coordinates: one for most
    /// nodes, one per line for an inline element of an ifc paragraph.
    fn node_rects(
        &self,
        dom: &Document,
        idx: usize,
        node: &Node,
        (abs_x, abs_y): (Pt, Pt),
        border_box: SceneRect,
        ifc_lines: Option<&IfcTextLines>,
    ) -> Vec<SceneRect> {
        if let Some(owned) = ifc_lines {
            let (root_x, root_y) = self
                .origin
                .get(&owned.root)
                .copied()
                .unwrap_or((abs_x, abs_y));
            let first_top = owned.lines.first().map_or(0.0, |l| l.top);
            let last_bottom = owned.lines.last().map_or(0.0, |l| l.bottom);
            let block_size = (last_bottom - first_top).max(0.0);
            if let Some(&(content_width, right_to_left)) = self.vertical.get(&owned.root) {
                let x = if right_to_left {
                    root_x + content_width - last_bottom
                } else {
                    root_x + first_top
                };
                vec![(x, root_y, block_size, owned.width)]
            } else {
                vec![(root_x, root_y + first_top, owned.width, block_size)]
            }
        } else {
            let pieces: Vec<SceneRect> = if node.in_ifc_subtree() {
                ifc_root_of(dom, idx)
                    .and_then(|root| self.pieces.get(&root))
                    .and_then(|pieces| pieces.get(&idx))
                    .cloned()
                    .unwrap_or_default()
            } else {
                Vec::new()
            };
            if pieces.is_empty() {
                vec![border_box]
            } else {
                pieces
            }
        }
    }
}

/// The snapshot of a text node: a text node outside every paragraph has no
/// lines.
fn paragraph_entry(ifc_lines: Option<&IfcTextLines>) -> ParagraphEntry {
    ParagraphEntry {
        line_count: ifc_lines.map_or(0, |owned| owned.lines.len()),
        ..ParagraphEntry::default()
    }
}

/// One fragment of the page per rectangle.
fn page_fragments(page_index: u32, rects: Vec<SceneRect>) -> impl Iterator<Item = Fragment> {
    rects
        .into_iter()
        .map(move |(x, y, width, height)| Fragment {
            page_index,
            x,
            y,
            width,
            height,
        })
}

/// The primitive snapshot of an element's style and size.
fn block_entry(
    dom: &Document,
    cv: &ComputedValues,
    node_id: NodeId,
    border_box: SceneRect,
) -> BlockEntry {
    BlockEntry {
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
        layout_size: Some((border_box.2, border_box.3)),
        ..BlockEntry::default()
    }
}

/// The paragraph root above a node inside an ifc paragraph.
fn ifc_root_of(dom: &Document, idx: usize) -> Option<usize> {
    dom.ancestors(idx)
        .find(|&id| dom.get_node(id).is_some_and(|node| node.is_ifc_root()))
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

/// Serialize a premultiplied RGBA8 buffer to PNG bytes.
///
/// `rgba` must have exactly `width * height * 4` bytes. The
/// `anyrender_vello_cpu` output is premultiplied RGBA8, while PNG stores
/// straight alpha, so the buffer is demultiplied in place with
/// `tiny_skia::Pixmap::take_demultiplied` (the same rounding
/// `tiny_skia::Pixmap::encode_png` uses, so decoded pixels are unchanged).
/// Taking ownership of `rgba` avoids the two whole-page copies the
/// `Pixmap::encode_png` route makes.
///
/// The encoder uses `png::Compression::Fast` (the `fdeflate` path) instead of
/// the default `Balanced` zlib level. For a full A4 page this is several
/// times faster to encode, at the cost of a larger file. The output is still
/// deterministic for a given buffer.
///
/// # Panics
/// - `rgba.len() != width * height * 4`
/// - `width == 0 || height == 0` (invalid `tiny_skia::IntSize`)
/// - PNG serialization fails (not expected for a well-formed buffer;
///   treated as an invariant violation).
fn encode_png(rgba: Vec<u8>, width: u32, height: u32) -> Vec<u8> {
    let expected = (width as usize) * (height as usize) * 4;
    assert_eq!(
        rgba.len(),
        expected,
        "encode_png: expected {expected} bytes for {width}x{height}, got {}",
        rgba.len(),
    );
    let size =
        tiny_skia::IntSize::from_wh(width, height).expect("encode_png: width/height must be > 0");
    let demultiplied = tiny_skia::Pixmap::from_vec(rgba, size)
        .expect("encode_png: Pixmap::from_vec rejected pre-validated buffer (tiny-skia invariant violation)")
        .take_demultiplied();

    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_compression(png::Compression::Fast);
    let mut writer = encoder
        .write_header()
        .expect("encode_png: writing the PNG header to a Vec should not fail");
    writer
        .write_image_data(&demultiplied)
        .expect("encode_png: writing PNG image data to a Vec should not fail");
    writer
        .finish()
        .expect("encode_png: finishing the PNG stream should not fail");
    out
}

#[cfg(test)]
mod tests;
