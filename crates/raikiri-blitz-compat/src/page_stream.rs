//! fulgur PageStream migration implementation — raikiri PageScene based replacement for blitz_adapter.
//!
//! `blitz_adapter::parse_and_layout` (screen-first blitz pipeline: parse → style → layout → paint)
//! を raikiri の streaming pipeline (`plan` / `render_streaming` / `render_batch`) 相当へ
//! 段階的に置換するための最小実装。現時点の raikiri は single-page `layout_single_page`
//! + `PageScene` まで実装済みで `plan`/`render_streaming` は 未実装のため、本モジュールは
//!   その single-page path を `PageFragment`/`PageStream` の将来 shape に見立ててラップする。
//!
//! # Architecture
//!
//! ```text
//! fulgur blitz_adapter::parse_and_layout(html, viewport)  (before)
//!        │
//!        └─► raikiri: parse → cascade → layout_single_page → build_page_scene → PageScene (after)
//!                                    │
//!                                    └─► PageScene::rasterize → PNG (visual verification)
//! ```
//!
//! # Viewport → PageBox mapping
//!
//! `Viewport::window_size` (physical px) を `Viewport::scale()` (hidpi * zoom) で割って
//! CSS px の `PageBox` に変換する。`window_size == (0,0)` または `scale == 0` の場合は
//! `PageBox::A4` にフォールバック ( fulgur が viewport 未設定時に A4 相当でレイアウトする
//! 挙動に合わせる )。

use raikiri::{FontContext, PageBox, PageScene, build_cascaded, build_page_scene_for_page_named};
use raikiri_html::{ParseOptions, parse as html_parse};
use raikiri_traits::RenderError;

use crate::shell::Viewport;

/// Single-page `PageStream` compatibility wrapper — `Vec<PageScene>` を streaming iterator 風にラップする.
///
/// 将来の `PageStream` は `PageFragment` を逐次 `RenderSink::accept_page` へ流す
/// streaming state machine だが、現時点の実装 は single-pass で全ページを先に確定して
/// `Vec` に保持する ( single page のみをサポートする `layout_single_page` の制約に由来 )。
/// Consumer は `pages()` / `into_pages()` / `Iterator` 経由で page を取得できる。
#[derive(Debug, Default)]
pub struct RaikiriPageStream {
    pages: Vec<PageScene>,
    cursor: usize,
}

impl RaikiriPageStream {
    /// `html` を parse → cascade → layout し `PageScene` の stream を生成する。
    ///
    /// `viewport` が `Some` かつ `window_size != (0,0)` の場合は viewport から
    /// `PageBox` を導出する ( `window_size / scale` )。`None` またはゼロサイズの
    /// 場合は `PageBox::A4` を使用する。
    pub fn from_html(html: &str, viewport: Option<Viewport>) -> Result<Self, RenderError> {
        let page_box = viewport
            .as_ref()
            .map(viewport_to_page_box)
            .unwrap_or_default();
        let pages = html_to_page_scenes(html, page_box)?;
        Ok(Self { pages, cursor: 0 })
    }

    /// Borrow all pages.
    pub fn pages(&self) -> &[PageScene] {
        &self.pages
    }

    /// Consume and return pages.
    pub fn into_pages(self) -> Vec<PageScene> {
        self.pages
    }

    /// Number of pages ( currently always 0 or 1 for single-page implementation ).
    pub fn len(&self) -> usize {
        self.pages.len()
    }

    /// Whether the stream is empty.
    pub fn is_empty(&self) -> bool {
        self.pages.is_empty()
    }
}

impl Iterator for RaikiriPageStream {
    type Item = PageScene;
    fn next(&mut self) -> Option<Self::Item> {
        if self.cursor < self.pages.len() {
            // Clone because PageScene is cheap to clone for single-page implementation.
            // Streaming impl will yield owned PageFragment without cloning.
            let page = self.pages[self.cursor].clone();
            self.cursor += 1;
            Some(page)
        } else {
            None
        }
    }
}

/// Viewport → PageBox 変換 ( physical px / scale → CSS px )。
///
/// `window_size == (0,0)` または `scale == 0.0` の場合は `PageBox::A4` を返す。
pub fn viewport_to_page_box(viewport: &Viewport) -> PageBox {
    let (w, h) = viewport.window_size;
    if w == 0 || h == 0 {
        return PageBox::A4;
    }
    let scale = viewport.scale();
    if scale == 0.0 || !scale.is_finite() {
        return PageBox::A4;
    }
    let mut pb = PageBox::new();
    pb.width = w as f32 / scale;
    pb.height = h as f32 / scale;
    pb
}

struct PageRenderData {
    dom: raikiri_dom::Document,
    pages: Vec<(PageScene, raikiri::CascadeResult, PageBox)>,
}

fn map_initial_page_context_error(error: raikiri_dom::InitialPageContextError) -> RenderError {
    match error {
        raikiri_dom::InitialPageContextError::Layout(error) => RenderError::Layout(error),
        raikiri_dom::InitialPageContextError::PageGeometryDidNotConverge { iterations } => {
            RenderError::PageGeometryDidNotConverge { iterations }
        }
    }
}

fn layout_page_render_data(html: &str, page_box: PageBox) -> Result<PageRenderData, RenderError> {
    let opts = ParseOptions {
        extra_stylesheets: &[],
        network: None,
        base_url: None,
    };
    let mut uncascaded = html_parse(html.as_bytes(), &opts).map_err(RenderError::Parse)?;
    let mut first_query = raikiri::PageContextQuery::default();
    first_query.is_first = true;
    first_query.is_right = true;
    let default_cascade = raikiri::build_cascaded_for_page(&uncascaded, &first_query);
    if let Some(name) = raikiri::first_page_name(&uncascaded.dom, &default_cascade) {
        first_query.page_name = Some(raikiri::Atom::from(name.as_str()));
    }
    let first_cascade = if first_query.page_name.is_some() {
        raikiri::build_cascaded_for_page(&uncascaded, &first_query)
    } else {
        default_cascade
    };
    let first_page_box = if first_cascade.page.size().is_some() {
        PageBox::from_page_size(first_cascade.page.size())
    } else {
        page_box
    };
    let initial_context = raikiri_dom::resolve_initial_page_context(
        &uncascaded.dom,
        first_query.page_name.as_ref().map(ToString::to_string),
        first_cascade,
        first_page_box,
        FontContext::new(),
        raikiri_dom::InitialPageProbeResources::default(),
        |page_name| {
            first_query.page_name = page_name.map(raikiri::Atom::from);
            let cascade = raikiri::build_cascaded_for_page(&uncascaded, &first_query);
            let page_box = if cascade.page.size().is_some() {
                PageBox::from_page_size(cascade.page.size())
            } else {
                page_box
            };
            (cascade, page_box, FontContext::new())
        },
    )
    .map_err(map_initial_page_context_error)?;
    let first_cascade = initial_context.cascade;
    let first_page_box = initial_context.page_box;
    let slices = raikiri_dom::layout_pages(
        &mut uncascaded.dom,
        &first_cascade,
        first_page_box,
        initial_context.font_context,
    )
    .map_err(RenderError::Layout)?;

    let mut pages = Vec::with_capacity(slices.len());
    for slice in slices {
        let mut query = raikiri::PageContextQuery::default();
        query.page_name = slice
            .page_name
            .clone()
            .map(|name| raikiri::Atom::from(name.as_str()));
        query.is_first = slice.page_index == 0;
        query.is_right = slice.page_index % 2 == 0;
        query.is_left = !query.is_right;
        let cascade = raikiri::build_cascaded_for_page(&uncascaded, &query);
        let effective_page_box = if cascade.page.size().is_some() {
            PageBox::from_page_size(cascade.page.size())
        } else {
            first_page_box
        };
        let scene = build_page_scene_for_page_named(
            &uncascaded.dom,
            &cascade,
            effective_page_box,
            slice.page_index,
            slice.content_origin_y,
            slice.page_name,
        );
        pages.push((scene, cascade, effective_page_box));
    }
    Ok(PageRenderData {
        dom: uncascaded.dom,
        pages,
    })
}

/// `html` を `PageBox` でレイアウトし、ページごとの `PageScene` を生成する。
///
/// 既存の single-page caller は先頭 scene を使える。複数ページ caller は
/// returned vector を順番に処理する。
///
/// # Errors
/// - `RenderError::Parse` — HTML parse 失敗
/// - `RenderError::Layout` — layout 失敗 ( body 欠落 / taffy error / parley shape error )
pub fn html_to_page_scenes(html: &str, page_box: PageBox) -> Result<Vec<PageScene>, RenderError> {
    Ok(layout_page_render_data(html, page_box)?
        .pages
        .into_iter()
        .map(|(scene, _, _)| scene)
        .collect())
}

/// Render every paginated page as a separate PNG.
///
/// The existing [`html_to_png_via_page_stream`] remains a compatibility helper
/// that returns only the first page. New consumers should use this function
/// when the document may contain forced page breaks.
pub fn html_to_png_pages_via_page_stream(
    html: &str,
    page_box: PageBox,
) -> Result<Vec<Vec<u8>>, RenderError> {
    let data = layout_page_render_data(html, page_box)?;
    Ok(data
        .pages
        .iter()
        .map(|(scene, cascade, page_box)| scene.rasterize(&data.dom, cascade, *page_box))
        .collect())
}

/// `blitz_adapter::parse_and_layout` 相当を raikiri で置換する 実装関数。
///
/// `html` と `viewport` から `Vec<PageScene>` を得る最短経路。fulgur 側は
/// 本関数を `blitz_adapter::parse_and_layout` の代替として呼び出せる
/// ( 戻り値型のみ `Vec<Document>` / `blitz_dom::Document` から `Vec<PageScene>`
/// へ変わるが、viewport からの PageBox 導出・フォント解決等の integration logic は
/// 本関数内で完結するため call-site の変更は最小 )。
///
/// 将来 `PageFragment` ( `raikiri_traits::PageFragment` ) が populate された際、
/// 本関数は `Vec<PageScene>` から `Vec<PageFragment>` への thin map に
/// 昇格する。現時点の `PageFragment` は empty value のため、
/// PageScene を直接返す方が visual verification に有用である。
///
/// # Example
/// ```rust
/// use raikiri_blitz_compat::page_stream::parse_and_layout_with_raikiri;
/// let pages = parse_and_layout_with_raikiri("<h1>Hi</h1><p>body</p>", None).expect("layout");
/// assert_eq!(pages.len(), 1);
/// assert!(pages[0].body_id.is_some());
/// ```
pub fn parse_and_layout_with_raikiri(
    html: &str,
    viewport: Option<Viewport>,
) -> Result<Vec<PageScene>, RenderError> {
    RaikiriPageStream::from_html(html, viewport).map(|s| s.into_pages())
}

/// `PageScene::rasterize` を使って `PageScene` を PNG bytes に変換する helper。
///
/// `html_to_page_scenes` が生成した `PageScene` から visual verification 用
/// PNG を得る最短経路。`PageScene` が保持する `PageDrawables` 自体は現時点
/// で paint に直接消費されない ( `rasterize` は `dom` + `cascade` を thread して
/// `raikiri_paint::paint_single_page` を verbatim call する ) が、PNG 出力
/// は byte-identical に保証される ( `raikiri` crate の `PageScene::rasterize`
/// doc 参照 )。
pub fn html_to_png_via_page_stream(html: &str, page_box: PageBox) -> Result<Vec<u8>, RenderError> {
    let opts = ParseOptions {
        extra_stylesheets: &[],
        network: None,
        base_url: None,
    };
    let mut uncascaded = html_parse(html.as_bytes(), &opts).map_err(RenderError::Parse)?;
    let cascade = build_cascaded(&uncascaded);
    let font_ctx = FontContext::new();
    // Clone for rasterize after layout ( PageScene holds snapshot but rasterize still needs dom+cascade )
    let slices = raikiri_dom::layout_pages(&mut uncascaded.dom, &cascade, page_box, font_ctx)
        .map_err(RenderError::Layout)?;
    let slice = slices.first().cloned().unwrap_or(raikiri_dom::PageSlice {
        page_index: 0,
        content_origin_y: 0.0,
        page_name: None,
    });
    let scene = build_page_scene_for_page_named(
        &uncascaded.dom,
        &cascade,
        page_box,
        slice.page_index,
        slice.content_origin_y,
        slice.page_name,
    );
    Ok(scene.rasterize(&uncascaded.dom, &cascade, page_box))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_page_context_errors_map_to_render_errors() {
        let layout_error = map_initial_page_context_error(
            raikiri_dom::InitialPageContextError::Layout(raikiri_traits::LayoutError::Internal {
                message: "test".to_owned(),
            }),
        );
        assert!(matches!(layout_error, RenderError::Layout(_)));
        assert!(matches!(
            map_initial_page_context_error(
                raikiri_dom::InitialPageContextError::PageGeometryDidNotConverge { iterations: 3 }
            ),
            RenderError::PageGeometryDidNotConverge { iterations: 3 }
        ));
    }

    const RECEIPT_HTML: &str = r#"<!DOCTYPE html><html><body>
        <h1>Receipt</h1>
        <p>Thank you for your purchase.</p>
        <table><tr><td>Item</td><td>Price</td></tr><tr><td>Widget</td><td>$10</td></tr></table>
    </body></html>"#;

    const TWO_PAGE_BREAK_HTML: &str = r#"<!DOCTYPE html><html><head><style>
        @page { margin: 0 }
        body { margin: 0 }
        .first { height: 20px; break-after: page }
        .second { height: 20px }
    </style></head><body>
        <div class="first">first</div><div class="second">second</div>
    </body></html>"#;

    #[test]
    fn forced_break_after_emits_two_page_scenes() {
        let pages = html_to_page_scenes(TWO_PAGE_BREAK_HTML, PageBox::A4).expect("layout");
        assert_eq!(pages.len(), 2);
        assert_eq!(pages[0].page_metadata.page_index, 0);
        assert_eq!(pages[1].page_metadata.page_index, 1);
        assert!(pages[1].content_origin_y > pages[0].content_origin_y);
    }

    #[test]
    fn resolved_grid_order_selects_first_named_page_before_text_layout() {
        const TEXT: &str = "alpha beta gamma delta epsilon zeta eta theta iota kappa lambda mu";
        let prefix = r#"<!doctype html><style>
            @page wide { size:200px 300px; margin:5px }
            @page narrow { size:120px 180px; margin:12px }
            body { margin:0 }
            .grid { display:grid; grid-template-columns:100%; grid-template-rows:auto auto }
        </style><body><div class="grid">"#;
        let wide_first = format!(
            r#"{prefix}<div style="display:block;grid-row:2;order:0;page:wide;height:10px">wide</div><div style="display:block;grid-row:1;order:1;page:narrow;font-size:10px;line-height:12px">{TEXT}</div></div></body>"#
        );
        let narrow_first = format!(
            r#"{prefix}<div style="display:block;grid-row:1;order:1;page:narrow;font-size:10px;line-height:12px">{TEXT}</div><div style="display:block;grid-row:2;order:0;page:wide;height:10px">wide</div></div></body>"#
        );

        let actual = layout_page_render_data(&wide_first, PageBox::A4).expect("actual layout");
        let expected = layout_page_render_data(&narrow_first, PageBox::A4).expect("control layout");
        let (actual_scene, actual_cascade, actual_page_box) = &actual.pages[0];
        let (expected_scene, expected_cascade, expected_page_box) = &expected.pages[0];

        assert_eq!(
            actual_scene.page_metadata.page_name.as_deref(),
            Some("narrow")
        );
        assert_eq!(
            (actual_page_box.width, actual_page_box.height),
            (120.0, 180.0)
        );
        assert_eq!(
            actual_scene.rasterize(&actual.dom, actual_cascade, *actual_page_box),
            expected_scene.rasterize(&expected.dom, expected_cascade, *expected_page_box),
            "the first page text should wrap at the resolved narrow-page width", // cov:ignore: assert_eq! only formats this message when the images differ
        );
    }

    #[test]
    fn resolved_grid_page_without_size_keeps_the_caller_page_box() {
        let html = r#"<!doctype html><style>
            @page wide { size:200px 300px; margin:5px }
            @page narrow { margin:12px }
            body { display:grid; grid-template-columns:100%; grid-template-rows:auto auto; margin:0 }
        </style><body>
            <div style="grid-row:2;order:0;page:wide;height:10px">wide</div>
            <div style="grid-row:1;order:1;page:narrow;height:10px">narrow</div>
        </body>"#;
        let mut fallback = PageBox::A4;
        fallback.width = 320.0;
        fallback.height = 240.0;
        let data = layout_page_render_data(html, fallback).expect("layout");
        let (scene, _, page_box) = &data.pages[0];

        assert_eq!(scene.page_metadata.page_name.as_deref(), Some("narrow"));
        assert_eq!((page_box.width, page_box.height), (320.0, 240.0));
    }

    #[test]
    fn forced_break_after_renders_two_png_pages() {
        let pages = html_to_png_pages_via_page_stream(TWO_PAGE_BREAK_HTML, PageBox::A4)
            .expect("render pages");
        assert_eq!(pages.len(), 2);
        for png in pages {
            assert!(png.len() > 8);
            assert_eq!(&png[..8], &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);
        }
    }

    #[test]
    fn legacy_page_break_before_emits_two_page_scenes() {
        let html = TWO_PAGE_BREAK_HTML.replace(
            ".second { height: 20px }",
            ".second { height: 20px; page-break-before: always }",
        );
        let pages = html_to_page_scenes(&html, PageBox::A4).expect("layout");
        assert_eq!(pages.len(), 2);
    }

    #[test]
    fn receipt_html_to_page_scenes_produces_one_page() {
        let scenes = html_to_page_scenes(RECEIPT_HTML, PageBox::A4).expect("layout ok");
        // cov:ignore: assertion text is only evaluated when this test fails
        assert_eq!(
            scenes.len(),
            1,
            "single-page implementation must produce exactly 1 PageScene"
        );
        let scene = &scenes[0];
        assert!(scene.root_id.is_some(), "root_id must be <html>");
        assert!(scene.body_id.is_some(), "body_id must be <body>");
        assert!(
            !scene.node_ids.is_empty(),
            "node_ids must contain at least body descendants"
        );
        // fragments coverage: every node_id has a fragment
        for id in &scene.node_ids {
            assert!(
                scene.fragments.contains_key(id),
                "fragment missing for {id:?}"
            );
        }
        // drawables: at least one block and one paragraph entry for h1/p + table cells
        assert!(
            !scene.drawables.block_styles.is_empty(),
            "block_styles must have entries for h1/p/table"
        );
        assert!(
            !scene.drawables.paragraphs.is_empty(),
            "paragraphs must have entries for text nodes"
        );
        // Verify body fragment is page-sized (A4)
        let body_id = scene.body_id.unwrap();
        let body_frags = scene.fragments.get(&body_id).expect("body fragment exists");
        assert_eq!(body_frags.len(), 1);
        assert!((body_frags[0].width - PageBox::A4.width).abs() < 1.0);
        assert!((body_frags[0].height - PageBox::A4.height).abs() < 1.0);
    }

    #[test]
    fn parse_and_layout_with_raikiri_viewport_none_uses_a4() {
        let pages = parse_and_layout_with_raikiri(RECEIPT_HTML, None).expect("layout");
        assert_eq!(pages.len(), 1);
        assert_eq!(
            pages[0].page_metadata.size,
            (PageBox::A4.width, PageBox::A4.height)
        );
    }

    #[test]
    fn parse_and_layout_with_raikiri_viewport_maps_to_page_box() {
        let vp = Viewport {
            window_size: (794, 1123),
            hidpi_scale: 1.0,
            zoom: 1.0,
            color_scheme: crate::shell::ColorScheme::Light,
        };
        let pages = parse_and_layout_with_raikiri(RECEIPT_HTML, Some(vp)).expect("layout");
        assert_eq!(pages.len(), 1);
        // 794x1123 with scale 1.0 => PageBox 794x1123
        assert!((pages[0].page_metadata.size.0 - 794.0).abs() < 0.5);
        assert!((pages[0].page_metadata.size.1 - 1123.0).abs() < 0.5);
    }

    #[test]
    fn viewport_to_page_box_fallbacks() {
        let vp_zero = Viewport {
            window_size: (0, 0),
            ..Default::default()
        };
        assert_eq!(viewport_to_page_box(&vp_zero), PageBox::A4);
        let mut vp_scale_zero = Viewport {
            window_size: (800, 600),
            ..Default::default()
        };
        vp_scale_zero.hidpi_scale = 0.0;
        assert_eq!(viewport_to_page_box(&vp_scale_zero), PageBox::A4);
    }

    #[test]
    fn raikiri_page_stream_iterator_yields_pages() {
        let stream = RaikiriPageStream::from_html(RECEIPT_HTML, None).expect("stream");
        assert_eq!(stream.len(), 1);
        let pages: Vec<_> = stream.collect();
        assert_eq!(pages.len(), 1);
        assert!(pages[0].body_id.is_some());
    }

    #[test]
    fn html_to_png_via_page_stream_produces_png() {
        let png = html_to_png_via_page_stream(RECEIPT_HTML, PageBox::A4).expect("png");
        assert!(png.len() > 8);
        assert_eq!(&png[..8], &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);
    }

    #[test]
    fn html_to_page_scenes_matches_html_to_png_via_page_stream_bytes() {
        // html_to_png_via_page_stream should be byte-identical to raikiri::html_to_png for same HTML
        let png_via_stream =
            html_to_png_via_page_stream(RECEIPT_HTML, PageBox::A4).expect("stream png");
        let png_via_umbrella = raikiri::html_to_png(RECEIPT_HTML.as_bytes()).expect("umbrella png");
        assert_eq!(
            png_via_stream, png_via_umbrella,
            "PageStream PNG must be byte-identical to raikiri::html_to_png"
        );
    }

    #[test]
    fn simple_h1_p_table_html_succeeds() {
        // Task requirement: h1 + p + table implementation
        let html = "<h1>Title</h1><p>Hello</p><table><tr><td>cell</td></tr></table>";
        let pages = parse_and_layout_with_raikiri(html, None).expect("h1+p+table layout ok");
        assert_eq!(pages.len(), 1);
        let scene = &pages[0];
        // Verify that all element kinds are represented via block_styles
        assert!(
            scene.drawables.block_styles.len() >= 3,
            "h1, p, table/td at least"
        );
    }
}

#[test]
fn table_rows_do_not_cross_page_boundaries() {
    let html = r#"<!DOCTYPE html><html><head><style>
        @page { size: 100px 60px; margin: 0 }
        html, body { margin: 0 }
        table { width: 100px; border-collapse: collapse }
        tr { height: 40px }
        td { padding: 0 }
        .one { background: red }
        .two { background: green }
        .three { background: blue }
    </style></head><body>
        <table><tbody>
            <tr><td class="one">one</td></tr>
            <tr><td class="two">two</td></tr>
            <tr><td class="three">three</td></tr>
        </tbody></table>
    </body></html>"#;
    let pages = html_to_page_scenes(html, PageBox::A4).expect("layout");
    assert_eq!(pages.len(), 3);
    assert_eq!(
        pages
            .iter()
            .map(|page| page.content_origin_y)
            .collect::<Vec<_>>(),
        vec![0.0, 60.0, 120.0]
    );
}

#[test]
fn column_flex_items_do_not_cross_page_boundaries() {
    let html = r#"<!DOCTYPE html><html><head><style>
        @page { size: 100px 60px; margin: 0 }
        html, body { margin: 0 }
        .flex { display: flex; flex-direction: column }
        .item { height: 40px }
        .one { background: red }
        .two { background: green }
        .three { background: blue }
    </style></head><body>
        <div class="flex">
            <div class="item one">one</div>
            <div class="item two">two</div>
            <div class="item three">three</div>
        </div>
    </body></html>"#;
    let pages = html_to_page_scenes(html, PageBox::A4).expect("layout");
    assert_eq!(pages.len(), 3);
    assert_eq!(
        pages
            .iter()
            .map(|page| page.content_origin_y)
            .collect::<Vec<_>>(),
        vec![0.0, 60.0, 120.0]
    );
}

#[test]
fn single_column_grid_items_do_not_cross_page_boundaries() {
    let html = r#"<!DOCTYPE html><html><head><style>
        @page { size: 100px 60px; margin: 0 }
        html, body { margin: 0 }
        .grid { display: grid; grid-template-columns: 100px }
        .item { height: 40px }
        .one { background: red }
        .two { background: green }
        .three { background: blue }
    </style></head><body>
        <div class="grid">
            <div class="item one">one</div>
            <div class="item two">two</div>
            <div class="item three">three</div>
        </div>
    </body></html>"#;
    let pages = html_to_page_scenes(html, PageBox::A4).expect("layout");
    assert_eq!(pages.len(), 3);
    assert_eq!(
        pages
            .iter()
            .map(|page| page.content_origin_y)
            .collect::<Vec<_>>(),
        vec![0.0, 60.0, 120.0]
    );
}
