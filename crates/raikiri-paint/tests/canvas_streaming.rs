//! Pages delivered while the input is still arriving paint the canvas
//! background as the pages of a layout of the whole document do.

use anyrender::{PaintScene, Scene};
use kurbo::Affine;
use raikiri_html::{
    ConsumerPropertyEvent, LayoutConfig, LayoutOptions, LayoutStatus, Page, PageDefaults, PageSink,
    RenderResources, StreamPage, StreamSummary, StreamingLayout, layout, parse_html_with_resources,
};
use raikiri_paint::paint_single_page_with_origin_and_page;

/// Pages as many as the divs of the document, so the document is longer
/// than the input a layout waits for before it delivers pages.
const DIVS: usize = 40;

/// The pixels of `page`, painted as a page of a document of `count` pages.
fn raster(page: &Page<'_>, count: u32) -> Vec<u8> {
    let (document, cascade, page_box, origin) = page.paint_inputs();
    let mut scene = Scene::new();
    paint_single_page_with_origin_and_page(
        &mut scene,
        document,
        cascade,
        page_box,
        origin,
        page.index(),
        count,
        false,
    )
    .expect("paint");
    anyrender::render_to_buffer::<anyrender_vello_cpu::VelloCpuImageRenderer, _>(
        |out| out.append_scene(scene, Affine::IDENTITY),
        100,
        100,
    )
}

/// Paints the pages it is handed, noting how many there were before the
/// input ended.
struct Painted(Vec<Vec<u8>>);

impl PageSink for &mut Painted {
    type Output = StreamSummary;

    fn page(
        &mut self,
        page: StreamPage<'_>,
        _events: Vec<ConsumerPropertyEvent>,
    ) -> std::io::Result<()> {
        self.0.push(raster(&page.page(), 9_999));
        Ok(())
    }

    fn finish(self, summary: StreamSummary) -> std::io::Result<StreamSummary> {
        Ok(summary)
    }
}

/// A document of `DIVS` pages, each a block of its own, under `css`. The
/// comments make the input long enough for the layout to deliver pages before
/// its end.
fn document(css: &str) -> String {
    let mut html = format!(
        "<style>@page {{ size: 100px 100px; margin: 0 }} body {{ margin: 0 }} \
         div {{ height: 100px; break-after: page }} {css}</style>"
    );
    for index in 0..DIVS {
        html.push_str(&format!(
            "<!-- {} --><div>{index}</div>",
            "padding ".repeat(400)
        ));
    }
    html
}

/// Count the pages whose pixels differ between a batch layout and a streamed
/// one.
fn differing_pages(html: &str) -> (usize, Vec<usize>) {
    let resources = RenderResources::new();
    let parsed = parse_html_with_resources(html.as_bytes(), &resources).expect("parse");
    let LayoutStatus::Completed(result) = layout(
        &parsed,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new().resources(&resources),
    )
    .expect("layout") else {
        panic!("expected a complete layout");
    };
    let count = result.page_count();
    let batch: Vec<Vec<u8>> = result.pages().map(|page| raster(&page, count)).collect();

    let mut streamed = Painted(Vec::new());
    let mut stream = StreamingLayout::new(
        &resources,
        PageDefaults::default(),
        LayoutConfig::default(),
        &mut streamed,
    );
    for piece in html.as_bytes().chunks(8_192) {
        stream.feed(piece).expect("feed");
    }
    stream.finish().expect("finish");
    assert_eq!(streamed.0.len(), batch.len(), "the pages delivered");
    let differing = streamed
        .0
        .iter()
        .zip(&batch)
        .enumerate()
        .filter(|(_, (streamed, batch))| streamed != batch)
        .map(|(index, _)| index)
        .collect();
    (batch.len(), differing)
}

#[test]
fn a_gradient_over_the_root_box_paints_as_in_a_batch_layout() {
    let html = document("html { background: linear-gradient(red, blue) }");
    assert_eq!(differing_pages(&html), (DIVS, Vec::new()));
}

#[test]
fn a_gradient_from_the_body_paints_as_in_a_batch_layout() {
    let html = document("body { background: linear-gradient(red, blue) }");
    assert_eq!(differing_pages(&html), (DIVS, Vec::new()));
}

#[test]
fn tiles_from_the_top_of_the_root_box_keep_their_phase_on_every_page() {
    // 37px tiles do not divide the 100px pages, so a tile row that began
    // anywhere but the top of the first page would show.
    let html = document(
        "html { background: linear-gradient(red 50%, blue 50%); background-size: 100px 37px }",
    );
    assert_eq!(differing_pages(&html), (DIVS, Vec::new()));
}

#[test]
fn a_gradient_fixed_to_the_page_area_paints_as_in_a_batch_layout() {
    let html = document("html { background: linear-gradient(red, blue) fixed }");
    assert_eq!(differing_pages(&html), (DIVS, Vec::new()));
}
