//! A flex or grid container whose `width` is `auto` and whose height it states
//! takes its width from that height through its `aspect-ratio`, but the content
//! raises that width to its min-content width unless the container scrolls or
//! states a `min-width`, and a `max-width` caps it (CSS Sizing 4 section 5.1).
//! The ratio relates the border box, which is no smaller than the padding and
//! border it holds. The expected boxes are the ones Chromium lays out for the
//! same markup, with an 8px body margin and a green border of 25px around the
//! items.

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

#[test]
fn absolute_intrinsic_widths_use_the_border_box_height_before_the_ratio() {
    for width in ["min-content", "max-content"] {
        for display in ["flex", "inline-flex", "grid", "inline-grid"] {
            let css = format!("display:{display};grid-auto-flow:column;{RATIO};width:{width}");
            assert_boxes(
                &absolute(&css, "XXXX XXXX"),
                &[("o", [8.0, 8.0, 100.0, 50.0])],
            );
            assert_boxes(&absolute(&css, ITEMS), &[("o", [8.0, 8.0, 120.0, 50.0])]);
        }
    }
}

#[test]
fn absolute_stated_widths_keep_the_content_minimum_without_a_ratio_width_floor() {
    for (width, expected) in [
        (0, 120.0),
        (30, 120.0),
        (80, 120.0),
        (100, 120.0),
        (200, 200.0),
    ] {
        let css = format!("display:flex;{RATIO};width:{width}px");
        assert_boxes(&absolute(&css, ITEMS), &[("o", [8.0, 8.0, expected, 50.0])]);
        let expected = (width as f32).max(90.0);
        assert_boxes(
            &absolute(&css, "XXXX XXXX"),
            &[("o", [8.0, 8.0, expected, 50.0])],
        );
    }
}

#[test]
fn an_absolute_zero_width_ratio_box_keeps_its_content_minimum_with_padding() {
    let html = format!(
        "<div style=\"position:relative;width:300px;height:300px\">\
        <div id=o style=\"position:absolute;display:flex;width:0;height:40px;aspect-ratio:2/1;padding:10px\">{ITEMS}</div></div>"
    );
    assert_boxes(&html, &[("o", [8.0, 8.0, 90.0, 40.0])]);
}

#[test]
fn absolute_width_content_floors_keep_their_opt_outs_and_own_bounds() {
    let tiny = "<div style=\"width:5px;height:5px\"></div>";
    for (width, modifier, sizes) in [
        (
            "30px",
            "min-width:0",
            [[50.0, 50.0], [50.0, 50.0], [50.0, 50.0]],
        ),
        (
            "30px",
            "overflow:hidden",
            [[50.0, 50.0], [50.0, 50.0], [50.0, 50.0]],
        ),
        (
            "30px",
            "max-width:90px",
            [[90.0, 50.0], [90.0, 50.0], [55.0, 50.0]],
        ),
        (
            "30px",
            "min-height:51px",
            [[90.0, 51.0], [120.0, 51.0], [55.0, 51.0]],
        ),
        (
            "30px",
            "box-sizing:content-box",
            [[90.0, 90.0], [120.0, 90.0], [80.0, 90.0]],
        ),
        (
            "min-content",
            "min-width:0",
            [[100.0, 50.0], [100.0, 50.0], [100.0, 50.0]],
        ),
        (
            "min-content",
            "overflow:hidden",
            [[100.0, 50.0], [100.0, 50.0], [100.0, 50.0]],
        ),
        (
            "min-content",
            "max-width:90px",
            [[90.0, 50.0], [90.0, 50.0], [90.0, 50.0]],
        ),
        (
            "min-content",
            "min-height:51px",
            [[102.0, 51.0], [120.0, 51.0], [102.0, 51.0]],
        ),
        (
            "min-content",
            "box-sizing:content-box",
            [[130.0, 90.0], [130.0, 90.0], [130.0, 90.0]],
        ),
        (
            "max-content",
            "min-width:0",
            [[100.0, 50.0], [100.0, 50.0], [100.0, 50.0]],
        ),
        (
            "max-content",
            "overflow:hidden",
            [[100.0, 50.0], [100.0, 50.0], [100.0, 50.0]],
        ),
        (
            "max-content",
            "max-width:90px",
            [[90.0, 50.0], [90.0, 50.0], [90.0, 50.0]],
        ),
        (
            "max-content",
            "min-height:51px",
            [[102.0, 51.0], [120.0, 51.0], [102.0, 51.0]],
        ),
        (
            "max-content",
            "box-sizing:content-box",
            [[130.0, 90.0], [130.0, 90.0], [130.0, 90.0]],
        ),
        (
            "50%",
            "min-width:0",
            [[150.0, 50.0], [150.0, 50.0], [150.0, 50.0]],
        ),
        (
            "50%",
            "overflow:hidden",
            [[150.0, 50.0], [150.0, 50.0], [150.0, 50.0]],
        ),
        (
            "50%",
            "max-width:90px",
            [[90.0, 50.0], [90.0, 50.0], [90.0, 50.0]],
        ),
        (
            "50%",
            "min-height:51px",
            [[150.0, 51.0], [150.0, 51.0], [150.0, 51.0]],
        ),
        (
            "50%",
            "box-sizing:content-box",
            [[200.0, 90.0], [200.0, 90.0], [200.0, 90.0]],
        ),
    ] {
        let css = format!("display:flex;{RATIO};width:{width};{modifier}");
        for (items, size) in ["XXXX XXXX", ITEMS, tiny].into_iter().zip(sizes) {
            assert_boxes(
                &absolute(&css, items),
                &[("o", [8.0, 8.0, size[0], size[1]])],
            );
        }
    }
}

#[test]
fn fixed_width_content_floors_are_limited_to_positioned_ratio_boxes() {
    for make in [in_flow, absolute] {
        let css = "display:flex;width:30px;height:40px";
        assert_boxes(&make(css, ITEMS), &[("o", [8.0, 8.0, 50.0, 50.0])]);
    }
    assert_boxes(
        &in_flow(
            "display:flex;width:30px;height:40px;aspect-ratio:2/1",
            ITEMS,
        ),
        &[("o", [8.0, 8.0, 50.0, 50.0])],
    );
}

#[test]
fn absolute_content_floors_work_in_each_parent_formatting_context() {
    for parent in ["block", "flex", "grid"] {
        for (display, width) in [("block", 90.0), ("flex", 120.0), ("grid", 120.0)] {
            let html = format!(
                "<div style=\"position:relative;display:{parent};width:300px;height:300px\">\
                 <div id=o style=\"position:absolute;display:{display};grid-auto-flow:column;\
                 width:30px;{RATIO};{BORDER}\">{ITEMS}</div></div>"
            );
            assert_boxes(&html, &[("o", [8.0, 8.0, width, 50.0])]);
        }
    }
}
