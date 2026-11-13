//! An inline flex or grid container whose items have size containment sits
//! on its line by the baseline it takes from those items. Size containment
//! keeps the baseline of an item's text; an item without text has none, so
//! the container synthesizes one from the item's border box. Where each box
//! falls is what Chromium lays out for the same documents.
use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

/// A document body and the id and border box of each box with an id in it.
type Row = (&'static str, &'static [(&'static str, [i32; 4])]);

/// The id and border box of every box with an id in `body`, laid out on one
/// page 1280px wide.
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
        "<!doctype html><style>@page{{size:1280px 1000px;margin:0}} body{{margin:0}}</style><body>{body}</body>"
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
fn a_column_container_takes_its_baseline_from_its_first_item() {
    // A column flex container takes its baseline from its first item. An
    // item with no text gives the bottom edge of its border box, and an item
    // with text gives the text's baseline. Size containment drops the
    // item's content-based minimum, so the item 20px tall by containment
    // shrinks to the 10px column with or without `min-height: 0`. Text
    // around the container, padding and border on the item, a second item
    // and a larger font on the line keep that.
    assert_rows(&[
        (
            "<div style='margin-top:300px;width:600px'><div id=f style='display:inline-flex;flex-direction:column;height:10px'><div id=c style='contain:size;contain-intrinsic-width:300px;contain-intrinsic-height:20px;min-height:0'><p style='margin:0;height:10px;width:20px'></p></div></div></div>",
            &[("f", [0, 303, 300, 10]), ("c", [0, 303, 300, 10])],
        ),
        (
            "<div style='margin-top:300px;width:600px'><div id=f style='display:inline-flex;flex-direction:column'><div id=c style='contain:size;contain-intrinsic-width:300px;contain-intrinsic-height:20px;min-height:0'><p style='margin:0;height:10px;width:20px'></p></div></div></div>",
            &[("f", [0, 300, 300, 20]), ("c", [0, 300, 300, 20])],
        ),
        (
            "<div style='margin-top:300px;width:600px'><div id=f style='display:inline-flex;flex-direction:column;height:10px'><div id=c style='contain:size;contain-intrinsic-width:300px;contain-intrinsic-height:20px;min-height:0'>x</div></div></div>",
            &[("f", [0, 300, 300, 10]), ("c", [0, 300, 300, 10])],
        ),
        (
            "<div style='margin-top:300px;width:600px'>ab<div id=f style='display:inline-flex;flex-direction:column;height:10px'><div id=c style='contain:size;contain-intrinsic-width:300px;contain-intrinsic-height:20px;min-height:0'><p style='margin:0;height:10px;width:20px'></p></div></div>cd</div>",
            &[("f", [32, 303, 300, 10]), ("c", [32, 303, 300, 10])],
        ),
        (
            "<div style='margin-top:300px;width:600px'><div id=f style='display:inline-flex;flex-direction:column;height:10px'><div id=c style='contain:size;contain-intrinsic-width:300px;contain-intrinsic-height:20px;min-height:0;padding:3px;border:2px solid'><p style='margin:0;height:10px;width:20px'></p></div></div></div>",
            &[("f", [0, 303, 310, 10]), ("c", [0, 303, 310, 10])],
        ),
        (
            "<div style='margin-top:300px;width:600px'><div id=f style='display:inline-flex;flex-direction:column;height:10px'><div id=c style='contain:size;contain-intrinsic-width:300px;contain-intrinsic-height:20px;min-height:0'>x</div><div id=d>y</div></div></div>",
            &[
                ("f", [0, 300, 300, 10]),
                ("c", [0, 300, 300, 0]),
                ("d", [0, 300, 300, 16]),
            ],
        ),
        (
            "<div style='margin-top:300px;width:600px'><div id=f style='display:inline-flex;flex-direction:column;height:10px'><div id=d style='height:5px'></div><div id=c style='contain:size;contain-intrinsic-width:300px;contain-intrinsic-height:20px;min-height:0'>x</div></div></div>",
            &[
                ("f", [0, 311, 300, 10]),
                ("d", [0, 311, 300, 2]),
                ("c", [0, 313, 300, 8]),
            ],
        ),
        (
            "<div style='margin-top:300px;width:600px'><div id=f style='display:inline-flex;flex-direction:column;height:10px'><div id=c style='contain:size;contain-intrinsic-width:300px;contain-intrinsic-height:20px'><p style='margin:0;height:10px;width:20px'></p></div></div></div>",
            &[("f", [0, 303, 300, 10]), ("c", [0, 303, 300, 10])],
        ),
        (
            "<div style='font-size:32px'><div style='margin-top:300px;width:600px'><div id=f style='display:inline-flex;flex-direction:column;height:10px'><div id=c style='contain:size;contain-intrinsic-width:300px;contain-intrinsic-height:20px'><p style='margin:0;height:10px;width:20px'></p></div></div></div></div>",
            &[("f", [0, 316, 300, 10]), ("c", [0, 316, 300, 10])],
        ),
    ]);
}

#[test]
fn a_row_or_grid_container_of_contained_items_sits_on_its_line() {
    // In a row flex container the item stretches to the 10px cross size,
    // and in a grid the item keeps its contained height as its minimum
    // unless `min-height: 0` lets the row track hold it to 10px. Either way
    // the container's baseline is the bottom edge of the item's border box.
    // With `align-items: baseline` the baseline comes from the items that
    // share it, so a first item without text does not set it. A contained
    // block inside an inline-block does not change the inline-block's
    // baseline either.
    assert_rows(&[
        (
            "<div style='margin-top:300px;width:600px'><div id=f style='display:inline-flex;height:10px'><div id=c style='contain:size;contain-intrinsic-width:300px;contain-intrinsic-height:20px;min-height:0'><p style='margin:0;height:10px;width:20px'></p></div></div></div>",
            &[("f", [0, 303, 300, 10]), ("c", [0, 303, 300, 10])],
        ),
        (
            "<div style='margin-top:300px;width:600px'><div id=f style='display:inline-flex;height:10px'><div id=c style='contain:size;contain-intrinsic-width:300px;contain-intrinsic-height:20px'><p style='margin:0;height:10px;width:20px'></p></div></div></div>",
            &[("f", [0, 303, 300, 10]), ("c", [0, 303, 300, 10])],
        ),
        (
            "<div style='margin-top:300px;width:600px'><div id=f style='display:inline-grid;height:10px'><div id=c style='contain:size;contain-intrinsic-width:300px;contain-intrinsic-height:20px;min-height:0'><p style='margin:0;height:10px;width:20px'></p></div></div></div>",
            &[("f", [0, 303, 300, 10]), ("c", [0, 303, 300, 10])],
        ),
        (
            "<div style='margin-top:300px;width:600px'><div id=f style='display:inline-grid;height:10px'><div id=c style='contain:size;contain-intrinsic-width:300px;contain-intrinsic-height:20px'><p style='margin:0;height:10px;width:20px'></p></div></div></div>",
            &[("f", [0, 300, 300, 10]), ("c", [0, 300, 300, 20])],
        ),
        (
            "<div style='margin-top:300px;width:600px'><div id=f style='display:inline-grid;grid-template-rows:5px'><div id=c style='contain:size;contain-intrinsic-width:300px;contain-intrinsic-height:20px;align-self:start'><p style='margin:0;height:10px;width:20px'></p></div></div></div>",
            &[("f", [0, 300, 300, 5]), ("c", [0, 300, 300, 20])],
        ),
        (
            "<div style='margin-top:300px;width:600px'><div id=f style='display:inline-flex;align-items:baseline;height:10px'><div id=c style='contain:size;contain-intrinsic-width:30px;contain-intrinsic-height:20px'></div><div id=d>y</div></div></div>",
            &[
                ("f", [0, 300, 46, 10]),
                ("c", [0, 300, 30, 20]),
                ("d", [30, 307, 16, 16]),
            ],
        ),
        (
            "<div style='margin-top:300px;width:600px'><div id=f style='display:inline-flex;align-items:baseline;height:10px'><div id=c style='contain:size;contain-intrinsic-width:30px;contain-intrinsic-height:20px'>x</div><div id=d>y</div></div></div>",
            &[
                ("f", [0, 300, 46, 10]),
                ("c", [0, 300, 30, 20]),
                ("d", [30, 300, 16, 16]),
            ],
        ),
        (
            "<div style='margin-top:300px;width:600px'><div id=f style='display:inline-block;height:10px'><div id=c style='contain:size;contain-intrinsic-width:300px;contain-intrinsic-height:20px'><p style='margin:0;height:10px;width:20px'></p></div></div></div>",
            &[("f", [0, 303, 300, 10]), ("c", [0, 303, 300, 20])],
        ),
    ]);
}
