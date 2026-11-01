//! A table with `table-layout: fixed` takes its columns from the width it
//! states, not from its cells, so a `max-width` below that width narrows the
//! columns along with the table (CSS 2.1 section 17.5.2.1).
//!
//! The expected boxes are the ones Chromium computes for the same markup, with
//! no body margin and `10px Ahem` text: each is the element's left, top, width
//! and height. A cell holds a 200px box, so a table that kept the width its
//! cells ask for would be 200px wide.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/data/text-autospace/Ahem.ttf"
));

/// The left, top, width and height of the elements of `body` that have an
/// `id`, on the first page.
fn boxes(body: &str) -> Vec<(String, [f32; 4])> {
    let html = format!(
        "<!DOCTYPE html><style>@page {{ size: 800px 600px; margin: 0 }} \
         html, body {{ margin: 0 }} body {{ font: 10px/10px Ahem }}</style>{body}"
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

/// Layout units are 1/64px; allow a little more for the 32-bit sums.
const TOLERANCE: f32 = 0.04;

#[test]
fn collapsed_fixed_columns_keep_their_share_of_the_natural_width() {
    let cases: &[(&str, &str, &[usize], bool, Expected)] = &[
        (
            "collapsed",
            "",
            &[1],
            false,
            &[
                ("t", 0.0, 0.0, 136.671_88, 30.0),
                ("a", 10.0, 10.0, 40.0, 10.0),
                ("b", 50.0, 10.0, 0.0, 10.0),
                ("c", 60.0, 10.0, 66.671875, 10.0),
            ],
        ),
        (
            "shown",
            "",
            &[],
            false,
            &[
                ("t", 0.0, 0.0, 200.0, 30.0),
                ("a", 10.0, 10.0, 40.0, 10.0),
                ("b", 60.0, 10.0, 53.328125, 10.0),
                ("c", 123.328125, 10.0, 66.671875, 10.0),
            ],
        ),
        (
            "minimum",
            "min-width:300px",
            &[1],
            false,
            &[
                ("t", 0.0, 0.0, 203.34375, 30.0),
                ("a", 10.0, 10.0, 65.0, 10.0),
                ("b", 75.0, 10.0, 0.0, 10.0),
                ("c", 85.0, 10.0, 108.34375, 10.0),
            ],
        ),
        (
            "maximum",
            "max-width:100px",
            &[1],
            false,
            &[
                ("t", 0.0, 0.0, 110.0, 30.0),
                ("a", 10.0, 10.0, 30.0, 10.0),
                ("b", 40.0, 10.0, 0.0, 10.0),
                ("c", 50.0, 10.0, 50.0, 10.0),
            ],
        ),
        (
            "all collapsed",
            "",
            &[0, 1, 2],
            false,
            &[
                ("t", 0.0, 0.0, 20.0, 30.0),
                ("a", 10.0, 10.0, 0.0, 10.0),
                ("b", 10.0, 10.0, 0.0, 10.0),
                ("c", 10.0, 10.0, 0.0, 10.0),
            ],
        ),
        (
            "caption",
            "",
            &[1],
            true,
            &[
                // Table fragments expose the grid alone, while Chromium's
                // div-table BCR includes the caption. Check its wrapper too.
                ("w", 0.0, 0.0, 300.0, 40.0),
                ("t", 0.0, 10.0, 203.34375, 30.0),
                ("cap", 0.0, 0.0, 300.0, 10.0),
                ("a", 10.0, 20.0, 65.0, 10.0),
                ("b", 75.0, 20.0, 0.0, 10.0),
                ("c", 85.0, 20.0, 108.34375, 10.0),
            ],
        ),
    ];
    let mut mismatches = Vec::new();
    for &(label, style, collapsed, caption, expected) in cases {
        let columns: String = (0..3)
            .map(|column| {
                format!(
                    "<div style='display:table-column;{}'></div>",
                    if collapsed.contains(&column) {
                        "visibility:collapse"
                    } else {
                        ""
                    }
                )
            })
            .collect();
        let cells: String = [30, 40, 50]
            .into_iter()
            .zip(['a', 'b', 'c'])
            .map(|(width, id)| {
                format!("<div id={id} style='display:table-cell;padding:0;width:{width}px'>x</div>")
            })
            .collect();
        let caption = if caption {
            "<div id=cap style='display:table-caption;min-width:300px'>x</div>"
        } else {
            ""
        };
        let markup = format!(
            "<div id=t style='display:table;table-layout:fixed;width:200px;border-spacing:10px;{style}'>{caption}{columns}<div style='display:table-row'>{cells}</div></div>"
        );
        let markup = if caption.is_empty() {
            markup
        } else {
            format!("<div id=w style='display:inline-block'>{markup}</div>")
        };
        check_boxes(&mut mismatches, label, &markup, expected);
    }
    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
}

#[test]
fn a_zero_width_auto_column_keeps_its_cell_at_the_column_origin() {
    let mut mismatches = Vec::new();
    check_boxes(
        &mut mismatches,
        "zero-width auto column",
        "<style>td{padding:5px;border:1px solid}</style>\
         <table id=t style='font:inherit;table-layout:fixed;width:150px;border-spacing:2px'>\
         <tr><td id=a style='width:100px'>Row 1 Left</td>\
         <td id=b style='width:100px'>y</td><td id=c>x</td></tr></table>",
        &[
            ("t", 0.0, 0.0, 232.0, 26.0),
            ("a", 2.0, 2.0, 112.0, 22.0),
            ("b", 116.0, 2.0, 112.0, 22.0),
            ("c", 230.0, 2.0, 0.0, 22.0),
        ],
    );
    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
}

#[test]
fn fixed_collapse_keeps_intrinsic_minimum_keywords_from_measuring_the_content() {
    let mut mismatches = Vec::new();
    for keyword in ["min-content", "max-content", "fit-content"] {
        let cells: String = [30,40,50].into_iter().zip(['a','b','c']).map(|(width,id)| format!("<div id={id} style='display:table-cell;padding:0;width:{width}px'>xxxxxxxxxxxxxxxxxxxx</div>")).collect();
        let markup = format!(
            "<div id=t style='display:table;table-layout:fixed;width:200px;border-spacing:10px;min-width:{keyword}'><div style='display:table-column'></div><div style='display:table-column;visibility:collapse'></div><div style='display:table-column'></div><div style='display:table-row'>{cells}</div></div>"
        );
        check_boxes(
            &mut mismatches,
            keyword,
            &markup,
            &[
                ("t", 0.0, 0.0, 136.671_88, 30.0),
                ("a", 10.0, 10.0, 40.0, 10.0),
                ("b", 50.0, 10.0, 0.0, 10.0),
                ("c", 60.0, 10.0, 66.671875, 10.0),
            ],
        );
    }
    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
}

#[test]
fn narrow_columns_and_single_cell_widths_match_chromium() {
    let cases: &[(&str, u32, &str, &str, Expected)] = &[
        (
            "narrow",
            240,
            "",
            "width:100px",
            &[
                ("t", 0.0, 0.0, 240.0, 26.0),
                ("a", 2.0, 2.0, 112.0, 22.0),
                ("b", 116.0, 2.0, 112.0, 22.0),
                ("c", 230.0, 2.0, 8.0, 22.0),
            ],
        ),
        (
            "ordinary",
            250,
            "",
            "width:100px",
            &[
                ("t", 0.0, 0.0, 250.0, 26.0),
                ("a", 2.0, 2.0, 112.0, 22.0),
                ("b", 116.0, 2.0, 112.0, 22.0),
                ("c", 230.0, 2.0, 18.0, 22.0),
            ],
        ),
        (
            "border box",
            400,
            "",
            "width:100px;box-sizing:border-box",
            &[
                ("t", 0.0, 0.0, 400.0, 36.0),
                ("a", 2.0, 2.0, 100.0, 32.0),
                ("b", 104.0, 2.0, 112.0, 32.0),
                ("c", 218.0, 2.0, 180.0, 32.0),
            ],
        ),
        (
            "column priority",
            400,
            "<col style='width:50px'><col><col>",
            "width:100px",
            &[
                ("t", 0.0, 0.0, 400.0, 46.0),
                ("a", 2.0, 2.0, 50.0, 42.0),
                ("b", 54.0, 2.0, 112.0, 42.0),
                ("c", 168.0, 2.0, 230.0, 42.0),
            ],
        ),
    ];
    let mut mismatches = Vec::new();
    for &(label, width, columns, first, expected) in cases {
        let markup = format!(
            "<style>td{{padding:5px;border:1px solid}}</style>\
            <table id=t style='font:inherit;table-layout:fixed;width:{width}px;border-spacing:2px'>{columns}\
            <tr><td id=a style='{first}'>Row 1 Left</td><td id=b style='width:100px'>y</td><td id=c>x</td></tr></table>"
        );
        check_boxes(&mut mismatches, label, &markup, expected);
    }
    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
}

#[test]
fn first_row_spanning_widths_contribute_to_unfixed_columns() {
    let cases: &[(&str, &str, &str, Expected)] = &[
        (
            "length",
            "",
            "width:100px",
            &[
                ("a", 2.0, 2.0, 112.0, 22.0),
                ("c", 116.0, 2.0, 182.0, 22.0),
                ("d", 2.0, 26.0, 55.0, 22.0),
                ("e", 59.0, 26.0, 55.0, 22.0),
                ("f", 116.0, 26.0, 182.0, 22.0),
            ],
        ),
        (
            "narrow column wins",
            "<col style='width:20px'><col><col>",
            "width:100px",
            &[
                ("a", 2.0, 2.0, 77.0, 32.0),
                ("c", 81.0, 2.0, 217.0, 32.0),
                ("d", 2.0, 36.0, 20.0, 22.0),
                ("e", 24.0, 36.0, 55.0, 22.0),
                ("f", 81.0, 36.0, 217.0, 22.0),
            ],
        ),
        (
            "wide column wins",
            "<col style='width:150px'><col><col>",
            "width:100px",
            &[
                ("a", 2.0, 2.0, 207.0, 22.0),
                ("c", 211.0, 2.0, 87.0, 22.0),
                ("d", 2.0, 26.0, 150.0, 22.0),
                ("e", 154.0, 26.0, 55.0, 22.0),
                ("f", 211.0, 26.0, 87.0, 22.0),
            ],
        ),
        (
            "percentage",
            "",
            "width:50%",
            &[
                ("a", 2.0, 2.0, 148.0, 22.0),
                ("c", 152.0, 2.0, 146.0, 22.0),
                ("d", 2.0, 26.0, 73.0, 22.0),
                ("e", 77.0, 26.0, 73.0, 22.0),
                ("f", 152.0, 26.0, 146.0, 22.0),
            ],
        ),
        (
            "border box",
            "",
            "box-sizing:border-box;width:100px",
            &[
                ("a", 2.0, 2.0, 100.0, 32.0),
                ("c", 104.0, 2.0, 194.0, 32.0),
                ("d", 2.0, 36.0, 49.0, 22.0),
                ("e", 53.0, 36.0, 49.0, 22.0),
                ("f", 104.0, 36.0, 194.0, 22.0),
            ],
        ),
    ];
    let mut mismatches = Vec::new();
    for &(label, columns, style, expected) in cases {
        let html = format!(
            "<style>td{{padding:5px;border:1px solid}}</style><table style='font:inherit;table-layout:fixed;width:300px;border-spacing:2px'>{columns}<tr><td id=a colspan=2 style='{style}'>Row 1 Left</td><td id=c>x</td></tr><tr><td id=d>1</td><td id=e>2</td><td id=f>3</td></tr></table>"
        );
        check_boxes(&mut mismatches, label, &html, expected);
    }
    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
}

type Expected = &'static [(&'static str, f32, f32, f32, f32)];

#[test]
fn a_span_that_fixes_the_last_auto_columns_shares_the_surplus_proportionally() {
    let mut mismatches = Vec::new();
    check_boxes(
        &mut mismatches,
        "no automatic columns",
        "<table style='font:inherit;table-layout:fixed;width:300px;border-spacing:0'>\
         <tr><td id=a colspan=2 style='width:100px;padding:0;border:0'>x</td>\
         <td id=c style='width:100px;padding:0;border:0'>x</td></tr>\
         <tr><td id=d style='padding:0;border:0'>x</td>\
         <td id=e style='padding:0;border:0'>x</td>\
         <td id=f style='padding:0;border:0'>x</td></tr></table>",
        &[
            ("a", 0.0, 0.0, 150.0, 10.0),
            ("c", 150.0, 0.0, 150.0, 10.0),
            ("d", 0.0, 10.0, 75.0, 10.0),
            ("e", 75.0, 10.0, 75.0, 10.0),
            ("f", 150.0, 10.0, 150.0, 10.0),
        ],
    );
    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
}

/// Lays out `html` and appends to `mismatches` each element of `expected`
/// whose box differs.
fn check_boxes(
    mismatches: &mut Vec<String>,
    label: &str,
    html: &str,
    expected: &[(&str, f32, f32, f32, f32)],
) {
    let actual = boxes(html);
    for &(id, left, top, width, height) in expected {
        let (_, rect) = actual
            .iter()
            .find(|(name, _)| name == id)
            .unwrap_or_else(|| panic!("{label}: no fragment for #{id}"));
        let want = [left, top, width, height];
        if !rect
            .iter()
            .zip(want)
            .all(|(got, want)| (got - want).abs() <= TOLERANCE)
        {
            mismatches.push(format!(
                "{label}: #{id} is {rect:?}, expected {want:?} (left, top, width, height)"
            ));
        }
    }
}

#[test]
fn fixed_percentages_and_intrinsic_limits_match_chromium() {
    let oracle: serde_json::Value =
        serde_json::from_str(include_str!("data/fixed-table-collapse-intrinsic.json"))
            .expect("Chromium measurements");
    let mut mismatches = Vec::new();
    for case in oracle["cases"].as_array().expect("cases") {
        let label = case["label"].as_str().expect("label");
        let limit = case["limit"].as_str().expect("limit");
        let edges = case["edges"].as_str().expect("edges");
        let column = case["column"].as_str().expect("column");
        let visibility = if case["collapse"].as_bool().expect("collapse") {
            "visibility:collapse"
        } else {
            ""
        };
        let text = case["text"].as_str().expect("text");
        let cells: String = case["widths"].as_array().expect("widths").iter().zip(['a','b','c']).map(|(width,id)| {
            let width = width.as_str().expect("width");
            format!("<div id={id} style='display:table-cell;padding:0;width:{width};{edges}'>{text}</div>")
        }).collect();
        let markup = format!(
            "<div id=t style='display:table;table-layout:fixed;width:200px;border-spacing:10px;{limit}'><div style='display:table-column'></div><div style='display:table-column;{visibility};width:{column}'></div><div style='display:table-column'></div><div style='display:table-row'>{cells}</div></div>"
        );
        let expected: Vec<_> = case["expected"]
            .as_array()
            .expect("expected")
            .iter()
            .map(|rect| {
                (
                    rect[0].as_str().expect("id"),
                    rect[1].as_f64().expect("x") as f32,
                    rect[2].as_f64().expect("y") as f32,
                    rect[3].as_f64().expect("width") as f32,
                    rect[4].as_f64().expect("height") as f32,
                )
            })
            .collect();
        check_boxes(&mut mismatches, label, &markup, &expected);
    }
    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
}

#[test]
fn a_first_row_percentage_span_keeps_its_origin_in_intrinsic_sizing() {
    let markup = "<table id=t style='font:inherit;table-layout:fixed;width:200px;border-spacing:10px;min-width:max-content'>\
        <col style='width:30px'><col style='visibility:collapse'><col>\
        <tr><td id=a colspan=2 style='font:inherit;padding:0;width:60%'>xxxxxxxxxxxxxxxxxxxx</td>\
        <td id=c style='font:inherit;padding:0;width:50px'>xxxxxxxxxxxxxxxxxxxx</td></tr>\
        <tr><td id=d style='padding:0'>x</td><td id=e style='padding:0'>x</td><td id=f style='padding:0'>x</td></tr></table>";
    let mut mismatches = Vec::new();
    check_boxes(
        &mut mismatches,
        "percentage span",
        markup,
        &[
            ("t", 0.0, 0.0, 251.656_25, 50.0),
            ("a", 10.0, 10.0, 83.125, 10.0),
            ("c", 103.125, 10.0, 138.531_25, 10.0),
            ("d", 10.0, 30.0, 83.125, 10.0),
            ("e", 93.125, 30.0, 0.0, 10.0),
            ("f", 103.125, 30.0, 138.531_25, 10.0),
        ],
    );
    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
}

/// A 300px wide block holding the table `t` in flow, with the fixed layout
/// algorithm and `style`.
fn in_flow(style: &str, rows: &str) -> String {
    format!(
        "<div style='width: 300px'><div id=t style='display: table; table-layout: fixed; \
         {style}'>{rows}</div></div>"
    )
}

/// The same table, absolutely positioned in a 300px wide box.
fn absolute(style: &str, rows: &str) -> String {
    format!(
        "<div style='position: relative; width: 300px; height: 50px'><div id=t \
         style='display: table; table-layout: fixed; position: absolute; top: 0; left: 0; \
         {style}'>{rows}</div></div>"
    )
}

/// The same table as an item of a 300px wide flex or grid container.
fn in_container(display: &str, style: &str, rows: &str) -> String {
    format!(
        "<div style='display: {display}; width: 300px'><div id=t style='display: table; \
         table-layout: fixed; {style}'>{rows}</div></div>"
    )
}

const ONE_CELL: &str = "<div style='display: table-row'><div id=d style='display: table-cell'>\
                        <div style='width: 200px; height: 10px'></div></div></div>";

const TWO_CELLS: &str = "<div style='display: table-row'><div id=d style='display: table-cell'>\
                         <div style='width: 200px; height: 10px'></div></div>\
                         <div id=e style='display: table-cell'>\
                         <div style='width: 200px; height: 10px'></div></div></div>";

const FIRST_CELL_20: &str = "<div style='display: table-row'><div id=d \
                             style='display: table-cell; width: 20px'>\
                             <div style='width: 200px; height: 10px'></div></div>\
                             <div id=e style='display: table-cell'>\
                             <div style='width: 200px; height: 10px'></div></div></div>";

/// A label, the table's style, its rows and the boxes it comes to.
const CASES: &[(&str, &str, &str, Expected)] = &[
    (
        "a max-width under the width",
        "width: 100px; max-width: 50px",
        ONE_CELL,
        &[("t", 0.0, 0.0, 50.0, 10.0), ("d", 0.0, 0.0, 50.0, 10.0)],
    ),
    (
        "with a border",
        "width: 100px; max-width: 50px; border: 2px solid #00f",
        ONE_CELL,
        &[("t", 0.0, 0.0, 54.0, 14.0), ("d", 2.0, 2.0, 50.0, 10.0)],
    ),
    (
        "two columns share the room that is left",
        "width: 100px; max-width: 50px",
        TWO_CELLS,
        &[
            ("t", 0.0, 0.0, 50.0, 10.0),
            ("d", 0.0, 0.0, 25.0, 10.0),
            ("e", 25.0, 0.0, 25.0, 10.0),
        ],
    ),
    (
        "a min-width over the width",
        "width: 100px; min-width: 150px",
        TWO_CELLS,
        &[
            ("t", 0.0, 0.0, 150.0, 10.0),
            ("d", 0.0, 0.0, 75.0, 10.0),
            ("e", 75.0, 0.0, 75.0, 10.0),
        ],
    ),
    (
        "a max-width far under the width",
        "width: 40px; max-width: 5px",
        ONE_CELL,
        &[("t", 0.0, 0.0, 5.0, 10.0), ("d", 0.0, 0.0, 5.0, 10.0)],
    ),
    (
        "border-box with padding and a border",
        "width: 100px; max-width: 50px; box-sizing: border-box; border: 2px solid #00f; \
         padding: 3px",
        ONE_CELL,
        &[("t", 0.0, 0.0, 50.0, 20.0), ("d", 5.0, 5.0, 40.0, 10.0)],
    ),
    (
        "a percentage max-width over the width",
        "width: 100px; max-width: 50%",
        ONE_CELL,
        &[("t", 0.0, 0.0, 100.0, 10.0), ("d", 0.0, 0.0, 100.0, 10.0)],
    ),
    (
        "a max-width over the width",
        "width: 100px; max-width: 120px",
        TWO_CELLS,
        &[
            ("t", 0.0, 0.0, 100.0, 10.0),
            ("d", 0.0, 0.0, 50.0, 10.0),
            ("e", 50.0, 0.0, 50.0, 10.0),
        ],
    ),
    (
        "a first cell with a width",
        "width: 100px; max-width: 50px",
        FIRST_CELL_20,
        &[
            ("t", 0.0, 0.0, 50.0, 10.0),
            ("d", 0.0, 0.0, 20.0, 10.0),
            ("e", 20.0, 0.0, 30.0, 10.0),
        ],
    ),
    (
        "border spacing",
        "width: 100px; max-width: 50px; border-spacing: 4px",
        TWO_CELLS,
        &[
            ("t", 0.0, 0.0, 50.0, 18.0),
            ("d", 4.0, 4.0, 19.0, 10.0),
            ("e", 27.0, 4.0, 19.0, 10.0),
        ],
    ),
    // A content-box table states the width of its content: the border and
    // padding come on top of it, and the columns fill the whole of it.
    (
        "a content-box width with a border",
        "width: 100px; border: 2px solid #00f",
        ONE_CELL,
        &[("t", 0.0, 0.0, 104.0, 14.0), ("d", 2.0, 2.0, 100.0, 10.0)],
    ),
    (
        "a max-width between the width and the border",
        "width: 100px; max-width: 98px; border: 2px solid #00f",
        ONE_CELL,
        &[("t", 0.0, 0.0, 102.0, 14.0), ("d", 2.0, 2.0, 98.0, 10.0)],
    ),
    (
        "a max-width over the width and the border",
        "width: 100px; max-width: 101px; border: 2px solid #00f",
        ONE_CELL,
        &[("t", 0.0, 0.0, 104.0, 14.0), ("d", 2.0, 2.0, 100.0, 10.0)],
    ),
    (
        "two columns with padding and a border",
        "width: 100px; padding: 3px; border: 2px solid #00f",
        TWO_CELLS,
        &[
            ("t", 0.0, 0.0, 110.0, 20.0),
            ("d", 5.0, 5.0, 50.0, 10.0),
            ("e", 55.0, 5.0, 50.0, 10.0),
        ],
    ),
    (
        "a max-width with padding and a border",
        "width: 100px; padding: 3px; max-width: 60px; border: 2px solid #00f",
        TWO_CELLS,
        &[
            ("t", 0.0, 0.0, 70.0, 20.0),
            ("d", 5.0, 5.0, 30.0, 10.0),
            ("e", 35.0, 5.0, 30.0, 10.0),
        ],
    ),
    (
        "a percentage width with a border",
        "width: 50%; border: 2px solid #00f",
        TWO_CELLS,
        &[
            ("t", 0.0, 0.0, 154.0, 14.0),
            ("d", 2.0, 2.0, 75.0, 10.0),
            ("e", 77.0, 2.0, 75.0, 10.0),
        ],
    ),
    (
        "a percentage max-width with a border",
        "width: 50%; max-width: 40%; border: 2px solid #00f",
        TWO_CELLS,
        &[
            ("t", 0.0, 0.0, 124.0, 14.0),
            ("d", 2.0, 2.0, 60.0, 10.0),
            ("e", 62.0, 2.0, 60.0, 10.0),
        ],
    ),
    (
        "a calc width with a border",
        "width: calc(100px + 10px); border: 2px solid #00f",
        TWO_CELLS,
        &[
            ("t", 0.0, 0.0, 114.0, 14.0),
            ("d", 2.0, 2.0, 55.0, 10.0),
            ("e", 57.0, 2.0, 55.0, 10.0),
        ],
    ),
    (
        "border spacing with a border",
        "width: 100px; border-spacing: 6px; border: 2px solid #00f",
        TWO_CELLS,
        &[
            ("t", 0.0, 0.0, 104.0, 26.0),
            ("d", 8.0, 8.0, 41.0, 10.0),
            ("e", 55.0, 8.0, 41.0, 10.0),
        ],
    ),
    (
        "a stretched width under a max-width",
        "width: stretch; max-width: 100px; border: 2px solid #00f",
        TWO_CELLS,
        &[
            ("t", 0.0, 0.0, 104.0, 14.0),
            ("d", 2.0, 2.0, 50.0, 10.0),
            ("e", 52.0, 2.0, 50.0, 10.0),
        ],
    ),
    (
        "a border-box max-width with a border",
        "width: 100px; max-width: 98px; box-sizing: border-box; border: 2px solid #00f",
        ONE_CELL,
        &[("t", 0.0, 0.0, 98.0, 14.0), ("d", 2.0, 2.0, 94.0, 10.0)],
    ),
];

#[test]
fn a_fixed_table_in_flow_follows_its_max_width() {
    let mut mismatches = Vec::new();
    for &(label, style, rows, expected) in CASES {
        check_boxes(&mut mismatches, label, &in_flow(style, rows), expected);
    }
    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
}

#[test]
fn a_fixed_table_out_of_flow_follows_its_max_width() {
    let mut mismatches = Vec::new();
    for &(label, style, rows, expected) in CASES {
        check_boxes(&mut mismatches, label, &absolute(style, rows), expected);
    }
    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
}

#[test]
fn a_fixed_table_in_a_flex_or_grid_container_follows_its_max_width() {
    let cases: [(&str, &str, Expected); 3] = [
        (
            "a max-width under the width",
            "width: 100px; max-width: 50px",
            &[
                ("t", 0.0, 0.0, 50.0, 10.0),
                ("d", 0.0, 0.0, 25.0, 10.0),
                ("e", 25.0, 0.0, 25.0, 10.0),
            ],
        ),
        (
            "a content-box width with a border",
            "width: 100px; border: 2px solid #00f",
            &[
                ("t", 0.0, 0.0, 104.0, 14.0),
                ("d", 2.0, 2.0, 50.0, 10.0),
                ("e", 52.0, 2.0, 50.0, 10.0),
            ],
        ),
        (
            "a max-width with a border",
            "width: 100px; max-width: 60px; border: 2px solid #00f",
            &[
                ("t", 0.0, 0.0, 64.0, 14.0),
                ("d", 2.0, 2.0, 30.0, 10.0),
                ("e", 32.0, 2.0, 30.0, 10.0),
            ],
        ),
    ];
    let mut mismatches = Vec::new();
    for display in ["flex", "grid"] {
        for (label, style, expected) in cases {
            let label = format!("{display}: {label}");
            check_boxes(
                &mut mismatches,
                &label,
                &in_container(display, style, TWO_CELLS),
                expected,
            );
        }
    }
    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
}

#[test]
fn inline_tables_lift_and_constrain_their_authored_width() {
    let vertical_rows = TWO_CELLS.replace("200px", "100px");
    let extra: &[(&str, &str, &str, Expected)] = &[
        (
            "auto layout keeps its intrinsic floor",
            "table-layout:auto;width:100px;max-width:50px;border:2px solid",
            TWO_CELLS,
            &[
                ("t", 0.0, 0.0, 404.0, 14.0),
                ("d", 2.0, 2.0, 200.0, 10.0),
                ("e", 202.0, 2.0, 200.0, 10.0),
            ],
        ),
        (
            "vertical writing keeps its physical width floor",
            "width:100px;max-width:50px;writing-mode:vertical-rl",
            &vertical_rows,
            &[
                ("t", 0.0, 0.0, 100.0, 20.0),
                ("d", 0.0, 0.0, 100.0, 10.0),
                ("e", 0.0, 10.0, 100.0, 10.0),
            ],
        ),
        (
            "min wins over max",
            "width:100px;min-width:150px;max-width:50px",
            TWO_CELLS,
            &[
                ("t", 0.0, 0.0, 150.0, 10.0),
                ("d", 0.0, 0.0, 75.0, 10.0),
                ("e", 75.0, 0.0, 75.0, 10.0),
            ],
        ),
        (
            "stretch content box",
            "width:stretch;border:2px solid",
            TWO_CELLS,
            &[
                ("t", 0.0, 0.0, 300.0, 14.0),
                ("d", 2.0, 2.0, 148.0, 10.0),
                ("e", 150.0, 2.0, 148.0, 10.0),
            ],
        ),
        (
            "border-box floor",
            "width:100px;max-width:2px;box-sizing:border-box;padding:3px;border:2px solid",
            TWO_CELLS,
            &[
                ("t", 0.0, 0.0, 10.0, 20.0),
                ("d", 5.0, 5.0, 0.0, 10.0),
                ("e", 5.0, 5.0, 0.0, 10.0),
            ],
        ),
    ];
    let mut mismatches = Vec::new();
    for &(label, style, rows, expected) in CASES.iter().chain(extra) {
        let markup = format!(
            "<div style='width:300px'><div id=t style='display:inline-table;table-layout:fixed;{style}'>{rows}</div></div>"
        );
        check_boxes(&mut mismatches, label, &markup, expected);
    }
    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
}

#[test]
fn the_columns_a_table_states_stay_wider_than_a_max_width() {
    // The widths of the columns stay (the table overflows its limit rather
    // than shrinking them), and so does the table, which is never narrower
    // than its columns.
    let cell_80 = "<div style='display: table-row'><div id=d \
                   style='display: table-cell; width: 80px'>\
                   <div style='width: 200px; height: 10px'></div></div></div>";
    let column_80 = format!(
        "<div style='display: table-column-group'><div style='display: table-column; \
         width: 80px'></div></div>{ONE_CELL}"
    );
    let mut mismatches = Vec::new();
    for (label, rows) in [("a cell width", cell_80), ("a column width", &column_80)] {
        check_boxes(
            &mut mismatches,
            label,
            &in_flow("width: 100px; max-width: 50px", rows),
            &[("t", 0.0, 0.0, 80.0, 10.0), ("d", 0.0, 0.0, 80.0, 10.0)],
        );
    }
    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
}
