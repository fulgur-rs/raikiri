//! raikiri-paint — Document + CascadeResult → anyrender::PaintScene walker.
//!
//! This crate follows the refutation in docs/feasibility-report.md §3.4:
//! it consumes `impl anyrender::PaintScene` directly, without a bridge trait.
//! It currently handles one A4 page, text glyphs, CSS Text Decoration Level 3
//! (line/style/color), text shadows, horizontal emphasis, outer box shadows,
//! and element background and border painting.
//!
//! ## Contract
//!
//! - `document` must be post-layout after `layout_single_page` returns
//!   (Node.unrounded_layout and the paragraph lines have been populated).
//! - `cascade.computed.len() == document.node_count()` (caller responsibility).
//! - The caller must call `scene.reset()` (as in blitz-paint).
//! - Counter snapshots come from [`Document::shared_counter_snapshots`], built
//!   once per document and cascade and shared by every page slice. Exceeding
//!   their limit returns `RenderError::LimitExceeded` before commands are
//!   emitted.

#![allow(rustdoc::private_intra_doc_links)]
use anyrender::PaintScene;
use raikiri_dom::Document;
use raikiri_style::CascadeResult;
use raikiri_traits::PageBox;

/// One custom-highlight range over a text node.
///
/// Offsets are UTF-8 byte boundaries in the DOM text node. The JS runtime
/// converts its UTF-16 Range offsets before transferring live selections to
/// the renderer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextHighlightRange {
    /// Name used by the matching `::highlight(name)` rule.
    pub name: String,
    /// DOM arena index of the selected text node.
    pub node: usize,
    /// Inclusive UTF-8 byte offset into the text node.
    pub start_byte: usize,
    /// Exclusive UTF-8 byte offset into the text node.
    pub end_byte: usize,
}

pub mod border;
mod capture;
mod column_rules;
#[allow(
    dead_code,
    reason = "no caller until the paint walk dispatches ifc roots"
)]
mod emphasis;
mod ifc_text;
mod standalone_text;
mod text;
mod transform;
mod walk;

/// Paint Document + CascadeResult onto one A4 (or specified PageBox) page.
///
/// # Call order
/// 1. `walk::paint_canvas_background` — fill the canvas background.
/// 2. `walk::paint_document` — walk from the body in DFS order, painting element
///    backgrounds/borders and text-node glyphs/decorations.
///
/// # Non-goals (current scope)
/// - Multi-page pagination
/// - Inset `box-shadow` painting and exact per-corner shadow radius geometry
///   (outer shadows already emit blur and spread through the scene backend)
/// - CSS Text Decoration Level 4 features (skip-ink / skip-spaces, thickness,
///   and vertical emphasis)
/// - Scrollbar painting for `overflow: scroll` / `auto` — descendants are
///   clipped, but scrollbar geometry and painting remain out of scope.
/// - z-index / stacking context
/// - CSS 3D transforms
/// - DPI scaling (planned as a separate `paint_single_page_scaled` function)
///
/// # Errors
///
/// Returns `RenderError::LimitExceeded` with
/// `LimitKind::CounterSnapshots` when the estimated counter snapshot cost
/// exceeds [`raikiri_dom::MAX_COUNTER_SNAPSHOT_ESTIMATED_BYTES`].
/// The check runs before this function emits scene commands.
///
/// # Panics
///
/// - (debug builds only) `cascade.computed.len() != document.node_count()` —
///   violates the caller-responsibility contract in this crate's module docs,
///   `## Contract` (`cascade` and `document` do not come from the same `cascade()`
///   call). Release builds omit this `debug_assert!`. Their behavior depends
///   on the direction of the mismatch: if the computed length is less than
///   `document.node_count()`, a raw-index site in `walk::paint_document` or
///   [`crate::ifc_text::draw_ifc_lines_with_resources`] (`cascade.computed[node_id]`) eventually panics
///   with "index out of bounds". If it is greater, the indices stay in bounds,
///   so painting completes without a panic while silently using computed
///   values from another document.
///
pub fn paint_single_page(
    scene: &mut impl PaintScene,
    document: &Document,
    cascade: &CascadeResult,
    page_box: PageBox,
) -> Result<(), raikiri_traits::RenderError> {
    paint_single_page_with_origin(scene, document, cascade, page_box, 0.0)
}

/// Paint one page from a document whose block flow may span several pages.
///
/// `content_origin_y` is the page's origin in the shared body-content layout.
/// The paper background is emitted at local `(0, 0)`; document boxes are
/// translated by the origin before the normal walk.  This lets callers render
/// independent page buffers without cloning the DOM or repainting another
/// page's content into the current one.
///
/// Without the number of pages, an auto-height root element is taken to end
/// with this page, so canvas images sized or placed against its height can
/// differ from page to page. Callers that know the page count should use
/// [`paint_single_page_with_origin_and_page`].
pub fn paint_single_page_with_origin(
    scene: &mut impl PaintScene,
    document: &Document,
    cascade: &CascadeResult,
    page_box: PageBox,
    content_origin_y: f32,
) -> Result<(), raikiri_traits::RenderError> {
    paint_single_page_with_origin_and_page(
        scene,
        document,
        cascade,
        page_box,
        content_origin_y,
        0,
        1,
        false,
    )
}

/// Paint one page with the page-counter context used by generated margin-box
/// content.
///
/// `page_index` is zero based and `page_count` is the final number of pages.
#[allow(clippy::too_many_arguments)]
pub fn paint_single_page_with_origin_and_page(
    scene: &mut impl PaintScene,
    document: &Document,
    cascade: &CascadeResult,
    page_box: PageBox,
    content_origin_y: f32,
    page_index: u32,
    page_count: u32,
    page_is_left: bool,
) -> Result<(), raikiri_traits::RenderError> {
    paint_single_page_with_origin_and_page_context(
        scene,
        document,
        cascade,
        page_box,
        content_origin_y,
        page_index,
        page_count,
        page_is_left,
        None,
    )
}

/// Paint one page while supplying the increment from the paired page side.
///
/// This optional context keeps parity-sensitive page counters stateful.
#[allow(clippy::too_many_arguments)]
pub fn paint_single_page_with_origin_and_page_context(
    scene: &mut impl PaintScene,
    document: &Document,
    cascade: &CascadeResult,
    page_box: PageBox,
    content_origin_y: f32,
    page_index: u32,
    page_count: u32,
    page_is_left: bool,
    paired_page_increment: Option<i32>,
) -> Result<(), raikiri_traits::RenderError> {
    paint_single_page_with_origin_and_page_context_impl(
        scene,
        document,
        cascade,
        page_box,
        content_origin_y,
        page_index,
        page_count,
        page_is_left,
        paired_page_increment,
        None,
        // No explicit width: fixed boxes resolve against this page's
        // page area, the containing block layout uses.
        f32::NAN,
        None,
        &[],
    )
}

/// Paint one page while filtering boxes whose computed named page differs from
/// the page slice being rendered.
///
/// Provide the slice's selected page name so a named class-A box is not painted
/// once on the preceding anonymous slice.
#[allow(clippy::too_many_arguments)]
pub fn paint_single_page_with_origin_and_page_context_named(
    scene: &mut impl PaintScene,
    document: &Document,
    cascade: &CascadeResult,
    page_box: PageBox,
    content_origin_y: f32,
    page_index: u32,
    page_count: u32,
    page_is_left: bool,
    paired_page_increment: Option<i32>,
    active_page_name: Option<&str>,
) -> Result<(), raikiri_traits::RenderError> {
    paint_single_page_with_origin_and_page_context_impl(
        scene,
        document,
        cascade,
        page_box,
        content_origin_y,
        page_index,
        page_count,
        page_is_left,
        paired_page_increment,
        Some(active_page_name),
        // No explicit width: fixed boxes resolve against this page's
        // page area, the containing block layout uses.
        f32::NAN,
        None,
        &[],
    )
}

/// Paint a named-page slice while using an explicit width for the
/// `position: fixed` containing block.
///
/// `fixed_page_width` is the width of the page area layout placed fixed boxes
/// against: the first page's paper width minus its margins, border and
/// padding. A non-finite or non-positive value falls back to this page's page
/// area. Named pages may change the paper width after a fixed box has been laid out.
/// This entry point keeps that fixed containing block stable.
#[allow(clippy::too_many_arguments)]
pub fn paint_single_page_with_origin_and_page_context_named_with_fixed_page_width(
    scene: &mut impl PaintScene,
    document: &Document,
    cascade: &CascadeResult,
    page_box: PageBox,
    content_origin_y: f32,
    page_index: u32,
    page_count: u32,
    page_is_left: bool,
    paired_page_increment: Option<i32>,
    active_page_name: Option<&str>,
    fixed_page_width: f32,
) -> Result<(), raikiri_traits::RenderError> {
    // cov:ignore: signature line has no executable coverage mapping.
    paint_single_page_with_origin_and_page_context_impl(
        scene,
        document,
        cascade,
        page_box,
        content_origin_y,
        page_index,
        page_count,
        page_is_left,
        paired_page_increment,
        Some(active_page_name),
        fixed_page_width,
        None,
        &[],
    )
}

/// Named-page slice paint entry point with a resolved image pixel source.
///
/// The layout caller must resolve replaced-element intrinsic sizes before
/// invoking this function. The same resolver cache can then provide decoded
/// pixels for both `<img>` and CSS `background-image` URLs.
#[allow(clippy::too_many_arguments)]
pub fn paint_single_page_with_origin_and_page_context_named_with_fixed_page_width_and_images(
    scene: &mut impl PaintScene,
    document: &Document,
    cascade: &CascadeResult,
    page_box: PageBox,
    content_origin_y: f32,
    page_index: u32,
    page_count: u32,
    page_is_left: bool,
    paired_page_increment: Option<i32>,
    active_page_name: Option<&str>,
    fixed_page_width: f32,
    pixel_source: &dyn raikiri_traits::ImagePixelSource,
) -> Result<(), raikiri_traits::RenderError> {
    // cov:ignore: signature line has no executable coverage mapping.
    paint_single_page_with_origin_and_page_context_impl(
        scene,
        document,
        cascade,
        page_box,
        content_origin_y,
        page_index,
        page_count,
        page_is_left,
        paired_page_increment,
        Some(active_page_name),
        fixed_page_width,
        Some(pixel_source),
        &[],
    )
}

/// Named-page paint entry point with custom text-highlight ranges.
#[allow(clippy::too_many_arguments)]
pub fn paint_single_page_with_origin_and_page_context_named_with_fixed_page_width_and_highlights(
    scene: &mut impl PaintScene,
    document: &Document,
    cascade: &CascadeResult,
    page_box: PageBox,
    content_origin_y: f32,
    page_index: u32,
    page_count: u32,
    page_is_left: bool,
    paired_page_increment: Option<i32>,
    active_page_name: Option<&str>,
    fixed_page_width: f32,
    custom_highlights: &[TextHighlightRange],
) -> Result<(), raikiri_traits::RenderError> {
    paint_single_page_with_origin_and_page_context_impl(
        scene,
        document,
        cascade,
        page_box,
        content_origin_y,
        page_index,
        page_count,
        page_is_left,
        paired_page_increment,
        Some(active_page_name),
        fixed_page_width,
        None,
        custom_highlights,
    )
}

/// Named-page paint entry point with both resolved images and text highlights.
#[allow(clippy::too_many_arguments)]
pub fn paint_single_page_with_origin_and_page_context_named_with_fixed_page_width_and_images_and_highlights(
    scene: &mut impl PaintScene,
    document: &Document,
    cascade: &CascadeResult,
    page_box: PageBox,
    content_origin_y: f32,
    page_index: u32,
    page_count: u32,
    page_is_left: bool,
    paired_page_increment: Option<i32>,
    active_page_name: Option<&str>,
    fixed_page_width: f32,
    pixel_source: &dyn raikiri_traits::ImagePixelSource,
    custom_highlights: &[TextHighlightRange],
) -> Result<(), raikiri_traits::RenderError> {
    paint_single_page_with_origin_and_page_context_impl(
        scene,
        document,
        cascade,
        page_box,
        content_origin_y,
        page_index,
        page_count,
        page_is_left,
        paired_page_increment,
        Some(active_page_name),
        fixed_page_width,
        Some(pixel_source),
        custom_highlights,
    )
}

#[allow(clippy::too_many_arguments)]
fn paint_single_page_with_origin_and_page_context_impl(
    scene: &mut impl PaintScene,
    document: &Document,
    cascade: &CascadeResult,
    page_box: PageBox,
    content_origin_y: f32,
    page_index: u32,
    page_count: u32,
    page_is_left: bool,
    paired_page_increment: Option<i32>,
    active_page_name: Option<Option<&str>>,
    fixed_page_width: f32,
    pixel_source: Option<&dyn raikiri_traits::ImagePixelSource>,
    custom_highlights: &[TextHighlightRange],
) -> Result<(), raikiri_traits::RenderError> {
    // The walker (`walk::paint_document` / `ifc_text::draw_ifc_lines_with_resources`) has several
    // raw-index reads of `cascade.computed[node_id]`. Each assumes the caller
    // meets the module doc's `## Contract` (`cascade.computed.len() ==
    // document.node_count()`). Without one enforcement point, a violation
    // produces a different panic depending on which raw-index site is reached
    // first (a generic "index out of bounds"). Check once at the walker's
    // entry point and fail fast with a message naming the contract.
    // This assertion is the single enforcement point.
    debug_assert!(
        cascade.computed.len() == document.node_count(),
        "cascade.computed.len() ({}) must equal document.node_count() ({}) — \
         violates this crate's module doc \"## Contract\": `cascade` and \
         `document` must come from the same `cascade()` call over the same \
         `document` (caller responsibility, not checked outside debug builds)",
        cascade.computed.len(),
        document.node_count(),
    );
    if root_generates_no_box(cascade) {
        walk::paint_blank_paper(scene, page_box);
        return Ok(());
    }
    let counter_snapshots = shared_counter_snapshots(document, cascade)?;
    let image_cache = walk::ImageBrushCache::for_document(document);
    let root = raikiri_dom::root_box(document, cascade, page_box, content_origin_y, page_count);
    let mut warnings = Vec::new();
    walk::paint_canvas_background(
        &image_cache,
        scene,
        document,
        cascade,
        page_box,
        root.as_ref(),
        pixel_source,
        &mut warnings,
    );
    walk::paint_page_border(scene, cascade, page_box);
    walk::paint_page_outline(scene, cascade, page_box);
    walk::paint_root_element_border(
        &image_cache,
        scene,
        document,
        cascade,
        page_box,
        pixel_source,
        &mut warnings,
    );
    walk::paint_page_margin_boxes(
        &image_cache,
        scene,
        document,
        cascade,
        page_box,
        page_index,
        page_count,
        page_is_left,
        paired_page_increment,
        pixel_source,
        &mut warnings,
    );
    let Some(transform) = root_content_transform(root.as_ref()) else {
        return Ok(());
    };
    // A page that follows the x axis holds the band of the flow across its
    // page area; the boxes of the pages beside it lie past its sides.
    let flow_band = raikiri_dom::pages_follow_x_axis(document, cascade).map(|_| {
        let margins = raikiri_dom::page_margins(cascade, page_box);
        let insets = raikiri_dom::page_content_insets(cascade, page_box);
        let left = f64::from(margins.left + insets.left);
        let extent = f64::from(walk::FLOW_EXTENT);
        kurbo::Rect::new(
            left,
            -extent,
            left + f64::from(insets.page_area_width(margins, page_box)),
            extent,
        )
    });
    if let Some(band) = &flow_band {
        scene.push_clip_layer(kurbo::Affine::IDENTITY, band);
    }
    let scene = &mut transform::TransformScene { scene, transform };
    if let Some(pixel_source) = pixel_source {
        walk::paint_document_with_images_and_warnings(
            &image_cache,
            scene,
            document,
            cascade,
            &counter_snapshots,
            page_box,
            content_origin_y,
            active_page_name,
            fixed_page_width,
            pixel_source,
            &mut warnings,
            custom_highlights,
            transform,
        );
    } else {
        walk::paint_document(
            &image_cache,
            scene,
            document,
            cascade,
            &counter_snapshots,
            page_box,
            content_origin_y,
            active_page_name,
            fixed_page_width,
            custom_highlights,
            transform,
        );
    }
    if flow_band.is_some() {
        scene.scene.pop_layer();
    }
    Ok(())
}

/// The document's shared counter snapshots, or the paint error for a document
/// whose snapshots exceed their memory limit.
fn shared_counter_snapshots(
    document: &Document,
    cascade: &CascadeResult,
) -> Result<std::sync::Arc<Vec<raikiri_dom::CounterSnapshot>>, raikiri_traits::RenderError> {
    document.shared_counter_snapshots(cascade).map_err(|error| {
        raikiri_traits::RenderError::LimitExceeded {
            kind: raikiri_traits::LimitKind::CounterSnapshots,
            limit: error.limit,
            actual: error.actual,
        }
    })
}

/// One step of the body walk, recorded in paint order.
#[doc(hidden)]
#[derive(Debug, Clone, PartialEq)]
pub enum PaintTraceEvent {
    /// An overflow clip on the padding box: `[x0, y0, x1, y1]` in page px.
    PushOverflowClip(usize, [f64; 4]),
    /// A multicol or fragmentainer clip.
    PushFragmentainerClip(usize),
    PopClip,
    PushOpacity(usize, f32),
    PopOpacity,
    /// The element's own box (background, border, shadow, outline).
    Box(usize),
    /// The content of a replaced element (`<img>`, `<canvas>`, inline `<svg>`).
    Replaced(usize),
    /// The lines of a paragraph root, or an anonymous cell's real source owner.
    Text(usize),
}

/// Walk the body as the painter does and return the steps in paint order,
/// drawing into a discarded scene.
#[doc(hidden)]
#[allow(clippy::too_many_arguments)]
pub fn trace_paint_order(
    document: &Document,
    cascade: &CascadeResult,
    page_box: PageBox,
    content_origin_y: f32,
    active_page_name: Option<Option<&str>>,
) -> Result<Vec<PaintTraceEvent>, raikiri_traits::RenderError> {
    // A root with `display: none` paints nothing beyond blank paper.
    if root_generates_no_box(cascade) {
        return Ok(Vec::new());
    }
    let counter_snapshots = shared_counter_snapshots(document, cascade)?;
    let mut scene = anyrender::Scene::new();
    let mut warnings = Vec::new();
    let mut events = Vec::new();
    walk::paint_document_impl(
        &crate::walk::ImageBrushCache::for_document(document),
        &mut scene,
        document,
        cascade,
        &counter_snapshots,
        page_box,
        content_origin_y,
        active_page_name,
        f32::NAN,
        None,
        &mut warnings,
        &[],
        kurbo::Affine::IDENTITY,
        Some(&mut events),
    );
    Ok(events)
}

/// Like [`paint_single_page`], but reads decoded `<img>` pixels from
/// `pixel_source` and actually paints them.
///
/// The name is `_with_images`, not `_with_resolver`, because painting receives
/// an `ImagePixelSource` (access to decoded pixels), not a `ReplacedResolver`
/// (which resolves intrinsic sizes and may fetch/decode in the process).
/// Resolution has already happened before layout
/// (`raikiri_dom::layout_single_page_with_resolver`).
pub fn paint_single_page_with_images(
    scene: &mut impl PaintScene,
    document: &Document,
    cascade: &CascadeResult,
    page_box: PageBox,
    pixel_source: &dyn raikiri_traits::ImagePixelSource,
) -> Result<(), raikiri_traits::RenderError> {
    paint_single_page_with_images_and_warnings(scene, document, cascade, page_box, pixel_source)
        .map(|_| ())
}

/// [`paint_single_page_with_images`] plus non-fatal image rasterization
/// warnings for visible elements whose source metadata was available.
pub fn paint_single_page_with_images_and_warnings(
    scene: &mut impl PaintScene,
    document: &Document,
    cascade: &CascadeResult,
    page_box: PageBox,
    pixel_source: &dyn raikiri_traits::ImagePixelSource,
) -> Result<Vec<raikiri_traits::RenderWarning>, raikiri_traits::RenderError> {
    debug_assert!(
        cascade.computed.len() == document.node_count(),
        "cascade.computed.len() ({}) must equal document.node_count() ({})",
        cascade.computed.len(),
        document.node_count(),
    );
    if root_generates_no_box(cascade) {
        walk::paint_blank_paper(scene, page_box);
        return Ok(Vec::new());
    }
    let counter_snapshots = shared_counter_snapshots(document, cascade)?;
    // Single-page only (no pagination integration, see this crate's design
    // non-goals) — mirrors `paint_single_page`'s own single-page defaults
    // (page 0 of 1, no named-page filtering, unpaired) rather than joining
    // the `paint_single_page_with_origin_and_page_context*` chain.
    let image_cache = walk::ImageBrushCache::for_document(document);
    let root = raikiri_dom::root_box(document, cascade, page_box, 0.0, 1);
    let mut warnings = Vec::new();
    walk::paint_canvas_background(
        &image_cache,
        scene,
        document,
        cascade,
        page_box,
        root.as_ref(),
        Some(pixel_source),
        &mut warnings,
    );
    walk::paint_page_border(scene, cascade, page_box);
    walk::paint_page_outline(scene, cascade, page_box);
    walk::paint_root_element_border(
        &image_cache,
        scene,
        document,
        cascade,
        page_box,
        Some(pixel_source),
        &mut warnings,
    );
    walk::paint_page_margin_boxes(
        &image_cache,
        scene,
        document,
        cascade,
        page_box,
        0,
        1,
        false,
        None,
        Some(pixel_source),
        &mut warnings,
    );
    let Some(transform) = root_content_transform(root.as_ref()) else {
        return Ok(warnings);
    };
    walk::paint_document_with_images_and_warnings(
        &image_cache,
        &mut transform::TransformScene { scene, transform },
        document,
        cascade,
        &counter_snapshots,
        page_box,
        0.0,
        None,
        // No explicit width: fixed boxes resolve against this page's
        // page area, the containing block layout uses.
        f32::NAN,
        pixel_source,
        &mut warnings,
        &[],
        transform,
    );
    Ok(warnings)
}

#[cfg(test)]
mod tests;

/// The transform the document's boxes paint through: the `transform` of the
/// root element's box `root`, which moves the whole canvas (CSS Transforms 1
/// §6), or the identity. `None` when that transform is singular, so the root
/// and its content are not rendered.
fn root_content_transform(root: Option<&raikiri_dom::RootBox>) -> Option<kurbo::Affine> {
    let Some(transform) = root.and_then(|root| root.transform) else {
        return Some(kurbo::Affine::IDENTITY);
    };
    let matrix = transform::affine(transform);
    transform::is_invertible(matrix).then_some(matrix)
}

/// Whether the root element is `display: none`, so the document generates
/// no boxes at all. Such a document prints as blank paper: the page
/// background, border and margin boxes are not drawn either, as browsers
/// do when printing it.
fn root_generates_no_box(cascade: &CascadeResult) -> bool {
    cascade.root_element_computed().display == raikiri_style::property::DisplayValue::None
}
