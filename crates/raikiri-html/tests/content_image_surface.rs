//! Images of `::before` and `::after` content as replaced atomic inlines
//! consumed by an independent painter.

use raikiri_html::{
    FontCollectionBuilder, GeneratedKind, LayoutOptions, LayoutStatus, PaintEvent, PaintRect,
    PositionedGlyphRun, RasterImage, RenderResources, RunSource, layout, parse_html_with_resources,
};
use raikiri_traits::{DecodedImage, ImagePixelSource, LayoutConfig, PageDefaults};
use std::sync::Arc;
use url::Url;

/// An 8x4 image at every URL but `missing.png`.
struct Pixels(Arc<DecodedImage>);
impl Pixels {
    fn new() -> Self {
        Self(Arc::new(DecodedImage {
            width: 8,
            height: 4,
            rgba: [0, 0, 255, 255].repeat(32),
        }))
    }
}
impl ImagePixelSource for Pixels {
    fn get_decoded(&self, url: &Url) -> Option<Arc<DecodedImage>> {
        (!url.path().ends_with("missing.png")).then(|| Arc::clone(&self.0))
    }
}

fn lay_out<P: ImagePixelSource + Send + Sync>(
    body: &str,
    css: &str,
    pixels: &P,
) -> raikiri_html::DocumentLayout {
    let fonts = FontCollectionBuilder::new()
        .font_bytes(
            "Ahem",
            include_bytes!("../../raikiri-dom/tests/data/text-autospace/Ahem.ttf").to_vec(),
        )
        .build()
        .unwrap();
    let resources = RenderResources::new()
        .fonts(fonts)
        .base_url(Url::parse("https://content.test/document.html").unwrap())
        .image_pixel_source(pixels);
    let html = format!(
        "<!doctype html><style>@page{{size:200px 100px;margin:0}}\
         body{{margin:0;font:20px/20px Ahem}}p{{margin:0}}{css}</style>{body}"
    );
    let parsed = parse_html_with_resources(html.as_bytes(), &resources).unwrap();
    let LayoutStatus::Completed(result) = layout(
        &parsed,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new().resources(&resources),
    )
    .unwrap() else {
        panic!("completed layout")
    };
    result
}

/// The content images of page 0 in paint order, with the page's text runs.
fn images_and_runs<'a>(
    document: &'a raikiri_html::DocumentLayout,
    pixels: &Pixels,
) -> (Vec<RasterImage>, Vec<PositionedGlyphRun<'a>>) {
    let page = document.page(0).unwrap();
    let runs = page.text_runs();
    let images = page
        .paint_order_for_text_runs(&runs)
        .iter()
        .filter_map(|event| match event {
            PaintEvent::GeneratedImage(image) => {
                Some(page.raster_generated_image(*image, pixels).unwrap())
            }
            _ => None,
        })
        .collect();
    (images, runs)
}

fn run<'a, 'b>(runs: &'b [PositionedGlyphRun<'a>], text: &str) -> &'b PositionedGlyphRun<'a> {
    runs.iter().find(|run| run.text == text).unwrap()
}

#[test]
fn before_image_is_an_atomic_inline_at_its_natural_size_on_the_baseline() {
    let pixels = Pixels::new();
    let document = lay_out(
        "<p id=p>AB</p>",
        "p::before{content:url(icon.png)}",
        &pixels,
    );
    let (images, runs) = images_and_runs(&document, &pixels);
    assert_eq!(images.len(), 1);
    let image = &images[0];
    assert_eq!(image.url.as_str(), "https://content.test/icon.png");
    // Ahem's ascent is 0.8em: the baseline of the 20px line is at 16px, where
    // the image's bottom edge sits (CSS 2.1 10.8.1).
    assert_eq!(image.rect, PaintRect::new(0.0, 12.0, 8.0, 4.0));
    assert_eq!(image.clip.rect, image.rect);
    assert_eq!(image.clip_owner, Some(image.node));
    assert!(Arc::ptr_eq(&image.pixels, &pixels.0));
    // The element's text follows the image's width.
    assert_eq!(run(&runs, "AB").origin.0, 8.0);
}

#[test]
fn paint_event_identifies_the_pseudo_element_and_image() {
    let pixels = Pixels::new();
    let document = lay_out(
        "<p>A</p>",
        "p::before{content:url(a.png) url(b.png)}p::after{content:url(c.png)}",
        &pixels,
    );
    let page = document.page(0).unwrap();
    let runs = page.text_runs();
    let ids: Vec<_> = page
        .paint_order_for_text_runs(&runs)
        .iter()
        .filter_map(|event| match event {
            PaintEvent::GeneratedImage(image) => Some((image.kind, image.index)),
            _ => None,
        })
        .collect();
    assert_eq!(
        ids,
        [
            (GeneratedKind::Before, 0),
            (GeneratedKind::Before, 1),
            (GeneratedKind::After, 0)
        ]
    );
    let (images, runs) = images_and_runs(&document, &pixels);
    let xs: Vec<_> = images.iter().map(|image| image.rect.x).collect();
    assert_eq!(xs, [0.0, 8.0, 36.0]);
    assert_eq!(images[2].url.as_str(), "https://content.test/c.png");
    assert_eq!(run(&runs, "A").origin.0, 16.0);
}

#[test]
fn image_mixed_with_strings_keeps_content_order() {
    let pixels = Pixels::new();
    let document = lay_out(
        "<p>A</p>",
        "p::before{content:'X' url(icon.png) 'Y'}",
        &pixels,
    );
    let (images, runs) = images_and_runs(&document, &pixels);
    assert_eq!(images.len(), 1);
    assert_eq!(images[0].rect.x, 20.0);
    let generated: Vec<_> = runs
        .iter()
        .filter(|run| matches!(run.source, RunSource::Generated(_, GeneratedKind::Before)))
        .map(|run| (run.text, run.origin.0))
        .collect();
    assert_eq!(generated, [("X", 0.0), ("Y", 28.0)]);
    assert_eq!(run(&runs, "A").origin.0, 48.0);
}

#[test]
fn width_and_height_size_a_replacing_image() {
    let pixels = Pixels::new();
    let document = lay_out(
        "<p>A</p>",
        "p::before{content:url(icon.png);width:16px}p::after{content:url(icon.png);width:30px;height:10px}",
        &pixels,
    );
    let (images, runs) = images_and_runs(&document, &pixels);
    assert_eq!(images.len(), 2);
    // A width alone keeps the natural 2:1 ratio (CSS 2.1 10.6.2).
    assert_eq!(images[0].rect, PaintRect::new(0.0, 8.0, 16.0, 8.0));
    assert_eq!(images[1].rect, PaintRect::new(36.0, 6.0, 30.0, 10.0));
    assert_eq!(run(&runs, "A").origin.0, 16.0);
}

#[test]
fn an_image_that_fails_to_load_represents_nothing() {
    let pixels = Pixels::new();
    let document = lay_out(
        "<p>A</p>",
        "p::before{content:url(missing.png) 'X'}",
        &pixels,
    );
    let (images, runs) = images_and_runs(&document, &pixels);
    assert!(images.is_empty());
    assert_eq!(run(&runs, "X").origin.0, 0.0);
    assert_eq!(run(&runs, "A").origin.0, 20.0);
}

#[test]
fn hidden_and_out_of_flow_pseudo_images_are_not_painted_in_lines() {
    let pixels = Pixels::new();
    let document = lay_out(
        "<p>A</p><p>B</p>",
        "p:first-child::before{content:url(icon.png);visibility:hidden}\
         p:last-child::before{content:url(icon.png);position:absolute}",
        &pixels,
    );
    let (images, runs) = images_and_runs(&document, &pixels);
    assert!(images.is_empty());
    // A hidden image still takes its place in the line.
    assert_eq!(run(&runs, "A").origin.0, 8.0);
    assert_eq!(run(&runs, "B").origin.0, 0.0);
}

#[test]
fn image_alone_in_an_empty_element_makes_its_line() {
    let pixels = Pixels::new();
    let document = lay_out("<div></div>", "div::before{content:url(icon.png)}", &pixels);
    let (images, _) = images_and_runs(&document, &pixels);
    assert_eq!(images.len(), 1);
    assert_eq!(images[0].rect, PaintRect::new(0.0, 12.0, 8.0, 4.0));
}

#[test]
fn an_image_belongs_only_to_the_page_of_its_line() {
    let pixels = Pixels::new();
    let document = lay_out(
        "<p>A<br>B<br>C<br>D<br>E<br>F<br>G</p>",
        "p::before{content:url(icon.png)}",
        &pixels,
    );
    assert!(document.page_count() > 1);
    let (images, _) = images_and_runs(&document, &pixels);
    assert_eq!(images.len(), 1);
    let page = document.page(1).unwrap();
    let runs = page.text_runs();
    assert!(runs.iter().any(|run| run.text == "F"));
    assert!(
        !page
            .paint_order_for_text_runs(&runs)
            .iter()
            .any(|event| matches!(event, PaintEvent::GeneratedImage(_)))
    );
}

#[test]
fn an_image_in_a_repeated_table_header_stops_after_the_table() {
    let pixels = Pixels::new();
    // The 10px header fits the quarter-page repeat limit.
    let document = lay_out(
        "<table><thead><tr><th>H</th></tr></thead><tbody><tr><td>B</td></tr><tr><td>C</td></tr></tbody></table><div style='height:70px'></div>",
        "table{border-spacing:0}th,td{padding:0;font-weight:normal}\
         th{text-align:left;font:10px/10px Ahem}td{height:50px}\
         th::before{content:url(icon.png)}",
        &pixels,
    );
    assert!(document.page_count() > 2);
    for page in document.pages() {
        let runs = page.text_runs();
        let images: Vec<_> = page
            .paint_order_for_text_runs(&runs)
            .iter()
            .filter_map(|event| match event {
                PaintEvent::GeneratedImage(image) => page.raster_generated_image(*image, &pixels),
                _ => None,
            })
            .collect();
        assert_eq!(
            images.len(),
            usize::from(page.index() < 2),
            "page {}",
            page.index()
        );
        // The header sits at the top of each page of the table, with its
        // image on the header's 8px baseline.
        if let Some(image) = images.first() {
            assert_eq!(image.rect, PaintRect::new(0.0, 4.0, 8.0, 4.0));
        }
    }
}

#[test]
fn rastering_fails_for_sources_without_valid_pixels_at_the_used_size() {
    struct Failing;
    impl ImagePixelSource for Failing {
        fn get_decoded(&self, url: &Url) -> Option<Arc<DecodedImage>> {
            match url.path() {
                "/short.png" => Some(Arc::new(DecodedImage {
                    width: 8,
                    height: 4,
                    rgba: vec![0; 4],
                })),
                "/empty.png" => Some(Arc::new(DecodedImage {
                    width: 0,
                    height: 4,
                    rgba: Vec::new(),
                })),
                _ => None,
            }
        }
    }
    let pixels = Pixels::new();
    let document = lay_out(
        "<p>A</p>",
        "p::before{content:url(none.png) url(short.png) url(empty.png)}",
        &pixels,
    );
    let page = document.page(0).unwrap();
    let runs = page.text_runs();
    let ids: Vec<_> = page
        .paint_order_for_text_runs(&runs)
        .iter()
        .filter_map(|event| match event {
            PaintEvent::GeneratedImage(image) => Some(*image),
            _ => None,
        })
        .collect();
    assert_eq!(ids.len(), 3);
    for id in ids {
        assert!(page.raster_generated_image(id, &pixels).is_some());
        assert!(page.raster_generated_image(id, &Failing).is_none());
    }
}

#[test]
fn rastering_rejects_a_rectangle_that_overflows_css_pixels() {
    struct Overflowing(Pixels);
    impl ImagePixelSource for Overflowing {
        fn get_decoded(&self, url: &Url) -> Option<Arc<DecodedImage>> {
            self.0.get_decoded(url)
        }
        fn intrinsic_size(&self, _: &Url) -> Option<raikiri_traits::ImageIntrinsicSize> {
            Some(raikiri_traits::ImageIntrinsicSize {
                width: Some(f32::MAX),
                height: None,
                aspect_ratio: Some(1e-20),
            })
        }
    }
    let source = Overflowing(Pixels::new());
    let document = lay_out("<p>A</p>", "p::before{content:url(icon.png)}", &source);
    let page = document.page(0).unwrap();
    let runs = page.text_runs();
    let ids: Vec<_> = page
        .paint_order_for_text_runs(&runs)
        .iter()
        .filter_map(|event| match event {
            PaintEvent::GeneratedImage(image) => Some(*image),
            _ => None,
        })
        .collect();
    // The image keeps its place in the paint order but has no raster.
    assert_eq!(ids.len(), 1);
    assert!(page.raster_generated_image(ids[0], &source).is_none());
}

// A long document with a content image in every paragraph completes within
// the default layout work budget: each paragraph's images are projected onto
// the pages its lines reach, not checked against every page.
#[test]
fn content_images_project_onto_long_documents() {
    let pixels = Pixels::new();
    let body = "<p>A</p>".repeat(300);
    let document = lay_out(
        &body,
        "p{height:100px}p::before{content:url(a.png)}",
        &pixels,
    );
    assert_eq!(document.page_count(), 300);
    for index in [0, 149, 299] {
        let page = document.page(index).unwrap();
        let runs = page.text_runs();
        let images = page
            .paint_order_for_text_runs(&runs)
            .iter()
            .filter(|event| matches!(event, PaintEvent::GeneratedImage(_)))
            .count();
        assert_eq!(images, 1, "page {index}");
    }
}

/// The images of a `content` list carry their place in the line's text, so
/// a consumer can read them in order with the generated text although paint
/// lists the images first.
#[test]
fn images_order_with_generated_text_by_text_offset() {
    let pixels = Pixels::new();
    let check = |css: &str, expected: &[&str]| {
        let document = lay_out("<p>X</p>", css, &pixels);
        let page = document.page(0).unwrap();
        let runs = page.text_runs();
        let mut items: Vec<(usize, usize, String)> = runs
            .iter()
            .map(|run| (run.line.index, run.text_offset, run.text.to_owned()))
            .collect();
        for event in page.paint_order_for_text_runs(&runs) {
            if let PaintEvent::GeneratedImage(image) = event {
                let (line, offset) = page.generated_image_text_position(image).unwrap();
                items.push((line.index, offset.unwrap(), format!("img{}", image.index)));
            }
        }
        items.sort_by_key(|&(line, offset, _)| (line, offset));
        let order: Vec<_> = items.iter().map(|(_, _, text)| text.as_str()).collect();
        assert_eq!(order, expected, "{css}");
    };
    check(
        "p::before{content:'N ' url(a.png) ' E'}",
        &["N ", "img0", " E", "X"],
    );
    check(
        "p::after{content:url(a.png) 'M' url(b.png)}",
        &["X", "img0", "M", "img1"],
    );
    // Left-to-right text in a right-to-left line mixes directions, where
    // the visual order does not give the logical one.
    let document = lay_out(
        "<p>X</p>",
        "p{direction:rtl}p::before{content:'N' url(a.png) 'E'}",
        &pixels,
    );
    let page = document.page(0).unwrap();
    let runs = page.text_runs();
    let positions: Vec<_> = page
        .paint_order_for_text_runs(&runs)
        .into_iter()
        .filter_map(|event| match event {
            PaintEvent::GeneratedImage(image) => page.generated_image_text_position(image),
            _ => None,
        })
        .collect();
    assert_eq!(positions.len(), 1);
    assert_eq!(positions[0].1, None);
    // Images alone in an override have no runs to show their reordering.
    let document = lay_out(
        "<p><span></span>X</p>",
        "span{unicode-bidi:bidi-override;direction:rtl}\
         span::before{content:url(a.png) url(b.png)}",
        &pixels,
    );
    let page = document.page(0).unwrap();
    let runs = page.text_runs();
    let offsets: Vec<_> = page
        .paint_order_for_text_runs(&runs)
        .into_iter()
        .filter_map(|event| match event {
            PaintEvent::GeneratedImage(image) => page.generated_image_text_position(image),
            _ => None,
        })
        .map(|(_, offset)| offset)
        .collect();
    assert_eq!(offsets, [None, None]);
}

#[test]
fn a_relative_pseudo_element_moves_its_images() {
    let pixels = Pixels::new();
    let document = lay_out(
        "<p>AB</p>",
        "p::before{content:url(icon.png);position:relative;left:10px;top:3px}",
        &pixels,
    );
    let (images, runs) = images_and_runs(&document, &pixels);
    assert_eq!(images.len(), 1);
    assert_eq!(images[0].rect, PaintRect::new(10.0, 15.0, 8.0, 4.0));
    // The offset moves the pseudo-element alone, not the text after it.
    assert_eq!(run(&runs, "AB").origin.0, 8.0);
    // With `display: contents` there is no box for the offset to move.
    let document = lay_out(
        "<p>AB</p>",
        "p::before{content:url(icon.png);display:contents;position:relative;left:10px;top:3px}",
        &pixels,
    );
    let (images, _) = images_and_runs(&document, &pixels);
    assert_eq!(images.len(), 1);
    assert_eq!(images[0].rect, PaintRect::new(0.0, 12.0, 8.0, 4.0));
}

/// An image alone in `content` replaces its pseudo-element, so `object-fit`
/// and `object-position` place it in the box, which clips it.
#[test]
fn object_fit_places_a_replacing_image_in_its_box() {
    let pixels = Pixels::new();
    let image = |css: &str| {
        let document = lay_out("<p>A</p>", css, &pixels);
        let (images, _) = images_and_runs(&document, &pixels);
        assert_eq!(images.len(), 1, "{css}");
        images[0].clone()
    };
    // The 8x4 image in a 40x40 box. Its line's baseline is the box's
    // bottom, 40px down.
    let contain =
        image("p::before{content:url(icon.png);width:40px;height:40px;object-fit:contain}");
    assert_eq!(contain.rect, PaintRect::new(0.0, 10.0, 40.0, 20.0));
    assert_eq!(contain.clip.rect, PaintRect::new(0.0, 0.0, 40.0, 40.0));
    let cover = image(
        "p::before{content:url(icon.png);width:40px;height:40px;object-fit:cover;object-position:left top}",
    );
    assert_eq!(cover.rect, PaintRect::new(0.0, 0.0, 80.0, 40.0));
    assert_eq!(cover.clip.rect, PaintRect::new(0.0, 0.0, 40.0, 40.0));
    // Among other content the image is not replaced content and fills its
    // box.
    let mixed =
        image("p::before{content:url(icon.png) 'B';width:40px;height:40px;object-fit:contain}");
    assert_eq!(mixed.rect, mixed.clip.rect);
}

/// A covering image whose `object-fit` rectangle is far larger than its box
/// is rastered at the size layout prepared it at, not at the whole object.
#[test]
fn a_covering_image_is_rastered_at_its_prepared_size() {
    /// A square image whose rasters above 10000 pixels are refused, as an
    /// image budget would.
    struct Bounded;
    impl ImagePixelSource for Bounded {
        fn get_decoded(&self, _: &Url) -> Option<Arc<DecodedImage>> {
            None
        }
        fn intrinsic_size(&self, _: &Url) -> Option<raikiri_traits::ImageIntrinsicSize> {
            Some(raikiri_traits::ImageIntrinsicSize {
                width: Some(1.0),
                height: Some(1.0),
                aspect_ratio: Some(1.0),
            })
        }
        fn get_decoded_at_size(
            &self,
            _: &Url,
            size: raikiri_traits::ImageRasterSize,
            _: Option<u64>,
        ) -> Option<Arc<DecodedImage>> {
            let width = size.width.ceil() as u32;
            let height = size.height.ceil() as u32;
            (width * height <= 10_000).then(|| {
                Arc::new(DecodedImage {
                    width,
                    height,
                    rgba: vec![0; (width * height * 4) as usize],
                })
            })
        }
    }
    // The 200x2 box holds a 200x200 object.
    let document = lay_out(
        "<p>A</p>",
        "p::before{content:url(icon.svg);width:200px;height:2px;object-fit:cover}",
        &Bounded,
    );
    let page = document.page(0).unwrap();
    let runs = page.text_runs();
    let ids: Vec<_> = page
        .paint_order_for_text_runs(&runs)
        .iter()
        .filter_map(|event| match event {
            PaintEvent::GeneratedImage(image) => Some(*image),
            _ => None,
        })
        .collect();
    assert_eq!(ids.len(), 1);
    let image = page.raster_generated_image(ids[0], &Bounded).unwrap();
    assert_eq!((image.rect.width, image.rect.height), (200.0, 200.0));
    assert_eq!((image.pixels.width, image.pixels.height), (20, 20));
}
