//! Whether an inline box with nothing in it makes a line when it has a margin
//! along the line.
//!
//! In standards mode it does, as for a padding or a border (CSS 2.1 section
//! 9.4.2): a block holding only `<span style="margin-left:5px"></span>` is one
//! line high. In quirks mode and limited-quirks mode a margin alone does not
//! count (Quirks Mode Standard, section 3.3, the line height calculation
//! quirk): the block is empty and takes no room, while a padding or a border
//! along the line still makes a line. The two modes give the same boxes here.
//!
//! The expected border boxes are the ones Chromium computes for the same
//! markup in an 800px by 600px viewport, with the body margin of 8px and a
//! 10px Ahem font. `o` is the box under test and `c` a 10px high box after it.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/data/text-autospace/Ahem.ttf"
));

/// A border box `(x, y, width, height)`.
type Rect = (f32, f32, f32, f32);

/// One markup and the boxes Chromium gives it in standards mode and in
/// quirks mode.
struct Case {
    name: &'static str,
    body: &'static str,
    standards: &'static [(&'static str, Rect)],
    quirks: &'static [(&'static str, Rect)],
}

const STANDARDS: &str = "<!DOCTYPE html>";
/// No doctype at all puts the document in quirks mode.
const QUIRKS: &str = "";
/// A transitional XHTML doctype with a system identifier puts it in
/// limited-quirks mode.
const LIMITED_QUIRKS: &str = "<!DOCTYPE html PUBLIC \"-//W3C//DTD XHTML 1.0 Transitional//EN\" \
     \"http://www.w3.org/TR/xhtml1/DTD/xhtml1-transitional.dtd\">";

/// The border boxes of the elements of `body` with an `id` on the first page.
fn border_boxes(doctype: &str, body: &str) -> Vec<(String, Rect)> {
    let html = format!(
        "{doctype}<style>@page {{ size: 800px 600px; margin: 0 }} \
         body {{ margin: 8px; font: 10px/10px Ahem }} div {{ box-sizing: border-box }}</style>{body}"
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
            Some((id.to_owned(), (rect.x, rect.y, rect.width, rect.height)))
        })
        .collect()
}

/// Layout units are 1/64px; allow a little more for the 32-bit sums.
const TOLERANCE: f32 = 0.02;

/// Lay out every case under `doctype` and report all the boxes that differ.
fn check_all(doctype: &str, expected: impl Fn(&Case) -> &'static [(&'static str, Rect)]) {
    let mut failures = Vec::new();
    for case in CASES {
        let actual = border_boxes(doctype, case.body);
        for &(id, (x, y, width, height)) in expected(case) {
            let found = actual.iter().find(|(name, _)| name == id).map(|(_, r)| *r);
            let close = |a: f32, b: f32| (a - b).abs() <= TOLERANCE;
            if !found.is_some_and(|f| {
                close(f.0, x) && close(f.1, y) && close(f.2, width) && close(f.3, height)
            }) {
                failures.push(format!(
                    "{}: #{id} is {found:?}, expected {:?}",
                    case.name,
                    (x, y, width, height)
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn standards_mode_a_side_margin_makes_a_line() {
    check_all(STANDARDS, |case| case.standards);
}

#[test]
fn quirks_mode_a_side_margin_alone_makes_no_line() {
    check_all(QUIRKS, |case| case.quirks);
}

#[test]
fn limited_quirks_mode_matches_quirks_mode() {
    check_all(LIMITED_QUIRKS, |case| case.quirks);
}

fn check_empty_generated(doctype: &str, quirks: bool) {
    let cases = [
        ("", 0.0, 0.0),
        ("padding-left:5px", 10.0, 10.0),
        ("padding-right:5px", 10.0, 10.0),
        ("border-left:3px solid", 10.0, 10.0),
        ("margin-left:5px", 10.0, 0.0),
        ("margin-left:-5px", 10.0, 0.0),
        ("padding-left:5%", 10.0, 10.0),
        ("padding-left:calc(5% + 1px)", 10.0, 10.0),
        ("padding-top:5px", 0.0, 0.0),
        ("border-top:3px solid", 0.0, 0.0),
        ("display:inline-block;width:5px;height:0", 10.0, 0.0),
        ("display:contents;padding-left:5px", 0.0, 0.0),
        ("display:none;padding-left:5px", 0.0, 0.0),
    ];
    let mut failures = Vec::new();
    for pseudo in ["before", "after"] {
        for (style, standards_height, quirks_height) in cases {
            let height = if quirks {
                quirks_height
            } else {
                standards_height
            };
            let body = format!(
                "<style>#o::{pseudo}{{content:\"\";{style}}}</style>\
                 <div id=o style='width:100px'></div><div id=c style='height:10px'></div>"
            );
            let actual = border_boxes(doctype, &body);
            for (id, expected) in [
                ("o", (8.0, 8.0, 100.0, height)),
                ("c", (8.0, 8.0 + height, 784.0, 10.0)),
            ] {
                let found = actual.iter().find(|(name, _)| name == id).map(|(_, r)| *r);
                if !found.is_some_and(|rect| {
                    [
                        rect.0 - expected.0,
                        rect.1 - expected.1,
                        rect.2 - expected.2,
                        rect.3 - expected.3,
                    ]
                    .iter()
                    .all(|difference| difference.abs() <= TOLERANCE)
                }) {
                    failures.push(format!(
                        "::{pseudo} {style}: #{id} is {found:?}, expected {expected:?}"
                    ));
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn standards_mode_empty_generated_boxes_keep_their_edges() {
    check_empty_generated(STANDARDS, false);
}

#[test]
fn quirks_mode_empty_generated_boxes_keep_their_edges() {
    check_empty_generated(QUIRKS, true);
}

#[test]
fn limited_quirks_mode_empty_generated_boxes_keep_their_edges() {
    check_empty_generated(LIMITED_QUIRKS, true);
}

const CASES: &[Case] = &[
    Case {
        name: "ml",
        body: r#"<div id=o style="width:100px"><span style="margin-left:5px"></span></div><div id=c style="height:10px"></div>"#,
        standards: &[
            ("o", (8.0, 8.0, 100.0, 10.0)),
            ("c", (8.0, 18.0, 784.0, 10.0)),
        ],
        quirks: &[
            ("o", (8.0, 8.0, 100.0, 0.0)),
            ("c", (8.0, 8.0, 784.0, 10.0)),
        ],
    },
    Case {
        name: "mr",
        body: r#"<div id=o style="width:100px"><span style="margin-right:5px"></span></div><div id=c style="height:10px"></div>"#,
        standards: &[
            ("o", (8.0, 8.0, 100.0, 10.0)),
            ("c", (8.0, 18.0, 784.0, 10.0)),
        ],
        quirks: &[
            ("o", (8.0, 8.0, 100.0, 0.0)),
            ("c", (8.0, 8.0, 784.0, 10.0)),
        ],
    },
    Case {
        name: "mboth",
        body: r#"<div id=o style="width:100px"><span style="margin:0 5px"></span></div><div id=c style="height:10px"></div>"#,
        standards: &[
            ("o", (8.0, 8.0, 100.0, 10.0)),
            ("c", (8.0, 18.0, 784.0, 10.0)),
        ],
        quirks: &[
            ("o", (8.0, 8.0, 100.0, 0.0)),
            ("c", (8.0, 8.0, 784.0, 10.0)),
        ],
    },
    Case {
        name: "mneg",
        body: r#"<div id=o style="width:100px"><span style="margin-left:-5px"></span></div><div id=c style="height:10px"></div>"#,
        standards: &[
            ("o", (8.0, 8.0, 100.0, 10.0)),
            ("c", (8.0, 18.0, 784.0, 10.0)),
        ],
        quirks: &[
            ("o", (8.0, 8.0, 100.0, 0.0)),
            ("c", (8.0, 8.0, 784.0, 10.0)),
        ],
    },
    Case {
        name: "mpercent",
        body: r#"<div id=o style="width:100px"><span style="margin-left:10%"></span></div><div id=c style="height:10px"></div>"#,
        standards: &[
            ("o", (8.0, 8.0, 100.0, 10.0)),
            ("c", (8.0, 18.0, 784.0, 10.0)),
        ],
        quirks: &[
            ("o", (8.0, 8.0, 100.0, 0.0)),
            ("c", (8.0, 8.0, 784.0, 10.0)),
        ],
    },
    Case {
        name: "pl",
        body: r#"<div id=o style="width:100px"><span style="padding-left:5px"></span></div><div id=c style="height:10px"></div>"#,
        standards: &[
            ("o", (8.0, 8.0, 100.0, 10.0)),
            ("c", (8.0, 18.0, 784.0, 10.0)),
        ],
        quirks: &[
            ("o", (8.0, 8.0, 100.0, 10.0)),
            ("c", (8.0, 18.0, 784.0, 10.0)),
        ],
    },
    Case {
        name: "bl",
        body: r#"<div id=o style="width:100px"><span style="border-left:5px solid"></span></div><div id=c style="height:10px"></div>"#,
        standards: &[
            ("o", (8.0, 8.0, 100.0, 10.0)),
            ("c", (8.0, 18.0, 784.0, 10.0)),
        ],
        quirks: &[
            ("o", (8.0, 8.0, 100.0, 10.0)),
            ("c", (8.0, 18.0, 784.0, 10.0)),
        ],
    },
    Case {
        name: "ml_pl",
        body: r#"<div id=o style="width:100px"><span style="margin-left:5px;padding-left:5px"></span></div><div id=c style="height:10px"></div>"#,
        standards: &[
            ("o", (8.0, 8.0, 100.0, 10.0)),
            ("c", (8.0, 18.0, 784.0, 10.0)),
        ],
        quirks: &[
            ("o", (8.0, 8.0, 100.0, 10.0)),
            ("c", (8.0, 18.0, 784.0, 10.0)),
        ],
    },
    Case {
        name: "nested_ml",
        body: r#"<div id=o style="width:100px"><span style="margin-left:5px"><span style="margin-left:5px"></span></span></div><div id=c style="height:10px"></div>"#,
        standards: &[
            ("o", (8.0, 8.0, 100.0, 10.0)),
            ("c", (8.0, 18.0, 784.0, 10.0)),
        ],
        quirks: &[
            ("o", (8.0, 8.0, 100.0, 0.0)),
            ("c", (8.0, 8.0, 784.0, 10.0)),
        ],
    },
    Case {
        name: "nested_ml_pl_inner",
        body: r#"<div id=o style="width:100px"><span style="margin-left:5px"><span style="padding-left:5px"></span></span></div><div id=c style="height:10px"></div>"#,
        standards: &[
            ("o", (8.0, 8.0, 100.0, 10.0)),
            ("c", (8.0, 18.0, 784.0, 10.0)),
        ],
        quirks: &[
            ("o", (8.0, 8.0, 100.0, 10.0)),
            ("c", (8.0, 18.0, 784.0, 10.0)),
        ],
    },
    Case {
        name: "two_spans",
        body: r#"<div id=o style="width:100px"><span style="margin-left:5px"></span><span style="margin-right:5px"></span></div><div id=c style="height:10px"></div>"#,
        standards: &[
            ("o", (8.0, 8.0, 100.0, 10.0)),
            ("c", (8.0, 18.0, 784.0, 10.0)),
        ],
        quirks: &[
            ("o", (8.0, 8.0, 100.0, 0.0)),
            ("c", (8.0, 8.0, 784.0, 10.0)),
        ],
    },
    Case {
        name: "ws_between",
        body: r#"<div id=o style="width:100px"><span style="margin-left:5px"></span> <span style="margin-right:5px"></span></div><div id=c style="height:10px"></div>"#,
        standards: &[
            ("o", (8.0, 8.0, 100.0, 10.0)),
            ("c", (8.0, 18.0, 784.0, 10.0)),
        ],
        quirks: &[
            ("o", (8.0, 8.0, 100.0, 0.0)),
            ("c", (8.0, 8.0, 784.0, 10.0)),
        ],
    },
    Case {
        name: "text_before",
        body: r#"<div id=o style="width:100px">x<span style="margin-left:5px"></span></div><div id=c style="height:10px"></div>"#,
        standards: &[
            ("o", (8.0, 8.0, 100.0, 10.0)),
            ("c", (8.0, 18.0, 784.0, 10.0)),
        ],
        quirks: &[
            ("o", (8.0, 8.0, 100.0, 10.0)),
            ("c", (8.0, 18.0, 784.0, 10.0)),
        ],
    },
    Case {
        name: "text_after",
        body: r#"<div id=o style="width:100px"><span style="margin-left:5px"></span>y</div><div id=c style="height:10px"></div>"#,
        standards: &[
            ("o", (8.0, 8.0, 100.0, 10.0)),
            ("c", (8.0, 18.0, 784.0, 10.0)),
        ],
        quirks: &[
            ("o", (8.0, 8.0, 100.0, 10.0)),
            ("c", (8.0, 18.0, 784.0, 10.0)),
        ],
    },
    Case {
        name: "ws_after",
        body: r#"<div id=o style="width:100px"><span style="margin-left:5px"></span> </div><div id=c style="height:10px"></div>"#,
        standards: &[
            ("o", (8.0, 8.0, 100.0, 10.0)),
            ("c", (8.0, 18.0, 784.0, 10.0)),
        ],
        quirks: &[
            ("o", (8.0, 8.0, 100.0, 0.0)),
            ("c", (8.0, 8.0, 784.0, 10.0)),
        ],
    },
    Case {
        name: "br_after",
        body: r#"<div id=o style="width:100px"><span style="margin-left:5px"></span><br></div><div id=c style="height:10px"></div>"#,
        standards: &[
            ("o", (8.0, 8.0, 100.0, 10.0)),
            ("c", (8.0, 18.0, 784.0, 10.0)),
        ],
        quirks: &[
            ("o", (8.0, 8.0, 100.0, 10.0)),
            ("c", (8.0, 18.0, 784.0, 10.0)),
        ],
    },
    Case {
        name: "nbsp_inside",
        body: r#"<div id=o style="width:100px"><span style="margin-left:5px">&nbsp;</span></div><div id=c style="height:10px"></div>"#,
        standards: &[
            ("o", (8.0, 8.0, 100.0, 10.0)),
            ("c", (8.0, 18.0, 784.0, 10.0)),
        ],
        quirks: &[
            ("o", (8.0, 8.0, 100.0, 10.0)),
            ("c", (8.0, 18.0, 784.0, 10.0)),
        ],
    },
    Case {
        name: "space_inside",
        body: r#"<div id=o style="width:100px"><span style="margin-left:5px"> </span></div><div id=c style="height:10px"></div>"#,
        standards: &[
            ("o", (8.0, 8.0, 100.0, 10.0)),
            ("c", (8.0, 18.0, 784.0, 10.0)),
        ],
        quirks: &[
            ("o", (8.0, 8.0, 100.0, 0.0)),
            ("c", (8.0, 8.0, 784.0, 10.0)),
        ],
    },
    Case {
        name: "inline_block_empty",
        body: r#"<div id=o style="width:100px"><span style="display:inline-block;margin-left:5px"></span></div><div id=c style="height:10px"></div>"#,
        standards: &[
            ("o", (8.0, 8.0, 100.0, 10.0)),
            ("c", (8.0, 18.0, 784.0, 10.0)),
        ],
        quirks: &[
            ("o", (8.0, 8.0, 100.0, 0.0)),
            ("c", (8.0, 8.0, 784.0, 10.0)),
        ],
    },
    Case {
        name: "in_float_w",
        body: r#"<div id=o style="float:left"><span style="margin-left:5px"></span></div><div id=c style="clear:both;height:10px"></div>"#,
        standards: &[
            ("o", (8.0, 8.0, 5.0, 10.0)),
            ("c", (8.0, 18.0, 784.0, 10.0)),
        ],
        quirks: &[("o", (8.0, 8.0, 5.0, 0.0)), ("c", (8.0, 8.0, 784.0, 10.0))],
    },
    Case {
        name: "in_ib_w",
        body: r#"<div id=o style="display:inline-block"><span style="margin-left:5px"></span></div>"#,
        standards: &[("o", (8.0, 8.0, 5.0, 10.0))],
        quirks: &[("o", (8.0, 8.0, 5.0, 0.0))],
    },
    Case {
        name: "in_ib_w2",
        body: r#"<div id=o style="display:inline-block"><span style="margin:0 5px"></span></div>"#,
        standards: &[("o", (8.0, 8.0, 10.0, 10.0))],
        quirks: &[("o", (8.0, 8.0, 10.0, 0.0))],
    },
    Case {
        name: "in_float_nested",
        body: r#"<div id=o style="float:left"><span style="margin-left:5px"><span style="margin-right:7px"></span></span></div><div id=c style="clear:both;height:10px"></div>"#,
        standards: &[
            ("o", (8.0, 8.0, 12.0, 10.0)),
            ("c", (8.0, 18.0, 784.0, 10.0)),
        ],
        quirks: &[("o", (8.0, 8.0, 12.0, 0.0)), ("c", (8.0, 8.0, 784.0, 10.0))],
    },
    Case {
        name: "in_cell",
        body: r#"<table style="border-spacing:0"><tr><td id=o style="padding:0"><span style="margin-left:5px"></span></td></tr></table>"#,
        standards: &[("o", (8.0, 8.0, 5.0, 10.0))],
        quirks: &[("o", (8.0, 8.0, 5.0, 0.0))],
    },
    Case {
        name: "before_block",
        body: r#"<div id=o style="width:100px"><span style="margin-left:5px"></span><div id=b style="height:10px"></div></div><div id=c style="height:10px"></div>"#,
        standards: &[
            ("o", (8.0, 8.0, 100.0, 20.0)),
            ("b", (8.0, 18.0, 100.0, 10.0)),
            ("c", (8.0, 28.0, 784.0, 10.0)),
        ],
        quirks: &[
            ("o", (8.0, 8.0, 100.0, 10.0)),
            ("b", (8.0, 8.0, 100.0, 10.0)),
            ("c", (8.0, 18.0, 784.0, 10.0)),
        ],
    },
    Case {
        name: "after_block",
        body: r#"<div id=o style="width:100px"><div id=b style="height:10px"></div><span style="margin-left:5px"></span></div><div id=c style="height:10px"></div>"#,
        standards: &[
            ("o", (8.0, 8.0, 100.0, 20.0)),
            ("b", (8.0, 8.0, 100.0, 10.0)),
            ("c", (8.0, 28.0, 784.0, 10.0)),
        ],
        quirks: &[
            ("o", (8.0, 8.0, 100.0, 10.0)),
            ("b", (8.0, 8.0, 100.0, 10.0)),
            ("c", (8.0, 18.0, 784.0, 10.0)),
        ],
    },
    Case {
        name: "block_in_span",
        body: r#"<div id=o style="width:100px"><span style="margin-left:5px"><div id=b style="height:10px"></div></span></div><div id=c style="height:10px"></div>"#,
        standards: &[
            ("o", (8.0, 8.0, 100.0, 20.0)),
            ("b", (8.0, 18.0, 100.0, 10.0)),
            ("c", (8.0, 28.0, 784.0, 10.0)),
        ],
        quirks: &[
            ("o", (8.0, 8.0, 100.0, 10.0)),
            ("b", (8.0, 8.0, 100.0, 10.0)),
            ("c", (8.0, 18.0, 784.0, 10.0)),
        ],
    },
    Case {
        name: "rtl",
        body: r#"<div id=o style="width:100px;direction:rtl"><span style="margin-left:5px"></span></div><div id=c style="height:10px"></div>"#,
        standards: &[
            ("o", (8.0, 8.0, 100.0, 10.0)),
            ("c", (8.0, 18.0, 784.0, 10.0)),
        ],
        quirks: &[
            ("o", (8.0, 8.0, 100.0, 0.0)),
            ("c", (8.0, 8.0, 784.0, 10.0)),
        ],
    },
    Case {
        name: "vertical_margin",
        body: r#"<div id=o style="width:100px"><span style="margin:5px 0"></span></div><div id=c style="height:10px"></div>"#,
        standards: &[
            ("o", (8.0, 8.0, 100.0, 0.0)),
            ("c", (8.0, 8.0, 784.0, 10.0)),
        ],
        quirks: &[
            ("o", (8.0, 8.0, 100.0, 0.0)),
            ("c", (8.0, 8.0, 784.0, 10.0)),
        ],
    },
    Case {
        name: "ml_with_pt",
        body: r#"<div id=o style="width:100px"><span style="margin-left:5px;padding-top:5px"></span></div><div id=c style="height:10px"></div>"#,
        standards: &[
            ("o", (8.0, 8.0, 100.0, 10.0)),
            ("c", (8.0, 18.0, 784.0, 10.0)),
        ],
        quirks: &[
            ("o", (8.0, 8.0, 100.0, 0.0)),
            ("c", (8.0, 8.0, 784.0, 10.0)),
        ],
    },
    Case {
        name: "ml_with_sibling_empty_p",
        body: r#"<div id=o style="width:100px"><span style="margin-left:5px"></span><span></span></div><div id=c style="height:10px"></div>"#,
        standards: &[
            ("o", (8.0, 8.0, 100.0, 10.0)),
            ("c", (8.0, 18.0, 784.0, 10.0)),
        ],
        quirks: &[
            ("o", (8.0, 8.0, 100.0, 0.0)),
            ("c", (8.0, 8.0, 784.0, 10.0)),
        ],
    },
    Case {
        name: "empty_a",
        body: r#"<div id=o style="width:100px"><a style="margin-left:5px"></a></div><div id=c style="height:10px"></div>"#,
        standards: &[
            ("o", (8.0, 8.0, 100.0, 10.0)),
            ("c", (8.0, 18.0, 784.0, 10.0)),
        ],
        quirks: &[
            ("o", (8.0, 8.0, 100.0, 0.0)),
            ("c", (8.0, 8.0, 784.0, 10.0)),
        ],
    },
    Case {
        name: "em_ml",
        body: r#"<div id=o style="width:100px"><span style="margin-left:1em"></span></div><div id=c style="height:10px"></div>"#,
        standards: &[
            ("o", (8.0, 8.0, 100.0, 10.0)),
            ("c", (8.0, 18.0, 784.0, 10.0)),
        ],
        quirks: &[
            ("o", (8.0, 8.0, 100.0, 0.0)),
            ("c", (8.0, 8.0, 784.0, 10.0)),
        ],
    },
    Case {
        name: "ml_auto",
        body: r#"<div id=o style="width:100px"><span style="margin-left:auto"></span></div><div id=c style="height:10px"></div>"#,
        standards: &[
            ("o", (8.0, 8.0, 100.0, 0.0)),
            ("c", (8.0, 8.0, 784.0, 10.0)),
        ],
        quirks: &[
            ("o", (8.0, 8.0, 100.0, 0.0)),
            ("c", (8.0, 8.0, 784.0, 10.0)),
        ],
    },
    Case {
        name: "lineheight_big",
        body: r#"<div id=o style="width:100px"><span style="margin-left:5px;line-height:50px"></span></div><div id=c style="height:10px"></div>"#,
        standards: &[
            ("o", (8.0, 8.0, 100.0, 50.0)),
            ("c", (8.0, 58.0, 784.0, 10.0)),
        ],
        quirks: &[
            ("o", (8.0, 8.0, 100.0, 0.0)),
            ("c", (8.0, 8.0, 784.0, 10.0)),
        ],
    },
    Case {
        name: "fs_big",
        body: r#"<div id=o style="width:100px"><span style="margin-left:5px;font-size:30px"></span></div><div id=c style="height:10px"></div>"#,
        standards: &[
            ("o", (8.0, 8.0, 100.0, 16.0)),
            ("c", (8.0, 24.0, 784.0, 10.0)),
        ],
        quirks: &[
            ("o", (8.0, 8.0, 100.0, 0.0)),
            ("c", (8.0, 8.0, 784.0, 10.0)),
        ],
    },
    Case {
        name: "li_outside_none_ml",
        body: r#"<div id=o style="display:list-item;width:100px;list-style:none"><span style="margin-left:5px"></span></div><div id=c style="height:10px"></div>"#,
        standards: &[
            ("o", (8.0, 8.0, 100.0, 10.0)),
            ("c", (8.0, 18.0, 784.0, 10.0)),
        ],
        quirks: &[
            ("o", (8.0, 8.0, 100.0, 0.0)),
            ("c", (8.0, 8.0, 784.0, 10.0)),
        ],
    },
    Case {
        name: "li_outside_none_empty",
        body: r#"<div id=o style="display:list-item;width:100px;list-style:none"></div><div id=c style="height:10px"></div>"#,
        standards: &[
            ("o", (8.0, 8.0, 100.0, 0.0)),
            ("c", (8.0, 8.0, 784.0, 10.0)),
        ],
        quirks: &[
            ("o", (8.0, 8.0, 100.0, 0.0)),
            ("c", (8.0, 8.0, 784.0, 10.0)),
        ],
    },
    Case {
        name: "li_inside_empty",
        body: r#"<div id=o style="display:list-item;list-style-position:inside;width:100px;"></div><div id=c style="height:10px"></div>"#,
        standards: &[
            ("o", (8.0, 8.0, 100.0, 10.0)),
            ("c", (8.0, 18.0, 784.0, 10.0)),
        ],
        quirks: &[
            ("o", (8.0, 8.0, 100.0, 10.0)),
            ("c", (8.0, 18.0, 784.0, 10.0)),
        ],
    },
    Case {
        name: "li_inside_ml",
        body: r#"<div id=o style="display:list-item;list-style-position:inside;width:100px;"><span style="margin-left:5px"></span></div><div id=c style="height:10px"></div>"#,
        standards: &[
            ("o", (8.0, 8.0, 100.0, 10.0)),
            ("c", (8.0, 18.0, 784.0, 10.0)),
        ],
        quirks: &[
            ("o", (8.0, 8.0, 100.0, 10.0)),
            ("c", (8.0, 18.0, 784.0, 10.0)),
        ],
    },
    Case {
        name: "li_outside_pl",
        body: r#"<div id=o style="display:list-item;width:100px;"><span style="padding-left:5px"></span></div><div id=c style="height:10px"></div>"#,
        standards: &[
            ("o", (8.0, 8.0, 100.0, 10.0)),
            ("c", (8.0, 18.0, 784.0, 10.0)),
        ],
        quirks: &[
            ("o", (8.0, 8.0, 100.0, 10.0)),
            ("c", (8.0, 18.0, 784.0, 10.0)),
        ],
    },
    Case {
        name: "li_outside_block",
        body: r#"<div id=o style="display:list-item;width:100px;"><div id=b style="height:10px"></div></div><div id=c style="height:10px"></div>"#,
        standards: &[
            ("o", (8.0, 8.0, 100.0, 10.0)),
            ("b", (8.0, 8.0, 100.0, 10.0)),
            ("c", (8.0, 18.0, 784.0, 10.0)),
        ],
        quirks: &[
            ("o", (8.0, 8.0, 100.0, 10.0)),
            ("b", (8.0, 8.0, 100.0, 10.0)),
            ("c", (8.0, 18.0, 784.0, 10.0)),
        ],
    },
    Case {
        name: "li_outside_span_block",
        body: r#"<div id=o style="display:list-item;width:100px;"><span style="margin-left:5px"></span><div id=b style="height:10px"></div></div><div id=c style="height:10px"></div>"#,
        standards: &[
            ("o", (8.0, 8.0, 100.0, 20.0)),
            ("b", (8.0, 18.0, 100.0, 10.0)),
            ("c", (8.0, 28.0, 784.0, 10.0)),
        ],
        quirks: &[
            ("o", (8.0, 8.0, 100.0, 10.0)),
            ("b", (8.0, 8.0, 100.0, 10.0)),
            ("c", (8.0, 18.0, 784.0, 10.0)),
        ],
    },
];

#[test]
fn an_empty_generated_prefix_keeps_the_following_first_letter() {
    let body = "<style>#o::before{content:'';padding-left:5px}\
        #o::first-letter{font-size:20px;line-height:20px}</style>\
        <div id=o style='width:100px'>x</div><div id=c style='height:10px'></div>";
    assert_eq!(
        border_boxes(STANDARDS, body),
        vec![
            ("o".to_owned(), (8.0, 8.0, 100.0, 20.0)),
            ("c".to_owned(), (8.0, 28.0, 784.0, 10.0)),
        ]
    );
}
