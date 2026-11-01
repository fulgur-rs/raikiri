//! A flex container whose `width` is `min-content` or `max-content` is as wide
//! as its items contribute at that size, and an item that cannot shrink or grow
//! contributes no less or no more than its flex base size (CSS Flexbox 1 section
//! 9.9.1): the width an item states is of no account when its `flex-basis` is
//! `0` and it cannot grow, and a `flex-basis` wider than its content is the
//! width it takes when it cannot shrink. A container that wraps its items is no
//! narrower than its widest item either. The expected boxes are the ones Chromium
//! lays out for the same markup, with an 8px body margin and a green border of
//! 25px around the items.

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

const BORDER: &str = "border:solid 25px green";

/// A flex container `o` with the given `container_css` that holds `items`.
fn flex(container_css: &str, items: &str) -> String {
    format!("<div id=o style=\"display:flex;{container_css};{BORDER}\">{items}</div>")
}

/// An item named `id` with the given `css` and `content`.
fn item(id: &str, css: &str, content: &str) -> String {
    format!("<div id={id} style=\"{css}\">{content}</div>")
}

/// An item with a box of 10px for its content.
fn item_of_content(id: &str, css: &str) -> String {
    item(id, css, "<div style=\"width:10px;height:10px\"></div>")
}

const KEYWORDS: [&str; 2] = ["width:min-content", "width:max-content"];

#[test]
fn an_item_that_cannot_grow_from_a_flex_basis_of_zero_adds_no_width_to_a_container_of_the_keyword_width()
 {
    for keyword in KEYWORDS {
        // Neither the 30px it states nor the 30px of its content count.
        assert_boxes(
            &flex(
                keyword,
                &item("i", "flex:0 0 0px;width:30px;height:50px", ""),
            ),
            [50.0, 100.0],
            &[("i", [33.0, 33.0, 0.0, 50.0])],
        );
        let items = [
            item("i", "flex:0 0 0px;width:30px;height:50px", ""),
            item("j", "flex:0 0 0px;width:40px;height:50px", ""),
        ]
        .concat();
        assert_boxes(
            &flex(keyword, &items),
            [50.0, 100.0],
            &[
                ("i", [33.0, 33.0, 0.0, 50.0]),
                ("j", [33.0, 33.0, 0.0, 50.0]),
            ],
        );
    }
}

#[test]
fn an_item_that_cannot_shrink_takes_its_flex_basis_into_a_container_of_the_keyword_width() {
    for keyword in KEYWORDS {
        // A flex basis wider than the 10px of the content, with no room to
        // shrink it to them, whether the item can grow or not.
        for flex_css in ["flex:0 0 100px", "flex:1 0 100px"] {
            assert_boxes(
                &flex(
                    keyword,
                    &item_of_content("i", &format!("{flex_css};height:50px")),
                ),
                [150.0, 100.0],
                &[("i", [33.0, 33.0, 100.0, 50.0])],
            );
        }
    }
}

#[test]
fn the_maximum_width_of_an_item_holds_back_its_flex_basis_in_a_container_of_the_keyword_width() {
    for keyword in KEYWORDS {
        assert_boxes(
            &flex(
                keyword,
                &item("i", "flex:0 0 100px;max-width:30px;height:50px", ""),
            ),
            [80.0, 100.0],
            &[("i", [33.0, 33.0, 30.0, 50.0])],
        );
    }
}

#[test]
fn the_margins_of_an_item_count_in_a_container_of_the_keyword_width() {
    assert_boxes(
        &flex(
            "width:max-content",
            &item("i", "flex:0 0 0px;width:30px;height:50px;margin:0 7px", ""),
        ),
        [64.0, 100.0],
        &[("i", [40.0, 33.0, 0.0, 50.0])],
    );
}

#[test]
fn an_item_that_can_flex_is_not_held_to_its_flex_basis_in_a_container_of_the_keyword_width() {
    for keyword in KEYWORDS {
        for nowrap_or_wrap in ["flex-wrap:nowrap", "flex-wrap:wrap"] {
            let container_css = format!("{keyword};{nowrap_or_wrap}");
            // The width it states is its contribution.
            assert_boxes(
                &flex(&container_css, &item("i", "width:30px;height:50px", "")),
                [80.0, 100.0],
                &[("i", [33.0, 33.0, 30.0, 50.0])],
            );
            // So is the `min-width` that raises it.
            assert_boxes(
                &flex(
                    &container_css,
                    &item("i", "width:30px;min-width:60px;height:50px", ""),
                ),
                [110.0, 100.0],
                &[("i", [33.0, 33.0, 60.0, 50.0])],
            );
            assert_boxes(
                &flex(
                    &container_css,
                    &item("i", "width:30px;height:50px;margin:0 7px", ""),
                ),
                [94.0, 100.0],
                &[("i", [40.0, 33.0, 30.0, 50.0])],
            );
        }
    }
}

#[test]
fn a_container_that_wraps_is_no_narrower_than_its_widest_item_at_its_max_content_width() {
    let wrap = "flex-wrap:wrap";
    // Items of 0 for a flex basis add nothing to the sum of the contributions;
    // the widest of them is the width of the container.
    assert_boxes(
        &flex(
            &format!("width:max-content;{wrap}"),
            &item("i", "flex:0 0 0px;width:30px;height:50px", ""),
        ),
        [80.0, 100.0],
        &[("i", [33.0, 33.0, 0.0, 50.0])],
    );
    let items = [
        item("i", "flex:0 0 0px;width:30px;height:50px", ""),
        item("j", "flex:0 0 0px;width:40px;height:50px", ""),
    ]
    .concat();
    assert_boxes(
        &flex(&format!("width:max-content;{wrap}"), &items),
        [90.0, 100.0],
        &[
            ("i", [33.0, 33.0, 0.0, 50.0]),
            ("j", [33.0, 33.0, 0.0, 50.0]),
        ],
    );
    assert_boxes(
        &flex(
            &format!("width:max-content;{wrap}"),
            &item("i", "flex:0 0 0px;width:30px;height:50px;margin:0 7px", ""),
        ),
        [94.0, 100.0],
        &[("i", [40.0, 33.0, 0.0, 50.0])],
    );
    // The text of an item gives it the width of its longest word.
    assert_boxes(
        &flex(
            &format!("width:max-content;{wrap}"),
            &item("i", "flex:0 0 0px;height:20px", "XX XX"),
        ),
        [70.0, 70.0],
        &[("i", [33.0, 33.0, 20.0, 20.0])],
    );
}

#[test]
fn an_item_that_cannot_shrink_leaves_a_container_that_wraps_as_wide_as_the_min_content_of_its_item()
{
    // The item keeps its flex basis of 100px, which the container does not
    // take into its min-content width.
    assert_boxes(
        &flex(
            "width:min-content;flex-wrap:wrap",
            &item_of_content("i", "flex:1 0 100px;height:50px"),
        ),
        [60.0, 100.0],
        &[("i", [33.0, 33.0, 100.0, 50.0])],
    );
    // At its max-content width it does.
    for flex_css in ["flex:0 0 100px", "flex:1 0 100px"] {
        assert_boxes(
            &flex(
                "width:max-content;flex-wrap:wrap",
                &item_of_content("i", &format!("{flex_css};height:50px")),
            ),
            [150.0, 100.0],
            &[("i", [33.0, 33.0, 100.0, 50.0])],
        );
    }
    assert_boxes(
        &flex(
            "width:max-content;flex-wrap:wrap",
            &item("i", "flex:0 0 100px;max-width:30px;height:50px", ""),
        ),
        [80.0, 100.0],
        &[("i", [33.0, 33.0, 30.0, 50.0])],
    );
}

#[test]
fn a_floated_container_of_the_keyword_width_counts_its_items_by_their_flex_basis() {
    for (container_css, width) in [
        ("width:max-content", 50.0),
        ("width:min-content", 50.0),
        ("width:max-content;flex-wrap:wrap", 80.0),
    ] {
        assert_boxes(
            &flex(
                &format!("float:left;{container_css}"),
                &item("i", "flex:0 0 0px;width:30px;height:50px", ""),
            ),
            [width, 100.0],
            &[("i", [33.0, 33.0, 0.0, 50.0])],
        );
    }
}

#[test]
fn a_container_of_the_keyword_width_in_an_inline_flex_container_is_as_wide_as_its_items_give_it() {
    let inner = |container_css: &str| {
        format!(
            "<div id=o style=\"display:inline-flex;border:solid 10px blue\">{}</div>",
            format_args!(
                "<div id=q style=\"display:flex;{container_css};{BORDER}\">{}</div>",
                item("i", "flex:0 0 0px;width:30px;height:50px", "")
            )
        )
    };
    assert_boxes(
        &inner("width:max-content"),
        [70.0, 120.0],
        &[
            ("q", [18.0, 18.0, 50.0, 100.0]),
            ("i", [43.0, 43.0, 0.0, 50.0]),
        ],
    );
    assert_boxes(
        &inner("width:max-content;flex-wrap:wrap"),
        [100.0, 120.0],
        &[
            ("q", [18.0, 18.0, 80.0, 100.0]),
            ("i", [43.0, 43.0, 0.0, 50.0]),
        ],
    );
}

#[test]
fn the_minimum_and_maximum_width_of_a_container_of_the_keyword_width_hold_it() {
    // The 150px of the item is held to the 60px of the maximum width.
    assert_boxes(
        &flex(
            "width:max-content;max-width:60px",
            &item_of_content("i", "flex:0 0 100px;height:50px"),
        ),
        [60.0, 100.0],
        &[("i", [33.0, 33.0, 100.0, 50.0])],
    );
    assert_boxes(
        &flex(
            "width:max-content;flex-wrap:wrap;min-width:120px",
            &item("i", "flex:0 0 0px;width:30px;height:50px", ""),
        ),
        [120.0, 100.0],
        &[("i", [33.0, 33.0, 0.0, 50.0])],
    );
    assert_boxes(
        &flex(
            "width:max-content;flex-wrap:wrap;max-width:60px",
            &item("i", "flex:0 0 0px;width:30px;height:50px", ""),
        ),
        [60.0, 100.0],
        &[("i", [33.0, 33.0, 0.0, 50.0])],
    );
}

#[test]
fn a_container_that_runs_right_to_left_or_in_a_column_is_sized_like_the_others() {
    let item_of_zero = item("i", "flex:0 0 0px;width:30px;height:50px", "");
    assert_boxes(
        &flex("width:max-content;direction:rtl", &item_of_zero),
        [50.0, 100.0],
        &[("i", [33.0, 33.0, 0.0, 50.0])],
    );
    assert_boxes(
        &flex(
            "width:max-content;direction:rtl;flex-wrap:wrap",
            &item_of_zero,
        ),
        [80.0, 100.0],
        &[("i", [63.0, 33.0, 0.0, 50.0])],
    );
    // The flex basis of an item of a column is a height.
    assert_boxes(
        &flex("width:max-content;flex-direction:column", &item_of_zero),
        [80.0, 50.0],
        &[("i", [33.0, 33.0, 30.0, 0.0])],
    );
}

#[test]
fn a_container_that_is_an_item_of_another_one_is_as_wide_as_its_own_items_make_it() {
    // The container `q` holds the item and is the item of the container `o`,
    // which is sized in the ways a container is asked for its intrinsic widths.
    let nested = |outer_css: &str, inner_css: &str, item: &str| {
        format!(
            "<div id=o style=\"{outer_css};border:solid 10px blue\">\
             <div id=q style=\"display:flex;{inner_css};{BORDER}\">{item}</div></div>"
        )
    };
    let zero = item("i", "flex:0 0 0px;width:30px;height:50px", "");
    for (outer_css, outer_width) in [
        ("display:inline-flex", 70.0),
        ("display:flex;width:300px", 300.0),
        ("display:flex;width:min-content", 70.0),
        ("display:flex;width:max-content", 70.0),
    ] {
        assert_boxes(
            &nested(outer_css, "flex-wrap:nowrap", &zero),
            [outer_width, 120.0],
            &[
                ("q", [18.0, 18.0, 50.0, 100.0]),
                ("i", [43.0, 43.0, 0.0, 50.0]),
            ],
        );
    }
    // A container that wraps is as wide as its widest item, and its item
    // keeps the 100px of its flex basis though the room is less.
    let basis = item_of_content("i", "flex:0 0 100px;height:50px");
    for (outer_css, outer_width, inner_width) in [
        ("display:inline-flex", 170.0, 150.0),
        ("display:flex;width:300px", 300.0, 150.0),
        ("display:flex;width:min-content", 80.0, 60.0),
        ("display:flex;width:max-content", 170.0, 150.0),
    ] {
        assert_boxes(
            &nested(outer_css, "flex-wrap:wrap", &basis),
            [outer_width, 120.0],
            &[
                ("q", [18.0, 18.0, inner_width, 100.0]),
                ("i", [43.0, 43.0, 100.0, 50.0]),
            ],
        );
    }
}
