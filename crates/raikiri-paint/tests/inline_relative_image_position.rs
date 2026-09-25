//! Ensures relative insets on inline replaced boxes are painted exactly once.

use std::sync::Arc;

use anyrender::Scene;
use anyrender::recording::RenderCommand;
use parley::FontContext;
use raikiri_dom::{Document, layout_single_page};
use raikiri_paint::paint_single_page_with_images;
use raikiri_style::{DisplayValue, build_rule_tree, cascade};
use raikiri_traits::{DecodedImage, ImagePixelSource, PageBox};
use taffy::Style;
use url::Url;

struct OneImageSource(Url, Arc<DecodedImage>);

impl ImagePixelSource for OneImageSource {
    fn get_decoded(&self, url: &Url) -> Option<Arc<DecodedImage>> {
        (*url == self.0).then(|| self.1.clone())
    }
}

#[test]
fn inline_relative_image_uses_taffy_inset_once_when_painted() {
    let mut document = Document::new();
    let html = document.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = document.append_element(Some(html), "body", Style::default(), None::<&str>);
    let container = document.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:flex;align-items:flex-start"),
    );
    let first = document.append_element(
        Some(container),
        "img",
        Style::default(),
        Some("display:inline-block;width:10px;height:10px"),
    );
    let second = document.append_element(
        Some(container),
        "img",
        Style::default(),
        Some("display:inline-block;width:10px;height:10px;position:relative;top:12px;left:7px"),
    );
    document.set_element_attributes(first, vec![("src".into(), "file:///inline.png".into())]);
    document.set_element_attributes(second, vec![("src".into(), "file:///inline.png".into())]);

    let rules = build_rule_tree(&document);
    let cascade = cascade(&document, &rules).expect("cascade succeeds");
    layout_single_page(&mut document, &cascade, PageBox::A4, FontContext::new())
        .expect("layout succeeds");

    let body_location = document.get_node(body).unwrap().unrounded_layout.location;
    let container_location = document
        .get_node(container)
        .unwrap()
        .unrounded_layout
        .location;
    let first_location = document.get_node(first).unwrap().unrounded_layout.location;
    let second_location = document.get_node(second).unwrap().unrounded_layout.location;
    let first_x = body_location.x + container_location.x + first_location.x;
    let first_y = body_location.y + container_location.y + first_location.y;
    let second_x = body_location.x + container_location.x + second_location.x;
    let second_y = body_location.y + container_location.y + second_location.y;
    let epsilon_f32 = 1e-4_f32;
    let epsilon_f64 = 1e-4_f64;
    assert!(
        matches!(
            cascade.computed[second].display,
            DisplayValue::InlineBlock
                | DisplayValue::InlineFlex
                | DisplayValue::InlineGrid
                | DisplayValue::InlineTable
        ),
        "the regression needs a replaced inline-level box"
    );
    assert!(
        (second_x - first_x - 10.0 - 7.0).abs() < epsilon_f32,
        "the 7px left inset should move only the second image horizontally: first_x={first_x}, second_x={second_x}, delta={} (expected 17)",
        second_x - first_x
    );
    assert!(
        (second_y - first_y - 12.0).abs() < epsilon_f32,
        "the 12px top inset should move only the second image vertically"
    );

    let url = Url::parse("file:///inline.png").unwrap();
    let source = OneImageSource(
        url,
        Arc::new(DecodedImage {
            width: 1,
            height: 1,
            rgba: vec![0, 0, 0, 255],
        }),
    );
    let mut scene = Scene::new();
    paint_single_page_with_images(&mut scene, &document, &cascade, PageBox::A4, &source);

    let image_fills: Vec<_> = scene
        .commands
        .iter()
        .filter_map(|command| match command {
            RenderCommand::Fill(fill)
                if matches!(fill.brush, anyrender::types::Paint::Image(_)) =>
            {
                Some(fill)
            }
            _ => None,
        })
        .collect();
    assert_eq!(image_fills.len(), 2, "both inline images should be painted");
    let second_transform = image_fills[1].transform.as_coeffs();
    assert!(
        (second_transform[4] - second_x as f64).abs() < epsilon_f64
            && (second_transform[5] - second_y as f64).abs() < epsilon_f64,
        "paint should use the second image's layout position exactly once: expected ({second_x}, {second_y}), got ({}, {})",
        second_transform[4],
        second_transform[5]
    );
}
