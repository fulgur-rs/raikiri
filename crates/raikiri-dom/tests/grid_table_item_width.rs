//! A table among the items of a grid is as wide as it lays itself out: the width
//! of its columns, held between its `min-width` and `max-width` (the keywords
//! of which stand for that very width), and never narrower than its narrowest
//! columns. The grid stretches an item to its area and clamps it by the
//! lengths its style gives, and neither knows the keywords nor the columns.
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

/// A `max-width` that is a keyword stops the table at the width of its columns, though the grid stretches it to its area, whether the area is the whole grid or one of its tracks.
const KEYWORD_MAXIMUMS: &[Case] = &[
    Case {
        label: "g1_maxw_maxc",
        markup: r##"<div id=g style="display:grid"><table id=t style="max-width:max-content"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 300.0, 18.0]),
            ("t", [0.0, 0.0, 38.0, 18.0]),
            ("a", [2.0, 2.0, 34.0, 14.0]),
        ],
    },
    Case {
        label: "g2_maxw_fitc",
        markup: r##"<div id=g style="display:grid"><table id=t style="max-width:fit-content"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 300.0, 18.0]),
            ("t", [0.0, 0.0, 38.0, 18.0]),
            ("a", [2.0, 2.0, 34.0, 14.0]),
        ],
    },
    Case {
        label: "g4_maxw_minc",
        markup: r##"<div id=g style="display:grid"><table id=t style="max-width:min-content"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 300.0, 18.0]),
            ("t", [0.0, 0.0, 38.0, 18.0]),
            ("a", [2.0, 2.0, 34.0, 14.0]),
        ],
    },
    Case {
        label: "g5_start",
        markup: r##"<div id=g style="display:grid;justify-items:start"><table id=t style="max-width:max-content"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 300.0, 18.0]),
            ("t", [0.0, 0.0, 38.0, 18.0]),
            ("a", [2.0, 2.0, 34.0, 14.0]),
        ],
    },
    Case {
        label: "g7_width_maxc",
        markup: r##"<div id=g style="display:grid"><table id=t style="width:max-content"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 300.0, 18.0]),
            ("t", [0.0, 0.0, 38.0, 18.0]),
            ("a", [2.0, 2.0, 34.0, 14.0]),
        ],
    },
    Case {
        label: "g11_cols",
        markup: r##"<div id=g style="display:grid;grid-template-columns:200px 100px"><table id=t style="max-width:max-content"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 300.0, 18.0]),
            ("t", [0.0, 0.0, 38.0, 18.0]),
            ("a", [2.0, 2.0, 34.0, 14.0]),
        ],
    },
    Case {
        label: "g12_pct",
        markup: r##"<div id=g style="display:grid"><table id=t style="max-width:max-content"><tr><td id=a style="width:50%;padding:0">BBB</td><td id=b style="padding:0">CC</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 300.0, 14.0]),
            ("t", [0.0, 0.0, 66.0, 14.0]),
            ("a", [2.0, 2.0, 30.0, 10.0]),
        ],
    },
    Case {
        label: "g14_alignself_stretch",
        markup: r##"<div id=g style="display:grid"><table id=t style="max-width:max-content;justify-self:stretch"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 300.0, 18.0]),
            ("t", [0.0, 0.0, 38.0, 18.0]),
            ("a", [2.0, 2.0, 34.0, 14.0]),
        ],
    },
    Case {
        label: "b14_grid_auto_cols",
        markup: r##"<div id=g style="display:grid;grid-template-columns:auto"><table id=t style="max-width:max-content"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 300.0, 18.0]),
            ("t", [0.0, 0.0, 38.0, 18.0]),
            ("a", [2.0, 2.0, 34.0, 14.0]),
        ],
    },
    Case {
        label: "b15_grid_fr",
        markup: r##"<div id=g style="display:grid;grid-template-columns:1fr 1fr"><table id=t style="max-width:max-content"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 300.0, 18.0]),
            ("t", [0.0, 0.0, 38.0, 18.0]),
            ("a", [2.0, 2.0, 34.0, 14.0]),
        ],
    },
    Case {
        label: "b16_grid_justifyself_normal",
        markup: r##"<div id=g style="display:grid"><table id=t style="max-width:max-content;justify-self:normal"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 300.0, 18.0]),
            ("t", [0.0, 0.0, 38.0, 18.0]),
            ("a", [2.0, 2.0, 34.0, 14.0]),
        ],
    },
    Case {
        label: "b17_grid_fixedwidth_maxc_cap",
        markup: r##"<div id=g style="display:grid"><table id=t style="width:200px;max-width:max-content"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 300.0, 18.0]),
            ("t", [0.0, 0.0, 38.0, 18.0]),
            ("a", [2.0, 2.0, 34.0, 14.0]),
        ],
    },
    Case {
        label: "g19_longtext",
        markup: r##"<div id=g style="display:grid"><table id=t style="max-width:max-content"><tr><td id=a style="padding:2px">BBB BBB BBB BBB BBB BBB BBB BBB BBB BBB BBB BBB BBB BBB BBB BBB BBB BBB BBB BBB BBB BBB BBB BBB BBB BBB BBB BBB BBB BBB BBB BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 300.0, 58.0]),
            ("t", [0.0, 0.0, 300.0, 58.0]),
            ("a", [2.0, 2.0, 296.0, 54.0]),
        ],
    },
    Case {
        label: "g20_maxw_fitc_long",
        markup: r##"<div id=g style="display:grid"><table id=t style="max-width:fit-content"><tr><td id=a style="padding:2px">BBB BBB BBB BBB BBB BBB BBB BBB BBB BBB BBB BBB BBB BBB BBB BBB BBB BBB BBB BBB BBB BBB BBB BBB BBB BBB BBB BBB BBB BBB BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 300.0, 58.0]),
            ("t", [0.0, 0.0, 300.0, 58.0]),
            ("a", [2.0, 2.0, 296.0, 54.0]),
        ],
    },
];

#[test]
fn a_keyword_maximum_stops_a_stretched_table_at_its_columns() {
    check(KEYWORD_MAXIMUMS);
}

/// A table is never narrower than its narrowest columns, whatever `width` or `max-width` the grid clamps it by.
const NARROWEST: &[Case] = &[
    Case {
        label: "g3_maxw_len",
        markup: r##"<div id=g style="display:grid"><table id=t style="max-width:36px"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 300.0, 18.0]),
            ("t", [0.0, 0.0, 38.0, 18.0]),
            ("a", [2.0, 2.0, 34.0, 14.0]),
        ],
    },
    Case {
        label: "g9_maxw_20",
        markup: r##"<div id=g style="display:grid"><table id=t style="max-width:20px"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 300.0, 18.0]),
            ("t", [0.0, 0.0, 38.0, 18.0]),
            ("a", [2.0, 2.0, 34.0, 14.0]),
        ],
    },
    Case {
        label: "b6_grid_len20",
        markup: r##"<div id=g style="display:grid"><table id=t style="max-width:20px"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 300.0, 18.0]),
            ("t", [0.0, 0.0, 38.0, 18.0]),
            ("a", [2.0, 2.0, 34.0, 14.0]),
        ],
    },
    Case {
        label: "b7_grid_start_len20",
        markup: r##"<div id=g style="display:grid;justify-items:start"><table id=t style="max-width:20px"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 300.0, 18.0]),
            ("t", [0.0, 0.0, 38.0, 18.0]),
            ("a", [2.0, 2.0, 34.0, 14.0]),
        ],
    },
    Case {
        label: "b8_grid_width20",
        markup: r##"<div id=g style="display:grid"><table id=t style="width:20px"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 300.0, 18.0]),
            ("t", [0.0, 0.0, 38.0, 18.0]),
            ("a", [2.0, 2.0, 34.0, 14.0]),
        ],
    },
    Case {
        label: "b11_grid_maxw_100",
        markup: r##"<div id=g style="display:grid"><table id=t style="max-width:100px"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 300.0, 18.0]),
            ("t", [0.0, 0.0, 100.0, 18.0]),
            ("a", [2.0, 2.0, 96.0, 14.0]),
        ],
    },
    Case {
        label: "b12_grid_maxw_50",
        markup: r##"<div id=g style="display:grid"><table id=t style="max-width:50px"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 300.0, 18.0]),
            ("t", [0.0, 0.0, 50.0, 18.0]),
            ("a", [2.0, 2.0, 46.0, 14.0]),
        ],
    },
    Case {
        label: "b10_grid_maxw_pct",
        markup: r##"<div id=g style="display:grid"><table id=t style="max-width:50%"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 300.0, 18.0]),
            ("t", [0.0, 0.0, 150.0, 18.0]),
            ("a", [2.0, 2.0, 146.0, 14.0]),
        ],
    },
];

#[test]
fn a_table_is_not_narrower_than_its_columns_in_a_grid() {
    check(NARROWEST);
}

/// A `min-width` holds a table that a keyword `max-width` stops, and its keywords stand for the width of its columns.
const MINIMUMS: &[Case] = &[
    Case {
        label: "b9_grid_maxc_with_min",
        markup: r##"<div id=g style="display:grid"><table id=t style="max-width:max-content;min-width:100px"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 300.0, 18.0]),
            ("t", [0.0, 0.0, 100.0, 18.0]),
            ("a", [2.0, 2.0, 96.0, 14.0]),
        ],
    },
    Case {
        label: "g10_minw_maxc",
        markup: r##"<div id=g style="display:grid"><table id=t style="min-width:max-content"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 300.0, 18.0]),
            ("t", [0.0, 0.0, 300.0, 18.0]),
            ("a", [2.0, 2.0, 296.0, 14.0]),
        ],
    },
    Case {
        label: "m1_grid_minw_len",
        markup: r##"<div id=g style="display:grid"><table id=t style="min-width:100px"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 300.0, 18.0]),
            ("t", [0.0, 0.0, 300.0, 18.0]),
            ("a", [2.0, 2.0, 296.0, 14.0]),
        ],
    },
    Case {
        label: "m2_grid_minw_len_start",
        markup: r##"<div id=g style="display:grid;justify-items:start"><table id=t style="min-width:100px"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 300.0, 18.0]),
            ("t", [0.0, 0.0, 100.0, 18.0]),
            ("a", [2.0, 2.0, 96.0, 14.0]),
        ],
    },
    Case {
        label: "m3_grid_minw_minc",
        markup: r##"<div id=g style="display:grid;justify-items:start"><table id=t style="min-width:min-content"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 300.0, 18.0]),
            ("t", [0.0, 0.0, 38.0, 18.0]),
            ("a", [2.0, 2.0, 34.0, 14.0]),
        ],
    },
    Case {
        label: "m4_grid_minw_fitc",
        markup: r##"<div id=g style="display:grid;justify-items:start"><table id=t style="min-width:fit-content"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 300.0, 18.0]),
            ("t", [0.0, 0.0, 38.0, 18.0]),
            ("a", [2.0, 2.0, 34.0, 14.0]),
        ],
    },
    Case {
        label: "m5_grid_w_pct",
        markup: r##"<div id=g style="display:grid"><table id=t style="width:50%"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 300.0, 18.0]),
            ("t", [0.0, 0.0, 150.0, 18.0]),
            ("a", [2.0, 2.0, 146.0, 14.0]),
        ],
    },
];

#[test]
fn a_minimum_width_holds_a_table_stopped_by_a_keyword() {
    check(MINIMUMS);
}

/// A table that does not fill its area sits where its alignment puts it, from the left, the right or the middle, in either direction, and keeps its margins.
const ALIGNED: &[Case] = &[
    Case {
        label: "g13_stretchself",
        markup: r##"<div id=g style="display:grid;justify-items:center"><table id=t style="max-width:max-content"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 300.0, 18.0]),
            ("t", [131.0, 0.0, 38.0, 18.0]),
            ("a", [133.0, 2.0, 34.0, 14.0]),
        ],
    },
    Case {
        label: "m6_grid_w_50_center",
        markup: r##"<div id=g style="display:grid;justify-items:center"><table id=t style="width:50px"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 300.0, 18.0]),
            ("t", [125.0, 0.0, 50.0, 18.0]),
            ("a", [127.0, 2.0, 46.0, 14.0]),
        ],
    },
    Case {
        label: "m7_grid_w_20_center",
        markup: r##"<div id=g style="display:grid;justify-items:center"><table id=t style="width:20px"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 300.0, 18.0]),
            ("t", [131.0, 0.0, 38.0, 18.0]),
            ("a", [133.0, 2.0, 34.0, 14.0]),
        ],
    },
    Case {
        label: "m8_grid_w_20_end",
        markup: r##"<div id=g style="display:grid;justify-items:end"><table id=t style="width:20px"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 300.0, 18.0]),
            ("t", [262.0, 0.0, 38.0, 18.0]),
            ("a", [264.0, 2.0, 34.0, 14.0]),
        ],
    },
    Case {
        label: "m9_grid_w_20_end_rtl",
        markup: r##"<div id=g style="display:grid;justify-items:end;direction:rtl"><table id=t style="width:20px"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 300.0, 18.0]),
            ("t", [0.0, 0.0, 38.0, 18.0]),
            ("a", [2.0, 2.0, 34.0, 14.0]),
        ],
    },
    Case {
        label: "m10_grid_maxc_rtl",
        markup: r##"<div id=g style="display:grid;direction:rtl"><table id=t style="max-width:max-content"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 300.0, 18.0]),
            ("t", [262.0, 0.0, 38.0, 18.0]),
            ("a", [264.0, 2.0, 34.0, 14.0]),
        ],
    },
    Case {
        label: "m11_grid_maxc_rtl_end",
        markup: r##"<div id=g style="display:grid;direction:rtl;justify-items:end"><table id=t style="max-width:max-content"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 300.0, 18.0]),
            ("t", [0.0, 0.0, 38.0, 18.0]),
            ("a", [2.0, 2.0, 34.0, 14.0]),
        ],
    },
    Case {
        label: "m12_grid_maxc_center",
        markup: r##"<div id=g style="display:grid;justify-items:center"><table id=t style="max-width:max-content;width:200px"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 300.0, 18.0]),
            ("t", [131.0, 0.0, 38.0, 18.0]),
            ("a", [133.0, 2.0, 34.0, 14.0]),
        ],
    },
    Case {
        label: "m13_grid_maxc_margin",
        markup: r##"<div id=g style="display:grid"><table id=t style="max-width:max-content;margin:0 10px"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 300.0, 18.0]),
            ("t", [10.0, 0.0, 38.0, 18.0]),
            ("a", [12.0, 2.0, 34.0, 14.0]),
        ],
    },
    Case {
        label: "m14_grid_maxc_automargin",
        markup: r##"<div id=g style="display:grid"><table id=t style="max-width:max-content;margin:0 auto"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 300.0, 18.0]),
            ("t", [131.0, 0.0, 38.0, 18.0]),
            ("a", [133.0, 2.0, 34.0, 14.0]),
        ],
    },
    Case {
        label: "b18_grid_maxc_end",
        markup: r##"<div id=g style="display:grid"><table id=t style="max-width:max-content;justify-self:end"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 300.0, 18.0]),
            ("t", [262.0, 0.0, 38.0, 18.0]),
            ("a", [264.0, 2.0, 34.0, 14.0]),
        ],
    },
    Case {
        label: "n1_self_start",
        markup: r##"<div id=g style="display:grid;justify-items:self-start"><table id=t style="max-width:max-content"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 300.0, 18.0]),
            ("t", [0.0, 0.0, 38.0, 18.0]),
            ("a", [2.0, 2.0, 34.0, 14.0]),
        ],
    },
    Case {
        label: "n2_self_end",
        markup: r##"<div id=g style="display:grid;justify-items:self-end"><table id=t style="max-width:max-content"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 300.0, 18.0]),
            ("t", [262.0, 0.0, 38.0, 18.0]),
            ("a", [264.0, 2.0, 34.0, 14.0]),
        ],
    },
    Case {
        label: "n3_self_start_rtl_table",
        markup: r##"<div id=g style="display:grid"><table id=t style="max-width:max-content;direction:rtl;justify-self:self-start"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 300.0, 18.0]),
            ("t", [262.0, 0.0, 38.0, 18.0]),
            ("a", [264.0, 2.0, 34.0, 14.0]),
        ],
    },
    Case {
        label: "n4_self_end_rtl_table",
        markup: r##"<div id=g style="display:grid"><table id=t style="max-width:max-content;direction:rtl;justify-self:self-end"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 300.0, 18.0]),
            ("t", [0.0, 0.0, 38.0, 18.0]),
            ("a", [2.0, 2.0, 34.0, 14.0]),
        ],
    },
    Case {
        label: "n5_self_start_rtl_grid",
        markup: r##"<div id=g style="display:grid;direction:rtl"><table id=t style="max-width:max-content;direction:ltr;justify-self:self-start"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 300.0, 18.0]),
            ("t", [0.0, 0.0, 38.0, 18.0]),
            ("a", [2.0, 2.0, 34.0, 14.0]),
        ],
    },
    Case {
        label: "n6_flex_start",
        markup: r##"<div id=g style="display:grid;justify-items:flex-start"><table id=t style="max-width:max-content"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 300.0, 18.0]),
            ("t", [0.0, 0.0, 38.0, 18.0]),
            ("a", [2.0, 2.0, 34.0, 14.0]),
        ],
    },
    Case {
        label: "n7_flex_end",
        markup: r##"<div id=g style="display:grid;justify-items:flex-end"><table id=t style="max-width:max-content"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 300.0, 18.0]),
            ("t", [262.0, 0.0, 38.0, 18.0]),
            ("a", [264.0, 2.0, 34.0, 14.0]),
        ],
    },
    Case {
        label: "n8_baseline",
        markup: r##"<div id=g style="display:grid;justify-items:baseline"><table id=t style="max-width:max-content"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 300.0, 18.0]),
            ("t", [0.0, 0.0, 38.0, 18.0]),
            ("a", [2.0, 2.0, 34.0, 14.0]),
        ],
    },
    Case {
        label: "k1_auto_both_w20",
        markup: r##"<div id=g style="display:grid"><table id=t style="width:20px;margin:0 auto"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 300.0, 18.0]),
            ("t", [131.0, 0.0, 38.0, 18.0]),
            ("a", [133.0, 2.0, 34.0, 14.0]),
        ],
    },
    Case {
        label: "k2_auto_left_w20",
        markup: r##"<div id=g style="display:grid"><table id=t style="width:20px;margin-left:auto"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 300.0, 18.0]),
            ("t", [262.0, 0.0, 38.0, 18.0]),
            ("a", [264.0, 2.0, 34.0, 14.0]),
        ],
    },
    Case {
        label: "k3_auto_right_w20",
        markup: r##"<div id=g style="display:grid"><table id=t style="width:20px;margin-right:auto"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 300.0, 18.0]),
            ("t", [0.0, 0.0, 38.0, 18.0]),
            ("a", [2.0, 2.0, 34.0, 14.0]),
        ],
    },
    Case {
        label: "k4_auto_both_w20_rtl",
        markup: r##"<div id=g style="display:grid;direction:rtl"><table id=t style="width:20px;margin:0 auto"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 300.0, 18.0]),
            ("t", [131.0, 0.0, 38.0, 18.0]),
            ("a", [133.0, 2.0, 34.0, 14.0]),
        ],
    },
    Case {
        label: "k5_auto_left_w20_rtl",
        markup: r##"<div id=g style="display:grid;direction:rtl"><table id=t style="width:20px;margin-left:auto"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 300.0, 18.0]),
            ("t", [262.0, 0.0, 38.0, 18.0]),
            ("a", [264.0, 2.0, 34.0, 14.0]),
        ],
    },
    Case {
        label: "k6_auto_right_w20_rtl",
        markup: r##"<div id=g style="display:grid;direction:rtl"><table id=t style="width:20px;margin-right:auto"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 300.0, 18.0]),
            ("t", [0.0, 0.0, 38.0, 18.0]),
            ("a", [2.0, 2.0, 34.0, 14.0]),
        ],
    },
    Case {
        label: "k7_auto_left_margins",
        markup: r##"<div id=g style="display:grid"><table id=t style="width:20px;margin:0 5px 0 auto"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 300.0, 18.0]),
            ("t", [257.0, 0.0, 38.0, 18.0]),
            ("a", [259.0, 2.0, 34.0, 14.0]),
        ],
    },
    Case {
        label: "k8_auto_maxw20",
        markup: r##"<div id=g style="display:grid"><table id=t style="max-width:20px;margin:0 auto"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 300.0, 18.0]),
            ("t", [131.0, 0.0, 38.0, 18.0]),
            ("a", [133.0, 2.0, 34.0, 14.0]),
        ],
    },
    Case {
        label: "k9_safe_center_w400",
        markup: r##"<div id=g style="display:grid;justify-items:safe center"><table id=t style="width:400px"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 300.0, 18.0]),
            ("t", [0.0, 0.0, 400.0, 18.0]),
            ("a", [2.0, 2.0, 396.0, 14.0]),
        ],
    },
    Case {
        label: "k10_center_w400",
        markup: r##"<div id=g style="display:grid;justify-items:center"><table id=t style="width:400px"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 300.0, 18.0]),
            ("t", [0.0, 0.0, 400.0, 18.0]),
            ("a", [2.0, 2.0, 396.0, 14.0]),
        ],
    },
    Case {
        label: "k11_end_w400",
        markup: r##"<div id=g style="display:grid;justify-items:end"><table id=t style="width:400px"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 300.0, 18.0]),
            ("t", [0.0, 0.0, 400.0, 18.0]),
            ("a", [2.0, 2.0, 396.0, 14.0]),
        ],
    },
    Case {
        label: "k12_safe_end_w400",
        markup: r##"<div id=g style="display:grid;justify-items:safe end"><table id=t style="width:400px"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 300.0, 18.0]),
            ("t", [0.0, 0.0, 400.0, 18.0]),
            ("a", [2.0, 2.0, 396.0, 14.0]),
        ],
    },
];

#[test]
fn a_table_that_does_not_fill_its_area_is_aligned_at_its_width() {
    check(ALIGNED);
}

/// Tables placed among other items, spanning rows, with borders and padding, or as inline tables.
const PLACED: &[Case] = &[
    Case {
        label: "m15_grid_maxc_selfstretch_row2",
        markup: r##"<div id=g style="display:grid;grid-template-columns:100px 100px"><div id=x>x</div><table id=t style="max-width:max-content"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 300.0, 18.0]),
            ("t", [100.0, 0.0, 38.0, 18.0]),
            ("a", [102.0, 2.0, 34.0, 14.0]),
            ("x", [0.0, 0.0, 100.0, 18.0]),
        ],
    },
    Case {
        label: "m16_grid_span",
        markup: r##"<div id=g style="display:grid;grid-template-columns:100px 100px"><div id=x style="grid-row:1/3">x</div><table id=t style="max-width:max-content;grid-column:2"><tr><td id=a style="padding:2px">BBB</td></tr></table><div id=y style="grid-column:2">y</div></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 300.0, 28.0]),
            ("t", [100.0, 0.0, 38.0, 18.0]),
            ("a", [102.0, 2.0, 34.0, 14.0]),
            ("x", [0.0, 0.0, 100.0, 28.0]),
            ("y", [100.0, 18.0, 100.0, 10.0]),
        ],
    },
    Case {
        label: "m18_grid_border",
        markup: r##"<div id=g style="display:grid"><table id=t style="max-width:max-content;border:5px solid;padding:3px"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 300.0, 34.0]),
            ("t", [0.0, 0.0, 54.0, 34.0]),
            ("a", [10.0, 10.0, 34.0, 14.0]),
        ],
    },
    Case {
        label: "m20_grid_inline_table",
        markup: r##"<div id=g style="display:grid"><table id=t style="display:inline-table;max-width:max-content"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 300.0, 18.0]),
            ("t", [0.0, 0.0, 38.0, 18.0]),
            ("a", [2.0, 2.0, 34.0, 14.0]),
        ],
    },
    Case {
        label: "g8_nomax",
        markup: r##"<div id=g style="display:grid"><table id=t style=""><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 300.0, 18.0]),
            ("t", [0.0, 0.0, 300.0, 18.0]),
            ("a", [2.0, 2.0, 296.0, 14.0]),
        ],
    },
    Case {
        label: "b13_grid_stretch_noMax",
        markup: r##"<div id=g style="display:grid;justify-items:stretch"><table id=t style=""><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 300.0, 18.0]),
            ("t", [0.0, 0.0, 300.0, 18.0]),
            ("a", [2.0, 2.0, 296.0, 14.0]),
        ],
    },
    Case {
        label: "g16_inlinegrid",
        markup: r##"<div id=g style="display:inline-grid"><table id=t style="max-width:max-content"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 38.0, 18.0]),
            ("t", [0.0, 0.0, 38.0, 18.0]),
            ("a", [2.0, 2.0, 34.0, 14.0]),
        ],
    },
    Case {
        label: "g17_flexcol",
        markup: r##"<div id=g style="display:flex;flex-direction:column"><table id=t style="max-width:max-content"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 300.0, 18.0]),
            ("t", [0.0, 0.0, 38.0, 18.0]),
            ("a", [2.0, 2.0, 34.0, 14.0]),
        ],
    },
    Case {
        label: "g18_flexrow",
        markup: r##"<div id=g style="display:flex"><table id=t style="max-width:max-content"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 300.0, 18.0]),
            ("t", [0.0, 0.0, 38.0, 18.0]),
            ("a", [2.0, 2.0, 34.0, 14.0]),
        ],
    },
    Case {
        label: "b4_flexcol_len20",
        markup: r##"<div id=g style="display:flex;flex-direction:column"><table id=t style="max-width:20px"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 300.0, 18.0]),
            ("t", [0.0, 0.0, 38.0, 18.0]),
            ("a", [2.0, 2.0, 34.0, 14.0]),
        ],
    },
    Case {
        label: "b5_flexrow_len20",
        markup: r##"<div id=g style="display:flex"><table id=t style="max-width:20px"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 300.0, 18.0]),
            ("t", [0.0, 0.0, 38.0, 18.0]),
            ("a", [2.0, 2.0, 34.0, 14.0]),
        ],
    },
    Case {
        label: "k13_vgrid_maxc",
        markup: r##"<div id=g style="display:grid;writing-mode:vertical-rl;height:200px"><table id=t style="max-width:max-content;writing-mode:horizontal-tb"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 38.0, 200.0]),
            ("t", [0.0, 0.0, 38.0, 200.0]),
            ("a", [2.0, 2.0, 34.0, 196.0]),
        ],
    },
    Case {
        label: "k15_vgrid_plain",
        markup: r##"<div id=g style="display:grid;writing-mode:vertical-rl;height:200px"><table id=t style="writing-mode:horizontal-tb"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 38.0, 200.0]),
            ("t", [0.0, 0.0, 38.0, 200.0]),
            ("a", [2.0, 2.0, 34.0, 196.0]),
        ],
    },
];

#[test]
fn a_stopped_table_keeps_its_place_among_the_items() {
    check(PLACED);
}

/// Whether a table overflows its area is settled by the width it ends up with, so a `safe` alignment gives way to `start` and auto margins lose their room by that width, not by the one the grid gave it first.
const OVERFLOW: &[Case] = &[
    Case {
        label: "s4_safe_center_w20_wide_track100",
        markup: r##"<div id=g style="display:grid;grid-template-columns:100px;"><table id=t style="width:20px;justify-self:safe center"><tr><td id=a style="padding:2px">BBBBBBBBBBBBBBBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 300.0, 18.0]),
            ("t", [0.0, 0.0, 168.0, 18.0]),
            ("a", [2.0, 2.0, 164.0, 14.0]),
        ],
    },
    Case {
        label: "s4_center_w20_wide_track100",
        markup: r##"<div id=g style="display:grid;grid-template-columns:100px;"><table id=t style="width:20px;justify-self:center"><tr><td id=a style="padding:2px">BBBBBBBBBBBBBBBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 300.0, 18.0]),
            ("t", [-34.0, 0.0, 168.0, 18.0]),
            ("a", [-32.0, 2.0, 164.0, 14.0]),
        ],
    },
    Case {
        label: "s4_end_w20_wide_track100",
        markup: r##"<div id=g style="display:grid;grid-template-columns:100px;"><table id=t style="width:20px;justify-self:end"><tr><td id=a style="padding:2px">BBBBBBBBBBBBBBBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 300.0, 18.0]),
            ("t", [-68.0, 0.0, 168.0, 18.0]),
            ("a", [-66.0, 2.0, 164.0, 14.0]),
        ],
    },
    Case {
        label: "s4_safe_end_w20_wide_track100",
        markup: r##"<div id=g style="display:grid;grid-template-columns:100px;"><table id=t style="width:20px;justify-self:safe end"><tr><td id=a style="padding:2px">BBBBBBBBBBBBBBBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 300.0, 18.0]),
            ("t", [0.0, 0.0, 168.0, 18.0]),
            ("a", [2.0, 2.0, 164.0, 14.0]),
        ],
    },
    Case {
        label: "s4_unsafe_center_w20_wide_track100",
        markup: r##"<div id=g style="display:grid;grid-template-columns:100px;"><table id=t style="width:20px;justify-self:unsafe center"><tr><td id=a style="padding:2px">BBBBBBBBBBBBBBBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 300.0, 18.0]),
            ("t", [-34.0, 0.0, 168.0, 18.0]),
            ("a", [-32.0, 2.0, 164.0, 14.0]),
        ],
    },
    Case {
        label: "s5_safe_center_w20_wide_track100_rtl",
        markup: r##"<div id=g style="display:grid;grid-template-columns:100px;direction:rtl;"><table id=t style="width:20px;justify-self:safe center"><tr><td id=a style="padding:2px">BBBBBBBBBBBBBBBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 300.0, 18.0]),
            ("t", [132.0, 0.0, 168.0, 18.0]),
            ("a", [134.0, 2.0, 164.0, 14.0]),
        ],
    },
    Case {
        label: "s5_center_w20_wide_track100_rtl",
        markup: r##"<div id=g style="display:grid;grid-template-columns:100px;direction:rtl;"><table id=t style="width:20px;justify-self:center"><tr><td id=a style="padding:2px">BBBBBBBBBBBBBBBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 300.0, 18.0]),
            ("t", [166.0, 0.0, 168.0, 18.0]),
            ("a", [168.0, 2.0, 164.0, 14.0]),
        ],
    },
    Case {
        label: "s5_end_w20_wide_track100_rtl",
        markup: r##"<div id=g style="display:grid;grid-template-columns:100px;direction:rtl;"><table id=t style="width:20px;justify-self:end"><tr><td id=a style="padding:2px">BBBBBBBBBBBBBBBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 300.0, 18.0]),
            ("t", [200.0, 0.0, 168.0, 18.0]),
            ("a", [202.0, 2.0, 164.0, 14.0]),
        ],
    },
    Case {
        label: "s5_safe_end_w20_wide_track100_rtl",
        markup: r##"<div id=g style="display:grid;grid-template-columns:100px;direction:rtl;"><table id=t style="width:20px;justify-self:safe end"><tr><td id=a style="padding:2px">BBBBBBBBBBBBBBBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 300.0, 18.0]),
            ("t", [132.0, 0.0, 168.0, 18.0]),
            ("a", [134.0, 2.0, 164.0, 14.0]),
        ],
    },
    Case {
        label: "s5_unsafe_end_w20_wide_track100_rtl",
        markup: r##"<div id=g style="display:grid;grid-template-columns:100px;direction:rtl;"><table id=t style="width:20px;justify-self:unsafe end"><tr><td id=a style="padding:2px">BBBBBBBBBBBBBBBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 300.0, 18.0]),
            ("t", [200.0, 0.0, 168.0, 18.0]),
            ("a", [202.0, 2.0, 164.0, 14.0]),
        ],
    },
    Case {
        label: "s5_start_w20_wide_track100_rtl",
        markup: r##"<div id=g style="display:grid;grid-template-columns:100px;direction:rtl;"><table id=t style="width:20px;justify-self:start"><tr><td id=a style="padding:2px">BBBBBBBBBBBBBBBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 300.0, 18.0]),
            ("t", [132.0, 0.0, 168.0, 18.0]),
            ("a", [134.0, 2.0, 164.0, 14.0]),
        ],
    },
    Case {
        label: "s6_auto_both_w20_wide_track100",
        markup: r##"<div id=g style="display:grid;grid-template-columns:100px;"><table id=t style="width:20px;margin:0 auto"><tr><td id=a style="padding:2px">BBBBBBBBBBBBBBBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 300.0, 18.0]),
            ("t", [0.0, 0.0, 168.0, 18.0]),
            ("a", [2.0, 2.0, 164.0, 14.0]),
        ],
    },
    Case {
        label: "s6b_auto_left_w20_wide_track100",
        markup: r##"<div id=g style="display:grid;grid-template-columns:100px;"><table id=t style="width:20px;margin-left:auto"><tr><td id=a style="padding:2px">BBBBBBBBBBBBBBBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 300.0, 18.0]),
            ("t", [0.0, 0.0, 168.0, 18.0]),
            ("a", [2.0, 2.0, 164.0, 14.0]),
        ],
    },
    Case {
        label: "s6c_auto_right_w20_wide_track100",
        markup: r##"<div id=g style="display:grid;grid-template-columns:100px;"><table id=t style="width:20px;margin-right:auto"><tr><td id=a style="padding:2px">BBBBBBBBBBBBBBBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 300.0, 18.0]),
            ("t", [0.0, 0.0, 168.0, 18.0]),
            ("a", [2.0, 2.0, 164.0, 14.0]),
        ],
    },
    Case {
        label: "s7_safe_center_w20_mid_track100",
        markup: r##"<div id=g style="display:grid;grid-template-columns:100px;"><table id=t style="width:20px;justify-self:safe center"><tr><td id=a style="padding:2px">BBBBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 300.0, 18.0]),
            ("t", [21.0, 0.0, 58.0, 18.0]),
            ("a", [23.0, 2.0, 54.0, 14.0]),
        ],
    },
    Case {
        label: "s10_safe_center_margins_w20_wide_track100",
        markup: r##"<div id=g style="display:grid;grid-template-columns:100px;"><table id=t style="width:20px;margin:0 10px;justify-self:safe center"><tr><td id=a style="padding:2px">BBBBBBBBBBBBBBBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 300.0, 18.0]),
            ("t", [10.0, 0.0, 168.0, 18.0]),
            ("a", [12.0, 2.0, 164.0, 14.0]),
        ],
    },
    Case {
        label: "s10b_center_margins_w20_wide_track100",
        markup: r##"<div id=g style="display:grid;grid-template-columns:100px;"><table id=t style="width:20px;margin:0 10px;justify-self:center"><tr><td id=a style="padding:2px">BBBBBBBBBBBBBBBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 300.0, 18.0]),
            ("t", [-34.0, 0.0, 168.0, 18.0]),
            ("a", [-32.0, 2.0, 164.0, 14.0]),
        ],
    },
    Case {
        label: "s11_safe_center_w20_wide_track100_rtl_margins",
        markup: r##"<div id=g style="display:grid;grid-template-columns:100px;direction:rtl;"><table id=t style="width:20px;margin:0 10px 0 4px;justify-self:safe center"><tr><td id=a style="padding:2px">BBBBBBBBBBBBBBBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 300.0, 18.0]),
            ("t", [122.0, 0.0, 168.0, 18.0]),
            ("a", [124.0, 2.0, 164.0, 14.0]),
        ],
    },
    Case {
        label: "t1_safe_center_w400_maxc",
        markup: r##"<div id=g style="display:grid;grid-template-columns:300px;"><table id=t style="width:400px;max-width:max-content;justify-self:safe center"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 300.0, 18.0]),
            ("t", [131.0, 0.0, 38.0, 18.0]),
            ("a", [133.0, 2.0, 34.0, 14.0]),
        ],
    },
    Case {
        label: "t1_safe_end_w400_maxc",
        markup: r##"<div id=g style="display:grid;grid-template-columns:300px;"><table id=t style="width:400px;max-width:max-content;justify-self:safe end"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 300.0, 18.0]),
            ("t", [262.0, 0.0, 38.0, 18.0]),
            ("a", [264.0, 2.0, 34.0, 14.0]),
        ],
    },
    Case {
        label: "t1_unsafe_center_w400_maxc",
        markup: r##"<div id=g style="display:grid;grid-template-columns:300px;"><table id=t style="width:400px;max-width:max-content;justify-self:unsafe center"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 300.0, 18.0]),
            ("t", [131.0, 0.0, 38.0, 18.0]),
            ("a", [133.0, 2.0, 34.0, 14.0]),
        ],
    },
    Case {
        label: "t1_center_w400_maxc",
        markup: r##"<div id=g style="display:grid;grid-template-columns:300px;"><table id=t style="width:400px;max-width:max-content;justify-self:center"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 300.0, 18.0]),
            ("t", [131.0, 0.0, 38.0, 18.0]),
            ("a", [133.0, 2.0, 34.0, 14.0]),
        ],
    },
    Case {
        label: "t1_unsafe_end_w400_maxc",
        markup: r##"<div id=g style="display:grid;grid-template-columns:300px;"><table id=t style="width:400px;max-width:max-content;justify-self:unsafe end"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 300.0, 18.0]),
            ("t", [262.0, 0.0, 38.0, 18.0]),
            ("a", [264.0, 2.0, 34.0, 14.0]),
        ],
    },
    Case {
        label: "t1_safe_start_w400_maxc",
        markup: r##"<div id=g style="display:grid;grid-template-columns:300px;"><table id=t style="width:400px;max-width:max-content;justify-self:safe start"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 300.0, 18.0]),
            ("t", [0.0, 0.0, 38.0, 18.0]),
            ("a", [2.0, 2.0, 34.0, 14.0]),
        ],
    },
    Case {
        label: "t2_safe_center_w400_maxc_rtl",
        markup: r##"<div id=g style="display:grid;grid-template-columns:300px;direction:rtl;"><table id=t style="width:400px;max-width:max-content;justify-self:safe center"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 300.0, 18.0]),
            ("t", [131.0, 0.0, 38.0, 18.0]),
            ("a", [133.0, 2.0, 34.0, 14.0]),
        ],
    },
    Case {
        label: "t2_safe_end_w400_maxc_rtl",
        markup: r##"<div id=g style="display:grid;grid-template-columns:300px;direction:rtl;"><table id=t style="width:400px;max-width:max-content;justify-self:safe end"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 300.0, 18.0]),
            ("t", [0.0, 0.0, 38.0, 18.0]),
            ("a", [2.0, 2.0, 34.0, 14.0]),
        ],
    },
    Case {
        label: "t2_unsafe_end_w400_maxc_rtl",
        markup: r##"<div id=g style="display:grid;grid-template-columns:300px;direction:rtl;"><table id=t style="width:400px;max-width:max-content;justify-self:unsafe end"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 300.0, 18.0]),
            ("t", [0.0, 0.0, 38.0, 18.0]),
            ("a", [2.0, 2.0, 34.0, 14.0]),
        ],
    },
    Case {
        label: "t3_auto_both_w400_maxc",
        markup: r##"<div id=g style="display:grid;grid-template-columns:300px;"><table id=t style="width:400px;max-width:max-content;margin:0 auto"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 300.0, 18.0]),
            ("t", [131.0, 0.0, 38.0, 18.0]),
            ("a", [133.0, 2.0, 34.0, 14.0]),
        ],
    },
    Case {
        label: "t3b_auto_left_w400_maxc",
        markup: r##"<div id=g style="display:grid;grid-template-columns:300px;"><table id=t style="width:400px;max-width:max-content;margin-left:auto"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 300.0, 18.0]),
            ("t", [262.0, 0.0, 38.0, 18.0]),
            ("a", [264.0, 2.0, 34.0, 14.0]),
        ],
    },
    Case {
        label: "t4_items_safe_center_w400_maxc",
        markup: r##"<div id=g style="display:grid;grid-template-columns:300px;justify-items:safe center;"><table id=t style="width:400px;max-width:max-content"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 300.0, 18.0]),
            ("t", [131.0, 0.0, 38.0, 18.0]),
            ("a", [133.0, 2.0, 34.0, 14.0]),
        ],
    },
    Case {
        label: "t5_safe_center_margins_w400_maxc",
        markup: r##"<div id=g style="display:grid;grid-template-columns:300px;"><table id=t style="width:400px;max-width:max-content;margin:0 10px;justify-self:safe center"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 300.0, 18.0]),
            ("t", [131.0, 0.0, 38.0, 18.0]),
            ("a", [133.0, 2.0, 34.0, 14.0]),
        ],
    },
    Case {
        label: "t6_safe_center_margins_w400_maxc_rtl",
        markup: r##"<div id=g style="display:grid;grid-template-columns:300px;direction:rtl;"><table id=t style="width:400px;max-width:max-content;margin:0 10px 0 4px;justify-self:safe center"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 300.0, 18.0]),
            ("t", [128.0, 0.0, 38.0, 18.0]),
            ("a", [130.0, 2.0, 34.0, 14.0]),
        ],
    },
    Case {
        label: "t7_safe_center_w400_maxc_pos",
        markup: r##"<div id=g style="display:grid;grid-template-columns:300px;"><table id=t style="width:400px;max-width:max-content;justify-self:safe center;position:relative;left:7px"><tr><td id=a style="padding:2px">BBB</td></tr></table></div>"##,
        boxes: &[
            ("g", [0.0, 0.0, 300.0, 18.0]),
            ("t", [138.0, 0.0, 38.0, 18.0]),
            ("a", [140.0, 2.0, 34.0, 14.0]),
        ],
    },
];

#[test]
fn a_table_falls_back_or_stays_by_the_width_it_ends_up_with() {
    check(OVERFLOW);
}
