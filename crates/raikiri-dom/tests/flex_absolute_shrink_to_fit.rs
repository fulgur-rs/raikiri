//! An absolutely positioned flex container whose `width` is `auto` and whose
//! insets leave it open is as wide as `fit-content` makes it: the room the
//! containing block leaves, but at least its min-content width and at most its
//! max-content width (CSS Positioned Layout 3 section 5.1.1, CSS Sizing 3
//! section 3.1.1). Both widths count the items by what they contribute, bounded
//! by their flex base sizes (CSS Flexbox 1 section 9.9.1). The expected boxes are
//! the ones Chromium lays out for the same markup, with an 8px body margin and a
//! green border of 25px around the items.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{
    IntrinsicBox, LayoutConfig, PageDefaults, ReplacedResolver, ResolveDisposition,
    ResolvedIntrinsic, ResolverError, ResolverRequest,
};
use url::Url;

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/data/text-autospace/Ahem.ttf"
));

/// Every image is 20x50 pixels.
struct Portrait;

impl ReplacedResolver for Portrait {
    fn resolve(&self, _request: ResolverRequest<'_>) -> Result<ResolvedIntrinsic, ResolverError> {
        Ok(ResolvedIntrinsic::new(
            IntrinsicBox::new(20.0, 50.0),
            ResolveDisposition::Ok,
        ))
    }
}

/// The border box `(x, y, width, height)` of every element with an `id` when
/// `body` is laid out on a page with an 8px margin.
fn boxes(body: &str) -> Vec<(String, [f32; 4])> {
    let html = format!(
        "<!DOCTYPE html><style>@page {{ size: 800px 600px; margin: 0 }} \
         body {{ margin: 8px; font: 10px/10px Ahem }} div {{ box-sizing: border-box }}</style>{body}"
    );
    let fonts = FontCollectionBuilder::new()
        .font_bytes("Ahem", AHEM.to_vec())
        .build()
        .expect("fonts");
    let resources = RenderResources::new()
        .fonts(fonts)
        .base_url(Url::parse("https://flex.test/document.html").expect("url"))
        .replaced_resolver(&Portrait);
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

/// Check that each element of `expected`, named by its `id`, has the given
/// border box `(x, y, width, height)`.
fn assert_boxes(body: &str, expected: &[(&str, [f32; 4])]) {
    let laid_out = boxes(body);
    for (id, want) in expected {
        let got = laid_out
            .iter()
            .find(|(found, _)| found == id)
            .unwrap_or_else(|| panic!("no box with id {id} in {body}"))
            .1;
        assert!(
            got.iter()
                .zip(want)
                .all(|(got, want)| (got - want).abs() < 0.01),
            "#{id} is {got:?}, expected {want:?} for {body}"
        );
    }
}

const BORDER: &str = "border:solid 25px green";

/// An item named `id` with the given `css` and `content`.
fn item(id: &str, css: &str, content: &str) -> String {
    format!("<div id={id} style=\"{css}\">{content}</div>")
}

/// An absolutely positioned flex container `o` with the given `container_css`
/// that holds `items`, in a containing block that is `room` wide.
fn absolute(room: u32, container_css: &str, items: &str) -> String {
    format!(
        "<div style=\"position:relative;width:{room}px;height:300px\">\
         <div id=o style=\"position:absolute;display:flex;{container_css};{BORDER}\">{items}</div></div>"
    )
}

/// An item of 50px that its maximum width holds to 30px.
fn held_back() -> String {
    item("i", "width:50px;height:50px;max-width:30px", "")
}

#[test]
fn the_maximum_width_of_an_item_sets_the_width_of_an_absolutely_positioned_container() {
    for wrap in ["flex-wrap:nowrap", "flex-wrap:wrap"] {
        assert_boxes(
            &absolute(300, wrap, &held_back()),
            &[
                ("o", [8.0, 8.0, 80.0, 100.0]),
                ("i", [33.0, 33.0, 30.0, 50.0]),
            ],
        );
    }
    // Whatever else it is, the container is as wide as the items make it.
    for container_css in [
        "display:inline-flex",
        "display:grid",
        "flex-direction:column",
    ] {
        assert_boxes(
            &absolute(300, container_css, &held_back()),
            &[
                ("o", [8.0, 8.0, 80.0, 100.0]),
                ("i", [33.0, 33.0, 30.0, 50.0]),
            ],
        );
    }
    let fixed = format!(
        "<div id=o style=\"position:fixed;display:flex;{BORDER}\">{}</div>",
        held_back()
    );
    assert_boxes(
        &fixed,
        &[
            ("o", [8.0, 8.0, 80.0, 100.0]),
            ("i", [33.0, 33.0, 30.0, 50.0]),
        ],
    );
}

#[test]
fn an_absolutely_positioned_container_is_as_wide_as_its_widest_item_where_it_wraps() {
    // The item is flexed to its flex basis of 0, yet the container is as wide
    // as the 30px it states, when it wraps its items.
    let item = item("i", "flex:0 0 0px;width:30px;height:50px", "");
    assert_boxes(
        &absolute(300, "flex-wrap:wrap", &item),
        &[
            ("o", [8.0, 8.0, 80.0, 100.0]),
            ("i", [33.0, 33.0, 0.0, 50.0]),
        ],
    );
    // The same container that does not wrap is as wide as the sum of 0.
    assert_boxes(
        &absolute(300, "flex-wrap:nowrap", &item),
        &[
            ("o", [8.0, 8.0, 50.0, 100.0]),
            ("i", [33.0, 33.0, 0.0, 50.0]),
        ],
    );
}

#[test]
fn the_room_of_the_containing_block_sets_the_width_of_an_absolutely_positioned_container() {
    // The room of 100px less the 50px of border leaves 50px for the text of
    // the item, whose longest line is 80px and longest word 20px.
    let text = item("i", "height:20px", "XX XX XX");
    for wrap in ["flex-wrap:nowrap", "flex-wrap:wrap"] {
        assert_boxes(
            &absolute(100, wrap, &text),
            &[
                ("o", [8.0, 8.0, 100.0, 70.0]),
                ("i", [33.0, 33.0, 50.0, 20.0]),
            ],
        );
    }
    let texts = [text, item("j", "height:20px", "XX XX")].concat();
    assert_boxes(
        &absolute(100, "", &texts),
        &[
            ("o", [8.0, 8.0, 100.0, 70.0]),
            ("i", [33.0, 33.0, 30.0, 20.0]),
            ("j", [63.0, 33.0, 20.0, 20.0]),
        ],
    );
    assert_boxes(
        &absolute(100, "flex-wrap:wrap", &texts),
        &[
            ("o", [8.0, 8.0, 100.0, 90.0]),
            ("i", [33.0, 33.0, 50.0, 20.0]),
            ("j", [33.0, 53.0, 50.0, 20.0]),
        ],
    );
    // An item with no flex basis to speak of takes the width of its text.
    assert_boxes(
        &absolute(100, "", &item("i", "flex:1 1 0px;height:20px", "XX XX")),
        &[
            ("o", [8.0, 8.0, 100.0, 70.0]),
            ("i", [33.0, 33.0, 50.0, 20.0]),
        ],
    );
    // An item that is wider than the room is not narrowed to it.
    assert_boxes(
        &absolute(100, "", &item("i", "width:300px;height:20px", "")),
        &[
            ("o", [8.0, 8.0, 350.0, 70.0]),
            ("i", [33.0, 33.0, 300.0, 20.0]),
        ],
    );
    // The room does not bound the items that fit it.
    assert_boxes(
        &absolute(300, "", &texts),
        &[
            ("o", [8.0, 8.0, 180.0, 70.0]),
            ("i", [33.0, 33.0, 80.0, 20.0]),
            ("j", [113.0, 33.0, 50.0, 20.0]),
        ],
    );
}

#[test]
fn the_items_of_an_absolutely_positioned_container_flex_in_the_width_it_is_given() {
    let items = [
        item("i", "flex:1 1 30px;height:20px", ""),
        item("j", "width:40px;height:20px", ""),
    ]
    .concat();
    assert_boxes(
        &absolute(100, "", &items),
        &[
            ("o", [8.0, 8.0, 90.0, 70.0]),
            ("i", [33.0, 33.0, 17.14, 20.0]),
            ("j", [50.14, 33.0, 22.86, 20.0]),
        ],
    );
    let items = [
        item("i", "width:30px;height:20px", ""),
        item("j", "width:40px;max-width:10px;height:20px", ""),
    ]
    .concat();
    assert_boxes(
        &absolute(100, "", &items),
        &[
            ("o", [8.0, 8.0, 90.0, 70.0]),
            ("i", [33.0, 33.0, 30.0, 20.0]),
            ("j", [63.0, 33.0, 10.0, 20.0]),
        ],
    );
    // The lines of a container that wraps break at the width it is given.
    let items = [
        item("i", "width:30px;height:20px", ""),
        item("j", "width:40px;height:20px", ""),
    ]
    .concat();
    assert_boxes(
        &absolute(100, "flex-wrap:wrap;gap:10px", &items),
        &[
            ("o", [8.0, 8.0, 100.0, 100.0]),
            ("i", [33.0, 33.0, 30.0, 20.0]),
            ("j", [33.0, 63.0, 40.0, 20.0]),
        ],
    );
    assert_boxes(
        &absolute(300, "flex-wrap:wrap;gap:10px", &items),
        &[
            ("o", [8.0, 8.0, 130.0, 70.0]),
            ("i", [33.0, 33.0, 30.0, 20.0]),
            ("j", [73.0, 33.0, 40.0, 20.0]),
        ],
    );
}

#[test]
fn the_insets_of_an_absolutely_positioned_container_leave_its_width_to_its_items() {
    for (container_css, container_box) in [
        ("left:20px", [28.0, 8.0, 80.0, 100.0]),
        ("right:0", [228.0, 8.0, 80.0, 100.0]),
        ("direction:rtl;right:0", [228.0, 8.0, 80.0, 100.0]),
        ("margin:0 7px", [15.0, 8.0, 80.0, 100.0]),
        ("top:10px;bottom:10px", [8.0, 18.0, 80.0, 280.0]),
        ("width:fit-content", [8.0, 8.0, 80.0, 100.0]),
    ] {
        let [x, y, ..] = container_box;
        assert_boxes(
            &absolute(300, container_css, &held_back()),
            &[
                ("o", container_box),
                ("i", [x + 25.0, y + 25.0, 30.0, 50.0]),
            ],
        );
    }
}

#[test]
fn an_absolutely_positioned_container_with_a_width_of_its_own_keeps_it() {
    for (container_css, container_box, item_width) in [
        ("left:20px;right:20px", [28.0, 8.0, 260.0, 100.0], 30.0),
        ("width:200px", [8.0, 8.0, 200.0, 100.0], 30.0),
        ("min-width:120px", [8.0, 8.0, 120.0, 100.0], 30.0),
        ("max-width:60px", [8.0, 8.0, 60.0, 100.0], 10.0),
    ] {
        let [x, y, ..] = container_box;
        assert_boxes(
            &absolute(300, container_css, &held_back()),
            &[
                ("o", container_box),
                ("i", [x + 25.0, y + 25.0, item_width, 50.0]),
            ],
        );
    }
}

#[test]
fn an_absolutely_positioned_container_in_other_containers_is_as_wide_as_its_items() {
    let container = |css: &str| {
        format!(
            "<div style=\"{css}\"><div id=o style=\"position:absolute;display:flex;{BORDER}\">{}</div></div>",
            held_back()
        )
    };
    for css in [
        "position:relative;width:200px",
        "display:flex;position:relative;width:300px;height:200px",
        "display:inline-block;position:relative;width:100px;height:100px",
    ] {
        assert_boxes(
            &container(css),
            &[
                ("o", [8.0, 8.0, 80.0, 100.0]),
                ("i", [33.0, 33.0, 30.0, 50.0]),
            ],
        );
    }
    assert_boxes(
        &format!(
            "<div style=\"width:200px\">XX<div id=o style=\"position:absolute;display:flex;{BORDER}\">{}</div></div>",
            held_back()
        ),
        &[
            ("o", [8.0, 18.0, 80.0, 100.0]),
            ("i", [33.0, 43.0, 30.0, 50.0]),
        ],
    );
    // In a vertical writing mode the containing block is on the other side.
    assert_boxes(
        &container("writing-mode:vertical-rl;position:relative;width:300px;height:300px"),
        &[
            ("o", [228.0, 8.0, 80.0, 100.0]),
            ("i", [253.0, 33.0, 30.0, 50.0]),
        ],
    );
    // A container that is the item of the container is sized as one.
    let nested = format!(
        "<div id=o style=\"position:absolute;display:flex;{BORDER}\">\
         <div id=q style=\"display:flex;border:solid 10px blue\">{}</div></div>",
        held_back()
    );
    assert_boxes(
        &nested,
        &[
            ("o", [8.0, 8.0, 100.0, 120.0]),
            ("q", [33.0, 33.0, 50.0, 70.0]),
            ("i", [43.0, 43.0, 30.0, 50.0]),
        ],
    );
}

#[test]
fn an_absolutely_positioned_container_with_columns_that_wrap_is_as_wide_as_they_need_within_the_room()
 {
    let items = ["i", "j", "k"]
        .map(|id| item(id, "width:30px;height:40px", ""))
        .concat();
    let css = "flex-direction:column;flex-wrap:wrap;height:100px";
    // The columns need 140px; the room leaves less, and the container is no
    // narrower than the 80px the least of them takes with their border.
    for (room, width) in [(60, 80.0), (100, 100.0), (300, 140.0)] {
        assert_boxes(
            &absolute(room, css, &items),
            &[
                ("o", [8.0, 8.0, width, 100.0]),
                ("i", [33.0, 33.0, 30.0, 40.0]),
                ("j", [63.0, 33.0, 30.0, 40.0]),
                ("k", [93.0, 33.0, 30.0, 40.0]),
            ],
        );
    }
}

#[test]
fn an_absolutely_positioned_container_in_a_vertical_writing_mode_is_sized_as_before() {
    let items = [
        item("i", "width:30px;height:20px", ""),
        item("j", "width:40px;height:20px", ""),
    ]
    .concat();
    for (css, height) in [
        ("writing-mode:vertical-rl", 90.0),
        ("writing-mode:vertical-rl;height:100px", 100.0),
    ] {
        assert_boxes(
            &absolute(60, css, &items),
            &[
                ("o", [8.0, 8.0, 90.0, height]),
                ("i", [43.0, 33.0, 30.0, 20.0]),
                ("j", [33.0, 53.0, 40.0, 20.0]),
            ],
        );
    }
}
