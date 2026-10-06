//! Whitespace-only text between block-level boxes collapses away and
//! generates no anonymous block box (CSS 2.1, 9.2.1.1 and 16.6.1), so it
//! does not separate adjoining vertical margins (CSS 2.1, 8.3.1). White
//! space that `white-space` preserves forms a line box instead.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/data/text-autospace/Ahem.ttf"
));

/// Border box `(y, height)` of each element of `ids` in `body`, laid out on
/// the first page of a standards-mode document with a 10px Ahem line height
/// and no body margin.
fn boxes<const N: usize>(body: &str, ids: [&str; N]) -> [(f32, f32); N] {
    let fonts = FontCollectionBuilder::new()
        .font_bytes("Ahem", AHEM.to_vec())
        .build()
        .expect("fonts");
    let resources = RenderResources::new().fonts(fonts);
    let html = format!(
        "<!DOCTYPE html><style>body{{margin:0;font:10px/10px Ahem}}</style><body>{body}</body>"
    );
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
    let page = result.page(0).expect("first page");
    let dom = page.dom();
    ids.map(|id| {
        let fragment = page
            .fragments()
            .find(|fragment| dom.attr(fragment.node(), "id") == Some(id))
            .unwrap_or_else(|| panic!("no fragment for #{id} in {body}"));
        let rect = fragment.rect();
        (rect.y, rect.height)
    })
}

/// Distance from the bottom of `#a` to the top of `#b`.
fn gap(body: &str) -> f32 {
    let [a, b] = boxes(body, ["a", "b"]);
    b.0 - (a.0 + a.1)
}

const A: &str = r#"<div id=a style="height:80px;margin-bottom:40px"></div>"#;
const B: &str = r#"<div id=b style="height:10px;margin-top:30px"></div>"#;

#[test]
fn siblings_with_source_whitespace_between_collapse_their_margins() {
    for between in ["", "\n", "\n    ", " \t\r\n "] {
        assert_eq!(gap(&format!("{A}{between}{B}")), 40.0, "{between:?}");
    }
}

#[test]
fn headings_separated_by_newlines_collapse_with_their_neighbours() {
    let body = "<style>h2{font-size:14px;margin-top:30px}</style>\n<h2>A</h2>\n\
        <div id=a style=\"height:80px;margin-bottom:40px\"></div>\n<h2 id=b>B</h2>";
    assert_eq!(gap(body), 40.0);
}

#[test]
fn comments_and_whitespace_between_siblings_generate_no_box() {
    for between in [
        "<!-- c -->",
        "\n<!-- c -->\n",
        "\n  <!-- c -->  <!-- d -->\n",
        "<!-- c -->\n<!-- d -->",
    ] {
        assert_eq!(gap(&format!("{A}{between}{B}")), 40.0, "{between:?}");
    }
}

#[test]
fn a_parent_and_its_first_child_collapse_their_top_margins_through_whitespace() {
    // The child's 30px margin escapes the parent and collapses with its 15px
    // one: both border boxes start 30px down.
    for space in ["", "\n", "\n  "] {
        let body = format!(
            r#"<div id=parent style="margin-top:15px">{space}<div id=child style="height:10px;margin-top:30px"></div>{space}</div>"#
        );
        let [parent, child] = boxes(&body, ["parent", "child"]);
        assert_eq!((parent.0, child.0), (30.0, 30.0), "{space:?}");
    }
}

#[test]
fn a_parent_and_its_last_child_collapse_their_bottom_margins_through_whitespace() {
    // The child's 30px margin escapes the parent's bottom edge and collapses
    // with its 15px one: the parent is as tall as the child and the next
    // block starts 30px below it.
    for space in ["", "\n", "\n  "] {
        let body = format!(
            r#"<div id=a style="margin-bottom:15px">{space}<div style="height:10px;margin-bottom:30px"></div>{space}</div>{space}<div id=b style="height:10px"></div>"#
        );
        let [a, b] = boxes(&body, ["a", "b"]);
        assert_eq!(a.1, 10.0, "{space:?}");
        assert_eq!(b.0 - (a.0 + a.1), 30.0, "{space:?}");
    }
}

#[test]
fn preserved_white_space_between_blocks_forms_a_line() {
    // CSS 2.1 9.4.2: a line box holding preserved white space is not
    // zero-height, so it separates the margins: 40 + one 10px line + 30.
    for (white_space, between) in [
        ("pre", "\n"),
        ("pre", "  "),
        ("pre-wrap", "\n"),
        ("pre-wrap", " "),
        ("break-spaces", " "),
        ("pre-line", "\n"),
        ("pre-line", " \n "),
    ] {
        let body = format!(r#"<div style="white-space:{white_space}">{A}{between}{B}</div>"#);
        assert_eq!(gap(&body), 80.0, "{white_space} {between:?}");
    }
}

#[test]
fn white_space_that_still_collapses_generates_no_line() {
    // `pre-line` keeps segment breaks only; `nowrap` collapses like `normal`.
    for (white_space, between) in [("pre-line", "  "), ("nowrap", "\n"), ("normal", "\n  ")] {
        let body = format!(r#"<div style="white-space:{white_space}">{A}{between}{B}</div>"#);
        assert_eq!(gap(&body), 40.0, "{white_space} {between:?}");
    }
}

#[test]
fn out_of_flow_boxes_and_whitespace_between_siblings_keep_margins_adjoining() {
    // Floats and absolutely positioned boxes are out of flow: the margins on
    // either side of them, and of the white space around them, still collapse.
    for between in [
        r#"<div style="float:left;width:20px;height:20px"></div>"#,
        "\n<div style=\"float:left;width:20px;height:20px\"></div>\n",
        "\n<div style=\"position:absolute;width:20px;height:20px\"></div>\n",
    ] {
        assert_eq!(gap(&format!("{A}{between}{B}")), 40.0, "{between:?}");
    }
}
