//! A `<style>`, `<script>`, `<title>` or `<noscript>` in the body generates
//! no box, as in Chromium (HTML "Rendering" §15.3.1), so page breaks do not
//! count it as content before the boxes that follow: each document lays out
//! as it does without it.
use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, PaintEvent, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

/// The page and border box of every box with an id in `body`, laid out in
/// Ahem at 20px with a 30px line height on pages 100px square.
fn boxes(body: &str) -> Vec<(u32, String, [i32; 4])> {
    let fonts = FontCollectionBuilder::new()
        .font_bytes(
            "Ahem",
            include_bytes!("../../raikiri-dom/tests/data/text-autospace/Ahem.ttf").to_vec(),
        )
        .build()
        .unwrap();
    let resources = RenderResources::new().fonts(fonts);
    let html = format!(
        "<!doctype html><style>@page{{size:100px 100px;margin:0}} body{{margin:0;font:20px/30px Ahem}}</style><body>{body}</body>"
    );
    let document = parse_html_with_resources(html.as_bytes(), &resources).unwrap();
    let LayoutStatus::Completed(result) = layout(
        &document,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new().resources(&resources),
    )
    .unwrap() else {
        panic!("layout must complete")
    };
    let mut boxes = Vec::new();
    for index in 0..result.page_count() {
        let page = result.page(index).unwrap();
        for event in page.paint_order() {
            if let PaintEvent::Box(fragment) = event {
                let Some(id) = page.dom().attr(fragment.node(), "id") else {
                    continue;
                };
                let rect = fragment.rect();
                boxes.push((
                    index,
                    id.to_owned(),
                    [rect.x, rect.y, rect.width, rect.height].map(|value| value.round() as i32),
                ));
            }
        }
    }
    boxes
}

#[test]
fn metadata_elements_in_the_body_do_not_count_as_content_before_a_break() {
    let documents = [
        "<div id=a style='break-before:page;height:20px'></div><div id=b style='height:20px'></div>",
        "<div style='padding-top:10px'><p id=p style='margin:0;width:20px;padding-top:10px;padding-bottom:60px'>A</p></div><div id=z>Z</div>",
        "<div id=a style='margin-top:30px;height:90px'></div><div id=b style='height:20px'></div>",
    ];
    for document in documents {
        let expected = boxes(document);
        for element in [
            "<style></style>",
            "<script></script>",
            "<title></title>",
            "<noscript></noscript>",
            "<style>p{}</style>",
            "<noscript>N</noscript>",
        ] {
            assert_eq!(
                boxes(&format!("{element}{document}")),
                expected,
                "{element}{document}"
            );
        }
    }
}
