//! Raster list markers retain shared cached pixels and page paint ownership.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, PaintEvent, PaintRect, RenderResources,
    layout, parse_html_with_resources,
};
use raikiri_traits::{DecodedImage, ImagePixelSource, LayoutConfig, PageDefaults};
use std::sync::Arc;
use url::Url;

struct Pixels(Arc<DecodedImage>);
impl Pixels {
    fn new() -> Self {
        Self(Arc::new(DecodedImage {
            width: 8,
            height: 4,
            rgba: [255, 0, 0, 255].repeat(32),
        }))
    }
}
impl ImagePixelSource for Pixels {
    fn get_decoded(&self, url: &Url) -> Option<Arc<DecodedImage>> {
        (!url.path().ends_with("missing.png")).then(|| Arc::clone(&self.0))
    }
}

fn lay_out(body: &str, css: &str, pixels: &Pixels) -> raikiri_html::DocumentLayout {
    let fonts = FontCollectionBuilder::new()
        .font_bytes(
            "Ahem",
            include_bytes!("../../raikiri-dom/tests/data/text-autospace/Ahem.ttf").to_vec(),
        )
        .build()
        .unwrap();
    let resources = RenderResources::new()
        .fonts(fonts)
        .base_url(Url::parse("https://markers.test/document.html").unwrap())
        .image_pixel_source(pixels);
    let html = format!(
        "<!doctype html><style>@page{{size:200px 60px;margin:0}}body{{margin:0;font:20px/20px Ahem}}ul{{margin:0;padding:0}}li{{margin-left:20px;list-style-image:url(marker.png)}}{css}</style>{body}"
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

#[test]
fn outside_png_marker_has_one_cached_payload_in_its_owner_opacity_group() {
    let pixels = Pixels::new();
    let document = lay_out("<ul><li style='opacity:.5'>A</li></ul>", "", &pixels);
    let page = document.page(0).unwrap();
    let runs = page.text_runs();
    let events = page.paint_order_for_text_runs(&runs);
    let images: Vec<_> = events
        .iter()
        .filter_map(|event| {
            if let PaintEvent::MarkerImage(owner) = event {
                page.raster_marker(*owner, &pixels)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(images.len(), 1);
    let image = &images[0];
    assert_eq!(image.url.as_str(), "https://markers.test/marker.png");
    assert_eq!(image.rect, PaintRect::new(8.0, 0.0, 8.0, 4.0));
    assert_eq!(image.clip.rect, image.rect);
    assert_eq!(image.clip_owner, page.dom().parent(image.node));
    assert!(Arc::ptr_eq(&image.pixels, &pixels.0));
    let opacity = events
        .iter()
        .position(|event| matches!(event,PaintEvent::PushOpacity(value) if *value==0.5))
        .unwrap();
    let marker = events
        .iter()
        .position(|event| matches!(event, PaintEvent::MarkerImage(_)))
        .unwrap();
    let pop = events
        .iter()
        .rposition(|event| matches!(event, PaintEvent::PopOpacity))
        .unwrap();
    assert!(opacity < marker && marker < pop);
}

#[test]
fn image_marker_is_only_on_the_first_principal_fragment_and_content_wins() {
    let pixels = Pixels::new();
    let document = lay_out("<ul><li>AA<br>BB<br>CC<br>DD<br>EE</li></ul>", "", &pixels);
    assert!(document.page_count() > 1);
    for page in document.pages() {
        let count = page
            .paint_order_for_text_runs(&page.text_runs())
            .iter()
            .filter(|event| matches!(event, PaintEvent::MarkerImage(_)))
            .count();
        assert_eq!(count, usize::from(page.index() == 0));
    }
    let overridden = lay_out("<ul><li>A</li></ul>", "li::marker{content:'X '}", &pixels);
    let page = overridden.page(0).unwrap();
    assert!(
        page.paint_order_for_text_runs(&page.text_runs())
            .iter()
            .all(|event| !matches!(event, PaintEvent::MarkerImage(_)))
    );
    assert!(page.text_runs().iter().any(|run| run.text == "X "));
}

#[test]
fn inside_png_marker_uses_its_atomic_rectangle_after_the_owner_clip() {
    let pixels = Pixels::new();
    let document = lay_out(
        "<ul><li>A</li></ul>",
        "li{list-style-position:inside;overflow:hidden}",
        &pixels,
    );
    let page = document.page(0).unwrap();
    let events = page.paint_order_for_text_runs(&page.text_runs());
    let marker = events
        .iter()
        .position(|event| matches!(event, PaintEvent::MarkerImage(_)))
        .unwrap();
    let PaintEvent::MarkerImage(owner) = events[marker] else {
        unreachable!()
    };
    let image = page.raster_marker(owner, &pixels).unwrap();
    assert_eq!(image.rect, PaintRect::new(20.0, 12.0, 8.0, 4.0));
    assert_eq!(image.clip_owner, Some(owner));
    let clip = events
        .iter()
        .position(|event| {
            matches!(
                event,
                PaintEvent::PushClip(_, raikiri_html::ClipKind::Overflow)
            )
        })
        .unwrap();
    assert!(clip < marker);
    assert!(
        events[marker + 1..]
            .iter()
            .any(|event| matches!(event, PaintEvent::TextLine(_)))
    );
}

#[test]
fn outside_image_precedes_the_owner_clip_and_empty_items_keep_it() {
    let pixels = Pixels::new();
    let document = lay_out("<ul><li></li></ul>", "li{overflow:hidden}", &pixels);
    let page = document.page(0).unwrap();
    let events = page.paint_order_for_text_runs(&page.text_runs());
    let marker = events
        .iter()
        .position(|event| matches!(event, PaintEvent::MarkerImage(_)))
        .unwrap();
    let clip = events
        .iter()
        .position(|event| {
            matches!(
                event,
                PaintEvent::PushClip(_, raikiri_html::ClipKind::Overflow)
            )
        })
        .unwrap();
    assert!(marker < clip);
    let PaintEvent::MarkerImage(owner) = events[marker] else {
        unreachable!()
    };
    assert_eq!(
        page.raster_marker(owner, &pixels).unwrap().rect,
        PaintRect::new(8.0, 0.0, 8.0, 4.0)
    );
}

#[test]
fn missing_images_fall_back_to_decimal_text_for_inside_and_outside() {
    let pixels = Pixels::new();
    for position in ["inside", "outside"] {
        let document = lay_out(
            "<ol><li>A</li></ol>",
            &format!("li{{list-style-position:{position};list-style-image:url(missing.png)}}"),
            &pixels,
        );
        let page = document.page(0).unwrap();
        let runs = page.text_runs();
        assert!(runs.iter().any(|run| run.text.trim() == "1."));
        assert!(
            page.paint_order_for_text_runs(&runs)
                .iter()
                .all(|event| !matches!(event, PaintEvent::MarkerImage(_)))
        );
    }
}

#[test]
fn suppressed_and_hidden_markers_have_no_image_event() {
    let pixels = Pixels::new();
    for css in [
        "li{visibility:hidden}",
        "li::marker{visibility:hidden}",
        "li::marker{content:none}",
        "li{display:none}",
    ] {
        let document = lay_out("<ul><li>A</li></ul>", css, &pixels);
        let page = document.page(0).unwrap();
        assert!(
            page.paint_order_for_text_runs(&page.text_runs())
                .iter()
                .all(|event| !matches!(event, PaintEvent::MarkerImage(_))),
            "{css}"
        );
    }
}

#[test]
fn fixed_markers_repeat_while_normal_inside_markers_do_not() {
    let pixels = Pixels::new();
    for position in ["inside", "outside"] {
        let document = lay_out(
            "<ul><li>A</li></ul><div style='height:140px'></div>",
            &format!(
                "ul{{position:fixed;top:0;left:0;width:100px}}li{{list-style-position:{position}}}"
            ),
            &pixels,
        );
        assert!(document.page_count() > 1);
        for page in document.pages() {
            let count = page
                .paint_order_for_text_runs(&page.text_runs())
                .iter()
                .filter(|event| matches!(event, PaintEvent::MarkerImage(_)))
                .count();
            assert_eq!(count, 1);
        }
    }
    let document = lay_out(
        "<ul><li>AA<br>BB<br>CC<br>DD<br>EE</li></ul>",
        "li{list-style-position:inside}",
        &pixels,
    );
    assert!(document.page_count() > 1);
    for page in document.pages() {
        let count = page
            .paint_order_for_text_runs(&page.text_runs())
            .iter()
            .filter(|event| matches!(event, PaintEvent::MarkerImage(_)))
            .count();
        assert_eq!(count, usize::from(page.index() == 0));
    }
}

#[test]
fn marker_payload_respects_pixel_admission_and_rejects_malformed_sources() {
    struct Source(Option<Arc<DecodedImage>>);
    impl ImagePixelSource for Source {
        fn get_decoded(&self, _: &Url) -> Option<Arc<DecodedImage>> {
            self.0.clone()
        }
    }
    let pixels = Pixels::new();
    let document = lay_out("<ul><li>A</li></ul>", "", &pixels);
    let page = document.page(0).unwrap();
    let owner = page
        .paint_order()
        .iter()
        .find_map(|event| match event {
            PaintEvent::MarkerImage(owner) => Some(*owner),
            _ => None,
        })
        .unwrap();
    assert!(page.raster_marker(owner, &Source(None)).is_none());
    for (width, height, rgba) in [
        (0, 4, Vec::new()),
        (8, 0, Vec::new()),
        (8, 4, vec![0; 127]),
        (u32::MAX, u32::MAX, Vec::new()),
    ] {
        assert!(
            page.raster_marker(
                owner,
                &Source(Some(Arc::new(DecodedImage {
                    width,
                    height,
                    rgba
                })))
            )
            .is_none()
        );
    }
    assert!(
        page.raster_marker(raikiri_html::NodeId::new(u64::MAX), &pixels)
            .is_none()
    );
}
