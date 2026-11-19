//! A page break avoided before a block that follows lines of its paragraph
//! is a worse break than one that leaves too few `orphans` or `widows`: when
//! the page would break before such a block, Chromium breaks before the last
//! line instead, and that line goes to the next page with the block. As the
//! first line of the paragraph, it takes the whole paragraph along. The
//! pages and positions are the ones Chromium prints for the same markup:
//! 100px pages without margins, Ahem at 20px on 30px lines.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/data/text-autospace/Ahem.ttf"
));

/// The page index and the top on that page of the first fragment of each of
/// the elements `#t` and `#z`.
fn places(body: &str) -> [(usize, f32); 2] {
    let html = format!(
        "<!DOCTYPE html><style>@page{{size:100px 100px;margin:0}}\
         body{{margin:0;font:20px/30px Ahem}}</style>{body}"
    );
    let fonts = FontCollectionBuilder::new()
        .font_bytes("Ahem", AHEM.to_vec())
        .build()
        .expect("fonts");
    let resources = RenderResources::new().fonts(fonts);
    let doc = parse_html_with_resources(html.as_bytes(), &resources).expect("parse");
    let status = layout(
        &doc,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new().resources(&resources),
    )
    .expect("layout");
    let LayoutStatus::Completed(result) = status else {
        panic!("expected a complete layout");
    };
    let mut found = [None, None];
    for (index, page) in result.pages().enumerate() {
        for fragment in page.fragments() {
            let slot = match page.dom().attr(fragment.node(), "id") {
                Some("t") => 0,
                Some("z") => 1,
                _ => continue,
            };
            found[slot].get_or_insert((index, fragment.rect().y));
        }
    }
    found.map(|place| place.unwrap_or_else(|| panic!("no fragment for #t or #z in {body}")))
}

/// Lays out every `(body, [#t, #z])` case and reports all that differ.
fn assert_places(cases: &[(String, [(usize, f32); 2])]) {
    let wrong: Vec<String> = cases
        .iter()
        .filter_map(|(body, expected)| {
            let got = places(body);
            got.iter()
                .zip(expected)
                .any(|(got, expected)| got.0 != expected.0 || (got.1 - expected.1).abs() >= 0.01)
                .then(|| format!("{body}: expected {expected:?}, got {got:?}"))
        })
        .collect();
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// A block `above` px tall, then a paragraph of `lines` lines of `A` and the
/// block `block`, then `#z`.
fn paragraph(above: u32, lines: usize, block: &str) -> String {
    format!(
        "<div style=\"width:20px\"><div style=\"height:{above}px\"></div>\
         <div>{}{block}</div><div><span id=z>Z</span></div></div>",
        vec!["A"; lines].join("<br>")
    )
}

/// A block `height` px tall with the style `style` that holds `#t`.
fn block(height: u32, style: &str) -> String {
    format!("<div style=\"height:{height}px;{style}\"><span id=t>T</span></div>")
}

/// A block with the style `style` that holds an image `height` px tall with
/// the attributes `image`, then `content`.
fn image_block(height: u32, style: &str, image: &str, content: &str) -> String {
    format!(
        "<div style=\"{style}\"><img {image} style=\"display:block;width:10px;\
         height:{height}px\">{content}</div>"
    )
}

#[test]
fn the_last_line_goes_with_the_block() {
    assert_places(&[
        (
            paragraph(10, 3, &block(20, "break-before:avoid")),
            [(1, 35.0), (1, 55.0)],
        ),
        (
            paragraph(10, 3, &block(20, "break-before:avoid-page")),
            [(1, 35.0), (1, 55.0)],
        ),
        // One line is left before the break, fewer than `orphans` asks for.
        (
            paragraph(30, 2, &block(40, "break-before:avoid")),
            [(1, 35.0), (1, 75.0)],
        ),
        // The paragraph moved whole for its orphans gives its last line too.
        (
            paragraph(50, 3, &block(20, "break-before:avoid")),
            [(2, 35.0), (2, 55.0)],
        ),
        // Without the avoided break, the line stays.
        (paragraph(10, 3, &block(20, "")), [(1, 5.0), (1, 25.0)]),
    ]);
}

#[test]
fn the_first_line_takes_the_paragraph() {
    assert_places(&[
        (
            paragraph(50, 1, &block(40, "break-before:avoid")),
            [(1, 35.0), (1, 75.0)],
        ),
        // With nothing of the flow before the paragraph, the line stays.
        (
            "<div style=\"width:20px;padding-top:60px\"><div>A\
             <div style=\"height:40px;break-before:avoid\"><span id=t>T</span></div></div>\
             <div><span id=z>Z</span></div></div>"
                .to_owned(),
            [(1, 5.0), (1, 45.0)],
        ),
    ]);
}

#[test]
fn an_image_first_in_the_block_is_its_first_piece() {
    let text = "<span id=t>T</span>";
    assert_places(&[
        (
            paragraph(30, 2, &image_block(20, "break-before:avoid", "", text)),
            [(1, 55.0), (2, 5.0)],
        ),
        (
            paragraph(
                30,
                1,
                &image_block(50, "display:flex;break-before:avoid", "id=t", ""),
            ),
            [(1, 30.0), (2, 5.0)],
        ),
        (
            paragraph(
                30,
                1,
                &image_block(
                    50,
                    "display:flex;flex-direction:column;break-before:avoid",
                    "id=t",
                    "",
                ),
            ),
            [(1, 30.0), (2, 5.0)],
        ),
        // A block inside an inline box before the text is not read for the
        // first piece, and the block breaks inside.
        (
            paragraph(
                30,
                2,
                "<div style=\"break-before:avoid\"><span><div style=\"height:20px\">\
                 </div></span><span id=t>T</span></div>",
            ),
            [(1, 15.0), (1, 45.0)],
        ),
        // A block with nothing in it breaks inside too.
        (
            "<div style=\"width:20px\"><div style=\"height:30px\"></div><div>A<br>A\
             <div style=\"break-before:avoid;height:40px\"><div></div></div></div>\
             <div><span id=t>T</span></div><div><span id=z>Z</span></div></div>"
                .to_owned(),
            [(1, 35.0), (1, 65.0)],
        ),
    ]);
}
