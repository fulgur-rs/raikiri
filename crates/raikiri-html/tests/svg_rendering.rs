//! The rendering choice of SVG pictures: the layout default, the
//! `--raikiri-svg-rendering` property of the element that owns the picture,
//! and the raster scale.

use raikiri_html::{
    DocumentLayout, FontCollectionBuilder, LayoutOptions, LayoutStatus, NodeId, Page,
    RenderResources, StreamPage, StreamStatus, StreamSummary, StreamingLayout, StringFetchMode,
    SvgRendering, layout, parse_html_with_resources,
};
use raikiri_traits::{ConsumerPropertyEvent, LayoutConfig, PageDefaults};

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../raikiri-dom/tests/data/text-autospace/Ahem.ttf"
));

const PAGE: &str = r#"<!doctype html><style>
    @page { size: 300px 200px; margin: 20px; @top-center { content: element(hdr) } }
    body { margin: 0; font: 10px/10px Ahem }
    svg { display: block }
    .hdr { position: running(hdr) }
    .raster { --raikiri-svg-rendering: raster }
    .vector { --raikiri-svg-rendering: vector }
    .shouting { --raikiri-svg-rendering: RASTER }
    .odd { --raikiri-svg-rendering: pixels }
</style>
<svg id="plain" width="10" height="10"></svg>
<div class="raster"><svg id="declared" width="10" height="10"></svg></div>
<div class="raster"><div><svg id="inherited" width="10" height="10"></svg></div></div>
<div class="raster"><svg id="back" class="vector" width="10" height="10"></svg></div>
<div class="shouting"><svg id="shouting" width="10" height="10"></svg></div>
<div class="odd"><svg id="odd" width="10" height="10"></svg></div>
<div class="raster"><div class="odd"><svg id="odd-in-raster" width="10" height="10"></svg></div></div>
<div class="hdr raster"><svg id="running" width="10" height="10"></svg></div>"#;

fn lay_out(
    options: impl for<'a> FnOnce(LayoutOptions<'a, 'a>) -> LayoutOptions<'a, 'a>,
) -> DocumentLayout {
    let fonts = FontCollectionBuilder::new()
        .font_bytes("Ahem", AHEM.to_vec())
        .build()
        .expect("fonts");
    let resources = RenderResources::new().fonts(fonts);
    let document = parse_html_with_resources(PAGE.as_bytes(), &resources).expect("parse");
    let LayoutStatus::Completed(result) = layout(
        &document,
        PageDefaults::default(),
        LayoutConfig::default(),
        options(LayoutOptions::new().resources(&resources)),
    )
    .expect("layout") else {
        panic!("expected a complete layout");
    };
    result
}

/// The element `id` of the first page.
fn node(page: &Page<'_>, id: &str) -> NodeId {
    page.fragments()
        .map(|fragment| fragment.node())
        .find(|&node| page.dom().attr(node, "id") == Some(id))
        .unwrap_or_else(|| panic!("no element {id} on the page"))
}

fn renderings(page: &Page<'_>) -> Vec<(&'static str, SvgRendering)> {
    [
        "plain",
        "declared",
        "inherited",
        "back",
        "shouting",
        "odd",
        "odd-in-raster",
    ]
    .into_iter()
    .map(|id| (id, page.svg_rendering(node(page, id))))
    .collect()
}

#[test]
fn a_picture_is_vector_unless_its_element_says_raster() {
    let result = lay_out(|options| options);
    let page = result.page(0).expect("page");
    assert_eq!(
        renderings(&page),
        vec![
            ("plain", SvgRendering::Vector),
            ("declared", SvgRendering::Raster),
            // The property inherits, so the picture in the subtree follows.
            ("inherited", SvgRendering::Raster),
            ("back", SvgRendering::Vector),
            // Keywords are not case sensitive.
            ("shouting", SvgRendering::Raster),
            // A value that names no rendering is as if it were not declared.
            ("odd", SvgRendering::Vector),
            ("odd-in-raster", SvgRendering::Raster),
        ]
    );
    assert_eq!(SvgRendering::default(), SvgRendering::Vector);
}

#[test]
fn the_layout_option_is_the_rendering_of_an_element_that_declares_none() {
    let result = lay_out(|options| options.svg_rendering(SvgRendering::Raster));
    let page = result.page(0).expect("page");
    assert_eq!(
        renderings(&page),
        vec![
            ("plain", SvgRendering::Raster),
            ("declared", SvgRendering::Raster),
            ("inherited", SvgRendering::Raster),
            ("back", SvgRendering::Vector),
            ("shouting", SvgRendering::Raster),
            ("odd", SvgRendering::Raster),
            ("odd-in-raster", SvgRendering::Raster),
        ]
    );
}

#[test]
fn a_node_the_page_does_not_have_gets_the_default() {
    let result = lay_out(|options| options.svg_rendering(SvgRendering::Raster));
    let page = result.page(0).expect("page");
    assert_eq!(
        page.svg_rendering(NodeId::new(u64::MAX)),
        SvgRendering::Raster
    );
}

#[test]
fn the_raster_scale_is_one_device_pixel_per_css_pixel_unless_set() {
    let result = lay_out(|options| options);
    assert_eq!(result.page(0).expect("page").svg_raster_scale(), 1.0);

    let result = lay_out(|options| options.svg_raster_scale(2.5));
    assert_eq!(result.page(0).expect("page").svg_raster_scale(), 2.5);

    // A scale that is no size is ignored, and the one set before it stays.
    for scale in [0.0, -1.0, f32::NAN, f32::INFINITY] {
        let result = lay_out(|options| options.svg_raster_scale(3.0).svg_raster_scale(scale));
        assert_eq!(result.page(0).expect("page").svg_raster_scale(), 3.0);
    }
}

#[test]
fn a_running_element_is_drawn_as_the_document_is() {
    let result = lay_out(|options| {
        options
            .svg_rendering(SvgRendering::Raster)
            .svg_raster_scale(2.0)
    });
    let page = result.page(0).expect("page");
    let running = page
        .running_element("hdr", StringFetchMode::First)
        .expect("the running element");
    let header = result
        .layout_running_element(running, 100.0)
        .expect("layout")
        .expect("a running element");
    let header = header.page();
    assert_eq!(header.svg_raster_scale(), 2.0);
    // The element's own class declares the rendering, as in the document.
    assert_eq!(
        header.svg_rendering(node(&header, "running")),
        SvgRendering::Raster
    );
}

/// Records what each streamed page says about the pictures on it.
#[derive(Default)]
struct Recorder {
    seen: Vec<(SvgRendering, f32)>,
}

impl raikiri_html::PageSink for Recorder {
    type Output = Vec<(SvgRendering, f32)>;

    fn page(
        &mut self,
        page: StreamPage<'_>,
        _events: Vec<ConsumerPropertyEvent>,
    ) -> std::io::Result<()> {
        let page = page.page();
        self.seen.push((
            page.svg_rendering(node(&page, "plain")),
            page.svg_raster_scale(),
        ));
        Ok(())
    }

    fn finish(self, _summary: StreamSummary) -> std::io::Result<Self::Output> {
        Ok(self.seen)
    }
}

#[test]
fn a_streaming_layout_takes_the_same_settings() {
    let fonts = FontCollectionBuilder::new()
        .font_bytes("Ahem", AHEM.to_vec())
        .build()
        .expect("fonts");
    let resources = RenderResources::new().fonts(fonts);
    let run = |settings: Option<(SvgRendering, f32)>| {
        let mut stream = StreamingLayout::new(
            &resources,
            PageDefaults::default(),
            LayoutConfig::default(),
            Recorder::default(),
        );
        if let Some((rendering, scale)) = settings {
            stream = stream.svg_rendering(rendering).svg_raster_scale(scale);
        }
        stream.feed(PAGE.as_bytes()).expect("feed");
        match stream.finish().expect("finish") {
            StreamStatus::Completed(seen) => seen,
            _ => panic!("expected a complete layout"),
        }
    };
    assert_eq!(run(None), vec![(SvgRendering::Vector, 1.0)]);
    assert_eq!(
        run(Some((SvgRendering::Raster, 4.0))),
        vec![(SvgRendering::Raster, 4.0)]
    );
}
