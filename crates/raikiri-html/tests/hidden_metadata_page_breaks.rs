//! A `<style>`, `<script>`, `<title>` or `<noscript>` in the body generates
//! no box, as in Chromium (HTML "Rendering" §15.3.1), and an empty inline
//! element lays out no line (CSS 2.1 §9.4.2), so page breaks do not count
//! either as content before the boxes that follow: each document lays out as
//! it does without it.
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

#[test]
fn empty_inline_elements_do_not_count_as_content_before_a_break() {
    let document = "<div style='padding-top:10px'><p id=p style='margin:0;width:20px;padding-top:10px;padding-bottom:60px'>A</p></div><div id=z>Z</div>";
    let expected = boxes(document);
    // Without text, generated content or inline-axis margins, borders and
    // padding, an inline element makes a phantom line, which Chromium
    // ignores when it breaks pages.
    for element in [
        "<span></span>",
        "<span> </span>",
        "<b><i></i></b>",
        "<span style='padding-top:5px'></span>",
        "<span style='display:none'>x</span><span></span>",
        "<span><span style='display:contents'></span></span>",
        "<span><span style='position:absolute'></span></span>",
    ] {
        assert_eq!(
            boxes(&format!("{element}{document}")),
            expected,
            "{element}{document}"
        );
    }
    // With inline-axis padding, a line break, a replaced element (even one of
    // no size), an atomic inline or text, the
    // line is real and Chromium moves the paragraph to the next page.
    for element in [
        "<span style='padding-left:5px'></span>",
        "<br>",
        "<wbr>",
        "<iframe width=0 height=0 style='border:0'></iframe>",
        "<span style='display:inline-block'></span>",
        "<span>x</span>",
    ] {
        let laid_out = boxes(&format!("{element}{document}"));
        assert_eq!(
            laid_out
                .iter()
                .find(|(_, id, _)| id == "p")
                .map(|(page, ..)| *page),
            Some(1),
            "{element}{document}: {laid_out:?}"
        );
    }
}

#[test]
fn empty_inline_elements_lay_out_no_line_in_vertical_writing() {
    // In vertical writing the block size of an inline box is its width.
    let document = "<div style='writing-mode:vertical-rl;height:100px'><span></span><div id=a style='width:20px'>A</div></div>";
    let without = "<div style='writing-mode:vertical-rl;height:100px'><div id=a style='width:20px'>A</div></div>";
    assert_eq!(boxes(document), boxes(without));
}

#[test]
fn an_empty_inline_flex_item_still_counts() {
    // Flex items are blockified, so an empty span is a box of the flex
    // container, not a phantom line.
    let document = "<div style='display:flex;flex-direction:column'><span style='margin-top:50px'></span><div style='padding-top:10px'><p id=p style='margin:0;width:20px;padding-top:10px;padding-bottom:60px'>A</p></div></div>";
    let laid_out = boxes(document);
    assert!(
        laid_out.iter().any(|(page, id, _)| id == "p" && *page == 1),
        "{laid_out:?}"
    );
    // A `display: contents` element between them changes nothing.
    let through_contents = document.replace(
        "<span style='margin-top:50px'></span>",
        "<div style='display:contents'><span style='margin-top:50px'></span></div>",
    );
    assert_eq!(boxes(&through_contents), laid_out);
}
