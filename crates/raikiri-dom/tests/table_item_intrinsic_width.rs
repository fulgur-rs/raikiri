//! A table among the items of a flex or grid container is as wide as it lays
//! itself out, in the sizes its container gathers before it lays the table out
//! as well as in the end: never narrower than its columns, whatever `width` it
//! states, and held to the columns by a `min-width` or `max-width` keyword. The
//! container reads the item's stated `width` and `max-width` from its style, and
//! knows neither the keywords nor the columns.
//!
//! The expected boxes are the ones Chromium computes for the same markup in a
//! 300px by 300px viewport with no body margin and a font of 10px Ahem: each is
//! an element's left, top, width and height. The box of a cell that collapses
//! borders, and the wrapper of a table with a caption, are separate matters:
//! only the cases without them are here.

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

/// A container that shrinks to fit a table is as wide as the table is, not as wide as the `width` the table states,
/// and a table that comes to more than it states is placed by the width it comes to.
const SHRINK_TO_FIT: &[Case] = &[
    Case {
        label: "a1_ig_w20",
        markup: r##"<div id=g style="display:inline-grid"><table id=t style="width:20px"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 38.0, 18.0]),
            ("t", [0.0, 0.0, 38.0, 18.0]),
            ("a", [2.0, 2.0, 34.0, 14.0]),
        ],
    },
    Case {
        label: "a2_if_w20",
        markup: r##"<div id=g style="display:inline-flex"><table id=t style="width:20px"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 38.0, 18.0]),
            ("t", [0.0, 0.0, 38.0, 18.0]),
            ("a", [2.0, 2.0, 34.0, 14.0]),
        ],
    },
    Case {
        label: "a4_ig_maxc_w200",
        markup: r##"<div id=g style="display:inline-grid"><table id=t style="max-width:max-content;width:200px"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 38.0, 18.0]),
            ("t", [0.0, 0.0, 38.0, 18.0]),
            ("a", [2.0, 2.0, 34.0, 14.0]),
        ],
    },
    Case {
        label: "a9_ig_w20_minw10",
        markup: r##"<div id=g style="display:inline-grid"><table id=t style="width:20px;min-width:10px"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 38.0, 18.0]),
            ("t", [0.0, 0.0, 38.0, 18.0]),
            ("a", [2.0, 2.0, 34.0, 14.0]),
        ],
    },
    Case {
        label: "a14_ig_maxw_minc_w200",
        markup: r##"<div id=g style="display:inline-grid"><table id=t style="max-width:min-content;width:200px"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 38.0, 18.0]),
            ("t", [0.0, 0.0, 38.0, 18.0]),
            ("a", [2.0, 2.0, 34.0, 14.0]),
        ],
    },
    Case {
        label: "a15_ig_maxw_fitc_w200",
        markup: r##"<div id=g style="display:inline-grid"><table id=t style="max-width:fit-content;width:200px"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 38.0, 18.0]),
            ("t", [0.0, 0.0, 38.0, 18.0]),
            ("a", [2.0, 2.0, 34.0, 14.0]),
        ],
    },
    Case {
        label: "a17_if_maxc_w200",
        markup: r##"<div id=g style="display:inline-flex"><table id=t style="max-width:max-content;width:200px"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 38.0, 18.0]),
            ("t", [0.0, 0.0, 38.0, 18.0]),
            ("a", [2.0, 2.0, 34.0, 14.0]),
        ],
    },
    Case {
        label: "a19_ig_w20_longtext",
        markup: r##"<div id=g style="display:inline-grid"><table id=t style="width:20px"><tr><td id=a style="padding:2px">BBB BBB BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 38.0, 38.0]),
            ("t", [0.0, 0.0, 38.0, 38.0]),
            ("a", [2.0, 2.0, 34.0, 34.0]),
        ],
    },
    Case {
        label: "a20_ig_w20_two_cols",
        markup: r##"<div id=g style="display:inline-grid"><table id=t style="width:20px"><tr><td id=a>BB</td><td id=b>CCC</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 60.0, 16.0]),
            ("t", [0.0, 0.0, 60.0, 16.0]),
            ("a", [2.0, 2.0, 22.0, 12.0]),
            ("b", [26.0, 2.0, 32.0, 12.0]),
        ],
    },
    Case {
        label: "a22_if_col_w20",
        markup: r##"<div id=g style="display:inline-flex;flex-direction:column"><table id=t style="width:20px"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 38.0, 18.0]),
            ("t", [0.0, 0.0, 38.0, 18.0]),
            ("a", [2.0, 2.0, 34.0, 14.0]),
        ],
    },
    Case {
        label: "b6_ig_borderbox_pad",
        markup: r##"<div id=g style="display:inline-grid"><table id=t style="width:20px;box-sizing:border-box;padding:5px"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 48.0, 28.0]),
            ("t", [0.0, 0.0, 48.0, 28.0]),
            ("a", [7.0, 7.0, 34.0, 14.0]),
        ],
    },
    Case {
        label: "b7_ig_contentbox_pad",
        markup: r##"<div id=g style="display:inline-grid"><table id=t style="width:20px;box-sizing:content-box;padding:5px"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 48.0, 28.0]),
            ("t", [0.0, 0.0, 48.0, 28.0]),
            ("a", [7.0, 7.0, 34.0, 14.0]),
        ],
    },
    Case {
        label: "b9_ig_separate_border",
        markup: r##"<div id=g style="display:inline-grid"><table id=t style="width:20px;border:4px solid"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 46.0, 26.0]),
            ("t", [0.0, 0.0, 46.0, 26.0]),
            ("a", [6.0, 6.0, 34.0, 14.0]),
        ],
    },
    Case {
        label: "b16_ig_w200_maxw20",
        markup: r##"<div id=g style="display:inline-grid"><table id=t style="width:200px;max-width:20px"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 38.0, 18.0]),
            ("t", [0.0, 0.0, 38.0, 18.0]),
            ("a", [2.0, 2.0, 34.0, 14.0]),
        ],
    },
    Case {
        label: "b17_ig_w20_minw100",
        markup: r##"<div id=g style="display:inline-grid"><table id=t style="width:20px;min-width:100px"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 100.0, 18.0]),
            ("t", [0.0, 0.0, 100.0, 18.0]),
            ("a", [2.0, 2.0, 96.0, 14.0]),
        ],
    },
    Case {
        label: "b18_ig_margins",
        markup: r##"<div id=g style="display:inline-grid"><table id=t style="width:20px;margin:0 5px"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 48.0, 18.0]),
            ("t", [5.0, 0.0, 38.0, 18.0]),
            ("a", [7.0, 2.0, 34.0, 14.0]),
        ],
    },
    Case {
        label: "b25_flexrow_reverse_ig",
        markup: r##"<div id=g style="display:inline-flex;flex-direction:row-reverse"><table id=t style="width:20px"><tr><td id=a style="padding:2px">BBB</td></tr></table><div id=x>xx</div></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 58.0, 18.0]),
            ("t", [20.0, 0.0, 38.0, 18.0]),
            ("a", [22.0, 2.0, 34.0, 14.0]),
            ("x", [0.0, 0.0, 20.0, 18.0]),
        ],
    },
    Case {
        label: "b28_ig_fixedlayout_w20",
        markup: r##"<div id=g style="display:inline-grid"><table id=t style="width:20px;table-layout:fixed"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 20.0, 18.0]),
            ("t", [0.0, 0.0, 20.0, 18.0]),
            ("a", [2.0, 2.0, 16.0, 14.0]),
        ],
    },
    Case {
        label: "b30_ig_w20_cols_pct",
        markup: r##"<div id=g style="display:inline-grid"><table id=t style="width:20px"><tr><td id=a style="width:50%">B</td><td id=b>CCC</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 50.0, 16.0]),
            ("t", [0.0, 0.0, 50.0, 16.0]),
            ("a", [2.0, 2.0, 12.0, 12.0]),
            ("b", [16.0, 2.0, 32.0, 12.0]),
        ],
    },
    Case {
        label: "b31_ig_w0",
        markup: r##"<div id=g style="display:inline-grid"><table id=t style="width:0"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 38.0, 18.0]),
            ("t", [0.0, 0.0, 38.0, 18.0]),
            ("a", [2.0, 2.0, 34.0, 14.0]),
        ],
    },
    Case {
        label: "b32_ig_w20_height",
        markup: r##"<div id=g style="display:inline-grid"><table id=t style="width:20px;height:50px"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 38.0, 50.0]),
            ("t", [0.0, 0.0, 38.0, 50.0]),
            ("a", [2.0, 2.0, 34.0, 46.0]),
        ],
    },
    Case {
        label: "b33_ig_w20_in_block_span",
        markup: r##"<div id=g><div id=p style="display:inline-grid"><table id=t style="width:20px"><tr><td id=a style="padding:2px">BBB</td></tr></table></div></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 300.0, 18.0]),
            ("t", [0.0, 0.0, 38.0, 18.0]),
            ("a", [2.0, 2.0, 34.0, 14.0]),
            ("p", [0.0, 0.0, 38.0, 18.0]),
        ],
    },
    Case {
        label: "b34_if_w20_flex1_sibling",
        markup: r##"<div id=g style="display:inline-flex"><table id=t style="width:20px"><tr><td id=a style="padding:2px">BBB</td></tr></table><div id=x style="flex:1">xx</div></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 58.0, 18.0]),
            ("t", [0.0, 0.0, 38.0, 18.0]),
            ("a", [2.0, 2.0, 34.0, 14.0]),
            ("x", [38.0, 0.0, 20.0, 18.0]),
        ],
    },
    Case {
        label: "b36_ig_w20_vertical_cell",
        markup: r##"<div id=g style="display:inline-grid"><table id=t style="width:20px"><tr><td id=a style="writing-mode:vertical-rl">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 20.0, 36.0]),
            ("t", [0.0, 0.0, 20.0, 36.0]),
            ("a", [2.0, 2.0, 16.0, 32.0]),
        ],
    },
    Case {
        label: "b38_ig_w20_rtl",
        markup: r##"<div id=g style="display:inline-grid;direction:rtl"><table id=t style="width:20px"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 38.0, 18.0]),
            ("t", [0.0, 0.0, 38.0, 18.0]),
            ("a", [2.0, 2.0, 34.0, 14.0]),
        ],
    },
    Case {
        label: "b27_nested_table_w",
        markup: r##"<div id=g style="display:inline-grid"><table id=t style="width:20px"><tr><td id=a><table id=u style="width:80px"><tr><td id=b>B</td></tr></table></td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 86.0, 22.0]),
            ("t", [0.0, 0.0, 86.0, 22.0]),
            ("a", [2.0, 2.0, 82.0, 18.0]),
            ("b", [5.0, 5.0, 76.0, 12.0]),
            ("u", [3.0, 3.0, 80.0, 16.0]),
        ],
    },
    Case {
        label: "b23_flexcol_center",
        markup: r##"<div id=g style="display:flex;flex-direction:column;align-items:center;width:200px"><table id=t style="width:20px"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 200.0, 18.0]),
            ("t", [81.0, 0.0, 38.0, 18.0]),
            ("a", [83.0, 2.0, 34.0, 14.0]),
        ],
    },
    Case {
        label: "e5_ig_fixed_cell_wider_than_table",
        markup: r##"<div id=g style="display:inline-grid"><table id=t style="table-layout:fixed;width:20px"><colgroup></colgroup><tr><td id=a style="padding:2px;width:50px">BBB</td></tr></table><div id=x style="width:20px">xx</div></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 58.0, 28.0]),
            ("t", [0.0, 0.0, 58.0, 18.0]),
            ("a", [2.0, 2.0, 54.0, 14.0]),
            ("x", [0.0, 18.0, 20.0, 10.0]),
        ],
    },
];

#[test]
fn a_container_shrinks_to_the_width_the_table_comes_to() {
    check(SHRINK_TO_FIT);
}

/// A grid track that sizes itself to a table is as wide as the table is.
const TRACKS: &[Case] = &[
    Case {
        label: "a3_grid_auto_auto_w20",
        markup: r##"<div id=g style="display:grid;grid-template-columns:auto auto;justify-content:start"><table id=t style="width:20px"><tr><td id=a style="padding:2px">BBB</td></tr></table><div id=x>xx</div></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 300.0, 18.0]),
            ("t", [0.0, 0.0, 38.0, 18.0]),
            ("a", [2.0, 2.0, 34.0, 14.0]),
            ("x", [38.0, 0.0, 20.0, 18.0]),
        ],
    },
    Case {
        label: "a18_grid_autocols_maxc_w200",
        markup: r##"<div id=g style="display:grid;grid-template-columns:auto auto;justify-content:start"><table id=t style="max-width:max-content;width:200px"><tr><td id=a style="padding:2px">BBB</td></tr></table><div id=x>xx</div></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 300.0, 18.0]),
            ("t", [0.0, 0.0, 38.0, 18.0]),
            ("a", [2.0, 2.0, 34.0, 14.0]),
            ("x", [38.0, 0.0, 20.0, 18.0]),
        ],
    },
    Case {
        label: "b11_grid_fr_fr",
        markup: r##"<div id=g style="display:grid;grid-template-columns:1fr 1fr;width:200px"><table id=t style="width:20px"><tr><td id=a style="padding:2px">BBB</td></tr></table><div id=x>xx</div></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 200.0, 18.0]),
            ("t", [0.0, 0.0, 38.0, 18.0]),
            ("a", [2.0, 2.0, 34.0, 14.0]),
            ("x", [100.0, 0.0, 100.0, 18.0]),
        ],
    },
    Case {
        label: "b12_grid_fr_small",
        markup: r##"<div id=g style="display:grid;grid-template-columns:1fr 1fr;width:60px"><table id=t style="width:20px"><tr><td id=a style="padding:2px">BBB</td></tr></table><div id=x>xx</div></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 60.0, 18.0]),
            ("t", [0.0, 0.0, 38.0, 18.0]),
            ("a", [2.0, 2.0, 34.0, 14.0]),
            ("x", [38.0, 0.0, 22.0, 18.0]),
        ],
    },
    Case {
        label: "b13_grid_mincontent_col",
        markup: r##"<div id=g style="display:inline-grid;grid-template-columns:min-content"><table id=t style="width:20px"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 38.0, 18.0]),
            ("t", [0.0, 0.0, 38.0, 18.0]),
            ("a", [2.0, 2.0, 34.0, 14.0]),
        ],
    },
    Case {
        label: "b14_grid_maxcontent_col",
        markup: r##"<div id=g style="display:inline-grid;grid-template-columns:max-content"><table id=t style="width:20px"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 38.0, 18.0]),
            ("t", [0.0, 0.0, 38.0, 18.0]),
            ("a", [2.0, 2.0, 34.0, 14.0]),
        ],
    },
    Case {
        label: "b15_grid_minmax_col",
        markup: r##"<div id=g style="display:inline-grid;grid-template-columns:minmax(0,auto)"><table id=t style="width:20px"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 38.0, 18.0]),
            ("t", [0.0, 0.0, 38.0, 18.0]),
            ("a", [2.0, 2.0, 34.0, 14.0]),
        ],
    },
    Case {
        label: "b19_ig_two_tables_cols",
        markup: r##"<div id=g style="display:inline-grid;grid-template-columns:auto auto"><table id=t style="width:20px"><tr><td id=a style="padding:2px">BBB</td></tr></table><table id=u style="width:10px"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 76.0, 18.0]),
            ("t", [0.0, 0.0, 38.0, 18.0]),
            ("u", [38.0, 0.0, 38.0, 18.0]),
        ],
    },
    Case {
        label: "b35_grid_span2_w20",
        markup: r##"<div id=g style="display:inline-grid;grid-template-columns:auto auto"><table id=t style="width:20px;grid-column:span 2"><tr><td id=a style="padding:2px">BBB</td></tr></table><div id=x>xx</div><div id=y>yy</div></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 40.0, 28.0]),
            ("t", [0.0, 0.0, 38.0, 18.0]),
            ("a", [2.0, 2.0, 34.0, 14.0]),
            ("x", [0.0, 18.0, 20.0, 10.0]),
            ("y", [20.0, 18.0, 20.0, 10.0]),
        ],
    },
];

#[test]
fn a_track_is_as_wide_as_the_table_it_holds() {
    check(TRACKS);
}

/// A table in a flex row shrinks under the width it states, down to its columns.
const FLEXING: &[Case] = &[
    Case {
        label: "a23_flex_w20_basis",
        markup: r##"<div id=g style="display:flex"><table id=t style="width:20px"><tr><td id=a style="padding:2px">BBB</td></tr></table><div id=x>xx</div></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 300.0, 18.0]),
            ("t", [0.0, 0.0, 38.0, 18.0]),
            ("a", [2.0, 2.0, 34.0, 14.0]),
            ("x", [38.0, 0.0, 20.0, 18.0]),
        ],
    },
    Case {
        label: "a24_flex_basis_auto_shrink",
        markup: r##"<div id=g style="display:flex;width:100px"><table id=t style="width:200px"><tr><td id=a style="padding:2px">BBB</td></tr></table><div id=x style="width:90px">xx</div></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 100.0, 18.0]),
            ("t", [0.0, 0.0, 68.96875, 18.0]),
            ("a", [2.0, 2.0, 64.96875, 14.0]),
            ("x", [68.96875, 0.0, 31.03125, 18.0]),
        ],
    },
    Case {
        label: "b1_flex_noshrink_w200",
        markup: r##"<div id=g style="display:flex;width:100px"><table id=t style="width:200px;flex-shrink:0"><tr><td id=a style="padding:2px">BBB</td></tr></table><div id=x style="width:90px">xx</div></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 100.0, 18.0]),
            ("t", [0.0, 0.0, 200.0, 18.0]),
            ("a", [2.0, 2.0, 196.0, 14.0]),
            ("x", [200.0, 0.0, 20.0, 18.0]),
        ],
    },
    Case {
        label: "b2_flex_grow",
        markup: r##"<div id=g style="display:flex;width:200px"><table id=t style="width:20px;flex-grow:1"><tr><td id=a style="padding:2px">BBB</td></tr></table><div id=x>xx</div></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 200.0, 18.0]),
            ("t", [0.0, 0.0, 180.0, 18.0]),
            ("a", [2.0, 2.0, 176.0, 14.0]),
            ("x", [180.0, 0.0, 20.0, 18.0]),
        ],
    },
    Case {
        label: "b3_flex_w20_x_wide",
        markup: r##"<div id=g style="display:flex;width:100px"><table id=t style="width:20px"><tr><td id=a style="padding:2px">BBB</td></tr></table><div id=x style="width:80px">xx</div></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 100.0, 18.0]),
            ("t", [0.0, 0.0, 38.0, 18.0]),
            ("a", [2.0, 2.0, 34.0, 14.0]),
            ("x", [38.0, 0.0, 62.0, 18.0]),
        ],
    },
    Case {
        label: "b4_flex_wide_cols_w20_shrinkcontainer",
        markup: r##"<div id=g style="display:flex;width:100px"><table id=t style="width:20px"><tr><td id=a style="padding:2px">BBBBBBBBBBBBBBBB</td></tr></table><div id=x style="width:80px">xx</div></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 100.0, 18.0]),
            ("t", [0.0, 0.0, 168.0, 18.0]),
            ("a", [2.0, 2.0, 164.0, 14.0]),
            ("x", [168.0, 0.0, 20.0, 18.0]),
        ],
    },
    Case {
        label: "b5_flex_two_tables",
        markup: r##"<div id=g style="display:flex;width:100px"><table id=t style="width:200px"><tr><td id=a style="padding:2px">BBB</td></tr></table><table id=u style="width:200px"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 100.0, 18.0]),
            ("t", [0.0, 0.0, 50.0, 18.0]),
            ("u", [50.0, 0.0, 50.0, 18.0]),
        ],
    },
    Case {
        label: "b26_flexwrap",
        markup: r##"<div id=g style="display:flex;flex-wrap:wrap;width:60px"><table id=t style="width:20px"><tr><td id=a style="padding:2px">BBB</td></tr></table><div id=x>xx</div><div id=y>yyyy</div></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 60.0, 28.0]),
            ("t", [0.0, 0.0, 38.0, 18.0]),
            ("a", [2.0, 2.0, 34.0, 14.0]),
            ("x", [38.0, 0.0, 20.0, 18.0]),
            ("y", [0.0, 18.0, 40.0, 10.0]),
        ],
    },
];

#[test]
fn a_table_in_a_flex_row_shrinks_to_its_columns() {
    check(FLEXING);
}

/// Tables whose stated width is what they come to, and the placement of a table that does not fill its area, keep their sizes.
/// A fixed table with a collapsed column shares its stated width among all its columns before the collapsed one is removed, so it
/// stays as wide as it states in the sizes its container gathers.
const SAME_AS_BEFORE: &[Case] = &[
    Case {
        label: "a5_block_w20",
        markup: r##"<div id=g><table id=t style="width:20px"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 300.0, 18.0]),
            ("t", [0.0, 0.0, 38.0, 18.0]),
            ("a", [2.0, 2.0, 34.0, 14.0]),
        ],
    },
    Case {
        label: "a6_ig_maxw20",
        markup: r##"<div id=g style="display:inline-grid"><table id=t style="max-width:20px"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 38.0, 18.0]),
            ("t", [0.0, 0.0, 38.0, 18.0]),
            ("a", [2.0, 2.0, 34.0, 14.0]),
        ],
    },
    Case {
        label: "a7_ig_nowidth",
        markup: r##"<div id=g style="display:inline-grid"><table id=t><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 38.0, 18.0]),
            ("t", [0.0, 0.0, 38.0, 18.0]),
            ("a", [2.0, 2.0, 34.0, 14.0]),
        ],
    },
    Case {
        label: "a8_ig_w200",
        markup: r##"<div id=g style="display:inline-grid"><table id=t style="width:200px"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 200.0, 18.0]),
            ("t", [0.0, 0.0, 200.0, 18.0]),
            ("a", [2.0, 2.0, 196.0, 14.0]),
        ],
    },
    Case {
        label: "a10_ig_w20_minw60",
        markup: r##"<div id=g style="display:inline-grid"><table id=t style="width:20px;min-width:60px"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 60.0, 18.0]),
            ("t", [0.0, 0.0, 60.0, 18.0]),
            ("a", [2.0, 2.0, 56.0, 14.0]),
        ],
    },
    Case {
        label: "a11_ig_w_minc",
        markup: r##"<div id=g style="display:inline-grid"><table id=t style="width:min-content"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 38.0, 18.0]),
            ("t", [0.0, 0.0, 38.0, 18.0]),
            ("a", [2.0, 2.0, 34.0, 14.0]),
        ],
    },
    Case {
        label: "a12_ig_w_maxc",
        markup: r##"<div id=g style="display:inline-grid"><table id=t style="width:max-content"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 38.0, 18.0]),
            ("t", [0.0, 0.0, 38.0, 18.0]),
            ("a", [2.0, 2.0, 34.0, 14.0]),
        ],
    },
    Case {
        label: "a13_ig_w_fitc",
        markup: r##"<div id=g style="display:inline-grid"><table id=t style="width:fit-content"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 38.0, 18.0]),
            ("t", [0.0, 0.0, 38.0, 18.0]),
            ("a", [2.0, 2.0, 34.0, 14.0]),
        ],
    },
    Case {
        label: "a16_ig_maxw_maxc_noW",
        markup: r##"<div id=g style="display:inline-grid"><table id=t style="max-width:max-content"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 38.0, 18.0]),
            ("t", [0.0, 0.0, 38.0, 18.0]),
            ("a", [2.0, 2.0, 34.0, 14.0]),
        ],
    },
    Case {
        label: "a21_ig_w100pct",
        markup: r##"<div id=g style="display:inline-grid"><table id=t style="width:100%"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 38.0, 18.0]),
            ("t", [0.0, 0.0, 38.0, 18.0]),
            ("a", [2.0, 2.0, 34.0, 14.0]),
        ],
    },
    Case {
        label: "a25_abspos_shrink_w20",
        markup: r##"<div id=g style="position:relative"><div id=p style="position:absolute"><table id=t style="width:20px"><tr><td id=a style="padding:2px">BBB</td></tr></table></div></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 300.0, 0.0]),
            ("t", [0.0, 0.0, 38.0, 18.0]),
            ("a", [2.0, 2.0, 34.0, 14.0]),
            ("p", [0.0, 0.0, 38.0, 18.0]),
        ],
    },
    Case {
        label: "a26_float_w20",
        markup: r##"<div id=g><div id=p style="float:left"><table id=t style="width:20px"><tr><td id=a style="padding:2px">BBB</td></tr></table></div></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 300.0, 0.0]),
            ("t", [0.0, 0.0, 38.0, 18.0]),
            ("a", [2.0, 2.0, 34.0, 14.0]),
            ("p", [0.0, 0.0, 38.0, 18.0]),
        ],
    },
    Case {
        label: "a27_inline_block_w20",
        markup: r##"<div id=g><div id=p style="display:inline-block"><table id=t style="width:20px"><tr><td id=a style="padding:2px">BBB</td></tr></table></div></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 300.0, 20.0]),
            ("t", [0.0, 0.0, 38.0, 18.0]),
            ("a", [2.0, 2.0, 34.0, 14.0]),
            ("p", [0.0, 0.0, 38.0, 18.0]),
        ],
    },
    Case {
        label: "b20_ig_pct10",
        markup: r##"<div id=g style="display:inline-grid"><table id=t style="width:10%"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 38.0, 18.0]),
            ("t", [0.0, 0.0, 38.0, 18.0]),
            ("a", [2.0, 2.0, 34.0, 14.0]),
        ],
    },
    Case {
        label: "b21_grid_pct10_fixed",
        markup: r##"<div id=g style="display:grid;grid-template-columns:300px"><table id=t style="width:10%"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 300.0, 18.0]),
            ("t", [0.0, 0.0, 38.0, 18.0]),
            ("a", [2.0, 2.0, 34.0, 14.0]),
        ],
    },
    Case {
        label: "b22_flexcol_start",
        markup: r##"<div id=g style="display:flex;flex-direction:column;align-items:flex-start;width:200px"><table id=t style="width:20px"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 200.0, 18.0]),
            ("t", [0.0, 0.0, 38.0, 18.0]),
            ("a", [2.0, 2.0, 34.0, 14.0]),
        ],
    },
    Case {
        label: "b24_flexcol_stretch",
        markup: r##"<div id=g style="display:flex;flex-direction:column;width:200px"><table id=t style="width:20px"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 200.0, 18.0]),
            ("t", [0.0, 0.0, 38.0, 18.0]),
            ("a", [2.0, 2.0, 34.0, 14.0]),
        ],
    },
    Case {
        label: "b29_ig_fixedlayout_w200",
        markup: r##"<div id=g style="display:inline-grid"><table id=t style="width:200px;table-layout:fixed"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 200.0, 18.0]),
            ("t", [0.0, 0.0, 200.0, 18.0]),
            ("a", [2.0, 2.0, 196.0, 14.0]),
        ],
    },
    Case {
        label: "b37_vertical_grid_w20",
        markup: r##"<div id=g style="display:inline-grid;writing-mode:vertical-rl"><table id=t style="width:20px"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 20.0, 38.0]),
            ("t", [0.0, 0.0, 20.0, 38.0]),
            ("a", [2.0, 2.0, 16.0, 34.0]),
        ],
    },
    Case {
        label: "e1_ig_fixed_collapsed_column_w200",
        markup: r##"<div id=g style="display:inline-grid"><table id=t style="table-layout:fixed;width:200px"><colgroup><col><col style="visibility:collapse"></colgroup><tr><td id=a style="padding:2px">BBB</td><td id=b style="padding:2px">CCC</td></tr></table><div id=x style="width:20px">xx</div></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 200.0, 28.0]),
            ("t", [0.0, 0.0, 101.0, 18.0]),
            ("a", [2.0, 2.0, 97.0, 14.0]),
            ("b", [99.0, 2.0, 0.0, 14.0]),
            ("x", [0.0, 18.0, 20.0, 10.0]),
        ],
    },
    Case {
        label: "e2_ig_fixed_collapsed_column_w20",
        markup: r##"<div id=g style="display:inline-grid"><table id=t style="table-layout:fixed;width:20px"><colgroup><col><col style="visibility:collapse"></colgroup><tr><td id=a style="padding:2px">BBB</td><td id=b style="padding:2px">CCC</td></tr></table><div id=x style="width:20px">xx</div></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 20.0, 28.0]),
            ("t", [0.0, 0.0, 11.0, 18.0]),
            ("a", [2.0, 2.0, 7.0, 14.0]),
            ("b", [9.0, 2.0, 0.0, 14.0]),
            ("x", [0.0, 18.0, 20.0, 10.0]),
        ],
    },
    Case {
        label: "e3_grid_fixed_collapsed_column_w200",
        markup: r##"<div id=g style="display:grid;grid-template-columns:auto auto;width:300px"><table id=t style="table-layout:fixed;width:200px"><colgroup><col><col style="visibility:collapse"></colgroup><tr><td id=a style="padding:2px">BBB</td><td id=b style="padding:2px">CCC</td></tr></table><div id=x style="width:20px">xx</div></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 300.0, 18.0]),
            ("t", [0.0, 0.0, 101.0, 18.0]),
            ("a", [2.0, 2.0, 97.0, 14.0]),
            ("b", [99.0, 2.0, 0.0, 14.0]),
            ("x", [240.0, 0.0, 20.0, 18.0]),
        ],
    },
    Case {
        label: "e4_grid_fixed_collapsed_column_w20",
        markup: r##"<div id=g style="display:grid;grid-template-columns:auto auto;width:300px"><table id=t style="table-layout:fixed;width:20px"><colgroup><col><col style="visibility:collapse"></colgroup><tr><td id=a style="padding:2px">BBB</td><td id=b style="padding:2px">CCC</td></tr></table><div id=x style="width:20px">xx</div></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 300.0, 18.0]),
            ("t", [0.0, 0.0, 11.0, 18.0]),
            ("a", [2.0, 2.0, 7.0, 14.0]),
            ("b", [9.0, 2.0, 0.0, 14.0]),
            ("x", [150.0, 0.0, 20.0, 18.0]),
        ],
    },
];

#[test]
fn a_table_that_comes_to_its_stated_width_keeps_it() {
    check(SAME_AS_BEFORE);
}
