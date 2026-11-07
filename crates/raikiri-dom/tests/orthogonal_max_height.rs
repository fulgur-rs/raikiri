//! A vertical box in a horizontal block, flex or paragraph container fits its
//! lines in the height of the initial containing block, the page area here,
//! unless the container is no taller than that: the room is cut at the
//! `max-height` of the container when it is a length, raised to its
//! `min-height` (CSS Writing Modes 4 §7.3.1, CSS Sizing 3 §5). A hundred words
//! of `aa` in Ahem at 10px are 20px wide with 10px between them, so a room of
//! 600px holds 20 words on a line (5 columns of 10px), and one of 300px holds
//! 10 (10 columns). The expected border boxes are the ones Chromium lays out
//! for the same markup, in an 800px by 600px viewport without margins.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/data/text-autospace/Ahem.ttf"
));

/// A hundred words, 3000px of text on a line.
fn hundred_words() -> String {
    ["aa"; 100].join(" ")
}

/// The border box `(x, y, width, height)` of every element with an `id` when
/// `body` is laid out on a page without margins.
fn boxes(body: &str) -> Vec<(String, [f32; 4])> {
    let html = format!(
        "<!DOCTYPE html><style>@page {{ size: 800px 600px; margin: 0 }} \
         body {{ margin: 0; font: 10px/10px Ahem }} div {{ box-sizing: border-box }}</style>{body}"
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
    let page = result.pages().next().expect("a page");
    let dom = page.dom();
    page.fragments()
        .filter_map(|fragment| {
            let id = dom.attr(fragment.node(), "id")?;
            let rect = fragment.rect();
            Some((id.to_owned(), [rect.x, rect.y, rect.width, rect.height]))
        })
        .collect()
}

/// Check that every `(id, expected box)` of `expected` is laid out as given.
fn assert_boxes(body: &str, expected: &[(&str, [f32; 4])]) {
    let actual = boxes(body);
    for &(id, want) in expected {
        let got = actual
            .iter()
            .find(|(name, _)| name == id)
            .unwrap_or_else(|| panic!("no fragment for #{id} in {body}"))
            .1;
        assert!(
            got.iter()
                .zip(want)
                .all(|(got, want)| (got - want).abs() < 0.01),
            "#{id} is {got:?}, expected {want:?} for {body}"
        );
    }
}

/// Check that every `(id, [x, y, width])` of `expected` is laid out as given.
/// A box that is taller than the page is cut at its bottom edge, so its height
/// is left out.
fn assert_origin_and_width(body: &str, expected: &[(&str, [f32; 3])]) {
    let actual = boxes(body);
    for &(id, want) in expected {
        let got = actual
            .iter()
            .find(|(name, _)| name == id)
            .unwrap_or_else(|| panic!("no fragment for #{id} in {body}"))
            .1;
        let got = [got[0], got[1], got[2]];
        assert!(
            got.iter()
                .zip(want)
                .all(|(got, want)| (got - want).abs() < 0.01),
            "#{id} starts at and is as wide as {got:?}, expected {want:?} for {body}"
        );
    }
}

/// The container `#p` of `container_css`, 300px wide, with the vertical box
/// `#o` of `tag` and `css` holding a hundred words.
fn container(container_css: &str, tag: &str, css: &str) -> String {
    format!(
        "<div id=p style=\"width:300px;{container_css}\">\
         <{tag} id=o style=\"writing-mode:vertical-rl;{css}\">{}</{tag}></div>",
        hundred_words()
    )
}

#[test]
fn a_max_height_of_the_container_is_the_height_a_vertical_box_fits_its_lines_in() {
    // The page is 600px tall, the container no taller than 300px: the lines
    // of `#o` break at 300px, ten columns of 10px, and not at 600px (five).
    for (container_css, tag, css, container_height) in [
        ("max-height:300px", "div", "", 300.0),
        ("max-height:300px;overflow:hidden", "div", "", 300.0),
        ("max-height:300px;display:flow-root", "div", "", 300.0),
        ("max-height:300px;display:table-cell", "div", "", 300.0),
        ("max-height:300px;position:absolute", "div", "", 300.0),
        ("max-height:300px", "div", "writing-mode:vertical-lr", 300.0),
        ("max-height:300px", "span", "display:inline-block", 300.0),
        ("max-height:300px", "div", "float:left", 0.0),
        ("max-height:300px;display:flex", "div", "", 300.0),
        (
            "max-height:300px;display:flex;flex-wrap:wrap",
            "div",
            "",
            300.0,
        ),
        ("max-height:300px;display:inline-flex", "div", "", 300.0),
        // The limit is a length computed from the page or from other lengths.
        ("max-height:calc(100px + 200px)", "div", "", 300.0),
        ("max-height:50vh", "div", "", 300.0),
    ] {
        assert_boxes(
            &container(container_css, tag, css),
            &[
                ("p", [0.0, 0.0, 300.0, container_height]),
                ("o", [0.0, 0.0, 100.0, 300.0]),
            ],
        );
    }
}

#[test]
fn a_max_height_of_a_table_cell_is_the_height_a_vertical_box_fits_its_lines_in() {
    // The cell is laid out in a row of a table, or in the anonymous table that
    // wraps it, and still bounds the room of the lines.
    let cell = container("max-height:300px;display:table-cell", "div", "");
    for body in [
        format!("<div style=\"display:table\">{cell}</div>"),
        format!("<div style=\"display:table\"><div style=\"display:table-row\">{cell}</div></div>"),
        format!("<div style=\"display:table-row\">{cell}</div>"),
    ] {
        assert_boxes(
            &body,
            &[
                ("p", [0.0, 0.0, 300.0, 300.0]),
                ("o", [0.0, 0.0, 100.0, 300.0]),
            ],
        );
    }
}

#[test]
fn text_before_an_inline_block_does_not_change_the_height_it_fits_its_lines_in() {
    assert_boxes(
        &format!(
            "<div id=p style=\"width:300px;max-height:300px\">x<span id=o \
             style=\"display:inline-block;writing-mode:vertical-rl\">{}</span></div>",
            hundred_words()
        ),
        &[("o", [10.0, 0.0, 100.0, 300.0])],
    );
}

#[test]
fn a_shrink_to_fit_container_is_as_wide_as_the_columns_that_fit_its_max_height() {
    // The width of the container is the width of its box, which fits 300px.
    for (container_css, tag, css) in [
        (
            "display:inline-block;max-height:300px",
            "span",
            "display:inline-block",
        ),
        ("display:inline-block;max-height:300px", "div", "float:left"),
        ("float:left;max-height:300px", "div", "float:left"),
    ] {
        assert_boxes(
            &format!(
                "<div id=p style=\"{container_css}\"><{tag} id=o \
                 style=\"writing-mode:vertical-rl;{css}\">{}</{tag}></div>",
                hundred_words()
            ),
            &[
                ("p", [0.0, 0.0, 100.0, 300.0]),
                ("o", [0.0, 0.0, 100.0, 300.0]),
            ],
        );
    }
}

#[test]
fn the_padding_and_border_of_the_container_come_off_a_border_box_max_height() {
    // `div` is border-box here: the 300px limit holds 280px of the content
    // box between 10px of padding, or 290px between 5px of border.
    for (container_css, tag, css) in [
        ("max-height:300px;padding:10px 0", "div", ""),
        (
            "max-height:300px;padding:10px 0",
            "span",
            "display:inline-block",
        ),
        ("max-height:300px;padding:10px 0;display:flex", "div", ""),
    ] {
        assert_boxes(
            &container(container_css, tag, css),
            &[
                ("p", [0.0, 0.0, 300.0, 300.0]),
                ("o", [0.0, 10.0, 120.0, 280.0]),
            ],
        );
    }
    assert_boxes(
        &container("max-height:300px;border:5px solid", "div", ""),
        &[
            ("p", [0.0, 0.0, 300.0, 300.0]),
            ("o", [5.0, 5.0, 100.0, 290.0]),
        ],
    );
    // A content-box limit is the height of the content box already.
    assert_boxes(
        &container(
            "box-sizing:content-box;max-height:300px;padding:10px 0",
            "div",
            "",
        ),
        &[
            ("p", [0.0, 0.0, 300.0, 320.0]),
            ("o", [0.0, 10.0, 100.0, 300.0]),
        ],
    );
}

#[test]
fn the_margins_of_the_vertical_box_come_off_the_height_the_limit_leaves() {
    // 300px less 10px of margin above and below leaves 280px: twelve columns.
    for (container_css, css) in [
        ("max-height:300px", "margin:10px 0"),
        ("max-height:300px;display:flex", "margin:10px 0"),
        ("max-height:300px", "float:left;margin:10px 0"),
    ] {
        assert_boxes(
            &container(container_css, "div", css),
            &[("o", [0.0, 10.0, 120.0, 280.0])],
        );
    }
}

#[test]
fn a_border_box_max_height_with_a_percentage_padding_above_or_below_is_no_limit() {
    // The percentage of the padding refers to the width of the wrapper, 400px:
    // 40px. A browser cannot take it off the limit, so the limit is not
    // applied, and the lines of `#o` break at the page height, five columns of
    // 10px, below the padding.
    for (container_css, tag, css, top) in [
        ("padding:10% 0", "div", "", 40.0),
        ("padding:10% 0", "span", "display:inline-block", 40.0),
        ("padding:10% 0", "div", "float:left", 40.0),
        (
            "padding:10% 0;display:flex;align-self:flex-start",
            "div",
            "",
            40.0,
        ),
        ("padding:calc(10% + 5px) 0", "div", "", 45.0),
        ("padding:10% 0 0", "div", "", 40.0),
        ("padding-top:10%", "div", "", 40.0),
        ("padding:10% 0;position:absolute", "div", "", 80.0),
    ] {
        assert_origin_and_width(
            &format!(
                "<div style=\"width:400px\">{}</div>",
                container(&format!("max-height:300px;{container_css}"), tag, css)
            ),
            &[("o", [0.0, top, 50.0])],
        );
    }
    assert_origin_and_width(
        &format!(
            "<div style=\"width:400px\">{}</div>",
            container("max-height:300px;padding:10% 0;border:5px solid", "div", "")
        ),
        &[("o", [5.0, 45.0, 50.0])],
    );
    // Whatever the padding comes to, also when it is nothing.
    for container_css in ["padding:0 0 10%", "padding:0% 0"] {
        assert_boxes(
            &format!(
                "<div style=\"width:400px\">{}</div>",
                container(&format!("max-height:300px;{container_css}"), "div", "")
            ),
            &[
                ("p", [0.0, 0.0, 300.0, 300.0]),
                ("o", [0.0, 0.0, 50.0, 600.0]),
            ],
        );
    }
}

#[test]
fn a_percentage_padding_beside_the_box_or_in_a_content_box_limit_still_cuts_the_room() {
    // A content-box limit is the content box already: 300px.
    assert_boxes(
        &format!(
            "<div style=\"width:400px\">{}</div>",
            container(
                "box-sizing:content-box;max-height:300px;padding:10% 0",
                "div",
                ""
            )
        ),
        &[
            ("p", [0.0, 0.0, 300.0, 380.0]),
            ("o", [0.0, 40.0, 100.0, 300.0]),
        ],
    );
    // The percentages on the left and right do not take part in the height.
    for (tag, css) in [("div", ""), ("span", "display:inline-block")] {
        assert_boxes(
            &format!(
                "<div style=\"width:400px\">{}</div>",
                container("max-height:300px;padding:10px 10%", tag, css)
            ),
            &[
                ("p", [0.0, 0.0, 300.0, 300.0]),
                ("o", [40.0, 10.0, 120.0, 280.0]),
            ],
        );
    }
    assert_boxes(
        &format!(
            "<div style=\"width:400px\">{}</div>",
            container("max-height:300px;padding:0 10%", "div", "")
        ),
        &[("o", [40.0, 0.0, 100.0, 300.0])],
    );
}

#[test]
fn a_min_height_raises_the_limit_of_a_smaller_max_height() {
    assert_boxes(
        &container("min-height:400px;max-height:300px", "div", ""),
        &[
            ("p", [0.0, 0.0, 300.0, 400.0]),
            ("o", [0.0, 0.0, 80.0, 400.0]),
        ],
    );
    // A min-height below the limit leaves it as it is.
    assert_boxes(
        &container("min-height:100px;max-height:300px", "div", ""),
        &[
            ("p", [0.0, 0.0, 300.0, 300.0]),
            ("o", [0.0, 0.0, 100.0, 300.0]),
        ],
    );
}

#[test]
fn a_definite_height_of_the_container_is_the_room_as_its_limits_clamp_it() {
    assert_boxes(
        &container("height:200px;max-height:300px", "div", ""),
        &[
            ("p", [0.0, 0.0, 300.0, 200.0]),
            ("o", [0.0, 0.0, 150.0, 200.0]),
        ],
    );
    // A height above the limit, or below the minimum, is the one it is
    // clamped to, for every kind of box.
    for (container_css, tag, css) in [
        ("height:500px;max-height:300px", "div", ""),
        (
            "height:500px;max-height:300px",
            "span",
            "display:inline-block",
        ),
        ("height:500px;max-height:300px", "div", "float:left"),
        ("height:500px;max-height:300px;display:flex", "div", ""),
        ("height:100px;min-height:300px", "div", ""),
        ("height:100px;min-height:300px", "div", "float:left"),
    ] {
        assert_boxes(
            &container(container_css, tag, css),
            &[
                ("p", [0.0, 0.0, 300.0, 300.0]),
                ("o", [0.0, 0.0, 100.0, 300.0]),
            ],
        );
    }
}

#[test]
fn a_max_height_that_does_not_cut_the_page_leaves_the_page_height_the_room() {
    let page = ("o", [0.0, 0.0, 50.0, 600.0]);
    // A limit above the page height, or a percentage of a height that is
    // definite: the container is cut at it, the lines of the box are not.
    assert_boxes(
        &container("max-height:900px", "div", ""),
        &[("p", [0.0, 0.0, 300.0, 600.0]), page],
    );
    assert_boxes(
        &format!(
            "<div style=\"height:400px\">{}</div>",
            container("max-height:50%", "div", "")
        ),
        &[("p", [0.0, 0.0, 300.0, 200.0]), page],
    );
    assert_boxes(
        &format!(
            "<div style=\"height:400px\">{}</div>",
            container("max-height:calc(50% + 100px)", "div", "")
        ),
        &[("p", [0.0, 0.0, 300.0, 300.0]), page],
    );
    // Only the container the box is in counts, not the ones around it.
    assert_boxes(
        &format!(
            "<div style=\"max-height:300px\">{}</div>",
            container("", "div", "")
        ),
        &[("p", [0.0, 0.0, 300.0, 600.0]), page],
    );
    let nested = format!(
        "<div id=p style=\"width:300px;max-height:300px\"><div id=q>\
         <div id=o style=\"writing-mode:vertical-rl\">{}</div></div></div>",
        hundred_words()
    );
    assert_boxes(&nested, &[("q", [0.0, 0.0, 300.0, 600.0]), page]);
}

#[test]
fn a_short_text_keeps_the_height_of_its_line_under_a_max_height() {
    assert_boxes(
        "<div id=p style=\"width:300px;max-height:300px\">\
         <div id=o style=\"writing-mode:vertical-rl\">aa aa aa</div></div>",
        &[
            ("p", [0.0, 0.0, 300.0, 80.0]),
            ("o", [0.0, 0.0, 10.0, 80.0]),
        ],
    );
}

#[test]
fn a_column_flex_container_stretches_a_vertical_item_to_its_max_height() {
    // The item is as wide as the container, not as the columns of a room.
    assert_boxes(
        &container(
            "display:flex;flex-direction:column;max-height:300px",
            "div",
            "",
        ),
        &[
            ("p", [0.0, 0.0, 300.0, 300.0]),
            ("o", [0.0, 0.0, 300.0, 300.0]),
        ],
    );
}
