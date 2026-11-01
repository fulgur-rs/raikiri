//! A flex or grid container whose `width` is `auto` and whose height it states
//! takes its width from that height through its `aspect-ratio`, but the content
//! raises that width to its min-content width unless the container scrolls or
//! states a `min-width`, and a `max-width` caps it (CSS Sizing 4 section 5.1).
//! The ratio relates the border box, which is no smaller than the padding and
//! border it holds. A grid item of any kind follows the same rule. The expected
//! boxes are the ones Chromium lays out for the same markup, with an 8px body
//! margin and a green border of 25px around the items.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};
use url::Url;

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/data/text-autospace/Ahem.ttf"
));

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
        .base_url(Url::parse("https://flex.test/document.html").expect("url"));
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

/// The ratio and height of the container in most cases: 2 / 1 and 40px, which
/// the 25px border raises to 50px.
const RATIO: &str = "aspect-ratio:2/1;height:40px";

/// Two items of 30px and 40px wide, 20px tall.
const ITEMS: &str = "<div id=i style=\"width:30px;height:20px\"></div>\
                     <div id=j style=\"width:40px;height:20px\"></div>";

/// A container `o` with the given `css` that holds `items`, in the flow.
fn in_flow(css: &str, items: &str) -> String {
    format!("<div id=o style=\"{css};{BORDER}\">{items}</div>")
}

/// A container `o` with the given `css` that holds `items`, absolutely
/// positioned in a containing block of 300px by 300px.
fn absolute(css: &str, items: &str) -> String {
    format!(
        "<div style=\"position:relative;width:300px;height:300px\">\
         <div id=o style=\"position:absolute;{css};{BORDER}\">{items}</div></div>"
    )
}

/// Both items in a row, the first at the left of the 25px border.
fn row(width: f32) -> [(&'static str, [f32; 4]); 3] {
    [
        ("o", [8.0, 8.0, width, 50.0]),
        ("i", [33.0, 33.0, 30.0, 20.0]),
        ("j", [63.0, 33.0, 40.0, 20.0]),
    ]
}

#[test]
fn the_content_raises_the_width_the_ratio_gives_a_flex_or_grid_container() {
    // The ratio gives 100px, the items need 70px and the border 50px.
    for css in [
        "display:flex",
        "display:inline-flex",
        "display:flex;float:left",
        "display:flex;width:fit-content",
        "display:grid;grid-auto-flow:column",
    ] {
        assert_boxes(&in_flow(&format!("{css};{RATIO}"), ITEMS), &row(120.0));
    }
}

#[test]
fn the_content_raises_the_width_the_ratio_gives_an_absolutely_positioned_container() {
    for css in [
        "display:flex",
        "display:inline-flex",
        "display:grid;grid-auto-flow:column",
        "display:flex;left:0;right:0",
    ] {
        assert_boxes(&absolute(&format!("{css};{RATIO}"), ITEMS), &row(120.0));
    }
    assert_boxes(
        &absolute(&format!("display:flex;left:10px;{RATIO}"), ITEMS),
        &[
            ("o", [18.0, 8.0, 120.0, 50.0]),
            ("i", [43.0, 33.0, 30.0, 20.0]),
            ("j", [73.0, 33.0, 40.0, 20.0]),
        ],
    );
    // A box positioned against the initial containing block.
    assert_boxes(
        &in_flow(&format!("position:fixed;display:flex;{RATIO}"), ITEMS),
        &row(120.0),
    );
}

#[test]
fn a_container_the_items_do_not_widen_keeps_the_width_of_the_ratio() {
    // Stacked, the items need 40px and the border 50px, which the 100px of the
    // ratio holds.
    for make in [in_flow, absolute] {
        assert_boxes(
            &make(&format!("display:grid;{RATIO}"), ITEMS),
            &[
                ("o", [8.0, 8.0, 100.0, 50.0]),
                ("i", [33.0, 33.0, 30.0, 20.0]),
                ("j", [33.0, 53.0, 40.0, 20.0]),
            ],
        );
        assert_boxes(
            &make(&format!("display:flex;flex-wrap:wrap;{RATIO}"), ITEMS),
            &[
                ("o", [8.0, 8.0, 100.0, 50.0]),
                ("i", [33.0, 33.0, 30.0, 20.0]),
                ("j", [33.0, 53.0, 40.0, 20.0]),
            ],
        );
        // A column of the items is as wide as the widest, 40px, within 100px.
        assert_boxes(
            &make(
                &format!("display:flex;flex-direction:column;{RATIO}"),
                ITEMS,
            ),
            &[("o", [8.0, 8.0, 100.0, 50.0])],
        );
    }
}

#[test]
fn the_ratio_relates_the_border_box_that_the_padding_and_border_make() {
    // The height of 40px is raised to the 50px of the border, so 2 / 1 gives
    // 100px, and 3 / 1 gives 150px, though the items need 5px or 70px.
    let small = "<div id=i style=\"width:5px;height:5px\"></div>";
    for make in [in_flow, absolute] {
        assert_boxes(
            &make(&format!("display:flex;{RATIO}"), small),
            &[
                ("o", [8.0, 8.0, 100.0, 50.0]),
                ("i", [33.0, 33.0, 5.0, 5.0]),
            ],
        );
        assert_boxes(
            &make("display:flex;aspect-ratio:3/1;height:40px", ITEMS),
            &[("o", [8.0, 8.0, 150.0, 50.0])],
        );
    }
    // A block holds the same ratio.
    assert_boxes(
        &in_flow(RATIO, "<div id=i style=\"width:10px;height:10px\"></div>"),
        &[
            ("o", [8.0, 8.0, 100.0, 50.0]),
            ("i", [33.0, 33.0, 10.0, 10.0]),
        ],
    );
}

#[test]
fn the_maximum_width_of_the_container_caps_the_width_the_content_asks_for() {
    for make in [in_flow, absolute] {
        assert_boxes(
            &make(&format!("display:flex;max-width:90px;{RATIO}"), ITEMS),
            &[
                ("o", [8.0, 8.0, 90.0, 50.0]),
                ("i", [33.0, 33.0, 17.14, 20.0]),
                ("j", [50.14, 33.0, 22.86, 20.0]),
            ],
        );
    }
}

#[test]
fn a_container_that_scrolls_or_states_a_minimum_width_is_not_raised() {
    for make in [in_flow, absolute] {
        for css in ["min-width:0", "overflow:hidden"] {
            assert_boxes(
                &make(&format!("display:flex;{css};{RATIO}"), ITEMS),
                &[
                    ("o", [8.0, 8.0, 100.0, 50.0]),
                    ("i", [33.0, 33.0, 21.42, 20.0]),
                    ("j", [54.42, 33.0, 28.58, 20.0]),
                ],
            );
        }
    }
}

#[test]
fn the_widest_item_and_the_text_set_the_minimum_width() {
    for make in [in_flow, absolute] {
        assert_boxes(
            &make(
                &format!("display:flex;{RATIO}"),
                "<div id=i style=\"width:30px;height:20px\"></div>\
                 <div id=j style=\"width:90px;height:20px\"></div>",
            ),
            &[
                ("o", [8.0, 8.0, 170.0, 50.0]),
                ("i", [33.0, 33.0, 30.0, 20.0]),
                ("j", [63.0, 33.0, 90.0, 20.0]),
            ],
        );
    }
    // Eight glyphs of the font are 80px wide.
    assert_boxes(
        &absolute(&format!("display:flex;{RATIO}"), "<div id=i>XXXXXXXX</div>"),
        &[("o", [8.0, 8.0, 130.0, 50.0])],
    );
}

#[test]
fn a_percentage_height_and_padding_give_the_ratio_a_box() {
    // 20% of 300px is 60px, and 2 / 1 gives 120px.
    assert_boxes(
        &absolute("display:flex;aspect-ratio:2/1;height:20%", ITEMS),
        &[("o", [8.0, 8.0, 120.0, 60.0])],
    );
    // Padding of 20px and a border of 5px make the 30px height 50px, so 2 / 1
    // gives 100px, and the items and edges need 70px and 50px.
    assert_boxes(
        &format!(
            "<div style=\"position:relative;width:300px;height:300px\">\
             <div id=o style=\"position:absolute;display:flex;aspect-ratio:2/1;height:30px;\
             padding:20px;border:solid 5px green\">{ITEMS}</div></div>"
        ),
        &[("o", [8.0, 8.0, 120.0, 50.0])],
    );
}

#[test]
fn a_container_the_ratio_gives_the_whole_size_is_left_alone() {
    // A content-box height of 40px makes the content-box width 80px.
    for make in [in_flow, absolute] {
        assert_boxes(
            &make(
                &format!("display:flex;box-sizing:content-box;{RATIO}"),
                ITEMS,
            ),
            &[("o", [8.0, 8.0, 130.0, 90.0])],
        );
        // A height above the border gives the width 120px by itself.
        assert_boxes(
            &make("display:flex;aspect-ratio:2/1;height:60px", ITEMS),
            &[("o", [8.0, 8.0, 120.0, 60.0])],
        );
    }
    // Without a height the ratio has nothing to give, and the box shrinks to
    // fit its items.
    assert_boxes(
        &absolute("display:flex;aspect-ratio:2/1", ITEMS),
        &[("o", [8.0, 8.0, 120.0, 70.0])],
    );
}

#[test]
fn a_minimum_height_the_height_meets_leaves_the_width_alone() {
    // The border makes the box 50px tall, which none of these minimums exceeds.
    for min_height in ["0", "40px", "50px"] {
        assert_boxes(
            &absolute(
                &format!("display:flex;min-height:{min_height};{RATIO}"),
                ITEMS,
            ),
            &row(120.0),
        );
    }
    // A content-box minimum is met by the content-box height next to it.
    assert_boxes(
        &absolute(
            &format!("display:flex;box-sizing:content-box;min-height:40px;{RATIO}"),
            ITEMS,
        ),
        &[("o", [8.0, 8.0, 130.0, 90.0])],
    );
}

#[test]
fn a_taller_minimum_height_carries_across_the_ratio() {
    // 100px of height at 2 / 1 is 200px of width, which the items do not need.
    let wide = [
        ("o", [8.0, 8.0, 200.0, 100.0]),
        ("i", [33.0, 33.0, 30.0, 20.0]),
        ("j", [63.0, 33.0, 40.0, 20.0]),
    ];
    for make in [in_flow, absolute] {
        assert_boxes(
            &make(&format!("display:flex;min-height:100px;{RATIO}"), ITEMS),
            &wide,
        );
    }
    // 20% of the 300px containing block is 60px, which the items' 120px holds.
    assert_boxes(
        &absolute(&format!("display:flex;min-height:20%;{RATIO}"), ITEMS),
        &[("o", [8.0, 8.0, 120.0, 60.0])],
    );
    // A content-box minimum of 60px is 110px of border box and 120px of content.
    assert_boxes(
        &absolute(
            &format!("display:flex;box-sizing:content-box;min-height:60px;{RATIO}"),
            ITEMS,
        ),
        &[("o", [8.0, 8.0, 170.0, 110.0])],
    );
    // Without a height, the minimum height gives the ratio its size.
    assert_boxes(
        &absolute("display:flex;aspect-ratio:2/1;min-height:30px", ITEMS),
        &[("o", [8.0, 8.0, 120.0, 60.0])],
    );
    assert_boxes(
        &absolute("display:flex;aspect-ratio:2/1;min-height:100px", ITEMS),
        &wide,
    );
}

/// A container `o` with the given `css` that holds `items`, absolutely
/// positioned in the second column of a grid container 200px by 100px.
fn in_grid_area(css: &str, items: &str) -> String {
    format!(
        "<div style=\"position:relative;display:grid;grid-template-columns:100px 100px;\
         grid-template-rows:100px;width:200px;height:100px\">\
         <div id=o style=\"position:absolute;grid-column:2;{css};{BORDER}\">{items}</div></div>"
    )
}

#[test]
fn a_container_placed_in_a_grid_area_follows_its_ratio_in_that_area() {
    // 50% of the 100px area is 50px, which gives 100px of width, and the items
    // need 70px with the border 120px.
    assert_boxes(
        &in_grid_area("display:flex;aspect-ratio:2/1;height:50%", ITEMS),
        &[
            ("o", [108.0, 8.0, 120.0, 50.0]),
            ("i", [133.0, 33.0, 30.0, 20.0]),
            ("j", [163.0, 33.0, 40.0, 20.0]),
        ],
    );
    assert_boxes(
        &in_grid_area(
            "display:flex;aspect-ratio:2/1;height:50%",
            "<div id=i style=\"width:5px;height:5px\"></div>",
        ),
        &[
            ("o", [108.0, 8.0, 100.0, 50.0]),
            ("i", [133.0, 33.0, 5.0, 5.0]),
        ],
    );
    // 90% of the area is 90px.
    assert_boxes(
        &in_grid_area(
            "display:flex;max-width:90%;aspect-ratio:2/1;height:40px",
            ITEMS,
        ),
        &[
            ("o", [108.0, 8.0, 90.0, 50.0]),
            ("i", [133.0, 33.0, 17.14, 20.0]),
            ("j", [150.14, 33.0, 22.86, 20.0]),
        ],
    );
}

#[test]
fn a_min_or_max_content_width_takes_the_minimum_of_the_ratio_too() {
    // With a definite height the keyword gives the width of the ratio, which
    // the items raise to their minimum.
    for css in [
        "display:flex;width:min-content",
        "display:flex;width:max-content",
        "display:grid;grid-auto-flow:column;width:min-content",
        "display:grid;grid-auto-flow:column;width:max-content",
    ] {
        assert_boxes(&in_flow(&format!("{css};{RATIO}"), ITEMS), &row(120.0));
    }
    assert_boxes(
        &in_flow(
            &format!("display:flex;width:max-content;{RATIO}"),
            "<div id=i>XXXX XXXX</div>",
        ),
        &[("o", [8.0, 8.0, 100.0, 50.0])],
    );
    // A maximum width caps it, and a minimum width of the container stops it.
    assert_boxes(
        &in_flow(
            &format!("display:flex;width:max-content;max-width:90px;{RATIO}"),
            ITEMS,
        ),
        &[
            ("o", [8.0, 8.0, 90.0, 50.0]),
            ("i", [33.0, 33.0, 17.14, 20.0]),
            ("j", [50.14, 33.0, 22.86, 20.0]),
        ],
    );
    assert_boxes(
        &in_flow(
            &format!("display:flex;width:max-content;min-width:0;{RATIO}"),
            ITEMS,
        ),
        &[
            ("o", [8.0, 8.0, 100.0, 50.0]),
            ("i", [33.0, 33.0, 21.42, 20.0]),
            ("j", [54.42, 33.0, 28.58, 20.0]),
        ],
    );
}

#[test]
fn a_fit_content_width_takes_the_width_of_the_ratio_and_the_minimum() {
    let small = "<div id=i style=\"width:5px;height:5px\"></div>";
    // The ratio gives 100px, which the small item does not shrink.
    for css in [
        "display:flex;width:fit-content",
        "display:flex;width:fit-content(300px)",
    ] {
        assert_boxes(
            &in_flow(&format!("{css};{RATIO}"), small),
            &[
                ("o", [8.0, 8.0, 100.0, 50.0]),
                ("i", [33.0, 33.0, 5.0, 5.0]),
            ],
        );
    }
    // A room narrower than the ratio width does not cap it, and the items
    // raise it to what they need.
    let narrow = |items: &str| {
        format!(
            "<div style=\"width:60px\">{}</div>",
            in_flow(&format!("display:flex;width:fit-content;{RATIO}"), items)
        )
    };
    assert_boxes(&narrow(small), &[("o", [8.0, 8.0, 100.0, 50.0])]);
    assert_boxes(&narrow(ITEMS), &row(120.0));
}

#[test]
fn an_absolutely_positioned_block_follows_the_same_minimum() {
    assert_boxes(
        &absolute(RATIO, "<div id=i style=\"width:100px;height:20px\"></div>"),
        &[
            ("o", [8.0, 8.0, 150.0, 50.0]),
            ("i", [33.0, 33.0, 100.0, 20.0]),
        ],
    );
    assert_boxes(
        &absolute(RATIO, "<div id=i style=\"width:5px;height:5px\"></div>"),
        &[
            ("o", [8.0, 8.0, 100.0, 50.0]),
            ("i", [33.0, 33.0, 5.0, 5.0]),
        ],
    );
}

#[test]
fn a_wrapping_column_container_is_measured_without_its_height() {
    // The items are 60px and 70px wide. The container breaks them into columns
    // at the room it is given, not at the height it ends up with, so they need
    // 120px with the border, and the ratio gives 140px or 200px.
    let wide = "<div id=i style=\"width:60px;height:20px\"></div>\
                <div id=j style=\"width:70px;height:20px\"></div>";
    for make in [in_flow, absolute] {
        for (height, width) in [(70.0, 140.0), (100.0, 200.0)] {
            assert_boxes(
                &make(
                    &format!(
                        "display:flex;flex-direction:column;flex-wrap:wrap;\
                         aspect-ratio:2/1;height:{height}px"
                    ),
                    wide,
                ),
                &[("o", [8.0, 8.0, width, height])],
            );
        }
    }
}

#[test]
fn nested_containers_and_vertical_writing_follow_the_ratio() {
    // The inner container is 80px wide by its ratio and the outer one 130px
    // for it and its border, which the 100px of the outer ratio does not hold.
    assert_boxes(
        &format!(
            "<div id=o style=\"display:flex;{RATIO};{BORDER}\">\
             <div id=p style=\"display:flex;aspect-ratio:3/1;height:20px;border:solid 5px red\">\
             <div id=i style=\"width:30px;height:5px\"></div>\
             <div id=j style=\"width:40px;height:5px\"></div></div></div>"
        ),
        &[
            ("o", [8.0, 8.0, 130.0, 50.0]),
            ("p", [33.0, 33.0, 80.0, 20.0]),
            ("i", [38.0, 38.0, 30.0, 5.0]),
            ("j", [68.0, 38.0, 40.0, 5.0]),
        ],
    );
    // The border makes the height 50px whatever the writing mode is.
    assert_boxes(
        &absolute(
            &format!("display:flex;writing-mode:vertical-rl;{RATIO}"),
            ITEMS,
        ),
        &[("o", [8.0, 8.0, 100.0, 50.0])],
    );
}

#[test]
fn vertical_ratio_containers_raise_a_small_stated_height_before_deriving_width() {
    let small = "<div style='width:10px;height:10px'></div>";
    for mode in ["vertical-rl", "vertical-lr", "sideways-rl", "sideways-lr"] {
        for display in ["flex", "grid", "inline-flex", "inline-grid"] {
            for position in ["", "position:absolute;"] {
                assert_boxes(
                    &in_flow(
                        &format!("display:{display};writing-mode:{mode};{position}{RATIO}"),
                        small,
                    ),
                    &[("o", [8.0, 8.0, 100.0, 50.0])],
                );
            }
        }
    }
}

#[test]
fn vertical_ratio_height_floors_preserve_other_authored_sizes_and_content_box_ratios() {
    let small = "<div style='width:10px;height:10px'></div>";
    for mode in ["vertical-rl", "vertical-lr", "sideways-rl", "sideways-lr"] {
        for display in ["flex", "grid", "inline-flex", "inline-grid"] {
            for (sizes, width, height) in [
                ("height:60px", 120.0, 60.0),
                ("height:40px;box-sizing:content-box", 130.0, 90.0),
                ("width:70px;height:40px", 70.0, 50.0),
                ("width:200px", 200.0, 100.0),
            ] {
                assert_boxes(
                    &in_flow(
                        &format!("display:{display};writing-mode:{mode};aspect-ratio:2/1;{sizes}"),
                        small,
                    ),
                    &[("o", [8.0, 8.0, width, height])],
                );
            }
        }
    }
}

#[test]
fn vertical_ratio_containers_include_padding_before_deriving_width() {
    for mode in ["vertical-rl", "vertical-lr", "sideways-rl", "sideways-lr"] {
        for display in ["flex", "grid", "inline-flex", "inline-grid"] {
            assert_boxes(
                &format!(
                    "<div id=o style='display:{display};writing-mode:{mode};{RATIO};padding:25px'><div style='width:10px;height:10px'></div></div>"
                ),
                &[("o", [8.0, 8.0, 100.0, 50.0])],
            );
        }
    }
}

#[test]
fn vertical_grid_content_still_raises_the_ratio_width_above_the_edge_floor() {
    for mode in ["vertical-rl", "vertical-lr", "sideways-rl", "sideways-lr"] {
        for (display, width) in [
            ("flex", 100.0),
            ("inline-flex", 100.0),
            ("grid", 120.0),
            ("inline-grid", 120.0),
        ] {
            assert_boxes(
                &in_flow(
                    &format!("display:{display};writing-mode:{mode};{RATIO}"),
                    ITEMS,
                ),
                &[("o", [8.0, 8.0, width, 50.0])],
            );
        }
    }
}

/// Check dimensions without constraining the parent's writing-mode placement.
fn assert_size(body: &str, expected: [f32; 2]) {
    let (_, rect) = boxes(body).into_iter().find(|(id, _)| id == "o").unwrap();
    assert_eq!([rect[2], rect[3]], expected, "{body}");
}

#[test]
fn vertical_ratio_containers_resolve_size_percentages_separately_from_padding() {
    for mode in ["vertical-rl", "vertical-lr", "sideways-rl", "sideways-lr"] {
        for display in ["flex", "grid", "inline-flex", "inline-grid"] {
            for (parent, sizes, expected) in [
                ("writing-mode:vertical-rl;", "height:40px", [100.0, 50.0]),
                ("", "height:40px", [140.0, 70.0]),
                ("writing-mode:vertical-rl;", "height:20%", [100.0, 50.0]),
                ("", "height:20%", [140.0, 70.0]),
                (
                    "writing-mode:vertical-rl;",
                    "width:20%;height:20%",
                    [60.0, 50.0],
                ),
            ] {
                assert_size(
                    &format!(
                        "<div style='width:300px;height:200px;{parent}'><div id=o style='display:{display};writing-mode:{mode};aspect-ratio:2/1;padding:10%;border:5px solid;{sizes}'><div style='width:10px;height:10px'></div></div></div>"
                    ),
                    expected,
                );
            }
        }
    }
}

#[test]
fn vertical_ratio_content_minimum_keeps_its_opt_outs_caps_and_grid_flow() {
    let wide = "<div style='width:200px;height:10px'></div>";
    for mode in ["vertical-rl", "vertical-lr", "sideways-rl", "sideways-lr"] {
        for display in ["flex", "grid", "inline-flex", "inline-grid"] {
            for (extra, content, width) in [
                ("min-width:0", wide, 100.0),
                ("overflow:hidden", wide, 100.0),
                ("max-width:90px", wide, 90.0),
                ("max-height:80px", wide, 250.0),
                ("", wide, 250.0),
                ("grid-auto-flow:column", ITEMS, 100.0),
            ] {
                assert_size(
                    &in_flow(
                        &format!("display:{display};writing-mode:{mode};{RATIO};{extra}"),
                        content,
                    ),
                    [width, 50.0],
                );
            }
        }
    }
}

#[test]
fn vertical_ratio_containers_keep_the_height_the_grid_parent_imposes() {
    for mode in ["vertical-rl", "vertical-lr", "sideways-rl", "sideways-lr"] {
        for display in ["flex", "grid", "inline-flex", "inline-grid"] {
            assert_size(
                &format!(
                    "<div style='display:grid;width:300px;height:100px;justify-items:start;align-items:stretch'><div id=o style='display:{display};writing-mode:{mode};aspect-ratio:2/1;{BORDER}'><div style='width:10px;height:10px'></div></div></div>"
                ),
                [200.0, 100.0],
            );
        }
    }
}

#[test]
fn a_minimum_width_does_not_raise_a_stated_height_through_the_ratio() {
    for make in [in_flow, absolute] {
        for display in ["flex", "inline-flex", "grid", "inline-grid"] {
            let css = format!("display:{display};grid-auto-flow:column;{RATIO};min-width:200px");
            let mut expected = row(200.0);
            if display.contains("grid") {
                expected[2].1[0] = 103.0;
            }
            assert_boxes(&make(&css, ITEMS), &expected);
        }
    }
}

#[test]
fn a_minimum_height_keeps_the_automatic_content_width_of_an_absolute_ratio_box() {
    for display in ["flex", "inline-flex", "grid", "inline-grid"] {
        for (items, width) in [
            (ITEMS, 120.0),
            ("<div id=i style=\"width:200px;height:20px\"></div>", 250.0),
        ] {
            let css = format!("display:{display};grid-auto-flow:column;{RATIO};min-height:51px");
            assert_boxes(&absolute(&css, items), &[("o", [8.0, 8.0, width, 51.0])]);
        }
    }
}

#[test]
fn ratio_width_minima_keep_authored_height_bounds_and_content_opt_outs() {
    let wide = "<div style=\"width:200px;height:20px\"></div>";
    for make in [in_flow, absolute] {
        for display in ["flex", "inline-flex", "grid", "inline-grid"] {
            for (limits, small_width, wide_width, height) in [
                ("min-height:51px", 120.0, 250.0, 51.0),
                ("min-height:80px;max-height:60px", 160.0, 250.0, 80.0),
                ("min-height:0", 120.0, 250.0, 50.0),
                ("min-width:0", 100.0, 100.0, 50.0),
                ("min-height:51px;max-width:90px", 90.0, 90.0, 51.0),
                ("min-width:200px", 200.0, 200.0, 50.0),
            ] {
                let css = format!("display:{display};grid-auto-flow:column;{RATIO};{limits}");
                for (items, width) in [(ITEMS, small_width), (wide, wide_width)] {
                    assert_boxes(&make(&css, items), &[("o", [8.0, 8.0, width, height])]);
                }
            }
        }
    }
}

/// A grid container whose columns are `template` and whose item `o` has the
/// given `css` and holds `items`.
fn grid_item(template: &str, css: &str, items: &str) -> String {
    format!(
        "<div style=\"display:grid;{template}\">\
         <div id=o style=\"{css};{BORDER}\">{items}</div></div>"
    )
}

/// An item 100px wide, which the 25px border makes 150px.
const WIDE: &str = "<div id=i style=\"width:100px;height:20px\"></div>";

/// The border box of the item `o` and its child `i` when the item is raised to
/// the 150px of `WIDE`.
fn wide_item(x: f32) -> [(&'static str, [f32; 4]); 2] {
    [
        ("o", [x, 8.0, 150.0, 50.0]),
        ("i", [x + 25.0, 33.0, 100.0, 20.0]),
    ]
}

#[test]
fn a_grid_item_takes_the_width_of_its_ratio_raised_to_its_content() {
    // The ratio gives 100px, which the child and the border raise to 150px,
    // whatever the track is: even a 60px one is overflowed.
    for template in [
        "grid-template-columns:max-content",
        "grid-template-columns:min-content",
        "grid-template-columns:1fr",
        "grid-template-columns:300px",
        "grid-template-columns:60px",
        "",
    ] {
        assert_boxes(&grid_item(template, RATIO, WIDE), &wide_item(8.0));
    }
    assert_boxes(
        &format!(
            "<div style=\"display:inline-grid\"><div id=o style=\"{RATIO};{BORDER}\">{WIDE}</div></div>"
        ),
        &wide_item(8.0),
    );
    // The small child does not shrink it below the 100px of the ratio.
    assert_boxes(
        &grid_item(
            "grid-template-columns:max-content",
            RATIO,
            "<div id=i style=\"width:5px;height:5px\"></div>",
        ),
        &[
            ("o", [8.0, 8.0, 100.0, 50.0]),
            ("i", [33.0, 33.0, 5.0, 5.0]),
        ],
    );
    // Text breaks at the room it is given, so the longest word of 40px with
    // the border needs less than the ratio gives.
    assert_boxes(
        &format!(
            "<div style=\"display:inline-grid\"><div id=o style=\"{RATIO};{BORDER};font:10px Ahem\">XXXX XXXX</div></div>"
        ),
        &[("o", [8.0, 8.0, 100.0, 50.0])],
    );
}

#[test]
fn a_grid_item_is_placed_by_the_width_it_takes() {
    let column = "grid-template-columns:300px";
    // The item is centered or at the end in a 300px track by its 150px.
    assert_boxes(
        &format!(
            "<div style=\"display:grid;{column};justify-items:center\">\
             <div id=o style=\"{RATIO};{BORDER}\">{WIDE}</div></div>"
        ),
        &wide_item(83.0),
    );
    assert_boxes(
        &grid_item(column, &format!("justify-self:end;{RATIO}"), WIDE),
        &wide_item(158.0),
    );
    assert_boxes(
        &grid_item(column, &format!("justify-self:start;{RATIO}"), WIDE),
        &wide_item(8.0),
    );
    // The track of a max-content column is as wide as the item, so the next
    // one starts after it, and a row gap or a spanned track keeps the width.
    assert_boxes(
        &format!(
            "<div style=\"display:grid;grid-template-columns:max-content max-content\">\
             <div id=o style=\"{RATIO};{BORDER}\">{WIDE}</div>\
             <div id=p style=\"width:30px;height:10px\"></div></div>"
        ),
        &[
            ("o", [8.0, 8.0, 150.0, 50.0]),
            ("p", [158.0, 8.0, 30.0, 10.0]),
        ],
    );
    assert_boxes(
        &format!(
            "<div style=\"display:grid;gap:10px\">\
             <div id=o style=\"{RATIO};{BORDER}\">{WIDE}</div>\
             <div id=p style=\"width:30px;height:10px\"></div></div>"
        ),
        &[
            ("o", [8.0, 8.0, 150.0, 50.0]),
            ("p", [8.0, 68.0, 30.0, 10.0]),
        ],
    );
    assert_boxes(
        &grid_item(
            "grid-template-columns:50px 50px 50px",
            &format!("grid-column:1/3;{RATIO}"),
            WIDE,
        ),
        &wide_item(8.0),
    );
    // A row of 100px or a stretch along the rows leaves the width alone.
    for css in [RATIO.to_owned(), format!("align-self:stretch;{RATIO}")] {
        assert_boxes(
            &format!(
                "<div style=\"display:grid;grid-template-rows:100px;grid-template-columns:max-content\">\
                 <div id=o style=\"{css};{BORDER}\">{WIDE}</div></div>"
            ),
            &wide_item(8.0),
        );
    }
}

#[test]
fn a_grid_item_keeps_the_height_it_states_in_an_auto_row() {
    // The minimum width the item gets must not carry across the ratio into a
    // minimum height: the tall child overflows the 50px item, and the auto row
    // is as high as the item states.
    assert_boxes(
        &format!(
            "<div style=\"display:grid;grid-template-columns:max-content\">\
             <div id=o style=\"{RATIO};{BORDER}\">\
             <div id=i style=\"width:100px;height:200px\"></div></div>\
             <div id=p style=\"width:30px;height:10px\"></div></div>"
        ),
        &[
            ("o", [8.0, 8.0, 150.0, 50.0]),
            ("i", [33.0, 33.0, 100.0, 200.0]),
            ("p", [8.0, 58.0, 30.0, 10.0]),
        ],
    );
}

#[test]
fn a_grid_item_whose_edges_follow_its_area_or_that_is_hidden_keeps_the_ratio_width() {
    let child = "<div id=i style=\"width:10px;height:10px\"></div>";
    // Percentage padding, also one that a `calc()` clamps to nothing at the
    // width of this track, is not known before the tracks are.
    assert_boxes(
        &format!(
            "<div style=\"display:grid;grid-template-columns:300px\">\
             <div id=o style=\"aspect-ratio:2/1;height:50px;padding:5%\">{child}</div></div>"
        ),
        &[
            ("o", [8.0, 8.0, 100.0, 50.0]),
            ("i", [23.0, 23.0, 10.0, 10.0]),
        ],
    );
    assert_boxes(
        &format!(
            "<div style=\"display:grid;grid-template-columns:300px\">\
             <div id=o style=\"aspect-ratio:2/1;height:50px;padding:calc(1% - 10px)\">{child}</div></div>"
        ),
        &[
            ("o", [8.0, 8.0, 100.0, 50.0]),
            ("i", [8.0, 8.0, 10.0, 10.0]),
        ],
    );
    // A hidden item takes no part, whatever it holds.
    assert_boxes(
        &format!(
            "<div style=\"display:grid;grid-template-columns:max-content\">\
             <div style=\"display:none;{RATIO};{BORDER}\">{WIDE}</div>\
             <div id=o style=\"{RATIO};{BORDER}\">{WIDE}</div></div>"
        ),
        &wide_item(8.0),
    );
}

#[test]
fn a_minimum_height_above_the_maximum_height_stays_the_height_of_a_grid_item() {
    // The minimum wins over the maximum, so the item is 80px high and its row
    // too, and the item after it starts below.
    let next = "<div id=p style=\"width:30px;height:10px\"></div>";
    for (css, items) in [
        (
            "aspect-ratio:2/1;height:100px;min-height:80px;max-height:20px",
            "<div id=i style=\"width:10px;height:10px\"></div>",
        ),
        (
            "aspect-ratio:2/1;height:100px;min-height:80px;max-height:20px;border:solid 25px green",
            "<div id=i style=\"width:300px;height:10px\"></div>",
        ),
    ] {
        assert_boxes(
            &format!(
                "<div style=\"display:grid;grid-template-columns:max-content\">\
                 <div id=o style=\"{css}\">{items}</div>{next}</div>"
            ),
            &[("p", [8.0, 88.0, 30.0, 10.0])],
        );
    }
    // A maximum the minimum fits under leaves the minimum width of the content
    // to the item: 90px high, and 350px wide for the child and the border.
    assert_boxes(
        &format!(
            "<div style=\"display:grid;grid-template-columns:max-content\">\
             <div id=o style=\"aspect-ratio:2/1;height:100px;min-height:80px;max-height:90px;{BORDER}\">\
             <div id=i style=\"width:300px;height:10px\"></div></div>{next}</div>"
        ),
        &[
            ("o", [8.0, 8.0, 350.0, 90.0]),
            ("p", [8.0, 98.0, 30.0, 10.0]),
        ],
    );
}

#[test]
fn a_grid_at_its_min_content_width_is_as_wide_as_the_ratio_items_it_holds() {
    let grid = |style: &str, items: &str| {
        format!("<div id=g style=\"display:grid;width:min-content;{style}\">{items}</div>")
    };
    let small = "<div id=o style=\"aspect-ratio:2/1;height:50px\"><div id=i style=\"width:10px;height:10px\"></div></div>";
    // The item takes the 100px its height gives through the ratio, and the
    // grid with it, not the 10px of its child.
    assert_boxes(
        &grid("", small),
        &[
            ("g", [8.0, 8.0, 100.0, 50.0]),
            ("o", [8.0, 8.0, 100.0, 50.0]),
        ],
    );
    assert_boxes(
        &grid(
            "grid-template-columns:auto auto",
            &format!("{small}<div id=p style=\"width:20px;height:10px\"></div>"),
        ),
        &[
            ("g", [8.0, 8.0, 120.0, 50.0]),
            ("o", [8.0, 8.0, 100.0, 50.0]),
            ("p", [108.0, 8.0, 20.0, 10.0]),
        ],
    );
    // A child wider than the ratio width, or the border, raises both.
    assert_boxes(
        &grid(
            "",
            "<div id=o style=\"aspect-ratio:2/1;height:50px\"><div style=\"width:200px;height:10px\"></div></div>",
        ),
        &[
            ("g", [8.0, 8.0, 200.0, 50.0]),
            ("o", [8.0, 8.0, 200.0, 50.0]),
        ],
    );
    assert_boxes(
        &grid(
            "",
            &format!("<div id=o style=\"{RATIO};{BORDER}\">{WIDE}</div>"),
        ),
        &[
            ("g", [8.0, 8.0, 150.0, 50.0]),
            ("o", [8.0, 8.0, 150.0, 50.0]),
        ],
    );
    // A `min-width` the author wrote still holds the item to it while the
    // grid is measured, and the item overflows the grid.
    assert_boxes(
        &grid(
            "",
            "<div id=o style=\"aspect-ratio:2/1;height:50px;min-width:30px\"><div style=\"width:10px;height:10px\"></div></div>",
        ),
        &[
            ("g", [8.0, 8.0, 30.0, 50.0]),
            ("o", [8.0, 8.0, 100.0, 50.0]),
        ],
    );
}

#[test]
fn a_grid_item_that_scrolls_or_has_a_limit_keeps_the_width_the_limit_gives() {
    let template = "grid-template-columns:max-content";
    assert_boxes(
        &grid_item(template, &format!("max-width:90px;{RATIO}"), WIDE),
        &[
            ("o", [8.0, 8.0, 90.0, 50.0]),
            ("i", [33.0, 33.0, 100.0, 20.0]),
        ],
    );
    for css in ["min-width:0", "overflow:hidden"] {
        assert_boxes(
            &grid_item(template, &format!("{css};{RATIO}"), WIDE),
            &[
                ("o", [8.0, 8.0, 100.0, 50.0]),
                ("i", [33.0, 33.0, 100.0, 20.0]),
            ],
        );
    }
}

#[test]
fn a_grid_item_follows_its_box_sizing_padding_and_children() {
    let template = "grid-template-columns:max-content";
    // A content-box height of 40px is 90px with the border, and 2 / 1 gives 80px
    // of content and 130px of box, which the child raises to 150px.
    assert_boxes(
        &grid_item(template, &format!("box-sizing:content-box;{RATIO}"), WIDE),
        &[("o", [8.0, 8.0, 150.0, 90.0])],
    );
    // Padding of 20px and a border of 5px make the 30px height 50px.
    assert_boxes(
        &format!(
            "<div style=\"display:grid;{template}\"><div id=o style=\"aspect-ratio:2/1;height:30px;\
             padding:20px;border:solid 5px green\">{WIDE}</div></div>"
        ),
        &[
            ("o", [8.0, 8.0, 150.0, 50.0]),
            ("i", [33.0, 33.0, 100.0, 20.0]),
        ],
    );
    // A percentage width of a child is of the item, not a minimum of it.
    assert_boxes(
        &grid_item(
            template,
            RATIO,
            "<div id=i style=\"width:50%;height:20px\"></div>",
        ),
        &[
            ("o", [8.0, 8.0, 100.0, 50.0]),
            ("i", [33.0, 33.0, 25.0, 20.0]),
        ],
    );
    // A box with a ratio of its own inside the item is as wide as its child.
    assert_boxes(
        &grid_item(
            template,
            RATIO,
            "<div id=p style=\"aspect-ratio:3/1;height:20px;border:solid 5px red\">\
             <div id=i style=\"width:90px;height:5px\"></div></div>",
        ),
        &[
            ("o", [8.0, 8.0, 150.0, 50.0]),
            ("p", [33.0, 33.0, 100.0, 20.0]),
            ("i", [38.0, 38.0, 90.0, 5.0]),
        ],
    );
    // An absolutely positioned sibling takes no part.
    assert_boxes(
        &format!(
            "<div style=\"display:grid;{template};position:relative\">\
             <div id=o style=\"{RATIO};{BORDER}\">{WIDE}</div>\
             <div id=p style=\"position:absolute;left:0;top:0;width:10px;height:10px\"></div></div>"
        ),
        &[
            ("o", [8.0, 8.0, 150.0, 50.0]),
            ("p", [8.0, 8.0, 10.0, 10.0]),
        ],
    );
}

#[test]
fn a_flex_or_grid_container_in_a_grid_follows_the_minimum_of_its_ratio() {
    // The ratio gives 100px, and the items need 70px with the 50px of border.
    for css in ["display:flex", "display:grid;grid-auto-flow:column"] {
        for template in [
            "grid-template-columns:max-content",
            "grid-template-columns:300px",
        ] {
            assert_boxes(
                &grid_item(template, &format!("{css};{RATIO}"), ITEMS),
                &row(120.0),
            );
        }
    }
}
