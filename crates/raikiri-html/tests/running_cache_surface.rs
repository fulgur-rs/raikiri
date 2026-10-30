//! Running cache policy through the default-feature external consumer API.
use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, PageSink, RenderResources, StreamPage,
    StreamStatus, StreamSummary, StreamingLayout, StringFetchMode, layout,
    parse_html_with_resources,
};
use raikiri_traits::{
    ConsumerPropertyEvent, DecodedImage, ImagePixelSource, LayoutConfig, LimitKind, PageDefaults,
    RenderError,
};
use std::sync::Arc;
use url::Url;

struct Pixels(Arc<DecodedImage>);
impl ImagePixelSource for Pixels {
    fn get_decoded(&self, _url: &Url) -> Option<Arc<DecodedImage>> {
        Some(self.0.clone())
    }
}

#[test]
fn configured_budget_keeps_width_geometry_fonts_pixels_and_borrowed_hits() {
    for size in [10, 20] {
        let pixels = Pixels(Arc::new(DecodedImage::new(
            2,
            1,
            [size, 0, 0, 255].repeat(2),
        )));
        let fonts = FontCollectionBuilder::new()
            .font_bytes(
                "Ahem",
                include_bytes!("../../raikiri-dom/tests/data/text-autospace/Ahem.ttf").to_vec(),
            )
            .build()
            .unwrap();
        let resources = RenderResources::new()
            .fonts(fonts)
            .base_url(Url::parse("https://running.test/doc").unwrap())
            .image_pixel_source(&pixels);
        let html = format!(
            "<style>h1{{position:running(h);font:{size}px Ahem}}img{{width:2px;height:1px}}</style><h1>AA<img src='pixel.png'></h1><p>body</p>"
        );
        let doc = parse_html_with_resources(html.as_bytes(), &resources).unwrap();
        let LayoutStatus::Completed(out) = layout(
            &doc,
            PageDefaults::default(),
            LayoutConfig::default(),
            LayoutOptions::new()
                .resources(&resources)
                .max_running_layout_entries(2),
        )
        .unwrap() else {
            panic!("aborted")
        };
        let node = out
            .page(0)
            .unwrap()
            .running_element("h", StringFetchMode::First)
            .unwrap();
        let first = out.layout_running_element(node, 200.0).unwrap().unwrap();
        for width in [200.0, 100.0] {
            let cached = out.layout_running_element(node, width).unwrap().unwrap();
            assert_eq!(cached.width(), width);
            let page = cached.page();
            let runs = page.text_runs();
            assert_eq!(runs.iter().map(|run| run.text).collect::<String>(), "AA");
            assert_eq!(runs[0].font_size, f32::from(size));
            let image_node = page
                .fragments()
                .find(|f| page.dom().local_name(f.node()) == Some("img"))
                .unwrap();
            let image = page.raster_image(&image_node, &pixels).unwrap();
            assert!(Arc::ptr_eq(&image.pixels, &pixels.0));
            assert_eq!(image.pixels.rgba, [size, 0, 0, 255].repeat(2));
        }
        assert!(matches!(
            out.layout_running_element(node, 300.0),
            Err(RenderError::LimitExceeded {
                kind: LimitKind::RunningLayoutEntries,
                limit: 2,
                actual: 3,
            })
        ));
        assert!(std::ptr::eq(
            first,
            out.layout_running_element(node, 200.0).unwrap().unwrap()
        ));
    }
}

struct Sink;
impl PageSink for Sink {
    type Output = ();
    fn page(
        &mut self,
        page: StreamPage<'_>,
        _events: Vec<ConsumerPropertyEvent>,
    ) -> std::io::Result<()> {
        let node = page
            .page()
            .running_element("h", StringFetchMode::First)
            .unwrap();
        let first = page.layout_running_element(node, 200.0).unwrap().unwrap();
        assert!(matches!(
            page.layout_running_element(node, 201.0),
            Err(RenderError::LimitExceeded {
                kind: LimitKind::RunningLayoutEntries,
                limit: 1,
                actual: 2,
            })
        ));
        assert!(std::ptr::eq(
            first,
            page.layout_running_element(node, 200.0).unwrap().unwrap()
        ));
        Ok(())
    }
    fn finish(self, _summary: StreamSummary) -> std::io::Result<()> {
        Ok(())
    }
}

#[test]
fn streaming_forwards_the_configured_cache_budget() {
    let resources = RenderResources::new();
    let mut stream = StreamingLayout::new(
        &resources,
        PageDefaults::default(),
        LayoutConfig::default(),
        Sink,
    )
    .max_running_layout_entries(1);
    stream
        .feed(b"<style>h1{position:running(h)}</style><h1>title</h1><p>body</p>")
        .unwrap();
    assert!(matches!(
        stream.finish().unwrap(),
        StreamStatus::Completed(())
    ));
}
