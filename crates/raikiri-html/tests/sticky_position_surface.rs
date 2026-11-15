//! A sticky box outside any scroll container sticks to the page area, and
//! one inside a scroll container to its content box, at the initial scroll
//! position. It moves only as far as it must to keep its insets from the
//! edges of that area, and never out of its containing block with its
//! margins. Where each box falls is what Chromium prints for the same
//! documents.
use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, PaintEvent, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

/// A document body and the page, id and border box of each box with an id.
type Row = (&'static str, &'static [(u32, &'static str, [i32; 4])]);

/// The page, id and border box of every box with an id in `body`, laid out
/// in Ahem at 10px with a 10px line height on pages 800px by 600px.
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
        "<!doctype html><style>@page{{size:800px 600px;margin:0}} body{{margin:0;font:10px/10px Ahem}}</style><body>{body}</body>"
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
        // Where the painter draws each box: its fragment moved by the
        // translations around it.
        let mut shifts = vec![(0.0, 0.0)];
        for event in page.paint_order() {
            match event {
                PaintEvent::PushTransform(transform) => {
                    let (x, y) = *shifts.last().unwrap();
                    shifts.push((x + transform.e, y + transform.f));
                }
                PaintEvent::PopTransform => {
                    shifts.pop();
                }
                PaintEvent::Box(fragment) => {
                    let Some(id) = page.dom().attr(fragment.node(), "id") else {
                        continue;
                    };
                    let (x, y) = *shifts.last().unwrap();
                    let rect = fragment.rect();
                    boxes.push((
                        index,
                        id.to_owned(),
                        [rect.x + x, rect.y + y, rect.width, rect.height]
                            .map(|value| value.round() as i32),
                    ));
                }
                _ => {}
            }
        }
        for run in page.text_runs() {
            boxes.push((
                index,
                format!("text {}", run.text),
                [
                    run.origin.0,
                    run.origin.1 - run.ascent,
                    run.advance,
                    run.ascent + run.descent,
                ]
                .map(|value| value.round() as i32),
            ));
        }
    }
    boxes
}

fn assert_rows(rows: &[Row]) {
    let mut failures = Vec::new();
    for &(body, expected) in rows {
        let expected = expected
            .iter()
            .map(|&(page, id, rect)| (page, id.to_owned(), rect))
            .collect::<Vec<_>>();
        let actual = boxes(body);
        if actual != expected {
            failures.push(format!(
                "{body}\n  expected {expected:?}\n  actual   {actual:?}"
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn a_sticky_box_moves_only_when_its_place_crosses_an_inset_edge_of_the_page_area() {
    // A box already below `top` or above `bottom` stays where the flow put
    // it, on whichever page; one that crosses the edge moves to it. Percentages
    // are of the page area. Of `left` and `right`, the one on the far side
    // moves the box last.
    assert_rows(&[
        (
            "<div id=a style='height:20px'></div><div id=b style='position:sticky;top:10px;height:20px'></div>",
            &[(0, "a", [0, 0, 800, 20]), (0, "b", [0, 20, 800, 20])],
        ),
        (
            "<div id=b style='position:sticky;top:10px;height:20px;margin-top:30px'></div>",
            &[(0, "b", [0, 30, 800, 20])],
        ),
        (
            "<div id=b style='position:sticky;bottom:10px;height:20px'></div>",
            &[(0, "b", [0, 0, 800, 20])],
        ),
        (
            "<div id=b style='position:sticky;top:10px;height:20px;margin-top:5px'></div><div id=c style='height:20px'></div>",
            &[(0, "c", [0, 25, 800, 20]), (0, "b", [0, 10, 800, 20])],
        ),
        (
            "<div id=p style='height:100px'><div id=b style='position:sticky;top:10px;height:20px'></div></div>",
            &[(0, "p", [0, 0, 800, 100]), (0, "b", [0, 10, 800, 20])],
        ),
        (
            "<div id=a style='height:620px'></div><div id=b style='position:sticky;top:10px;height:20px'></div>",
            &[
                (0, "a", [0, 0, 800, 600]),
                (1, "a", [0, 0, 800, 20]),
                (1, "b", [0, 20, 800, 20]),
            ],
        ),
        (
            "<div id=a style='height:600px'></div><div id=b style='position:sticky;top:10px;height:20px'></div>",
            &[(0, "a", [0, 0, 800, 600]), (1, "b", [0, 0, 800, 20])],
        ),
        (
            "<div id=p style='height:1500px'><div id=b style='position:sticky;top:10px;height:20px'></div></div>",
            &[
                (0, "p", [0, 0, 800, 600]),
                (0, "b", [0, 10, 800, 20]),
                (1, "p", [0, 0, 800, 600]),
                (2, "p", [0, 0, 800, 300]),
            ],
        ),
        (
            "<div id=a style='height:580px'></div><div id=b style='position:sticky;bottom:10px;height:20px'></div><div id=c style='height:50px'></div>",
            &[
                (0, "a", [0, 0, 800, 580]),
                (0, "b", [0, 570, 800, 20]),
                (1, "c", [0, 0, 800, 50]),
            ],
        ),
        (
            "<div id=b style='position:sticky;left:10px;width:20px;height:20px'></div>",
            &[(0, "b", [10, 0, 20, 20])],
        ),
        (
            "<div id=b style='position:sticky;right:10px;width:20px;height:20px;margin-left:700px'></div>",
            &[(0, "b", [700, 0, 20, 20])],
        ),
        (
            "<div id=p style='width:400px;height:40px'><div id=b style='position:sticky;left:10%;top:10%;width:20px;height:20px'></div></div>",
            &[(0, "p", [0, 0, 400, 40]), (0, "b", [80, 20, 20, 20])],
        ),
        (
            "<div id=p style='height:100px'><div id=b style='position:sticky;top:10px;bottom:10px;height:20px'></div></div>",
            &[(0, "p", [0, 0, 800, 100]), (0, "b", [0, 10, 800, 20])],
        ),
        (
            "<div id=p dir=rtl style='height:20px'><div id=b style='position:sticky;left:10px;right:10px;width:20px;height:20px'></div></div>",
            &[(0, "p", [0, 0, 800, 20]), (0, "b", [770, 0, 20, 20])],
        ),
        (
            "<div id=p style='height:20px;width:200px'><div id=b style='position:sticky;left:10px;right:700px;width:20px;height:20px'></div></div>",
            &[(0, "p", [0, 0, 200, 20]), (0, "b", [10, 0, 20, 20])],
        ),
    ]);
}

#[test]
fn a_sticky_box_stays_inside_its_containing_block_with_its_margins() {
    // The margin box stays inside the content box of the containing block, and
    // a box that already stands outside it is not moved further out.
    assert_rows(&[
        (
            "<div id=p style='height:25px'><div id=b style='position:sticky;top:10px;height:20px'></div></div>",
            &[(0, "p", [0, 0, 800, 25]), (0, "b", [0, 5, 800, 20])],
        ),
        (
            "<div id=p style='height:25px;padding-bottom:10px'><div id=b style='position:sticky;top:10px;height:20px;margin-bottom:2px'></div></div>",
            &[(0, "p", [0, 0, 800, 35]), (0, "b", [0, 3, 800, 20])],
        ),
        (
            "<div id=p style='height:30px;padding:0 0 10px'><div id=b style='position:sticky;top:20px;height:20px;margin-bottom:4px'></div></div>",
            &[(0, "p", [0, 0, 800, 40]), (0, "b", [0, 6, 800, 20])],
        ),
        (
            "<div id=p style='height:30px'><div id=b style='position:sticky;top:20px;height:20px;margin-bottom:-6px'></div></div>",
            &[(0, "p", [0, 0, 800, 30]), (0, "b", [0, 16, 800, 20])],
        ),
        (
            "<div id=p style='width:30px;height:20px;margin-left:-10px'><div id=b style='position:sticky;left:10px;width:20px;height:20px;margin-right:5px'></div></div>",
            &[(0, "p", [-10, 0, 30, 20]), (0, "b", [-5, 0, 20, 20])],
        ),
        (
            "<div id=a style='height:595px'></div><div id=p style='height:40px'><div id=b style='position:sticky;bottom:10px;height:20px;margin-top:5px'></div></div>",
            &[
                (0, "a", [0, 0, 800, 595]),
                (1, "p", [0, 0, 800, 40]),
                (1, "b", [0, 0, 800, 20]),
            ],
        ),
        (
            "<div id=p style='padding-top:650px'><div id=b style='position:sticky;bottom:10px;height:20px'></div></div>",
            &[
                (0, "p", [0, 0, 800, 600]),
                (1, "p", [0, 0, 800, 70]),
                (1, "b", [0, 50, 800, 20]),
            ],
        ),
        (
            "<div id=b style='position:sticky;right:10px;width:20px;height:20px;margin-left:790px'></div>",
            &[(0, "b", [790, 0, 20, 20])],
        ),
    ]);
}

#[test]
fn flex_and_grid_items_floats_and_inline_blocks_stick_alike() {
    // Boxes inside a sticky box move with it, and a sticky box inside another
    // one is measured where the outer one moved it.
    assert_rows(&[
        (
            "<div id=p style='display:flex;height:100px'><div id=b style='position:sticky;top:10px;width:20px;height:20px'></div><div id=c style='width:20px;height:20px'></div></div>",
            &[
                (0, "p", [0, 0, 800, 100]),
                (0, "c", [20, 0, 20, 20]),
                (0, "b", [0, 10, 20, 20]),
            ],
        ),
        (
            "<div id=p style='display:grid;height:100px;grid-template-rows:50px 50px'><div id=b style='position:sticky;top:10px;height:20px'></div></div>",
            &[(0, "p", [0, 0, 800, 100]), (0, "b", [0, 10, 800, 20])],
        ),
        (
            "<div id=p style='height:100px'><div id=b style='display:inline-block;position:sticky;top:10px;width:20px;height:20px'></div></div>",
            &[(0, "p", [0, 0, 800, 100]), (0, "b", [0, 10, 20, 20])],
        ),
        (
            "<div id=p style='height:100px'><div id=b style='float:left;position:sticky;top:10px;width:20px;height:20px'></div></div>",
            &[(0, "p", [0, 0, 800, 100]), (0, "b", [0, 10, 20, 20])],
        ),
        (
            "<div id=p style='height:200px'><div id=b style='position:sticky;top:10px;height:100px'><div id=c style='position:sticky;top:30px;height:20px'></div></div></div>",
            &[
                (0, "p", [0, 0, 800, 200]),
                (0, "b", [0, 10, 800, 100]),
                (0, "c", [0, 30, 800, 20]),
            ],
        ),
        (
            "<div id=p style='height:100px'><div id=b style='position:sticky;top:10px;height:40px'><div id=c style='height:10px;margin-top:5px'></div></div></div>",
            &[
                (0, "p", [0, 5, 800, 100]),
                (0, "b", [0, 10, 800, 40]),
                (0, "c", [0, 10, 800, 10]),
            ],
        ),
        (
            "<div id=p style='height:100px'><div id=b style='position:sticky;top:10px;height:40px'><div id=c style='position:absolute;top:5px;width:10px;height:10px'></div></div></div>",
            &[
                (0, "p", [0, 0, 800, 100]),
                (0, "b", [0, 10, 800, 40]),
                (0, "c", [0, 15, 10, 10]),
            ],
        ),
    ]);
}

#[test]
fn a_sticky_box_in_a_scroll_container_sticks_to_its_content_box() {
    // The content box of the nearest scroll container, at its initial scroll
    // position, is the view; `overflow: clip` makes no scroll container.
    assert_rows(&[
        (
            "<div id=s style='overflow:hidden;height:100px;margin-top:50px'><div id=a style='height:20px'></div><div id=b style='position:sticky;top:10px;height:20px'></div></div>",
            &[
                (0, "s", [0, 50, 800, 100]),
                (0, "a", [0, 50, 800, 20]),
                (0, "b", [0, 70, 800, 20]),
            ],
        ),
        (
            "<div id=s style='overflow:hidden;height:100px;margin-top:50px'><div id=b style='position:sticky;top:10px;height:20px'></div><div id=a style='height:200px'></div></div>",
            &[
                (0, "s", [0, 50, 800, 100]),
                (0, "a", [0, 70, 800, 200]),
                (0, "b", [0, 60, 800, 20]),
            ],
        ),
        (
            "<div id=s style='overflow:hidden;height:100px;padding:5px'><div id=a style='height:150px'></div><div id=b style='position:sticky;bottom:10px;height:20px'></div></div>",
            &[
                (0, "s", [0, 0, 800, 110]),
                (0, "a", [5, 5, 790, 150]),
                (0, "b", [5, 75, 790, 20]),
            ],
        ),
        (
            "<div id=s style='overflow:clip;height:100px;margin-top:50px'><div id=b style='position:sticky;top:10px;height:20px'></div><div id=a style='height:200px'></div></div>",
            &[
                (0, "s", [0, 50, 800, 100]),
                (0, "a", [0, 70, 800, 200]),
                (0, "b", [0, 50, 800, 20]),
            ],
        ),
        (
            "<div id=s style='overflow:scroll;height:100px'><div id=a style='height:150px'></div><div id=b style='position:sticky;bottom:10px;height:20px'></div></div>",
            &[
                (0, "s", [0, 0, 800, 100]),
                (0, "a", [0, 0, 800, 150]),
                (0, "b", [0, 70, 800, 20]),
            ],
        ),
        // A fixed box takes what it holds out of the scroll container.
        (
            "<div id=s style='overflow:hidden;height:100px;margin-top:300px'><div id=f style='position:fixed;top:0;left:0;width:100px;height:100px'><div id=a style='height:20px'></div><div id=b style='position:sticky;top:10px;height:20px'></div></div></div>",
            &[
                (0, "s", [0, 300, 800, 100]),
                (0, "f", [0, 0, 100, 100]),
                (0, "a", [0, 0, 100, 20]),
                (0, "b", [0, 20, 100, 20]),
            ],
        ),
    ]);
}

#[test]
fn a_box_below_the_first_page_is_not_pulled_up_by_its_bottom_inset() {
    // Chromium pulls the box onto the first page, 570px down, over what the
    // pages cut there. The painter draws a box on the page the flow cuts it
    // onto, where that pull would hide it, so it keeps its place.
    assert_rows(&[(
        "<div id=a style='height:700px'></div><div id=b style='position:sticky;bottom:10px;height:20px'></div>",
        &[
            (0, "a", [0, 0, 800, 600]),
            (1, "a", [0, 0, 800, 100]),
            (1, "b", [0, 100, 800, 20]),
        ],
    )]);
}
