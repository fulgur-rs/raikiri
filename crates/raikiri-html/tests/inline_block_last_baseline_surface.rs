//! The baseline an inline-block sits on is the last one of its content, found
//! the way Chromium finds it: a block container that is a scroll container
//! gives its block-end margin edge, whatever lines it holds; a box whose
//! writing mode runs across the inline-block gives none; and a relative
//! offset moves a box without moving the baseline it gives. Where each box
//! falls is what Chromium lays out for the same documents. The inline-flex
//! rows keep a column of a size-contained item, which shrinks below its
//! contained height, sitting on that item's bottom edge.
use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

/// A document body and the id and border box of each box with an id in it.
type Row = (&'static str, &'static [(&'static str, [i32; 4])]);

/// The id and border box of every box with an id in `body`, laid out in Ahem
/// at 10px with a 10px line height on one page 800px wide.
fn boxes(body: &str) -> Vec<(String, [i32; 4])> {
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
    let page = result.page(0).unwrap();
    page.fragments()
        .filter_map(|fragment| {
            let id = page.dom().attr(fragment.node(), "id")?;
            let rect = fragment.rect();
            Some((
                id.to_owned(),
                [rect.x, rect.y, rect.width, rect.height].map(|value| value.round() as i32),
            ))
        })
        .collect()
}

fn assert_rows(rows: &[Row]) {
    for &(body, expected) in rows {
        let expected = expected
            .iter()
            .map(|&(id, rect)| (id.to_owned(), rect))
            .collect::<Vec<_>>();
        assert_eq!(boxes(body), expected, "{body}");
    }
}

#[test]
fn a_scroll_container_in_an_inline_block_gives_its_bottom_margin_edge() {
    // A scroll container 30px tall, empty or holding a line, with a margin
    // below it, behind or ahead of a line, inside a padded block, or followed
    // by a float, puts the baseline at its bottom margin edge, so the line
    // grows to hold the inline-block above the strut's descent. `overflow:
    // clip`, a flex scroll container and a scroll container that is a flex
    // item keep the baseline of the line inside. Layout containment does not
    // take the edge away from a scroll container.
    assert_rows(&[
        (
            "<div id=w style='margin-top:300px;width:600px'><span id=a>A</span><div id=f style='display:inline-block;width:40px'><div id=i style='overflow:hidden;height:30px'>X</div></div>B</div>",
            &[
                ("w", [0, 300, 600, 32]),
                ("a", [0, 322, 10, 10]),
                ("f", [10, 300, 40, 30]),
                ("i", [10, 300, 40, 30]),
            ],
        ),
        (
            "<div id=w style='margin-top:300px;width:600px'><span id=a>A</span><div id=f style='display:inline-block;width:40px'><div id=i style='overflow:hidden;height:30px'></div></div>B</div>",
            &[
                ("w", [0, 300, 600, 32]),
                ("a", [0, 322, 10, 10]),
                ("f", [10, 300, 40, 30]),
                ("i", [10, 300, 40, 30]),
            ],
        ),
        (
            "<div id=w style='margin-top:300px;width:600px'><span id=a>A</span><div id=f style='display:inline-block;width:40px'><div id=i style='overflow:hidden;height:30px;margin-bottom:7px'>X</div></div>B</div>",
            &[
                ("w", [0, 300, 600, 39]),
                ("a", [0, 329, 10, 10]),
                ("f", [10, 300, 40, 37]),
                ("i", [10, 300, 40, 30]),
            ],
        ),
        (
            "<div id=w style='margin-top:300px;width:600px'><span id=a>A</span><div id=f style='display:inline-block;width:40px'><div id=i style='overflow:hidden;height:30px;margin-bottom:-12px'>X</div></div>B</div>",
            &[
                ("w", [0, 300, 600, 20]),
                ("a", [0, 310, 10, 10]),
                ("f", [10, 300, 40, 18]),
                ("i", [10, 300, 40, 30]),
            ],
        ),
        (
            "<div id=w style='margin-top:300px;width:600px'><span id=a>A</span><div id=f style='display:inline-block;width:40px'><div id=i style='display:flow-root;overflow:auto;height:30px'>X</div></div>B</div>",
            &[
                ("w", [0, 300, 600, 32]),
                ("a", [0, 322, 10, 10]),
                ("f", [10, 300, 40, 30]),
                ("i", [10, 300, 40, 30]),
            ],
        ),
        (
            "<div id=w style='margin-top:300px;width:600px'><span id=a>A</span><div id=f style='display:inline-block;width:40px'><div id=i style='overflow-x:hidden;height:30px'>X</div></div>B</div>",
            &[
                ("w", [0, 300, 600, 32]),
                ("a", [0, 322, 10, 10]),
                ("f", [10, 300, 40, 30]),
                ("i", [10, 300, 40, 30]),
            ],
        ),
        (
            "<div id=w style='margin-top:300px;width:600px'><span id=a>A</span><div id=f style='display:inline-block;width:40px'><div>X</div><div id=i style='overflow:hidden;height:30px'></div></div>B</div>",
            &[
                ("w", [0, 300, 600, 42]),
                ("a", [0, 332, 10, 10]),
                ("f", [10, 300, 40, 40]),
                ("i", [10, 310, 40, 30]),
            ],
        ),
        (
            "<div id=w style='margin-top:300px;width:600px'><span id=a>A</span><div id=f style='display:inline-block;width:40px'><div id=i style='overflow:hidden;height:30px'>X</div><div>X</div></div>B</div>",
            &[
                ("w", [0, 300, 600, 40]),
                ("a", [0, 330, 10, 10]),
                ("f", [10, 300, 40, 40]),
                ("i", [10, 300, 40, 30]),
            ],
        ),
        (
            "<div id=w style='margin-top:300px;width:600px'><span id=a>A</span><div id=f style='display:inline-block;width:40px'><div id=i style='padding-bottom:6px'><div style='overflow:hidden;height:30px'>X</div></div></div>B</div>",
            &[
                ("w", [0, 300, 600, 36]),
                ("a", [0, 322, 10, 10]),
                ("f", [10, 300, 40, 36]),
                ("i", [10, 300, 40, 36]),
            ],
        ),
        (
            "<div id=w style='margin-top:300px;width:600px'><span id=a>A</span><div id=f style='display:inline-block;width:40px'><div id=i style='overflow:hidden;height:30px'>X</div><div style='float:left;height:50px;width:5px'></div></div>B</div>",
            &[
                ("w", [0, 300, 600, 80]),
                ("a", [0, 322, 10, 10]),
                ("f", [10, 300, 40, 80]),
                ("i", [10, 300, 40, 30]),
            ],
        ),
        (
            "<div id=w style='margin-top:300px;width:600px'><span id=a>A</span><div id=f style='display:inline-block;width:40px'><div id=i style='overflow:hidden;height:40px'>X<br>X</div></div>B</div>",
            &[
                ("w", [0, 300, 600, 42]),
                ("a", [0, 332, 10, 10]),
                ("f", [10, 300, 40, 40]),
                ("i", [10, 300, 40, 40]),
            ],
        ),
        (
            "<div id=w style='margin-top:300px;width:600px'><span id=a>A</span><div id=f style='display:inline-block;width:40px'><div id=i style='overflow:clip;height:30px'>X</div></div>B</div>",
            &[
                ("w", [0, 300, 600, 30]),
                ("a", [0, 300, 10, 10]),
                ("f", [10, 300, 40, 30]),
                ("i", [10, 300, 40, 30]),
            ],
        ),
        (
            "<div id=w style='margin-top:300px;width:600px'><span id=a>A</span><div id=f style='display:inline-block;width:40px'><div id=i style='display:flex;overflow:hidden;height:30px'>X</div></div>B</div>",
            &[
                ("w", [0, 300, 600, 30]),
                ("a", [0, 300, 10, 10]),
                ("f", [10, 300, 40, 30]),
                ("i", [10, 300, 40, 30]),
            ],
        ),
        (
            "<div id=w style='margin-top:300px;width:600px'><span id=a>A</span><div id=f style='display:inline-block;width:40px'><div id=i style='display:flex'><div style='overflow:hidden;height:30px'>X</div></div></div>B</div>",
            &[
                ("w", [0, 300, 600, 30]),
                ("a", [0, 300, 10, 10]),
                ("f", [10, 300, 40, 30]),
                ("i", [10, 300, 40, 30]),
            ],
        ),
        (
            "<div id=w style='margin-top:300px;width:600px'><span id=a>A</span><div id=f style='display:inline-block;width:40px'><div>X</div><div id=i style='overflow:hidden;contain:layout;height:30px'></div></div>B</div>",
            &[
                ("w", [0, 300, 600, 42]),
                ("a", [0, 332, 10, 10]),
                ("f", [10, 300, 40, 40]),
                ("i", [10, 310, 40, 30]),
            ],
        ),
    ]);
}

#[test]
fn a_box_in_another_writing_mode_gives_an_inline_block_no_baseline() {
    // A vertical box holds no line the inline-block could sit on: the
    // line before it gives the baseline, and without one the inline-block
    // sits on its bottom margin edge. A vertical `display: contents` element
    // generates no box, so the horizontal box inside it still gives one.
    assert_rows(&[
        (
            "<div id=w style='margin-top:300px;width:600px'><span id=a>A</span><div id=f style='display:inline-block;width:40px'><div>X</div><div id=i style='writing-mode:vertical-rl;height:30px;width:20px'>X</div></div>B</div>",
            &[
                ("w", [0, 300, 600, 40]),
                ("a", [0, 300, 10, 10]),
                ("f", [10, 300, 40, 40]),
                ("i", [10, 310, 20, 30]),
            ],
        ),
        (
            "<div id=w style='margin-top:300px;width:600px'><span id=a>A</span><div id=f style='display:inline-block;width:40px'><div id=i style='writing-mode:vertical-rl;height:30px;width:20px'>X</div></div>B</div>",
            &[
                ("w", [0, 300, 600, 32]),
                ("a", [0, 322, 10, 10]),
                ("f", [10, 300, 40, 30]),
                ("i", [10, 300, 20, 30]),
            ],
        ),
        (
            "<div id=w style='margin-top:300px;width:600px'><span id=a>A</span><div id=f style='display:inline-block;width:40px'><div>X</div><div id=i style='padding-bottom:5px'><div style='writing-mode:vertical-rl;height:30px;width:20px'>X</div></div></div>B</div>",
            &[
                ("w", [0, 300, 600, 45]),
                ("a", [0, 300, 10, 10]),
                ("f", [10, 300, 40, 45]),
                ("i", [10, 310, 40, 35]),
            ],
        ),
        (
            "<div id=w style='margin-top:300px;width:600px'><span id=a>A</span><div id=f style='display:inline-block;width:40px'><div>X</div><div style='display:contents;writing-mode:vertical-rl'><div id=i style='writing-mode:horizontal-tb'>X<br>X</div></div></div>B</div>",
            &[
                ("w", [0, 300, 600, 30]),
                ("a", [0, 320, 10, 10]),
                ("f", [10, 300, 40, 30]),
                ("i", [10, 310, 40, 20]),
            ],
        ),
        (
            "<div id=w style='margin-top:300px;width:600px'><span id=a>A</span><div id=f style='display:inline-block;width:40px'><div>X</div><div style='display:contents;writing-mode:vertical-rl'><div id=i>X<br>X</div></div></div>B</div>",
            &[
                ("w", [0, 300, 600, 20]),
                ("a", [0, 300, 10, 10]),
                ("f", [10, 300, 40, 20]),
                ("i", [10, 310, 20, 10]),
            ],
        ),
    ]);
}

#[test]
fn a_relative_offset_does_not_move_the_baseline_of_an_inline_block() {
    // Boxes moved by `top`, `bottom`, a percentage of a definite height (a
    // height that is itself a percentage, a grid area, a flex container or a
    // scroll container), or a relative offset of their parent too, give the
    // baseline of where they would be.
    assert_rows(&[
        (
            "<div id=w style='margin-top:300px;width:600px'><span id=a>A</span><div id=f style='display:inline-block;width:40px'><div id=i style='height:30px;position:relative;top:9px'>X</div></div>B</div>",
            &[
                ("w", [0, 300, 600, 30]),
                ("a", [0, 300, 10, 10]),
                ("f", [10, 300, 40, 30]),
                ("i", [10, 309, 40, 30]),
            ],
        ),
        (
            "<div id=w style='margin-top:300px;width:600px'><span id=a>A</span><div id=f style='display:inline-block;width:40px'><div id=i style='height:30px;position:relative;bottom:7px'>X</div></div>B</div>",
            &[
                ("w", [0, 300, 600, 30]),
                ("a", [0, 300, 10, 10]),
                ("f", [10, 300, 40, 30]),
                ("i", [10, 293, 40, 30]),
            ],
        ),
        (
            "<div id=w style='margin-top:300px;width:600px'><span id=a>A</span><div id=f style='display:inline-block;width:40px'><div id=i style='overflow:hidden;height:30px;position:relative;top:9px'>X</div></div>B</div>",
            &[
                ("w", [0, 300, 600, 32]),
                ("a", [0, 322, 10, 10]),
                ("f", [10, 300, 40, 30]),
                ("i", [10, 309, 40, 30]),
            ],
        ),
        (
            "<div id=w style='margin-top:300px;width:600px'><span id=a>A</span><div id=f style='display:inline-block;width:40px'><div style='height:40px'><div id=i style='height:20px;position:relative;top:50%'>X</div></div></div>B</div>",
            &[
                ("w", [0, 300, 600, 40]),
                ("a", [0, 300, 10, 10]),
                ("f", [10, 300, 40, 40]),
                ("i", [10, 320, 40, 20]),
            ],
        ),
        (
            "<div id=w style='margin-top:300px;width:600px'><span id=a>A</span><div id=f style='display:inline-block;width:40px'><div style='position:relative;top:4px'><div id=i style='height:20px;position:relative;top:3px'>X</div></div></div>B</div>",
            &[
                ("w", [0, 300, 600, 20]),
                ("a", [0, 300, 10, 10]),
                ("f", [10, 300, 40, 20]),
                ("i", [10, 307, 40, 20]),
            ],
        ),
        (
            "<div id=w style='margin-top:300px;width:600px'><span id=a>A</span><div id=f style='display:inline-block;width:40px'><div style='height:100px'><div style='height:50%'><div id=i style='position:relative;top:10%'>X</div></div></div></div>B</div>",
            &[
                ("w", [0, 300, 600, 100]),
                ("a", [0, 300, 10, 10]),
                ("f", [10, 300, 40, 100]),
                ("i", [10, 305, 40, 10]),
            ],
        ),
        (
            "<div id=w style='margin-top:300px;width:600px'><span id=a>A</span><div id=f style='display:inline-block;width:40px'><div style='display:grid;height:100px;grid-template-rows:50px 50px'><div id=i style='position:relative;top:10%'>X</div></div></div>B</div>",
            &[
                ("w", [0, 300, 600, 100]),
                ("a", [0, 300, 10, 10]),
                ("f", [10, 300, 40, 100]),
                ("i", [10, 305, 40, 50]),
            ],
        ),
        (
            "<div id=w style='margin-top:300px;width:600px'><span id=a>A</span><div id=f style='display:inline-block;width:40px'><div style='display:flex;height:100px'><div id=i style='position:relative;top:10%'>X</div></div></div>B</div>",
            &[
                ("w", [0, 300, 600, 100]),
                ("a", [0, 300, 10, 10]),
                ("f", [10, 300, 40, 100]),
                ("i", [10, 310, 10, 100]),
            ],
        ),
        (
            "<div id=w style='margin-top:300px;width:600px'><span id=a>A</span><div id=f style='display:inline-block;width:40px'><div style='overflow:hidden;height:100px'><div id=i style='position:relative;top:10%'>X</div></div></div>B</div>",
            &[
                ("w", [0, 300, 600, 102]),
                ("a", [0, 392, 10, 10]),
                ("f", [10, 300, 40, 100]),
                ("i", [10, 310, 40, 10]),
            ],
        ),
    ]);
}

#[test]
fn an_inline_flex_column_sits_on_its_shrunk_size_contained_item() {
    // A size-contained item 20px tall in an inline-flex column 10px tall
    // shrinks to 10px, with or without `min-height: 0`, and the inline-flex
    // sits on the item's bottom edge.
    assert_rows(&[
        (
            "<div id=w style='margin-top:300px;width:600px'><span id=a>A</span><div id=f style='display:inline-flex;flex-direction:column;height:10px'><div id=i style='contain:size;contain-intrinsic-width:300px;contain-intrinsic-height:20px;min-height:0'><p style='margin:0;height:10px;width:20px'></p></div></div>B</div>",
            &[
                ("w", [0, 300, 600, 12]),
                ("a", [0, 302, 10, 10]),
                ("f", [10, 300, 300, 10]),
                ("i", [10, 300, 300, 10]),
            ],
        ),
        (
            "<div id=w style='margin-top:300px;width:600px'><span id=a>A</span><div id=f style='display:inline-flex;flex-direction:column;height:10px'><div id=i style='contain:size;contain-intrinsic-width:300px;contain-intrinsic-height:20px'><p style='margin:0;height:10px;width:20px'></p></div></div>B</div>",
            &[
                ("w", [0, 300, 600, 12]),
                ("a", [0, 302, 10, 10]),
                ("f", [10, 300, 300, 10]),
                ("i", [10, 300, 300, 10]),
            ],
        ),
        (
            "<div id=w style='margin-top:300px;width:600px'><span id=a>A</span><div id=f style='display:inline-flex;flex-direction:column'><div id=i style='contain:size;contain-intrinsic-width:300px;contain-intrinsic-height:20px;min-height:0'><p style='margin:0;height:10px;width:20px'></p></div></div>B</div>",
            &[
                ("w", [0, 300, 600, 22]),
                ("a", [0, 312, 10, 10]),
                ("f", [10, 300, 300, 20]),
                ("i", [10, 300, 300, 20]),
            ],
        ),
        (
            "<div id=w style='margin-top:300px;width:600px'><span id=a>A</span><div id=f style='display:inline-flex;height:10px'><div id=i style='contain:size;contain-intrinsic-width:300px;contain-intrinsic-height:20px'><p style='margin:0;height:10px;width:20px'></p></div></div>B</div>",
            &[
                ("w", [0, 300, 600, 12]),
                ("a", [0, 302, 10, 10]),
                ("f", [10, 300, 300, 10]),
                ("i", [10, 300, 300, 10]),
            ],
        ),
    ]);
}

#[test]
fn a_float_gives_an_inline_block_no_baseline() {
    // A float, scrolling or not, directly in the inline-block or in a block
    // inside it, gives no baseline even with lines of its own: the line
    // before it does, and without one the inline-block sits on its bottom
    // margin edge. A flex item that declares a float does not float.
    assert_rows(&[
        (
            "<div id=w style='margin-top:300px;width:600px'><span id=a>A</span><div id=f style='display:inline-block;width:40px'><div>X</div><div id=i style='float:left;overflow:hidden;height:50px;width:5px'></div></div>B</div>",
            &[
                ("w", [0, 300, 600, 60]),
                ("a", [0, 300, 10, 10]),
                ("f", [10, 300, 40, 60]),
                ("i", [10, 310, 5, 50]),
            ],
        ),
        (
            "<div id=w style='margin-top:300px;width:600px'><span id=a>A</span><div id=f style='display:inline-block;width:40px'><div>X</div><div id=i style='float:left;overflow:hidden;height:50px;width:15px'>X</div></div>B</div>",
            &[
                ("w", [0, 300, 600, 60]),
                ("a", [0, 300, 10, 10]),
                ("f", [10, 300, 40, 60]),
                ("i", [10, 310, 15, 50]),
            ],
        ),
        (
            "<div id=w style='margin-top:300px;width:600px'><span id=a>A</span><div id=f style='display:inline-block;width:40px'><div>X</div><div id=i style='float:left;height:50px;width:15px'><br>X</div></div>B</div>",
            &[
                ("w", [0, 300, 600, 60]),
                ("a", [0, 300, 10, 10]),
                ("f", [10, 300, 40, 60]),
                ("i", [10, 310, 15, 50]),
            ],
        ),
        (
            "<div id=w style='margin-top:300px;width:600px'><span id=a>A</span><div id=f style='display:inline-block;width:40px'><div id=i style='float:left;height:50px;width:15px'><br>X</div></div>B</div>",
            &[
                ("w", [0, 300, 600, 52]),
                ("a", [0, 342, 10, 10]),
                ("f", [10, 300, 40, 50]),
                ("i", [10, 300, 15, 50]),
            ],
        ),
        (
            "<div id=w style='margin-top:300px;width:600px'><span id=a>A</span><div id=f style='display:inline-block;width:40px'><div>X</div><div><div id=i style='float:left;height:50px;width:15px'><br>X</div></div></div>B</div>",
            &[
                ("w", [0, 300, 600, 60]),
                ("a", [0, 300, 10, 10]),
                ("f", [10, 300, 40, 60]),
                ("i", [10, 310, 15, 50]),
            ],
        ),
        (
            "<div id=w style='margin-top:300px;width:600px'><span id=a>A</span><div id=f style='display:inline-block;width:40px'><div style='display:flex'><div id=i style='float:left;overflow:hidden;height:30px'>X</div></div></div>B</div>",
            &[
                ("w", [0, 300, 600, 30]),
                ("a", [0, 300, 10, 10]),
                ("f", [10, 300, 40, 30]),
                ("i", [10, 300, 10, 30]),
            ],
        ),
    ]);
}
