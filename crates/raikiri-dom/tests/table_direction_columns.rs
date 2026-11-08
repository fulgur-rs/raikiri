//! The columns of a table run along its inline direction. A table whose
//! `direction` is `rtl` puts its first column on the right, and one in a
//! vertical writing mode stacks its columns along the height of the table: from
//! the top for `ltr`, from the bottom for `rtl`. In `sideways-lr` the inline
//! axis itself runs upwards, so `ltr` starts at the bottom there. The rows
//! follow the block direction of the mode, as they always did.
//!
//! The expected boxes are the ones Chromium computes for the same markup in a
//! 300px by 300px viewport with no body margin and a font of 10px Ahem: each is
//! an element's left, top, width and height. Of the tables that collapse their
//! borders only the width of the table and the text of the cells are compared:
//! the box of a cell that collapses borders is a separate matter.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/data/text-autospace/Ahem.ttf"
));

/// A fragment of markup and the boxes Chromium computes for some of its elements.
struct Case {
    label: &'static str,
    markup: &'static str,
    boxes: &'static [(&'static str, [f32; 4])],
}

/// The boxes of the elements with an id in `markup`, on the first page of
/// 300px by 300px.
fn border_boxes(markup: &str) -> Vec<(String, [f32; 4])> {
    let html = format!(
        "<!DOCTYPE html><style>@page {{ size: 300px 300px; margin: 0 }} \
         body {{ margin: 0; font: 10px/10px Ahem }} div {{ box-sizing: border-box }}</style>{markup}"
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

/// Compares the boxes of every case, reporting every mismatch at once.
fn check(cases: &[Case]) {
    let mut mismatches = Vec::new();
    for case in cases {
        let found = border_boxes(case.markup);
        for (id, want) in case.boxes {
            match found.iter().find(|(found_id, _)| found_id == id) {
                None => mismatches.push(format!("{}: no fragment for #{id}", case.label)),
                Some((_, got)) => {
                    if !got
                        .iter()
                        .zip(want)
                        .all(|(g, w)| (g - w).abs() <= TOLERANCE)
                    {
                        mismatches.push(format!(
                            "{}: #{id} is {got:?}, expected {want:?} (left, top, width, height)",
                            case.label
                        ));
                    }
                }
            }
        }
    }
    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
}

/// Tables in `vertical-rl` and `vertical-lr`: an `rtl` table stacks its columns
/// from the bottom, whether the direction comes from the table or its parent.
const VERTICAL: &[Case] = &[
    Case {
        label: "vertical-rl_rtl_two",
        markup: r##"<div id="p" style="writing-mode:vertical-rl;direction:rtl;"><div id="t" style="display:table;"><div id="r0" style="display:table-row"><div id="c00" style="display:table-cell;">aa</div><div id="c01" style="display:table-cell;">b</div></div></div></div>"##,
        boxes: &[
            ("t", [0.0, 0.0, 10.0, 30.0]),
            ("r0", [0.0, 0.0, 10.0, 30.0]),
            ("c00", [0.0, 10.0, 10.0, 20.0]),
            ("c01", [0.0, 0.0, 10.0, 10.0]),
        ],
    },
    Case {
        label: "vertical-lr_rtl_two",
        markup: r##"<div id="p" style="writing-mode:vertical-lr;direction:rtl;"><div id="t" style="display:table;"><div id="r0" style="display:table-row"><div id="c00" style="display:table-cell;">aa</div><div id="c01" style="display:table-cell;">b</div></div></div></div>"##,
        boxes: &[
            ("t", [0.0, 0.0, 10.0, 30.0]),
            ("r0", [0.0, 0.0, 10.0, 30.0]),
            ("c00", [0.0, 10.0, 10.0, 20.0]),
            ("c01", [0.0, 0.0, 10.0, 10.0]),
        ],
    },
    Case {
        label: "vrl_rtl_grid",
        markup: r##"<div id="p" style="writing-mode:vertical-rl;direction:rtl;"><div id="t" style="display:table;border-spacing:3px"><div id="r0" style="display:table-row"><div id="c00" style="display:table-cell;">aaa</div><div id="c01" style="display:table-cell;">b</div><div id="c02" style="display:table-cell;">cc</div></div><div id="r1" style="display:table-row"><div id="c10" style="display:table-cell;">d</div><div id="c11" style="display:table-cell;">eeee</div><div id="c12" style="display:table-cell;">f</div></div></div></div>"##,
        boxes: &[
            ("t", [0.0, 0.0, 29.0, 102.0]),
            ("r0", [16.0, 3.0, 10.0, 96.0]),
            ("c00", [16.0, 69.0, 10.0, 30.0]),
            ("c01", [16.0, 26.0, 10.0, 40.0]),
            ("c02", [16.0, 3.0, 10.0, 20.0]),
            ("r1", [3.0, 3.0, 10.0, 96.0]),
            ("c10", [3.0, 69.0, 10.0, 30.0]),
            ("c11", [3.0, 26.0, 10.0, 40.0]),
            ("c12", [3.0, 3.0, 10.0, 20.0]),
        ],
    },
    Case {
        label: "vrl_rtl_colspan",
        markup: r##"<div id="p" style="writing-mode:vertical-rl;direction:rtl;"><div id="t" style="display:table;border-spacing:2px"><div id="r0" style="display:table-row"><div id="c00" style="display:table-cell;">aaa</div><div id="c01" style="display:table-cell;">b</div></div><div id="r1" style="display:table-row"><div id="c10" style="display:table-cell;colspan:2">xxxxxx;</div></div></div></div>"##,
        boxes: &[
            ("t", [0.0, 0.0, 26.0, 86.0]),
            ("r0", [14.0, 2.0, 10.0, 82.0]),
            ("c00", [14.0, 14.0, 10.0, 70.0]),
            ("c01", [14.0, 2.0, 10.0, 10.0]),
            ("r1", [2.0, 2.0, 10.0, 82.0]),
            ("c10", [2.0, 14.0, 10.0, 70.0]),
        ],
    },
    Case {
        label: "vrl_rtl_tall",
        markup: r##"<div id="p" style="writing-mode:vertical-rl;direction:rtl;"><div id="t" style="display:table;height:100px"><div id="r0" style="display:table-row"><div id="c00" style="display:table-cell;">aa</div><div id="c01" style="display:table-cell;">b</div></div></div></div>"##,
        boxes: &[
            ("t", [0.0, 0.0, 10.0, 100.0]),
            ("r0", [0.0, 0.0, 10.0, 100.0]),
            ("c00", [0.0, 33.34375, 10.0, 66.65625]),
            ("c01", [0.0, 0.0, 10.0, 33.34375]),
        ],
    },
    Case {
        label: "vrl_rtl_pad",
        markup: r##"<div id="p" style="writing-mode:vertical-rl;direction:rtl;"><div id="t" style="display:table;height:100px;padding:5px;border:2px solid;border-spacing:4px"><div id="r0" style="display:table-row"><div id="c00" style="display:table-cell;">aa</div><div id="c01" style="display:table-cell;">b</div></div></div></div>"##,
        boxes: &[
            ("t", [0.0, 0.0, 32.0, 100.0]),
            ("r0", [11.0, 11.0, 10.0, 78.0]),
            ("c00", [11.0, 39.671875, 10.0, 49.328125]),
            ("c01", [11.0, 11.0, 10.0, 24.671875]),
        ],
    },
    Case {
        label: "vrl_tableonly_rtl",
        markup: r##"<div id="p" style="writing-mode:vertical-rl;direction:ltr;"><div id="t" style="display:table;direction:rtl"><div id="r0" style="display:table-row"><div id="c00" style="display:table-cell;">aa</div><div id="c01" style="display:table-cell;">b</div></div></div></div>"##,
        boxes: &[
            ("t", [0.0, 0.0, 10.0, 30.0]),
            ("r0", [0.0, 0.0, 10.0, 30.0]),
            ("c00", [0.0, 10.0, 10.0, 20.0]),
            ("c01", [0.0, 0.0, 10.0, 10.0]),
        ],
    },
    Case {
        label: "vrl_cellonly_rtl",
        markup: r##"<div id="p" style="writing-mode:vertical-rl"><div id="t" style="display:table"><div id="r0" style="display:table-row"><div id="c00" style="display:table-cell;direction:rtl">aa</div><div id="c01" style="display:table-cell;direction:rtl">b</div></div></div></div>"##,
        boxes: &[
            ("t", [0.0, 0.0, 10.0, 30.0]),
            ("r0", [0.0, 0.0, 10.0, 30.0]),
            ("c00", [0.0, 0.0, 10.0, 20.0]),
            ("c01", [0.0, 20.0, 10.0, 10.0]),
        ],
    },
    Case {
        label: "vrl_rtl_realtable",
        markup: r##"<table id="t" style="writing-mode:vertical-rl;direction:rtl;border-spacing:0"><tr id="r0"><td id="c00" style="padding:0">aa</td><td id="c01" style="padding:0">b</td></tr></table>"##,
        boxes: &[
            ("t", [0.0, 0.0, 10.0, 30.0]),
            ("r0", [0.0, 0.0, 10.0, 30.0]),
            ("c00", [0.0, 10.0, 10.0, 20.0]),
            ("c01", [0.0, 0.0, 10.0, 10.0]),
        ],
    },
    Case {
        label: "vert_rtl_span",
        markup: r##"<table id="t" style="writing-mode:vertical-rl;direction:rtl;border-spacing:3px"><tr id="r0"><td id="a" rowspan="2" style="padding:0">aaa</td><td id="b" style="padding:0">b</td><td id="c" style="padding:0">cc</td></tr><tr id="r1"><td id="d" colspan="2" style="padding:0">dddd</td></tr></table>"##,
        boxes: &[
            ("t", [0.0, 0.0, 29.0, 79.0]),
            ("r0", [16.0, 3.0, 10.0, 73.0]),
            ("a", [3.0, 46.0, 23.0, 30.0]),
            ("b", [16.0, 30.671875, 10.0, 12.328125]),
            ("c", [16.0, 3.0, 10.0, 24.671875]),
            ("r1", [3.0, 3.0, 10.0, 73.0]),
            ("d", [3.0, 3.0, 10.0, 40.0]),
        ],
    },
    Case {
        label: "vert_rtl_fixed",
        markup: r##"<table id="t" style="writing-mode:vertical-rl;direction:rtl;table-layout:fixed;width:120px;border-spacing:0"><col style="width:20px"><col><col style="width:50px"><tr id="r0"><td id="a" style="padding:0">a</td><td id="b" style="padding:0">b</td><td id="c" style="padding:0">c</td></tr></table>"##,
        boxes: &[
            ("t", [0.0, 0.0, 120.0, 30.0]),
            ("r0", [0.0, 0.0, 120.0, 30.0]),
            ("a", [0.0, 20.0, 120.0, 10.0]),
            ("b", [0.0, 10.0, 120.0, 10.0]),
            ("c", [0.0, 0.0, 120.0, 10.0]),
        ],
    },
];

#[test]
fn vertical_tables_stack_rtl_columns_from_the_bottom() {
    check(VERTICAL);
}

/// Tables in the horizontal mode: an `rtl` table puts its first column on the
/// right, and its column widths, spans, padding and captions go with it.
const HORIZONTAL: &[Case] = &[
    Case {
        label: "horizontal-tb_rtl_two",
        markup: r##"<div id="p" style="writing-mode:horizontal-tb;direction:rtl;"><div id="t" style="display:table;"><div id="r0" style="display:table-row"><div id="c00" style="display:table-cell;">aa</div><div id="c01" style="display:table-cell;">b</div></div></div></div>"##,
        boxes: &[
            ("t", [270.0, 0.0, 30.0, 10.0]),
            ("r0", [270.0, 0.0, 30.0, 10.0]),
            ("c00", [280.0, 0.0, 20.0, 10.0]),
            ("c01", [270.0, 0.0, 10.0, 10.0]),
        ],
    },
    Case {
        label: "h_tableonly_div",
        markup: r##"<div id="p"><div id="t" style="display:table;direction:rtl"><div id="r0" style="display:table-row"><div id="c00" style="display:table-cell">aa</div><div id="c01" style="display:table-cell">b</div></div></div></div>"##,
        boxes: &[
            ("t", [0.0, 0.0, 30.0, 10.0]),
            ("r0", [0.0, 0.0, 30.0, 10.0]),
            ("c00", [10.0, 0.0, 20.0, 10.0]),
            ("c01", [0.0, 0.0, 10.0, 10.0]),
        ],
    },
    Case {
        label: "h_parent_div",
        markup: r##"<div id="p" style="direction:rtl"><div id="t" style="display:table"><div id="r0" style="display:table-row"><div id="c00" style="display:table-cell">aa</div><div id="c01" style="display:table-cell">b</div></div></div></div>"##,
        boxes: &[
            ("t", [270.0, 0.0, 30.0, 10.0]),
            ("r0", [270.0, 0.0, 30.0, 10.0]),
            ("c00", [280.0, 0.0, 20.0, 10.0]),
            ("c01", [270.0, 0.0, 10.0, 10.0]),
        ],
    },
    Case {
        label: "h_realtable",
        markup: r##"<table id="t" style="direction:rtl;border-spacing:0"><tr id="r0"><td id="c00" style="padding:0">aa</td><td id="c01" style="padding:0">b</td></tr></table>"##,
        boxes: &[
            ("t", [0.0, 0.0, 30.0, 10.0]),
            ("r0", [0.0, 0.0, 30.0, 10.0]),
            ("c00", [10.0, 0.0, 20.0, 10.0]),
            ("c01", [0.0, 0.0, 10.0, 10.0]),
        ],
    },
    Case {
        label: "h_realtable_cols",
        markup: r##"<table id="t" style="direction:rtl;border-spacing:0"><col style="width:50px"><col style="width:100px"><tr id="r0"><td id="c00" style="padding:0">aa</td><td id="c01" style="padding:0">b</td></tr></table>"##,
        boxes: &[
            ("t", [0.0, 0.0, 150.0, 10.0]),
            ("r0", [0.0, 0.0, 150.0, 10.0]),
            ("c00", [100.0, 0.0, 50.0, 10.0]),
            ("c01", [0.0, 0.0, 100.0, 10.0]),
        ],
    },
    Case {
        label: "hori_rtl_fixed",
        markup: r##"<table id="t" style="writing-mode:horizontal-tb;direction:rtl;table-layout:fixed;width:120px;border-spacing:0"><col style="width:20px"><col><col style="width:50px"><tr id="r0"><td id="a" style="padding:0">a</td><td id="b" style="padding:0">b</td><td id="c" style="padding:0">c</td></tr></table>"##,
        boxes: &[
            ("t", [0.0, 0.0, 120.0, 10.0]),
            ("r0", [0.0, 0.0, 120.0, 10.0]),
            ("a", [100.0, 0.0, 20.0, 10.0]),
            ("b", [50.0, 0.0, 50.0, 10.0]),
            ("c", [0.0, 0.0, 50.0, 10.0]),
        ],
    },
    Case {
        label: "hori_rtl_edges",
        markup: r##"<table id="t" style="writing-mode:horizontal-tb;direction:rtl;border-left:10px solid;border-right:2px solid;border-top:4px solid;border-bottom:6px solid;padding:3px 5px 7px 1px;border-spacing:2px 4px"><tr id="r0"><td id="a"  style="padding:0;">aa</td><td id="b"  style="padding:0;">b</td></tr></table>"##,
        boxes: &[
            ("t", [0.0, 0.0, 54.0, 38.0]),
            ("r0", [13.0, 11.0, 32.0, 10.0]),
            ("a", [25.0, 11.0, 20.0, 10.0]),
            ("b", [13.0, 11.0, 10.0, 10.0]),
        ],
    },
    Case {
        label: "hori_rtl_groups",
        markup: r##"<table id="t" style="writing-mode:horizontal-tb;direction:rtl;border-spacing:2px"><thead id="g1"><tr id="r0"><td id="a" style="padding:0">aa</td><td id="b" style="padding:0">b</td></tr></thead><tbody id="g2"><tr id="r1"><td id="c" style="padding:0">c</td><td id="d" style="padding:0">dddd</td></tr></tbody></table>"##,
        boxes: &[
            ("t", [0.0, 0.0, 66.0, 26.0]),
            ("g1", [2.0, 2.0, 62.0, 10.0]),
            ("r0", [2.0, 2.0, 62.0, 10.0]),
            ("a", [44.0, 2.0, 20.0, 10.0]),
            ("b", [2.0, 2.0, 40.0, 10.0]),
            ("g2", [2.0, 14.0, 62.0, 10.0]),
            ("r1", [2.0, 14.0, 62.0, 10.0]),
            ("c", [44.0, 14.0, 20.0, 10.0]),
            ("d", [2.0, 14.0, 40.0, 10.0]),
        ],
    },
    Case {
        label: "hori_rtl_text",
        markup: r##"<table id="t" style="writing-mode:horizontal-tb;direction:rtl;border-spacing:0"><tr id="r0"><td id="a" style="padding:0;width:60px"><span id="s1">a</span></td><td id="b" style="padding:0;width:40px"><span id="s2">b</span></td></tr></table>"##,
        boxes: &[
            ("t", [0.0, 0.0, 100.0, 10.0]),
            ("r0", [0.0, 0.0, 100.0, 10.0]),
            ("a", [40.0, 0.0, 60.0, 10.0]),
            ("s1", [90.0, 0.0, 10.0, 10.0]),
            ("b", [0.0, 0.0, 40.0, 10.0]),
            ("s2", [30.0, 0.0, 10.0, 10.0]),
        ],
    },
    Case {
        label: "hori_rtl_nested",
        markup: r##"<table id="t" style="writing-mode:horizontal-tb;direction:rtl;border-spacing:0"><tr id="r0"><td id="a" style="padding:0"><table id="n" style="border-spacing:0"><tr><td id="x" style="padding:0">xx</td><td id="y" style="padding:0">y</td></tr></table></td><td id="b" style="padding:0">bbbb</td></tr></table>"##,
        boxes: &[
            ("t", [0.0, 0.0, 70.0, 10.0]),
            ("r0", [0.0, 0.0, 70.0, 10.0]),
            ("a", [40.0, 0.0, 30.0, 10.0]),
            ("n", [40.0, 0.0, 30.0, 10.0]),
            ("x", [50.0, 0.0, 20.0, 10.0]),
            ("y", [40.0, 0.0, 10.0, 10.0]),
            ("b", [0.0, 0.0, 40.0, 10.0]),
        ],
    },
    Case {
        label: "hori_rtl_caption",
        markup: r##"<table id="t" style="writing-mode:horizontal-tb;direction:rtl;border-spacing:2px"><caption id="cap">capcap</caption><tr id="r0"><td id="a" style="padding:0">aa</td><td id="b" style="padding:0">b</td></tr></table>"##,
        boxes: &[
            ("cap", [0.0, 0.0, 60.0, 10.0]),
            ("r0", [2.0, 12.0, 56.0, 10.0]),
            ("a", [22.0, 12.0, 36.0, 10.0]),
            ("b", [2.0, 12.0, 18.0, 10.0]),
        ],
    },
];

#[test]
fn horizontal_tables_put_the_first_rtl_column_on_the_right() {
    check(HORIZONTAL);
}

/// `sideways-lr` and `sideways-rl` run the inline axis along the height as well,
/// but `sideways-lr` runs it upwards.
const SIDEWAYS: &[Case] = &[
    Case {
        label: "slr_ltr_two",
        markup: r##"<table id="t" style="writing-mode:sideways-lr;direction:ltr;border-spacing:2px"><tr id="r0"><td id="a"  style="padding:0;">aa</td><td id="b"  style="padding:0;">b</td></tr></table>"##,
        boxes: &[
            ("t", [0.0, 0.0, 14.0, 36.0]),
            ("r0", [2.0, 2.0, 10.0, 32.0]),
            ("a", [2.0, 14.0, 10.0, 20.0]),
            ("b", [2.0, 2.0, 10.0, 10.0]),
        ],
    },
    Case {
        label: "srl_ltr_two",
        markup: r##"<table id="t" style="writing-mode:sideways-rl;direction:ltr;border-spacing:2px"><tr id="r0"><td id="a"  style="padding:0;">aa</td><td id="b"  style="padding:0;">b</td></tr></table>"##,
        boxes: &[
            ("t", [0.0, 0.0, 14.0, 36.0]),
            ("r0", [2.0, 2.0, 10.0, 32.0]),
            ("a", [2.0, 2.0, 10.0, 20.0]),
            ("b", [2.0, 24.0, 10.0, 10.0]),
        ],
    },
    Case {
        label: "slr_rtl_two",
        markup: r##"<table id="t" style="writing-mode:sideways-lr;direction:rtl;border-spacing:2px"><tr id="r0"><td id="a"  style="padding:0;">aa</td><td id="b"  style="padding:0;">b</td></tr></table>"##,
        boxes: &[
            ("t", [0.0, 0.0, 14.0, 36.0]),
            ("r0", [2.0, 2.0, 10.0, 32.0]),
            ("a", [2.0, 2.0, 10.0, 20.0]),
            ("b", [2.0, 24.0, 10.0, 10.0]),
        ],
    },
    Case {
        label: "srl_rtl_two",
        markup: r##"<table id="t" style="writing-mode:sideways-rl;direction:rtl;border-spacing:2px"><tr id="r0"><td id="a"  style="padding:0;">aa</td><td id="b"  style="padding:0;">b</td></tr></table>"##,
        boxes: &[
            ("t", [0.0, 0.0, 14.0, 36.0]),
            ("r0", [2.0, 2.0, 10.0, 32.0]),
            ("a", [2.0, 14.0, 10.0, 20.0]),
            ("b", [2.0, 2.0, 10.0, 10.0]),
        ],
    },
];

#[test]
fn sideways_tables_follow_the_inline_axis() {
    check(SIDEWAYS);
}

/// Tables that collapse their borders in the `rtl` direction: the border on the
/// left of a cell is the one next to the column that follows it, and the tables
/// are as wide as Chromium makes them.
const COLLAPSED: &[Case] = &[
    Case {
        label: "h_rtl_sym",
        markup: r##"<table id="t" style="direction:rtl;border-collapse:collapse;"><tr id="r0"><td id="a"  style="padding:0;border:2px solid"><span id="sa">aa</span></td><td id="b"  style="padding:0;border:2px solid"><span id="sb">b</span></td></tr></table>"##,
        boxes: &[
            ("t", [0.0, 0.0, 36.0, 14.0]),
            ("sa", [14.0, 2.0, 20.0, 10.0]),
            ("sb", [2.0, 2.0, 10.0, 10.0]),
        ],
    },
    Case {
        label: "h_rtl_cellL",
        markup: r##"<table id="t" style="direction:rtl;border-collapse:collapse;"><tr id="r0"><td id="a"  style="padding:0;border-left:6px solid"><span id="sa">aa</span></td><td id="b"  style="padding:0;"><span id="sb">b</span></td></tr></table>"##,
        boxes: &[
            ("t", [0.0, 0.0, 36.0, 10.0]),
            ("sa", [16.0, 0.0, 20.0, 10.0]),
            ("sb", [0.0, 0.0, 10.0, 10.0]),
        ],
    },
    Case {
        label: "h_rtl_cellR",
        markup: r##"<table id="t" style="direction:rtl;border-collapse:collapse;"><tr id="r0"><td id="a"  style="padding:0;"><span id="sa">aa</span></td><td id="b"  style="padding:0;border-right:6px solid"><span id="sb">b</span></td></tr></table>"##,
        boxes: &[
            ("t", [0.0, 0.0, 36.0, 10.0]),
            ("sa", [16.0, 0.0, 20.0, 10.0]),
            ("sb", [0.0, 0.0, 10.0, 10.0]),
        ],
    },
    Case {
        label: "h_rtl_mid",
        markup: r##"<table id="t" style="direction:rtl;border-collapse:collapse;"><tr id="r0"><td id="a"  style="padding:0;border-right:6px solid"><span id="sa">aa</span></td><td id="b"  style="padding:0;border-left:2px solid"><span id="sb">b</span></td></tr></table>"##,
        boxes: &[
            ("t", [0.0, 0.0, 38.0, 10.0]),
            ("sa", [12.0, 0.0, 20.0, 10.0]),
            ("sb", [2.0, 0.0, 10.0, 10.0]),
        ],
    },
    Case {
        label: "h_rtl_mid2",
        markup: r##"<table id="t" style="direction:rtl;border-collapse:collapse;"><tr id="r0"><td id="a"  style="padding:0;border-left:6px solid"><span id="sa">aa</span></td><td id="b"  style="padding:0;border-right:2px solid"><span id="sb">b</span></td></tr></table>"##,
        boxes: &[
            ("t", [0.0, 0.0, 36.0, 10.0]),
            ("sa", [16.0, 0.0, 20.0, 10.0]),
            ("sb", [0.0, 0.0, 10.0, 10.0]),
        ],
    },
    Case {
        label: "h_rtl_tblL",
        markup: r##"<table id="t" style="direction:rtl;border-collapse:collapse;border-left:8px solid"><tr id="r0"><td id="a"  style="padding:0;"><span id="sa">aa</span></td><td id="b"  style="padding:0;"><span id="sb">b</span></td></tr></table>"##,
        boxes: &[
            ("t", [0.0, 0.0, 38.0, 10.0]),
            ("sa", [18.0, 0.0, 20.0, 10.0]),
            ("sb", [8.0, 0.0, 10.0, 10.0]),
        ],
    },
    Case {
        label: "h_rtl_tblR",
        markup: r##"<table id="t" style="direction:rtl;border-collapse:collapse;border-right:8px solid"><tr id="r0"><td id="a"  style="padding:0;"><span id="sa">aa</span></td><td id="b"  style="padding:0;"><span id="sb">b</span></td></tr></table>"##,
        boxes: &[
            ("t", [0.0, 0.0, 38.0, 10.0]),
            ("sa", [10.0, 0.0, 20.0, 10.0]),
            ("sb", [0.0, 0.0, 10.0, 10.0]),
        ],
    },
    Case {
        label: "h_rtl_tblLcellL",
        markup: r##"<table id="t" style="direction:rtl;border-collapse:collapse;border-left:8px solid"><tr id="r0"><td id="a"  style="padding:0;border-left:4px solid"><span id="sa">aa</span></td><td id="b"  style="padding:0;"><span id="sb">b</span></td></tr></table>"##,
        boxes: &[
            ("t", [0.0, 0.0, 42.0, 10.0]),
            ("sa", [22.0, 0.0, 20.0, 10.0]),
            ("sb", [8.0, 0.0, 10.0, 10.0]),
        ],
    },
    Case {
        label: "h_rtl_tblRcellL",
        markup: r##"<table id="t" style="direction:rtl;border-collapse:collapse;border-right:8px solid"><tr id="r0"><td id="a"  style="padding:0;border-left:4px solid"><span id="sa">aa</span></td><td id="b"  style="padding:0;border-left:4px solid"><span id="sb">b</span></td></tr></table>"##,
        boxes: &[
            ("t", [0.0, 0.0, 46.0, 10.0]),
            ("sa", [18.0, 0.0, 20.0, 10.0]),
            ("sb", [4.0, 0.0, 10.0, 10.0]),
        ],
    },
    Case {
        label: "h_rtl_tblcells",
        markup: r##"<table id="t" style="direction:rtl;border-collapse:collapse;border-left:8px solid;border-right:2px solid"><tr id="r0"><td id="a"  style="padding:0;border-left:4px solid;border-right:2px solid"><span id="sa">aa</span></td><td id="b"  style="padding:0;border-left:6px solid;border-right:2px solid"><span id="sb">b</span></td></tr></table>"##,
        boxes: &[
            ("t", [0.0, 0.0, 44.0, 10.0]),
            ("sa", [22.0, 0.0, 20.0, 10.0]),
            ("sb", [8.0, 0.0, 10.0, 10.0]),
        ],
    },
];

#[test]
fn collapsed_borders_follow_the_rtl_column_order() {
    check(COLLAPSED);
}
