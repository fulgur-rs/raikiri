//! Contract tests for consumer-supplied page defaults.

use raikiri_html::{
    DocumentLayout, LayoutConfig, LayoutOptions, LayoutStatus, PageDefaults, RenderResources,
    layout, parse_html_with_resources,
};
use raikiri_traits::PageBox;

fn completed(html: &str, page_box: PageBox) -> DocumentLayout {
    let resources = RenderResources::new();
    let document = parse_html_with_resources(html.as_bytes(), &resources).unwrap();
    match layout(
        &document,
        PageDefaults::builder().page_box(page_box).build(),
        LayoutConfig::default(),
        LayoutOptions::new().resources(&resources),
    )
    .unwrap()
    {
        LayoutStatus::Completed(result) => result,
        _ => panic!("expected a completed layout"),
    }
}

fn first_page_box(html: &str, page_box: PageBox) -> (f32, f32) {
    let result = completed(html, page_box);
    let geometry = result.pages().next().unwrap().geometry();
    (geometry.page_box.width, geometry.page_box.height)
}

#[test]
fn ua_chosen_page_sizes_use_the_default_page_box() {
    let letter = PageBox::US_LETTER;
    for (css, expected) in [
        ("", (816.0, 1056.0)),
        ("@page { size: auto }", (816.0, 1056.0)),
        ("@page { size: portrait }", (816.0, 1056.0)),
        ("@page { size: landscape }", (1056.0, 816.0)),
        ("@page { size: 300px 200px }", (300.0, 200.0)),
    ] {
        let html = format!("<style>{css}</style><p>text</p>");
        assert_eq!(first_page_box(&html, letter), expected, "{css}");
    }
}

#[test]
fn named_page_sizes_ignore_the_default_page_box() {
    let html = "<style>@page { size: A4 landscape }</style><p>text</p>";
    let (width, height) = first_page_box(html, PageBox::US_LETTER);
    assert!((width - PageBox::A4.height).abs() < 0.01, "{width}");
    assert!((height - PageBox::A4.width).abs() < 0.01, "{height}");
}
