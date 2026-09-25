//! raikiri-paint — Document + CascadeResult → anyrender::PaintScene walker.
//!
//! この crate の設計は docs/feasibility-report.md §3.4 の refute 結果に従い、
//! bridge trait を挟まず `impl anyrender::PaintScene` を直接消費する。現状は
//! 単一 A4 ページ + text glyphs + CSS Text Decoration Level 3
//! (line/style/color) と、element background / border の最小描画を扱う。
//!
//! ## Contract
//!
//! - `document` は `layout_single_page` を呼び終えた post-layout 状態を前提
//!   (Node.unrounded_layout / Node.text_layout populate 済)
//! - `cascade.computed.len() == document.node_count()` を前提 (caller 責任)
//! - `scene.reset()` の呼び出しは caller 責任 (blitz-paint と同じ convention)
//! - infallible — raikiri-traits::RenderError に Paint variant はない
//!   (pre-shape / layout 済の Document 消費が原理的 infallible)

#![allow(rustdoc::private_intra_doc_links)]
use anyrender::PaintScene;
use raikiri_dom::Document;
use raikiri_style::CascadeResult;
use raikiri_traits::PageBox;

pub mod border;
mod text;
mod walk;

/// 単一 A4 (or 指定 PageBox) ページに Document + CascadeResult を paint する。
///
/// # 呼び出し順序
/// 1. `walk::paint_canvas_background` — canvas 背景 fill
/// 2. `walk::paint_document` — body から始まる DFS walk、element background/border と
///    text node の glyph/decoration を描画
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
/// - CSS transform (rotate/scale/skew)
/// - DPI scaling (`paint_single_page_scaled` 別関数で将来拡張予定)
///
/// # Panics
///
/// - (debug build のみ) `cascade.computed.len() != document.node_count()` —
///   この crate の module doc `## Contract` の caller-responsibility 契約
///   違反 (`cascade` と `document` が同じ `cascade()` 呼び出しに由来しない)。
///   release build ではこの `debug_assert!` 自体が消える。その場合の挙動は
///   違反の方向で異なる: `cascade.computed.len() < document.node_count()`
///   なら walk 中の raw index site (`walk::paint_document` /
///   `text::draw_text_node` の `cascade.computed[node_id]`) が in-bounds を
///   超えて "index out of bounds" で panic するが、逆方向
///   (`cascade.computed.len() > document.node_count()`) は同じ index が
///   常に in-bounds のまま残るため panic せず、別 document の computed
///   values を silent に誤用したまま paint が完了する。
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
    // walk 本体 (`walk::paint_document` / `text::draw_text_node`) は
    // `cascade.computed[node_id]` を raw index で読む複数 site を持ち、それぞれが
    // この crate の module doc `## Contract` (`cascade.computed.len() ==
    // document.node_count()`) を caller 責任として前提にしている。単一の
    // enforcement point が無いと、契約違反時にどの raw-index site が最初に
    // 踏むかで panic message が変わってしまう (generic な "index out of
    // bounds")。walk 全体の入口であるここで一度だけ検査し、契約を名指しした
    // message で fail-fast させる。
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

/// [`paint_single_page`] と同一だが、`<img>` の decode 済み pixel を
/// `pixel_source` から取得して実際に描画する。
///
/// 名前が `_with_images` であって `_with_resolver` でないのは、paint 段が
/// 受け取るのが `ImagePixelSource` (decode 済み pixel の読み出し口) であり、
/// `ReplacedResolver` (intrinsic size を解決し、その過程で fetch/decode を
/// 起こしうる) ではないため。resolve は layout 前に済んでいる
/// (`raikiri_dom::layout_single_page_with_resolver`)。
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
