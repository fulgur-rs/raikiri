//! A row flex container that wraps its items is as wide as its widest item at
//! its min-content width, and the width of an item at that size is the one it
//! states or the one of its content, held within its `min-width` and
//! `max-width`: the flex base size of an item, which a `flex-basis` of `0` or a
//! `max-width` below the width it states does not leave as it is, plays no part
//! (CSS Flexbox 1 section 9.9.1). The expected boxes are the ones Chromium lays
//! out for the same markup, with an 8px body margin and a green border of 25px
//! around the items.

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

/// Check that the container `o` of `body` has the given border box size,
/// starting at the body margin, and that each of `items` is laid out with the
/// given border box `(x, y, width, height)`.
fn assert_boxes(body: &str, container: [f32; 2], items: &[(&str, [f32; 4])]) {
    let laid_out = boxes(body);
    let find = |id: &str| {
        laid_out
            .iter()
            .find(|(found, _)| found == id)
            .unwrap_or_else(|| panic!("no box with id {id} in {body}"))
            .1
    };
    let container_box = [8.0, 8.0, container[0], container[1]];
    for (id, want) in std::iter::once(&("o", container_box)).chain(items) {
        let got = find(id);
        assert!(
            got.iter()
                .zip(want)
                .all(|(got, want)| (got - want).abs() < 0.01),
            "#{id} is {got:?}, expected {want:?} for {body}"
        );
    }
}

/// An inline flex container that wraps its items, with a green border of 25px.
const WRAP: &str = "display:inline-flex;flex-wrap:wrap;border:solid 25px green";

/// A container with the given `container_css` that holds `items`.
fn container(container_css: &str, items: &str) -> String {
    format!("<div id=o style=\"{container_css}\">{items}</div>")
}

/// An item named `id` with the given `css` and `content`.
fn item(id: &str, css: &str, content: &str) -> String {
    format!("<div id={id} style=\"{css}\">{content}</div>")
}

#[test]
fn a_maximum_width_below_the_width_an_item_states_sets_the_width_of_a_container_that_wraps() {
    let image = "<img id=i src=\"https://flex.test/portrait.png\" \
                 style=\"height:50px;max-width:10px\">";
    for (items, size, item_box) in [
        (
            item("i", "width:50px;height:50px;max-width:30px", ""),
            [80.0, 100.0],
            [33.0, 33.0, 30.0, 50.0],
        ),
        // The flex basis is no wider than the maximum width either.
        (
            item("i", "flex:0 0 100px;max-width:30px;height:50px", ""),
            [80.0, 100.0],
            [33.0, 33.0, 30.0, 50.0],
        ),
        (
            item("i", "flex:0 0 30px;max-width:20px;height:50px", ""),
            [70.0, 100.0],
            [33.0, 33.0, 20.0, 50.0],
        ),
        // The width of an image follows its height, within its maximum width.
        (image.to_owned(), [60.0, 100.0], [33.0, 33.0, 10.0, 50.0]),
        // The margins of the item count.
        (
            item(
                "i",
                "width:50px;height:50px;max-width:30px;margin:0 7px",
                "",
            ),
            [94.0, 100.0],
            [40.0, 33.0, 30.0, 50.0],
        ),
    ] {
        assert_boxes(&container(WRAP, &items), size, &[("i", item_box)]);
    }
}

#[test]
fn an_item_with_a_ratio_in_a_container_that_wraps_counts_by_its_maximum_width() {
    // The ratio of 1 gives 40px for the height of 40px, which the maximum width
    // of 30px holds back to 30px.
    let div = item("i", "aspect-ratio:1/1;height:40px;max-width:30px", "");
    let image = "<img id=i src=\"https://flex.test/portrait.png\" \
                 style=\"aspect-ratio:1/1;height:40px;max-width:30px\">";
    for items in [div, image.to_owned()] {
        assert_boxes(
            &container(WRAP, &items),
            [80.0, 90.0],
            &[("i", [33.0, 33.0, 30.0, 40.0])],
        );
    }
}

#[test]
fn a_flex_basis_of_zero_does_not_take_the_width_an_item_states_out_of_a_container_that_wraps() {
    // The container is as wide as the widest item states, 30px, though every
    // item is flexed to its flex basis of 0.
    assert_boxes(
        &container(WRAP, &item("i", "flex:0 0 0px;width:30px;height:50px", "")),
        [80.0, 100.0],
        &[("i", [33.0, 33.0, 0.0, 50.0])],
    );
    // It is the widest of the items, not their sum.
    let items = [
        item("i", "flex:0 0 0px;width:20px;height:50px", ""),
        item("j", "flex:0 0 0px;width:30px;height:50px", ""),
        item("k", "flex:0 0 0px;width:40px;height:50px", ""),
    ]
    .concat();
    assert_boxes(
        &container(WRAP, &items),
        [90.0, 100.0],
        &[
            ("i", [33.0, 33.0, 0.0, 50.0]),
            ("j", [33.0, 33.0, 0.0, 50.0]),
            ("k", [33.0, 33.0, 0.0, 50.0]),
        ],
    );
}

#[test]
fn a_container_that_wraps_breaks_its_items_onto_lines_of_their_own_at_its_min_content_width() {
    // The room offered, 100px, is less than the three items put side by side
    // need and more than the widest of them does, 40px, so each line holds one.
    let items = ["i", "j", "k"]
        .map(|id| item(id, "width:60px;max-width:40px;height:20px", ""))
        .concat();
    let body = format!(
        "<div style=\"width:100px\">{}</div>",
        container(WRAP, &items)
    );
    assert_boxes(
        &body,
        [100.0, 110.0],
        &[
            ("i", [33.0, 33.0, 40.0, 20.0]),
            ("j", [33.0, 53.0, 40.0, 20.0]),
            ("k", [33.0, 73.0, 40.0, 20.0]),
        ],
    );
    // The same with items of 0 for a flex basis.
    let items = ["i", "j", "k"]
        .map(|id| item(id, "flex:0 0 0px;width:40px;height:20px", ""))
        .concat();
    let body = format!(
        "<div style=\"width:100px\">{}</div>",
        container(WRAP, &items)
    );
    assert_boxes(
        &body,
        [90.0, 70.0],
        &[
            ("i", [33.0, 33.0, 0.0, 20.0]),
            ("j", [33.0, 33.0, 0.0, 20.0]),
            ("k", [33.0, 33.0, 0.0, 20.0]),
        ],
    );
}

#[test]
fn a_container_that_wraps_takes_the_longest_word_of_its_items_for_its_min_content_width() {
    let items = [
        item("i", "height:20px", "XX XX XX"),
        item("j", "height:20px", "XX XX"),
    ]
    .concat();
    // The room of 60px holds the words of 20px of each item but not the items.
    let body = format!(
        "<div style=\"width:60px\">{}</div>",
        container(WRAP, &items)
    );
    assert_boxes(
        &body,
        [70.0, 90.0],
        &[
            ("i", [33.0, 33.0, 20.0, 20.0]),
            ("j", [33.0, 53.0, 20.0, 20.0]),
        ],
    );
}

#[test]
fn a_container_that_wraps_with_a_min_content_width_is_as_wide_as_its_widest_item() {
    let item = item("i", "width:50px;height:50px;max-width:30px", "");
    assert_boxes(
        &container(&format!("{WRAP};width:min-content"), &item),
        [80.0, 100.0],
        &[("i", [33.0, 33.0, 30.0, 50.0])],
    );
    // A float is as wide as the container would be at its preferred width,
    // which that of its widest item may not exceed.
    assert_boxes(
        &container(&format!("{WRAP};float:left"), &item),
        [80.0, 100.0],
        &[("i", [33.0, 33.0, 30.0, 50.0])],
    );
    assert_boxes(
        &container(&format!("{WRAP};flex-direction:row-reverse"), &item),
        [80.0, 100.0],
        &[("i", [33.0, 33.0, 30.0, 50.0])],
    );
}

#[test]
fn the_width_of_a_container_that_wraps_is_unchanged_where_the_items_state_what_they_are() {
    // An item that cannot shrink is no narrower than its flex basis.
    assert_boxes(
        &container(
            WRAP,
            &item("i", "flex:0 0 100px;width:30px;height:50px", ""),
        ),
        [150.0, 100.0],
        &[("i", [33.0, 33.0, 100.0, 50.0])],
    );
    // The gap lies between the items on a line.
    let items = [
        item("i", "width:30px;height:20px", ""),
        item("j", "width:40px;height:20px", ""),
    ]
    .concat();
    assert_boxes(
        &container(&format!("{WRAP};column-gap:10px;row-gap:5px"), &items),
        [130.0, 70.0],
        &[
            ("i", [33.0, 33.0, 30.0, 20.0]),
            ("j", [73.0, 33.0, 40.0, 20.0]),
        ],
    );
}

#[test]
fn an_item_that_can_shrink_counts_by_its_content_in_a_container_that_wraps() {
    // The items can grow and shrink, so each contributes the width of its text,
    // 30px and 20px, and the container is 50px wide inside its border. The flex
    // bases of 60px do not fit on a line of that width side by side: each item
    // has a line to itself, as wide as the container.
    let items = [
        item("i", "flex:1 1 60px;height:20px", "XXX"),
        item("j", "flex:1 1 60px;height:20px", "XX"),
    ]
    .concat();
    assert_boxes(
        &container(WRAP, &items),
        [100.0, 90.0],
        &[
            ("i", [33.0, 33.0, 50.0, 20.0]),
            ("j", [33.0, 53.0, 50.0, 20.0]),
        ],
    );
}

#[test]
fn a_container_that_wraps_is_not_sized_by_its_items_where_something_else_sizes_it() {
    let two = [
        item("i", "width:40px;height:20px", ""),
        item("j", "width:40px;height:20px", ""),
    ]
    .concat();
    // A width of its own, inside a container that measures it.
    let inner = format!("<div id=m style=\"display:flex;flex-wrap:wrap;width:60px\">{two}</div>");
    assert_boxes(
        &container("display:inline-flex;border:solid 25px green", &inner),
        [110.0, 90.0],
        &[
            ("m", [33.0, 33.0, 60.0, 40.0]),
            ("i", [33.0, 33.0, 40.0, 20.0]),
            ("j", [33.0, 53.0, 40.0, 20.0]),
        ],
    );
    // The same inside a container that wraps its items, which asks for the width
    // the container contributes.
    assert_boxes(
        &container(WRAP, &inner),
        [110.0, 90.0],
        &[
            ("m", [33.0, 33.0, 60.0, 40.0]),
            ("i", [33.0, 33.0, 40.0, 20.0]),
            ("j", [33.0, 53.0, 40.0, 20.0]),
        ],
    );
    // A container that wraps its items onto vertical lines.
    let lines = [
        item("i", "width:30px;height:20px", ""),
        item("j", "width:40px;height:20px", ""),
    ]
    .concat();
    assert_boxes(
        &container(&format!("{WRAP};writing-mode:vertical-rl"), &lines),
        [90.0, 90.0],
        &[
            ("i", [43.0, 33.0, 30.0, 20.0]),
            ("j", [33.0, 53.0, 40.0, 20.0]),
        ],
    );
    // A container of columns.
    assert_boxes(
        &container(
            &format!("{WRAP};flex-direction:column;height:100px"),
            &[
                item("i", "width:30px;height:60px", ""),
                item("j", "width:40px;height:60px", ""),
            ]
            .concat(),
        ),
        [120.0, 100.0],
        &[
            ("i", [33.0, 33.0, 30.0, 50.0]),
            ("j", [63.0, 33.0, 40.0, 50.0]),
        ],
    );
}

#[test]
fn a_container_that_wraps_inside_another_is_as_wide_as_its_widest_item_at_the_least_room() {
    // The outer container is 100px wide, 50px inside its border, and the three
    // items of the inner one break onto lines of their own at that width.
    let three = ["i", "j", "k"]
        .map(|id| item(id, "width:40px;height:20px", ""))
        .concat();
    let inner = format!(
        "<div id=m style=\"display:flex;flex-wrap:wrap;border:solid 25px green\">{three}</div>"
    );
    let body = format!(
        "<div style=\"width:100px\">{}</div>",
        container("display:inline-flex;border:solid 25px green", &inner)
    );
    assert_boxes(
        &body,
        [140.0, 160.0],
        &[
            ("m", [33.0, 33.0, 90.0, 110.0]),
            ("i", [58.0, 58.0, 40.0, 20.0]),
            ("j", [58.0, 78.0, 40.0, 20.0]),
            ("k", [58.0, 98.0, 40.0, 20.0]),
        ],
    );
}

#[test]
fn a_container_that_wraps_is_as_wide_as_its_widest_item_whatever_clamps_its_keyword_width() {
    let three = ["i", "j", "k"]
        .map(|id| item(id, "width:60px;max-width:40px;height:20px", ""))
        .concat();
    let lines = [
        ("i", [8.0, 8.0, 40.0, 20.0]),
        ("j", [8.0, 28.0, 40.0, 20.0]),
        ("k", [8.0, 48.0, 40.0, 20.0]),
    ];
    // A `min-width` or a `max-width` of the container does not change what its
    // items contribute.
    for limit in ["max-width:500px", "min-width:10px"] {
        let body = format!(
            "<div style=\"width:100px\">{}</div>",
            container(
                &format!("display:flex;flex-wrap:wrap;width:min-content;{limit}"),
                &three
            )
        );
        assert_boxes(&body, [40.0, 60.0], &lines);
    }
    // `fit-content` is the room offered, 100px, between the widest item and all
    // of them side by side.
    let body = format!(
        "<div style=\"width:100px\">{}</div>",
        container("display:flex;flex-wrap:wrap;width:fit-content", &three)
    );
    assert_boxes(
        &body,
        [100.0, 40.0],
        &[
            ("i", [8.0, 8.0, 40.0, 20.0]),
            ("j", [48.0, 8.0, 40.0, 20.0]),
            ("k", [8.0, 28.0, 40.0, 20.0]),
        ],
    );
}

#[test]
fn a_container_that_wraps_does_not_count_the_items_that_take_no_room() {
    // An absolutely positioned item and an item that is not displayed add
    // nothing to the width, which the item of 30px sets.
    let items = [
        item("i", "width:30px;height:20px", ""),
        item("j", "position:absolute;width:80px;height:10px", ""),
        item("k", "display:none;width:90px", ""),
    ]
    .concat();
    assert_boxes(
        &container(WRAP, &items),
        [80.0, 70.0],
        &[
            ("i", [33.0, 33.0, 30.0, 20.0]),
            ("j", [33.0, 33.0, 80.0, 10.0]),
        ],
    );
}
