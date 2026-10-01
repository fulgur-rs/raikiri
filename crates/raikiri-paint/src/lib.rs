//! raikiri-paint — Document + CascadeResult → anyrender::PaintScene walker.
//!
//! This crate follows the refutation in docs/feasibility-report.md §3.4:
//! it consumes `impl anyrender::PaintScene` directly, without a bridge trait.
//! It currently handles one A4 page, text glyphs, CSS Text Decoration Level 3
//! (line/style/color), and minimal element background and border painting.
//!
//! ## Contract
//!
//! - `document` must be post-layout after `layout_single_page` returns
//!   (Node.unrounded_layout and the paragraph lines have been populated).
//! - `cascade.computed.len() == document.node_count()` (caller responsibility).
//! - The caller must call `scene.reset()` (as in blitz-paint).
//! - Infallible: raikiri-traits::RenderError has no Paint variant because
//!   consuming an already shaped and laid-out Document cannot fail.

#![allow(rustdoc::private_intra_doc_links)]
use anyrender::PaintScene;
use raikiri_dom::Document;
use raikiri_style::CascadeResult;
use raikiri_traits::PageBox;

pub mod border;
#[allow(
    dead_code,
    reason = "no caller until the paint walk dispatches ifc roots"
)]
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
/// - Advanced element `box-shadow` painting (inset shadows and non-zero blur,
///   spread, or radius interactions remain deferred follow-up work)
/// - CSS Text Decoration Level 4 features (skip-ink / skip-spaces, thickness,
///   emphasis, text-shadow, and vertical writing)
/// - Scrollbar painting for `overflow: scroll` / `auto` — descendants are
///   clipped, but scrollbar geometry and painting remain out of scope.
/// - z-index / stacking context
/// - CSS 3D transforms
/// - DPI scaling (planned as a separate `paint_single_page_scaled` function)
///
/// # Panics
///
/// - (debug builds only) `cascade.computed.len() != document.node_count()` —
///   violates the caller-responsibility contract in this crate's module docs,
///   `## Contract` (`cascade` and `document` do not come from the same `cascade()`
///   call). Release builds omit this `debug_assert!`. Their behavior depends
///   on the direction of the mismatch: if the computed length is less than
///   `document.node_count()`, a raw-index site in `walk::paint_document` or
///   `text::draw_text_node` (`cascade.computed[node_id]`) eventually panics
///   with "index out of bounds". If it is greater, the indices stay in bounds,
///   so painting completes without a panic while silently using computed
///   values from another document.
///
pub fn paint_single_page(
    scene: &mut impl PaintScene,
    document: &Document,
    cascade: &CascadeResult,
    page_box: PageBox,
) {
    paint_single_page_with_origin(scene, document, cascade, page_box, 0.0);
}

/// Paint one page from a document whose block flow may span several pages.
///
/// `content_origin_y` is the page's origin in the shared body-content layout.
/// The paper background is emitted at local `(0, 0)`; document boxes are
/// translated by the origin before the normal walk.  This lets callers render
/// independent page buffers without cloning the DOM or repainting another
/// page's content into the current one.
pub fn paint_single_page_with_origin(
    scene: &mut impl PaintScene,
    document: &Document,
    cascade: &CascadeResult,
    page_box: PageBox,
    content_origin_y: f32,
) {
    paint_single_page_with_origin_and_page(
        scene,
        document,
        cascade,
        page_box,
        content_origin_y,
        0,
        1,
        false,
    );
}

/// Paint one page with the page-counter context used by generated margin-box
/// content.
///
/// `page_index` is zero based and `page_count` is the final number of pages.
/// The older [`paint_single_page_with_origin`] entry point remains equivalent
/// to page zero of a one-page document.
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
) {
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
    );
}

/// Paint one page while supplying the increment from the paired page side.
///
/// This optional context keeps parity-sensitive page counters stateful without
/// changing the established page-paint entry point.
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
) {
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
        page_box.width,
        None,
    );
}

/// Paint one page while filtering boxes whose computed named page differs from
/// the page slice being rendered.
///
/// The ordinary context entry point remains unchanged for existing callers;
/// paged consumers should provide the slice's selected page name so a named
/// class-A box is not painted once on the preceding anonymous slice.
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
) {
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
        page_box.width,
        None,
    );
}

/// Paint a named-page slice while using an explicit initial viewport width for
/// `position: fixed` containing-block resolution.
///
/// Named pages may change the paper width after a fixed box has been laid out.
/// This entry point keeps that fixed containing block stable without changing
/// the established paint API above.
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
) {
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
    );
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
) {
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
    );
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
) {
    // The walker (`walk::paint_document` / `text::draw_text_node`) has several
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
    let mut warnings = Vec::new();
    walk::paint_canvas_background(
        scene,
        document,
        cascade,
        page_box,
        pixel_source,
        &mut warnings,
    );
    walk::paint_page_border(scene, cascade, page_box);
    walk::paint_page_outline(scene, cascade, page_box);
    walk::paint_root_element_border(scene, document, cascade, page_box);
    walk::paint_page_margin_boxes(
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
    if let Some(pixel_source) = pixel_source {
        walk::paint_document_with_images_and_warnings(
            scene,
            document,
            cascade,
            page_box,
            content_origin_y,
            active_page_name,
            fixed_page_width,
            pixel_source,
            &mut warnings,
        );
    } else {
        walk::paint_document(
            scene,
            document,
            cascade,
            page_box,
            content_origin_y,
            active_page_name,
            fixed_page_width,
        );
    }
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
) {
    let _ = paint_single_page_with_images_and_warnings(
        scene,
        document,
        cascade,
        page_box,
        pixel_source,
    );
}

/// [`paint_single_page_with_images`] plus non-fatal image rasterization
/// warnings for visible elements whose source metadata was available.
pub fn paint_single_page_with_images_and_warnings(
    scene: &mut impl PaintScene,
    document: &Document,
    cascade: &CascadeResult,
    page_box: PageBox,
    pixel_source: &dyn raikiri_traits::ImagePixelSource,
) -> Vec<raikiri_traits::RenderWarning> {
    debug_assert!(
        cascade.computed.len() == document.node_count(),
        "cascade.computed.len() ({}) must equal document.node_count() ({})",
        cascade.computed.len(),
        document.node_count(),
    );
    // Single-page only (no pagination integration, see this crate's design
    // non-goals) — mirrors `paint_single_page`'s own single-page defaults
    // (page 0 of 1, no named-page filtering, unpaired) rather than joining
    // the `paint_single_page_with_origin_and_page_context*` chain.
    let mut warnings = Vec::new();
    walk::paint_canvas_background(
        scene,
        document,
        cascade,
        page_box,
        Some(pixel_source),
        &mut warnings,
    );
    walk::paint_page_border(scene, cascade, page_box);
    walk::paint_page_outline(scene, cascade, page_box);
    walk::paint_root_element_border(scene, document, cascade, page_box);
    walk::paint_page_margin_boxes(
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
    walk::paint_document_with_images_and_warnings(
        scene,
        document,
        cascade,
        page_box,
        0.0,
        None,
        page_box.width,
        pixel_source,
        &mut warnings,
    );
    warnings
}

#[cfg(test)]
mod tests;
