//! The line that the outside marker of a list item makes.
//!
//! The marker of a list item sits outside its content, on the item's first
//! line (CSS Lists 3, section 3.1), so an item that holds nothing else is one
//! line high: an empty `<li>`, one with only white space, a comment, an empty
//! inline element, a float or an absolutely positioned box, and an item
//! (`display: list-item`) that is not an `<li>`. Without a marker box
//! (`list-style: none`, `::marker { content: none }`) the item is empty and
//! takes no room, and an inside marker joins the item's own line as before.
//! The line has the item's own strut, so its height follows `line-height`
//! and `font-size`, and the item still grows by its padding, border and
//! `min-height`.
//!
//! The expected border boxes are the ones Chromium computes for the same
//! markup in an 800px by 600px viewport, with the body margin of 8px and a
//! 10px Ahem font, in standards mode. The quirks and limited-quirks modes give
//! the same boxes unless a case says otherwise. `o` is the box under test and
//! `c` a 10px high box after the list.

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

/// One markup and the boxes Chromium gives it in standards mode.
struct Case {
    name: &'static str,
    body: &'static str,
    /// Whether quirks mode and limited-quirks mode give the same boxes.
    all_modes: bool,
    expected: &'static [(&'static str, Rect)],
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

/// Lay out the cases that `applies` selects under `doctype` and report all the
/// boxes that differ.
fn check_all(doctype: &str, applies: impl Fn(&Case) -> bool) {
    let mut failures = Vec::new();
    for case in CASES.iter().filter(|case| applies(case)) {
        let actual = border_boxes(doctype, case.body);
        for &(id, (x, y, width, height)) in case.expected {
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
fn standards_mode_an_outside_marker_makes_a_line() {
    check_all(STANDARDS, |_| true);
}

#[test]
fn quirks_mode_matches_standards_mode() {
    check_all(QUIRKS, |case| case.all_modes);
}

#[test]
fn limited_quirks_mode_matches_standards_mode() {
    check_all(LIMITED_QUIRKS, |case| case.all_modes);
}

/// The page, from 0, and the top and height on that page's area of the first
/// fragment of each element with an `id`.
type Placement = (String, usize, f32, f32);

/// Where the elements with an `id` go on pages of 480px by 288px with 48px
/// margins, in a document of 10px lines.
fn paged_boxes(body: &str) -> Vec<Placement> {
    let html = format!(
        "<!DOCTYPE html><style>@page {{ size: 480px 288px; margin: 48px }} \
         body {{ margin: 0; line-height: 10px }} ul {{ margin: 0; padding: 0 }}</style>{body}"
    );
    let resources = RenderResources::new();
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
    let mut found: Vec<Placement> = Vec::new();
    for (index, page) in result.pages().enumerate() {
        let dom = page.dom();
        for fragment in page.fragments() {
            let Some(id) = dom.attr(fragment.node(), "id") else {
                continue;
            };
            if found.iter().all(|(name, ..)| name != id) {
                let rect = fragment.rect();
                found.push((id.to_owned(), index, rect.y - 48.0, rect.height));
            }
        }
    }
    found
}

/// Lays `template` out twice, with `@ITEM@` standing for an empty list item
/// in a list without padding and for a block 10px high, and returns both
/// placements. The two must agree: an empty list item is a box one line high.
fn item_and_block(template: &str) -> (Vec<Placement>, Vec<Placement>) {
    let item = template
        .replace("@ITEM@", "li")
        .replace("@STYLE@", "")
        .replace("@OPEN@", "<ul>")
        .replace("@CLOSE@", "</ul>");
    let block = template
        .replace("@ITEM@", "div")
        .replace("@STYLE@", "height:10px;")
        .replace("@OPEN@", "")
        .replace("@CLOSE@", "");
    (paged_boxes(&item), paged_boxes(&block))
}

fn assert_same_placement(template: &str) -> Vec<Placement> {
    let (item, block) = item_and_block(template);
    assert_eq!(item.len(), block.len(), "{template}");
    for (a, b) in item.iter().zip(&block) {
        assert!(
            a.0 == b.0 && a.1 == b.1 && (a.2 - b.2).abs() <= 0.04 && (a.3 - b.3).abs() <= 0.04,
            "{template}: #{} is on page {} at {} high {} as a list item and on page {} at {} \
             high {} as a block 10px high",
            a.0,
            a.1,
            a.2,
            a.3,
            b.1,
            b.2,
            b.3
        );
    }
    item
}

/// A stack of empty items fills a page and goes on to the next one, a line at
/// a time.
#[test]
fn empty_items_fill_pages_a_line_at_a_time() {
    // The area is 192px high: with the 2px block ahead, nineteen lines fill it
    // exactly, so no item falls on the edge.
    let mut template = String::from("<div id=s style='height:2px'></div>@OPEN@");
    for index in 0..25 {
        template.push_str(&format!("<@ITEM@ id=i{index} style='@STYLE@'></@ITEM@>"));
    }
    template.push_str("@CLOSE@");
    let placed = assert_same_placement(&template);
    let at = |name: &str| placed.iter().find(|p| p.0 == name).expect(name);
    assert_eq!((at("i18").1, at("i18").2, at("i18").3), (0, 182.0, 10.0));
    assert_eq!((at("i19").1, at("i19").2, at("i19").3), (1, 0.0, 10.0));
}

/// A forced break before or after an empty item acts on the box of its line.
#[test]
fn forced_breaks_around_an_empty_item() {
    let template = "<div id=a style='height:10px'></div>@OPEN@\
                    <@ITEM@ id=x style='@STYLE@break-before:page'></@ITEM@>\
                    <@ITEM@ id=y style='@STYLE@break-after:page'></@ITEM@>\
                    <@ITEM@ id=z style='@STYLE@'></@ITEM@>@CLOSE@";
    let placed = assert_same_placement(template);
    let at = |name: &str| placed.iter().find(|p| p.0 == name).expect(name);
    assert_eq!((at("x").1, at("x").2), (1, 0.0));
    assert_eq!((at("y").1, at("y").2), (1, 10.0));
    assert_eq!((at("z").1, at("z").2), (2, 0.0));
}

/// The margins of an empty item do not collapse through it, as they would
/// through an empty block.
#[test]
fn the_margins_of_an_empty_item_stay_apart() {
    let template = "<div id=a style='height:10px'></div>@OPEN@\
                    <@ITEM@ id=x style='@STYLE@margin:20px 0'></@ITEM@>@CLOSE@\
                    <div id=d style='height:10px'></div>";
    let placed = assert_same_placement(template);
    let at = |name: &str| placed.iter().find(|p| p.0 == name).expect(name);
    assert_eq!((at("x").2, at("x").3), (30.0, 10.0));
    assert_eq!(at("d").2, 60.0);
}

const CASES: &[Case] = &[
    Case {
        name: "empty",
        body: "<ul style=\"margin:0;padding:0 0 0 40px\"><li id=o ></li></ul><div id=c style=\"height:10px\"></div>",
        all_modes: true,
        expected: &[
            ("o", (48.0, 8.0, 744.0, 10.0)),
            ("c", (8.0, 18.0, 784.0, 10.0)),
        ],
    },
    Case {
        name: "two_empty",
        body: "<ul style=\"margin:0;padding:0 0 0 40px\"><li id=o></li><li id=p></li></ul><div id=c style=\"height:10px\"></div>",
        all_modes: true,
        expected: &[
            ("o", (48.0, 8.0, 744.0, 10.0)),
            ("p", (48.0, 18.0, 744.0, 10.0)),
            ("c", (8.0, 28.0, 784.0, 10.0)),
        ],
    },
    Case {
        name: "three_empty_ol",
        body: "<ol style=\"margin:0;padding:0 0 0 40px\"><li id=o></li><li id=p></li><li id=q></li></ol><div id=c style=\"height:10px\"></div>",
        all_modes: true,
        expected: &[
            ("o", (48.0, 8.0, 744.0, 10.0)),
            ("p", (48.0, 18.0, 744.0, 10.0)),
            ("q", (48.0, 28.0, 744.0, 10.0)),
            ("c", (8.0, 38.0, 784.0, 10.0)),
        ],
    },
    Case {
        name: "whitespace",
        body: "<ul style=\"margin:0;padding:0 0 0 40px\"><li id=o > \n  </li></ul><div id=c style=\"height:10px\"></div>",
        all_modes: true,
        expected: &[
            ("o", (48.0, 8.0, 744.0, 10.0)),
            ("c", (8.0, 18.0, 784.0, 10.0)),
        ],
    },
    Case {
        name: "comment_only",
        body: "<ul style=\"margin:0;padding:0 0 0 40px\"><li id=o ><!-- x --></li></ul><div id=c style=\"height:10px\"></div>",
        all_modes: true,
        expected: &[
            ("o", (48.0, 8.0, 744.0, 10.0)),
            ("c", (8.0, 18.0, 784.0, 10.0)),
        ],
    },
    Case {
        name: "span_only",
        body: "<ul style=\"margin:0;padding:0 0 0 40px\"><li id=o ><span></span></li></ul><div id=c style=\"height:10px\"></div>",
        all_modes: true,
        expected: &[
            ("o", (48.0, 8.0, 744.0, 10.0)),
            ("c", (8.0, 18.0, 784.0, 10.0)),
        ],
    },
    Case {
        name: "empty_text_span",
        body: "<ul style=\"margin:0;padding:0 0 0 40px\"><li id=o ><span> </span></li></ul><div id=c style=\"height:10px\"></div>",
        all_modes: true,
        expected: &[
            ("o", (48.0, 8.0, 744.0, 10.0)),
            ("c", (8.0, 18.0, 784.0, 10.0)),
        ],
    },
    Case {
        name: "div_list_item",
        body: "<div id=o style=\"display:list-item;list-style:disc\"></div><div id=c style=\"height:10px\"></div>",
        all_modes: true,
        expected: &[
            ("o", (8.0, 8.0, 784.0, 10.0)),
            ("c", (8.0, 18.0, 784.0, 10.0)),
        ],
    },
    Case {
        name: "div_list_item_inside",
        body: "<div id=o style=\"display:list-item;list-style:disc inside\"></div><div id=c style=\"height:10px\"></div>",
        all_modes: true,
        expected: &[
            ("o", (8.0, 8.0, 784.0, 10.0)),
            ("c", (8.0, 18.0, 784.0, 10.0)),
        ],
    },
    Case {
        name: "list_style_none",
        body: "<ul style=\"margin:0;padding:0 0 0 40px\"><li id=o style=\"list-style:none\"></li></ul><div id=c style=\"height:10px\"></div>",
        all_modes: true,
        expected: &[
            ("o", (48.0, 8.0, 744.0, 0.0)),
            ("c", (8.0, 8.0, 784.0, 10.0)),
        ],
    },
    Case {
        name: "list_style_none_ws",
        body: "<ul style=\"margin:0;padding:0 0 0 40px\"><li id=o style=\"list-style:none\"> </li></ul><div id=c style=\"height:10px\"></div>",
        all_modes: true,
        expected: &[
            ("o", (48.0, 8.0, 744.0, 0.0)),
            ("c", (8.0, 8.0, 784.0, 10.0)),
        ],
    },
    Case {
        name: "height_zero",
        body: "<ul style=\"margin:0;padding:0 0 0 40px\"><li id=o style=\"height:0\"></li></ul><div id=c style=\"height:10px\"></div>",
        all_modes: true,
        expected: &[
            ("o", (48.0, 8.0, 744.0, 0.0)),
            ("c", (8.0, 8.0, 784.0, 10.0)),
        ],
    },
    Case {
        name: "height_20",
        body: "<ul style=\"margin:0;padding:0 0 0 40px\"><li id=o style=\"height:20px\"></li></ul><div id=c style=\"height:10px\"></div>",
        all_modes: true,
        expected: &[
            ("o", (48.0, 8.0, 744.0, 20.0)),
            ("c", (8.0, 28.0, 784.0, 10.0)),
        ],
    },
    Case {
        name: "line_height_50",
        body: "<ul style=\"margin:0;padding:0 0 0 40px\"><li id=o style=\"line-height:50px\"></li></ul><div id=c style=\"height:10px\"></div>",
        all_modes: true,
        expected: &[
            ("o", (48.0, 8.0, 744.0, 50.0)),
            ("c", (8.0, 58.0, 784.0, 10.0)),
        ],
    },
    Case {
        name: "font_size_20",
        body: "<ul style=\"margin:0;padding:0 0 0 40px\"><li id=o style=\"font-size:20px;line-height:normal\"></li></ul><div id=c style=\"height:10px\"></div>",
        all_modes: true,
        expected: &[
            ("o", (48.0, 8.0, 744.0, 20.0)),
            ("c", (8.0, 28.0, 784.0, 10.0)),
        ],
    },
    Case {
        name: "padding_5",
        body: "<ul style=\"margin:0;padding:0 0 0 40px\"><li id=o style=\"padding:5px 0\"></li></ul><div id=c style=\"height:10px\"></div>",
        all_modes: true,
        expected: &[
            ("o", (48.0, 8.0, 744.0, 20.0)),
            ("c", (8.0, 28.0, 784.0, 10.0)),
        ],
    },
    Case {
        name: "border_3",
        body: "<ul style=\"margin:0;padding:0 0 0 40px\"><li id=o style=\"border:3px solid\"></li></ul><div id=c style=\"height:10px\"></div>",
        all_modes: true,
        expected: &[
            ("o", (48.0, 8.0, 744.0, 16.0)),
            ("c", (8.0, 24.0, 784.0, 10.0)),
        ],
    },
    Case {
        name: "min_height_25",
        body: "<ul style=\"margin:0;padding:0 0 0 40px\"><li id=o style=\"min-height:25px\"></li></ul><div id=c style=\"height:10px\"></div>",
        all_modes: true,
        expected: &[
            ("o", (48.0, 8.0, 744.0, 25.0)),
            ("c", (8.0, 33.0, 784.0, 10.0)),
        ],
    },
    Case {
        name: "margin_10",
        body: "<ul style=\"margin:0;padding:0 0 0 40px\"><li id=o style=\"margin:10px 0\"></li></ul><div id=c style=\"height:10px\"></div>",
        all_modes: true,
        expected: &[
            ("o", (48.0, 10.0, 744.0, 10.0)),
            ("c", (8.0, 30.0, 784.0, 10.0)),
        ],
    },
    Case {
        name: "float_only",
        body: "<ul style=\"margin:0;padding:0 0 0 40px\"><li id=o ><div style=\"float:left;width:10px;height:5px\"></div></li></ul><div id=c style=\"height:10px\"></div>",
        all_modes: true,
        expected: &[
            ("o", (48.0, 8.0, 744.0, 10.0)),
            ("c", (8.0, 18.0, 784.0, 10.0)),
        ],
    },
    Case {
        name: "float_tall",
        body: "<ul style=\"margin:0;padding:0 0 0 40px\"><li id=o ><div style=\"float:left;width:10px;height:50px\"></div></li></ul><div id=c style=\"height:10px\"></div>",
        all_modes: true,
        expected: &[
            ("o", (48.0, 8.0, 744.0, 10.0)),
            ("c", (8.0, 18.0, 784.0, 10.0)),
        ],
    },
    Case {
        name: "abspos_only",
        body: "<ul style=\"margin:0;padding:0 0 0 40px\"><li id=o ><div style=\"position:absolute\"></div></li></ul><div id=c style=\"height:10px\"></div>",
        all_modes: true,
        expected: &[
            ("o", (48.0, 8.0, 744.0, 10.0)),
            ("c", (8.0, 18.0, 784.0, 10.0)),
        ],
    },
    Case {
        name: "flex_item",
        body: "<ul style=\"margin:0;padding:0 0 0 40px;display:flex\"><li id=o></li></ul><div id=c style=\"height:10px\"></div>",
        all_modes: true,
        expected: &[
            ("o", (48.0, 8.0, 0.0, 10.0)),
            ("c", (8.0, 18.0, 784.0, 10.0)),
        ],
    },
    Case {
        name: "grid_item",
        body: "<ul style=\"margin:0;padding:0 0 0 40px;display:grid\"><li id=o></li></ul><div id=c style=\"height:10px\"></div>",
        all_modes: true,
        expected: &[
            ("o", (48.0, 8.0, 744.0, 10.0)),
            ("c", (8.0, 18.0, 784.0, 10.0)),
        ],
    },
    Case {
        name: "inside",
        body: "<ul style=\"margin:0;padding:0 0 0 40px\"><li id=o style=\"list-style-position:inside\"></li></ul><div id=c style=\"height:10px\"></div>",
        all_modes: true,
        expected: &[
            ("o", (48.0, 8.0, 744.0, 10.0)),
            ("c", (8.0, 18.0, 784.0, 10.0)),
        ],
    },
    Case {
        name: "inside_ws",
        body: "<ul style=\"margin:0;padding:0 0 0 40px\"><li id=o style=\"list-style-position:inside\"> </li></ul><div id=c style=\"height:10px\"></div>",
        all_modes: true,
        expected: &[
            ("o", (48.0, 8.0, 744.0, 10.0)),
            ("c", (8.0, 18.0, 784.0, 10.0)),
        ],
    },
    Case {
        name: "block_child_only",
        body: "<ul style=\"margin:0;padding:0 0 0 40px\"><li id=o ><div style=\"height:10px\"></div></li></ul><div id=c style=\"height:10px\"></div>",
        all_modes: true,
        expected: &[
            ("o", (48.0, 8.0, 744.0, 10.0)),
            ("c", (8.0, 18.0, 784.0, 10.0)),
        ],
    },
    Case {
        name: "text_child",
        body: "<ul style=\"margin:0;padding:0 0 0 40px\"><li id=o >x</li></ul><div id=c style=\"height:10px\"></div>",
        all_modes: true,
        expected: &[
            ("o", (48.0, 8.0, 744.0, 10.0)),
            ("c", (8.0, 18.0, 784.0, 10.0)),
        ],
    },
    Case {
        name: "decimal",
        body: "<ul style=\"margin:0;padding:0 0 0 40px\"><li id=o style=\"list-style-type:decimal\"></li></ul><div id=c style=\"height:10px\"></div>",
        all_modes: true,
        expected: &[
            ("o", (48.0, 8.0, 744.0, 10.0)),
            ("c", (8.0, 18.0, 784.0, 10.0)),
        ],
    },
    Case {
        name: "square",
        body: "<ul style=\"margin:0;padding:0 0 0 40px\"><li id=o style=\"list-style-type:square\"></li></ul><div id=c style=\"height:10px\"></div>",
        all_modes: true,
        expected: &[
            ("o", (48.0, 8.0, 744.0, 10.0)),
            ("c", (8.0, 18.0, 784.0, 10.0)),
        ],
    },
    Case {
        name: "string_marker",
        body: "<ul style=\"margin:0;padding:0 0 0 40px\"><li id=o style=\"list-style-type:'x'\"></li></ul><div id=c style=\"height:10px\"></div>",
        all_modes: true,
        expected: &[
            ("o", (48.0, 8.0, 744.0, 10.0)),
            ("c", (8.0, 18.0, 784.0, 10.0)),
        ],
    },
    Case {
        name: "empty_string_marker",
        body: "<ul style=\"margin:0;padding:0 0 0 40px\"><li id=o style=\"list-style-type:''\"></li></ul><div id=c style=\"height:10px\"></div>",
        all_modes: true,
        expected: &[
            ("o", (48.0, 8.0, 744.0, 10.0)),
            ("c", (8.0, 18.0, 784.0, 10.0)),
        ],
    },
    Case {
        name: "marker_content_empty",
        body: "<style>#o::marker{content:\"\"}</style><ul style=\"margin:0;padding:0 0 0 40px\"><li id=o ></li></ul><div id=c style=\"height:10px\"></div>",
        all_modes: true,
        expected: &[
            ("o", (48.0, 8.0, 744.0, 10.0)),
            ("c", (8.0, 18.0, 784.0, 10.0)),
        ],
    },
    Case {
        name: "marker_content_none",
        body: "<style>#o::marker{content:none}</style><ul style=\"margin:0;padding:0 0 0 40px\"><li id=o ></li></ul><div id=c style=\"height:10px\"></div>",
        all_modes: true,
        expected: &[
            ("o", (48.0, 8.0, 744.0, 0.0)),
            ("c", (8.0, 8.0, 784.0, 10.0)),
        ],
    },
    Case {
        name: "marker_content_text",
        body: "<style>#o::marker{content:\"ab\"}</style><ul style=\"margin:0;padding:0 0 0 40px\"><li id=o ></li></ul><div id=c style=\"height:10px\"></div>",
        all_modes: true,
        expected: &[
            ("o", (48.0, 8.0, 744.0, 10.0)),
            ("c", (8.0, 18.0, 784.0, 10.0)),
        ],
    },
    Case {
        name: "marker_font_5",
        body: "<style>#o::marker{font-size:5px}</style><ul style=\"margin:0;padding:0 0 0 40px\"><li id=o ></li></ul><div id=c style=\"height:10px\"></div>",
        all_modes: true,
        expected: &[
            ("o", (48.0, 8.0, 744.0, 10.0)),
            ("c", (8.0, 18.0, 784.0, 10.0)),
        ],
    },
    Case {
        name: "overflow_hidden",
        body: "<ul style=\"margin:0;padding:0 0 0 40px\"><li id=o style=\"overflow:hidden\"></li></ul><div id=c style=\"height:10px\"></div>",
        all_modes: true,
        expected: &[
            ("o", (48.0, 8.0, 744.0, 10.0)),
            ("c", (8.0, 18.0, 784.0, 10.0)),
        ],
    },
    Case {
        name: "float_li",
        body: "<ul style=\"margin:0;padding:0 0 0 40px\"><li id=o style=\"float:left\"></li></ul><div id=c style=\"height:10px\"></div>",
        all_modes: true,
        expected: &[
            ("o", (48.0, 8.0, 0.0, 10.0)),
            ("c", (8.0, 8.0, 784.0, 10.0)),
        ],
    },
    Case {
        name: "abspos_li",
        body: "<ul style=\"margin:0;padding:0 0 0 40px\"><li id=o style=\"position:absolute\"></li></ul><div id=c style=\"height:10px\"></div>",
        all_modes: true,
        expected: &[
            ("o", (48.0, 8.0, 0.0, 10.0)),
            ("c", (8.0, 8.0, 784.0, 10.0)),
        ],
    },
    Case {
        name: "flow_root_li",
        body: "<ul style=\"margin:0;padding:0 0 0 40px\"><li id=o style=\"display:flow-root list-item\"></li></ul><div id=c style=\"height:10px\"></div>",
        all_modes: true,
        expected: &[
            ("o", (48.0, 8.0, 744.0, 10.0)),
            ("c", (8.0, 18.0, 784.0, 10.0)),
        ],
    },
    Case {
        name: "rtl_li",
        body: "<ul style=\"margin:0;padding:0 0 0 40px\"><li id=o style=\"direction:rtl\"></li></ul><div id=c style=\"height:10px\"></div>",
        all_modes: true,
        expected: &[
            ("o", (48.0, 8.0, 744.0, 10.0)),
            ("c", (8.0, 18.0, 784.0, 10.0)),
        ],
    },
    Case {
        name: "width_50",
        body: "<ul style=\"margin:0;padding:0 0 0 40px\"><li id=o style=\"width:50px\"></li></ul><div id=c style=\"height:10px\"></div>",
        all_modes: true,
        expected: &[
            ("o", (48.0, 8.0, 50.0, 10.0)),
            ("c", (8.0, 18.0, 784.0, 10.0)),
        ],
    },
    Case {
        name: "nested_empty",
        body: "<ul style=\"margin:0;padding:0 0 0 40px\"><li id=o><ul style=\"margin:0;padding:0 0 0 40px\"><li id=p></li></ul></li></ul><div id=c style=\"height:10px\"></div>",
        all_modes: false,
        expected: &[
            ("o", (48.0, 8.0, 744.0, 10.0)),
            ("p", (88.0, 8.0, 704.0, 10.0)),
            ("c", (8.0, 18.0, 784.0, 10.0)),
        ],
    },
    Case {
        name: "text_then_block_in_li",
        body: "<ul style=\"margin:0;padding:0 0 0 40px\"><li id=o >x<div id=d style=\"height:10px\"></div></li></ul><div id=c style=\"height:10px\"></div>",
        all_modes: true,
        expected: &[
            ("o", (48.0, 8.0, 744.0, 20.0)),
            ("d", (48.0, 18.0, 744.0, 10.0)),
            ("c", (8.0, 28.0, 784.0, 10.0)),
        ],
    },
    Case {
        name: "li_first_line_pseudo",
        body: "<style>#o::first-line{font-size:20px}</style><ul style=\"margin:0;padding:0 0 0 40px\"><li id=o ></li></ul><div id=c style=\"height:10px\"></div>",
        all_modes: true,
        expected: &[
            ("o", (48.0, 8.0, 744.0, 10.0)),
            ("c", (8.0, 18.0, 784.0, 10.0)),
        ],
    },
    Case {
        name: "li_before_empty",
        body: "<style>#o::before{content:\"\"}</style><ul style=\"margin:0;padding:0 0 0 40px\"><li id=o ></li></ul><div id=c style=\"height:10px\"></div>",
        all_modes: true,
        expected: &[
            ("o", (48.0, 8.0, 744.0, 10.0)),
            ("c", (8.0, 18.0, 784.0, 10.0)),
        ],
    },
    Case {
        name: "li_after_empty",
        body: "<style>#o::after{content:\"\"}</style><ul style=\"margin:0;padding:0 0 0 40px\"><li id=o ></li></ul><div id=c style=\"height:10px\"></div>",
        all_modes: true,
        expected: &[
            ("o", (48.0, 8.0, 744.0, 10.0)),
            ("c", (8.0, 18.0, 784.0, 10.0)),
        ],
    },
    Case {
        name: "li_before_text",
        body: "<style>#o::before{content:\"x\"}</style><ul style=\"margin:0;padding:0 0 0 40px\"><li id=o ></li></ul><div id=c style=\"height:10px\"></div>",
        all_modes: true,
        expected: &[
            ("o", (48.0, 8.0, 744.0, 10.0)),
            ("c", (8.0, 18.0, 784.0, 10.0)),
        ],
    },
    Case {
        name: "display_contents_child",
        body: "<ul style=\"margin:0;padding:0 0 0 40px\"><li id=o ><span style=\"display:contents\"></span></li></ul><div id=c style=\"height:10px\"></div>",
        all_modes: true,
        expected: &[
            ("o", (48.0, 8.0, 744.0, 10.0)),
            ("c", (8.0, 18.0, 784.0, 10.0)),
        ],
    },
    Case {
        name: "margin_only_span",
        body: "<ul style=\"margin:0;padding:0 0 0 40px\"><li id=o ><span style=\"margin-left:5px\"></span></li></ul><div id=c style=\"height:10px\"></div>",
        all_modes: true,
        expected: &[
            ("o", (48.0, 8.0, 744.0, 10.0)),
            ("c", (8.0, 18.0, 784.0, 10.0)),
        ],
    },
    Case {
        name: "clear_float",
        body: "<div style=\"float:left;width:10px;height:50px\"></div><ul style=\"margin:0;padding:0 0 0 40px\"><li id=o></li></ul><div id=c style=\"height:10px\"></div>",
        all_modes: true,
        expected: &[
            ("o", (48.0, 8.0, 744.0, 10.0)),
            ("c", (8.0, 18.0, 784.0, 10.0)),
        ],
    },
];
