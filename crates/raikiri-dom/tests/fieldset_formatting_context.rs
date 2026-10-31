//! A block-level `<fieldset>` is the root of its own block formatting context
//! (HTML Living Standard, "Rendering", the fieldset and legend elements). Its
//! border box therefore stays clear of the margin boxes of the floats in the
//! formatting context around it: beside a float when it fits in the room the
//! float leaves, below it when it does not (CSS 2.1 section 9.5). It also
//! contains its own floats and keeps the margins of its children from
//! collapsing with its own (section 9.4.1).
//!
//! The expected border boxes are the ones Chromium computes for the same
//! markup in an 800px by 600px viewport, with no margin on the body. The
//! fieldsets carry no margin, padding or border of their own unless a case
//! says otherwise, so the user agent style of a fieldset does not change the
//! sizes. `o` is the container and `c` the fieldset. A fieldset carries no
//! user agent margin, padding, border or legend here: that part of its style is
//! not implemented yet.

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

/// An element `id`, the page it starts on, from 0, and the top and height of
/// its first fragment on that page.
type Placement = (String, usize, f32, f32);

/// What a case is about.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Group {
    /// How the border box of a fieldset keeps clear of floats.
    Floats,
    /// What a fieldset keeps inside: its floats and the margins of its children.
    Contents,
    /// A fieldset in a flex or grid container, out of flow, between lines.
    Elsewhere,
}

/// One markup and the boxes Chromium gives it.
struct Case {
    name: &'static str,
    group: Group,
    body: &'static str,
    expected: &'static [(&'static str, Rect)],
}

/// The border boxes of the elements of `body` with an `id` on the first page.
fn border_boxes(body: &str) -> Vec<(String, Rect)> {
    let html = format!(
        "<!DOCTYPE html><style>@page {{ size: 800px 600px; margin: 0 }} \
         body {{ margin: 0; font: 10px/10px Ahem }} div {{ box-sizing: border-box }}</style>{body}"
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

/// Lay out every case of `group` and report all the boxes that differ.
fn check_group(group: Group) {
    let mut failures = Vec::new();
    let mut checked = 0;
    for case in CASES.iter().filter(|case| case.group == group) {
        checked += 1;
        let actual = border_boxes(case.body);
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
    assert!(checked > 0, "no case in the group");
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn a_fieldset_keeps_clear_of_the_margin_boxes_of_floats() {
    check_group(Group::Floats);
}

#[test]
fn a_fieldset_holds_its_floats_and_its_children_margins() {
    check_group(Group::Contents);
}

#[test]
fn a_fieldset_in_a_flex_or_grid_container_out_of_flow_or_between_lines() {
    check_group(Group::Elsewhere);
}

/// The page, from 0, and the top and height on that page's area of the first
/// fragment of each element with an `id`, for pages of 480px by 288px with
/// 48px margins.
fn paged_boxes(body: &str) -> Vec<Placement> {
    let html = format!(
        "<!DOCTYPE html><style>@page {{ size: 480px 288px; margin: 48px }} \
         body {{ margin: 0 }}</style>{body}"
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

/// Lays `template` out twice, with `@TAG@` and `@STYLE@` standing for a
/// fieldset and for a `flow-root` block, and returns both placements. The
/// fieldset carries no margin, padding or border of its own.
fn fieldset_and_flow_root(template: &str) -> (Vec<Placement>, Vec<Placement>) {
    let with = |tag: &str, style: &str| {
        paged_boxes(&template.replace("@TAG@", tag).replace("@STYLE@", style))
    };
    (
        with("fieldset", "display:block;padding:0;border:0"),
        with("div", "display:flow-root"),
    )
}

fn assert_same_placement(template: &str) {
    let (fieldset, flow_root) = fieldset_and_flow_root(template);
    assert_eq!(fieldset.len(), flow_root.len(), "{template}");
    for (a, b) in fieldset.iter().zip(&flow_root) {
        assert!(
            a.0 == b.0 && a.1 == b.1 && (a.2 - b.2).abs() <= 0.04 && (a.3 - b.3).abs() <= 0.04,
            "{template}: #{} is on page {} at {} high {} as a fieldset and on page {} at {} high {} as a flow-root block",
            a.0,
            a.1,
            a.2,
            a.3,
            b.1,
            b.2,
            b.3
        );
    }
}

/// The margin of the first child stays inside the fieldset after a forced
/// break, as it does inside a block that roots a context, and the fieldset
/// starts the page.
#[test]
fn the_margin_of_a_first_child_stays_in_a_fieldset_after_a_forced_break() {
    let template = "<div id=a style='height:10px;break-after:page'></div>\
                    <@TAG@ id=x style='@STYLE@'>\
                    <div id=c style='margin-top:30px;height:10px'></div></@TAG@>";
    assert_same_placement(template);
    let (fieldset, _) = fieldset_and_flow_root(template);
    let x = fieldset.iter().find(|b| b.0 == "x").expect("x");
    assert_eq!((x.1, x.2, x.3), (1, 0.0, 40.0));
}

/// The margins of an empty fieldset do not collapse through it, so they keep
/// the boxes around it apart.
#[test]
fn the_margins_of_an_empty_fieldset_do_not_collapse_through_it_at_a_break() {
    let template = "<div id=a style='height:10px;break-after:page'></div>\
                    <@TAG@ id=x style='@STYLE@;margin:20px 0'></@TAG@>\
                    <div id=d style='height:10px'></div>";
    assert_same_placement(template);
    let (fieldset, _) = fieldset_and_flow_root(template);
    let d = fieldset.iter().find(|b| b.0 == "d").expect("d");
    assert_eq!((d.1, d.2, d.3), (1, 40.0, 10.0));
}

/// The margin of the last child stays inside a fieldset that ends in a forced
/// break.
#[test]
fn the_margin_of_a_last_child_stays_in_a_fieldset_before_a_forced_break() {
    let template = "<@TAG@ id=x style='@STYLE@;break-after:page'>\
                    <div id=c style='margin-bottom:30px;height:10px'></div></@TAG@>\
                    <div id=d style='height:10px'></div>";
    assert_same_placement(template);
    let (fieldset, _) = fieldset_and_flow_root(template);
    let x = fieldset.iter().find(|b| b.0 == "x").expect("x");
    assert_eq!((x.1, x.2, x.3), (0, 0.0, 40.0));
}

/// The margin of the last child of a fieldset is not part of the margin between
/// the fieldset and the box after a forced break.
#[test]
fn only_the_margin_of_a_fieldset_itself_counts_before_a_forced_break() {
    let template = "<div id=w><@TAG@ id=x style='@STYLE@;margin-bottom:20px'>\
                    <div id=c style='margin-bottom:30px;height:10px'></div></@TAG@>\
                    <div id=d style='height:10px;break-before:page'></div></div>";
    assert_same_placement(template);
}

/// A page edge that falls in the margin of a fieldset truncates that margin
/// only, not the larger margin of the last child inside the fieldset.
#[test]
fn a_page_edge_in_the_margin_of_a_fieldset_truncates_that_margin_only() {
    let template = "<div style='height:150px'></div>\
                    <@TAG@ id=x style='@STYLE@;margin-bottom:20px'>\
                    <div id=c style='margin-bottom:30px;height:10px'></div></@TAG@>\
                    <div id=d style='height:10px'></div>";
    assert_same_placement(template);
}

const CASES: &[Case] = &[
    Case {
        name: "beside_w50",
        group: Group::Floats,
        body: "<div id=o style=\"width:200px\"><div style=\"float:left;width:100px;height:100px\"></div><fieldset id=c style=\"display:block;margin:0;padding:0;border:0;width:50px;height:20px\"></fieldset></div>",
        expected: &[
            ("o", (0.0, 0.0, 200.0, 20.0)),
            ("c", (100.0, 0.0, 50.0, 20.0)),
        ],
    },
    Case {
        name: "below_w150",
        group: Group::Floats,
        body: "<div id=o style=\"width:200px\"><div style=\"float:left;width:100px;height:100px\"></div><fieldset id=c style=\"display:block;margin:0;padding:0;border:0;width:150px;height:20px\"></fieldset></div>",
        expected: &[
            ("o", (0.0, 0.0, 200.0, 120.0)),
            ("c", (0.0, 100.0, 150.0, 20.0)),
        ],
    },
    Case {
        name: "right_beside_w50",
        group: Group::Floats,
        body: "<div id=o style=\"width:200px\"><div style=\"float:right;width:100px;height:100px\"></div><fieldset id=c style=\"display:block;margin:0;padding:0;border:0;width:50px;height:20px\"></fieldset></div>",
        expected: &[
            ("o", (0.0, 0.0, 200.0, 20.0)),
            ("c", (0.0, 0.0, 50.0, 20.0)),
        ],
    },
    Case {
        name: "right_below_w150",
        group: Group::Floats,
        body: "<div id=o style=\"width:200px\"><div style=\"float:right;width:100px;height:100px\"></div><fieldset id=c style=\"display:block;margin:0;padding:0;border:0;width:150px;height:20px\"></fieldset></div>",
        expected: &[
            ("o", (0.0, 0.0, 200.0, 120.0)),
            ("c", (0.0, 100.0, 150.0, 20.0)),
        ],
    },
    Case {
        name: "exact_fit_w100",
        group: Group::Floats,
        body: "<div id=o style=\"width:200px\"><div style=\"float:left;width:100px;height:100px\"></div><fieldset id=c style=\"display:block;margin:0;padding:0;border:0;width:100px;height:20px\"></fieldset></div>",
        expected: &[
            ("o", (0.0, 0.0, 200.0, 20.0)),
            ("c", (100.0, 0.0, 100.0, 20.0)),
        ],
    },
    Case {
        name: "auto_width",
        group: Group::Floats,
        body: "<div id=o style=\"width:200px\"><div style=\"float:left;width:100px;height:100px\"></div><fieldset id=c style=\"display:block;margin:0;padding:0;border:0;height:20px\"></fieldset></div>",
        expected: &[
            ("o", (0.0, 0.0, 200.0, 20.0)),
            ("c", (100.0, 0.0, 100.0, 20.0)),
        ],
    },
    Case {
        name: "auto_width_text",
        group: Group::Floats,
        body: "<div id=o style=\"width:200px\"><div style=\"float:left;width:100px;height:100px\"></div><fieldset id=c style=\"display:block;margin:0;padding:0;border:0\">aaaa</fieldset></div>",
        expected: &[
            ("o", (0.0, 0.0, 200.0, 10.0)),
            ("c", (100.0, 0.0, 100.0, 10.0)),
        ],
    },
    Case {
        name: "auto_width_right",
        group: Group::Floats,
        body: "<div id=o style=\"width:200px\"><div style=\"float:right;width:100px;height:100px\"></div><fieldset id=c style=\"display:block;margin:0;padding:0;border:0;height:20px\"></fieldset></div>",
        expected: &[
            ("o", (0.0, 0.0, 200.0, 20.0)),
            ("c", (0.0, 0.0, 100.0, 20.0)),
        ],
    },
    Case {
        name: "margin_left",
        group: Group::Floats,
        body: "<div id=o style=\"width:200px\"><div style=\"float:left;width:100px;height:100px\"></div><fieldset id=c style=\"display:block;margin:0;padding:0;border:0;margin-left:10px;width:50px;height:20px\"></fieldset></div>",
        expected: &[
            ("o", (0.0, 0.0, 200.0, 20.0)),
            ("c", (100.0, 0.0, 50.0, 20.0)),
        ],
    },
    Case {
        name: "margin_auto_beside",
        group: Group::Floats,
        body: "<div id=o style=\"width:200px\"><div style=\"float:left;width:100px;height:100px\"></div><fieldset id=c style=\"display:block;margin:0;padding:0;border:0;margin:0 auto;width:50px;height:20px\"></fieldset></div>",
        expected: &[
            ("o", (0.0, 0.0, 200.0, 20.0)),
            ("c", (125.0, 0.0, 50.0, 20.0)),
        ],
    },
    Case {
        name: "margin_left_auto",
        group: Group::Floats,
        body: "<div id=o style=\"width:200px\"><div style=\"float:left;width:100px;height:100px\"></div><fieldset id=c style=\"display:block;margin:0;padding:0;border:0;margin-left:auto;width:50px;height:20px\"></fieldset></div>",
        expected: &[
            ("o", (0.0, 0.0, 200.0, 20.0)),
            ("c", (150.0, 0.0, 50.0, 20.0)),
        ],
    },
    Case {
        name: "clear_left",
        group: Group::Floats,
        body: "<div id=o style=\"width:200px\"><div style=\"float:left;width:100px;height:100px\"></div><fieldset id=c style=\"display:block;margin:0;padding:0;border:0;clear:left;width:50px;height:20px\"></fieldset></div>",
        expected: &[
            ("o", (0.0, 0.0, 200.0, 120.0)),
            ("c", (0.0, 100.0, 50.0, 20.0)),
        ],
    },
    Case {
        name: "after_text",
        group: Group::Floats,
        body: "<div id=o style=\"width:200px\"><div style=\"float:left;width:100px;height:100px\"></div><div style=\"height:10px\"></div><fieldset id=c style=\"display:block;margin:0;padding:0;border:0;width:50px;height:20px\"></fieldset></div>",
        expected: &[
            ("o", (0.0, 0.0, 200.0, 30.0)),
            ("c", (100.0, 10.0, 50.0, 20.0)),
        ],
    },
    Case {
        name: "float_after_fieldset",
        group: Group::Floats,
        body: "<div id=o style=\"width:200px\"><fieldset id=c style=\"display:block;margin:0;padding:0;border:0;width:50px;height:20px\"></fieldset><div style=\"float:left;width:100px;height:100px\"></div></div>",
        expected: &[
            ("o", (0.0, 0.0, 200.0, 20.0)),
            ("c", (0.0, 0.0, 50.0, 20.0)),
        ],
    },
    Case {
        name: "two_floats",
        group: Group::Floats,
        body: "<div id=o style=\"width:200px\"><div style=\"float:left;width:100px;height:100px\"></div><div style=\"float:left;width:50px;height:50px\"></div><fieldset id=c style=\"display:block;margin:0;padding:0;border:0;width:30px;height:20px\"></fieldset></div>",
        expected: &[
            ("o", (0.0, 0.0, 200.0, 20.0)),
            ("c", (150.0, 0.0, 30.0, 20.0)),
        ],
    },
    Case {
        name: "two_floats_wide",
        group: Group::Floats,
        body: "<div id=o style=\"width:200px\"><div style=\"float:left;width:100px;height:100px\"></div><div style=\"float:left;width:50px;height:50px\"></div><fieldset id=c style=\"display:block;margin:0;padding:0;border:0;width:60px;height:20px\"></fieldset></div>",
        expected: &[
            ("o", (0.0, 0.0, 200.0, 70.0)),
            ("c", (100.0, 50.0, 60.0, 20.0)),
        ],
    },
    Case {
        name: "contains_float",
        group: Group::Contents,
        body: "<div id=o style=\"width:200px\"><fieldset id=c style=\"display:block;margin:0;padding:0;border:0\"><div id=f style=\"float:left;width:30px;height:40px\"></div></fieldset><div id=d style=\"height:5px\"></div></div>",
        expected: &[
            ("o", (0.0, 0.0, 200.0, 45.0)),
            ("c", (0.0, 0.0, 200.0, 40.0)),
            ("f", (0.0, 0.0, 30.0, 40.0)),
            ("d", (0.0, 40.0, 200.0, 5.0)),
        ],
    },
    Case {
        name: "child_margin_top",
        group: Group::Contents,
        body: "<div id=o style=\"width:200px\"><fieldset id=c style=\"display:block;margin:0;padding:0;border:0\"><div id=d style=\"margin-top:20px;height:10px\"></div></fieldset></div>",
        expected: &[
            ("o", (0.0, 0.0, 200.0, 30.0)),
            ("c", (0.0, 0.0, 200.0, 30.0)),
            ("d", (0.0, 20.0, 200.0, 10.0)),
        ],
    },
    Case {
        name: "child_margin_bottom",
        group: Group::Contents,
        body: "<div id=o style=\"width:200px\"><fieldset id=c style=\"display:block;margin:0;padding:0;border:0\"><div id=d style=\"margin-bottom:20px;height:10px\"></div></fieldset><div id=e style=\"height:5px\"></div></div>",
        expected: &[
            ("o", (0.0, 0.0, 200.0, 35.0)),
            ("c", (0.0, 0.0, 200.0, 30.0)),
            ("d", (0.0, 0.0, 200.0, 10.0)),
            ("e", (0.0, 30.0, 200.0, 5.0)),
        ],
    },
    Case {
        name: "own_margin_collapse_prev",
        group: Group::Contents,
        body: "<div id=o style=\"width:200px\"><div style=\"margin-bottom:20px;height:10px\"></div><fieldset id=c style=\"display:block;margin:0;padding:0;border:0;margin-top:30px;height:10px\"></fieldset></div>",
        expected: &[
            ("o", (0.0, 0.0, 200.0, 50.0)),
            ("c", (0.0, 40.0, 200.0, 10.0)),
        ],
    },
    Case {
        name: "own_margin_collapse_next",
        group: Group::Contents,
        body: "<div id=o style=\"width:200px\"><fieldset id=c style=\"display:block;margin:0;padding:0;border:0;margin-bottom:30px;height:10px\"></fieldset><div id=d style=\"margin-top:20px;height:10px\"></div></div>",
        expected: &[
            ("o", (0.0, 0.0, 200.0, 50.0)),
            ("c", (0.0, 0.0, 200.0, 10.0)),
            ("d", (0.0, 40.0, 200.0, 10.0)),
        ],
    },
    Case {
        name: "empty_collapse_through",
        group: Group::Contents,
        body: "<div id=o style=\"width:200px\"><div style=\"height:10px\"></div><fieldset id=c style=\"display:block;margin:0;padding:0;border:0;margin:20px 0\"></fieldset><div id=d style=\"height:10px\"></div></div>",
        expected: &[
            ("o", (0.0, 0.0, 200.0, 60.0)),
            ("c", (0.0, 30.0, 200.0, 0.0)),
            ("d", (0.0, 50.0, 200.0, 10.0)),
        ],
    },
    Case {
        name: "in_flex_row",
        group: Group::Elsewhere,
        body: "<div id=o style=\"display:flex;width:200px\"><fieldset id=c style=\"display:block;margin:0;padding:0;border:0\">aaaa</fieldset><div id=d style=\"width:30px;height:10px\"></div></div>",
        expected: &[
            ("o", (0.0, 0.0, 200.0, 10.0)),
            ("c", (0.0, 0.0, 40.0, 10.0)),
            ("d", (40.0, 0.0, 30.0, 10.0)),
        ],
    },
    Case {
        name: "in_flex_column",
        group: Group::Elsewhere,
        body: "<div id=o style=\"display:flex;flex-direction:column;width:200px\"><fieldset id=c style=\"display:block;margin:0;padding:0;border:0\">aaaa</fieldset></div>",
        expected: &[
            ("o", (0.0, 0.0, 200.0, 10.0)),
            ("c", (0.0, 0.0, 200.0, 10.0)),
        ],
    },
    Case {
        name: "in_grid",
        group: Group::Elsewhere,
        body: "<div id=o style=\"display:grid;width:200px\"><fieldset id=c style=\"display:block;margin:0;padding:0;border:0\">aaaa</fieldset></div>",
        expected: &[
            ("o", (0.0, 0.0, 200.0, 10.0)),
            ("c", (0.0, 0.0, 200.0, 10.0)),
        ],
    },
    Case {
        name: "abspos_auto",
        group: Group::Elsewhere,
        body: "<div id=o style=\"position:relative;width:200px;height:50px\"><fieldset id=c style=\"display:block;margin:0;padding:0;border:0;position:absolute;left:10px;top:5px\">aaaa</fieldset></div>",
        expected: &[
            ("o", (0.0, 0.0, 200.0, 50.0)),
            ("c", (10.0, 5.0, 40.0, 10.0)),
        ],
    },
    Case {
        name: "between_text",
        group: Group::Elsewhere,
        body: "<div id=o style=\"width:200px\">aa<fieldset id=c style=\"display:block;margin:0;padding:0;border:0\">bb</fieldset>cc</div>",
        expected: &[
            ("o", (0.0, 0.0, 200.0, 30.0)),
            ("c", (0.0, 10.0, 200.0, 10.0)),
        ],
    },
    Case {
        name: "nested",
        group: Group::Elsewhere,
        body: "<div id=o style=\"width:200px\"><div style=\"float:left;width:100px;height:100px\"></div><fieldset id=c style=\"display:block;margin:0;padding:0;border:0\"><fieldset id=d style=\"display:block;margin:0;padding:0;border:0;height:10px\"></fieldset></fieldset></div>",
        expected: &[
            ("o", (0.0, 0.0, 200.0, 10.0)),
            ("c", (100.0, 0.0, 100.0, 10.0)),
            ("d", (100.0, 0.0, 100.0, 10.0)),
        ],
    },
    Case {
        name: "text_height",
        group: Group::Elsewhere,
        body: "<div id=o style=\"width:200px\"><fieldset id=c style=\"display:block;margin:0;padding:0;border:0\">aaaa<br>bb</fieldset></div>",
        expected: &[
            ("o", (0.0, 0.0, 200.0, 20.0)),
            ("c", (0.0, 0.0, 200.0, 20.0)),
        ],
    },
    Case {
        name: "percent_width",
        group: Group::Floats,
        body: "<div id=o style=\"width:200px\"><div style=\"float:left;width:100px;height:100px\"></div><fieldset id=c style=\"display:block;margin:0;padding:0;border:0;width:50%;height:20px\"></fieldset></div>",
        expected: &[
            ("o", (0.0, 0.0, 200.0, 20.0)),
            ("c", (100.0, 0.0, 100.0, 20.0)),
        ],
    },
    Case {
        name: "max_width",
        group: Group::Floats,
        body: "<div id=o style=\"width:200px\"><div style=\"float:left;width:100px;height:100px\"></div><fieldset id=c style=\"display:block;margin:0;padding:0;border:0;max-width:60px;height:20px\"></fieldset></div>",
        expected: &[
            ("o", (0.0, 0.0, 200.0, 20.0)),
            ("c", (100.0, 0.0, 60.0, 20.0)),
        ],
    },
    Case {
        name: "min_width_wide",
        group: Group::Floats,
        body: "<div id=o style=\"width:200px\"><div style=\"float:left;width:100px;height:100px\"></div><fieldset id=c style=\"display:block;margin:0;padding:0;border:0;min-width:150px;height:20px\"></fieldset></div>",
        expected: &[
            ("o", (0.0, 0.0, 200.0, 120.0)),
            ("c", (0.0, 100.0, 200.0, 20.0)),
        ],
    },
    Case {
        name: "rtl_beside",
        group: Group::Floats,
        body: "<div id=o style=\"width:200px;direction:rtl\"><div style=\"float:right;width:100px;height:100px\"></div><fieldset id=c style=\"display:block;margin:0;padding:0;border:0;width:50px;height:20px\"></fieldset></div>",
        expected: &[
            ("o", (0.0, 0.0, 200.0, 20.0)),
            ("c", (50.0, 0.0, 50.0, 20.0)),
        ],
    },
    Case {
        name: "float_in_child_text",
        group: Group::Contents,
        body: "<div id=o style=\"width:200px\"><fieldset id=c style=\"display:block;margin:0;padding:0;border:0\"><div style=\"float:left;width:100px;height:100px\"></div>aaaa</fieldset></div>",
        expected: &[
            ("o", (0.0, 0.0, 200.0, 100.0)),
            ("c", (0.0, 0.0, 200.0, 100.0)),
        ],
    },
    Case {
        name: "clear_both_inside",
        group: Group::Contents,
        body: "<div id=o style=\"width:200px\"><fieldset id=c style=\"display:block;margin:0;padding:0;border:0\"><div style=\"float:left;width:100px;height:100px\"></div><div id=d style=\"clear:both;height:10px\"></div></fieldset></div>",
        expected: &[
            ("o", (0.0, 0.0, 200.0, 110.0)),
            ("c", (0.0, 0.0, 200.0, 110.0)),
            ("d", (0.0, 100.0, 200.0, 10.0)),
        ],
    },
    Case {
        name: "sibling_clear",
        group: Group::Floats,
        body: "<div id=o style=\"width:200px\"><div style=\"float:left;width:100px;height:100px\"></div><fieldset id=c style=\"display:block;margin:0;padding:0;border:0;height:20px\"></fieldset><div id=d style=\"height:10px\"></div></div>",
        expected: &[
            ("o", (0.0, 0.0, 200.0, 30.0)),
            ("c", (100.0, 0.0, 100.0, 20.0)),
            ("d", (0.0, 20.0, 200.0, 10.0)),
        ],
    },
    Case {
        name: "margins_with_float",
        group: Group::Floats,
        body: "<div id=o style=\"width:200px\"><div style=\"float:left;width:100px;height:100px\"></div><fieldset id=c style=\"display:block;margin:0;padding:0;border:0;margin:5px 0;width:50px;height:20px\"></fieldset></div>",
        expected: &[
            ("o", (0.0, 5.0, 200.0, 20.0)),
            ("c", (100.0, 5.0, 50.0, 20.0)),
        ],
    },
    Case {
        name: "margin_collapse_float_top",
        group: Group::Floats,
        body: "<div id=o style=\"width:200px\"><div style=\"height:10px\"></div><fieldset id=c style=\"display:block;margin:0;padding:0;border:0;margin-top:20px;width:50px;height:20px\"></fieldset></div>",
        expected: &[
            ("o", (0.0, 0.0, 200.0, 50.0)),
            ("c", (0.0, 30.0, 50.0, 20.0)),
        ],
    },
    Case {
        name: "in_percent_wrapper",
        group: Group::Elsewhere,
        body: "<div id=o style=\"width:50%\"><div style=\"width:80%\"><fieldset id=c style=\"display:block;margin:0;padding:0;border:0\">aaaa</fieldset></div></div>",
        expected: &[
            ("o", (0.0, 0.0, 400.0, 10.0)),
            ("c", (0.0, 0.0, 320.0, 10.0)),
        ],
    },
    Case {
        name: "in_auto_chain",
        group: Group::Elsewhere,
        body: "<div id=o style=\"width:200px\"><div><div><fieldset id=c style=\"display:block;margin:0;padding:0;border:0;height:10px\"></fieldset></div></div></div>",
        expected: &[
            ("o", (0.0, 0.0, 200.0, 10.0)),
            ("c", (0.0, 0.0, 200.0, 10.0)),
        ],
    },
    Case {
        name: "in_inline_block",
        group: Group::Elsewhere,
        body: "<div id=o style=\"display:inline-block\"><fieldset id=c style=\"display:block;margin:0;padding:0;border:0\">aaaa</fieldset></div>",
        expected: &[("o", (0.0, 0.0, 40.0, 10.0)), ("c", (0.0, 0.0, 40.0, 10.0))],
    },
    Case {
        name: "in_float",
        group: Group::Elsewhere,
        body: "<div id=o style=\"float:left\"><fieldset id=c style=\"display:block;margin:0;padding:0;border:0\">aaaa</fieldset></div>",
        expected: &[("o", (0.0, 0.0, 40.0, 10.0)), ("c", (0.0, 0.0, 40.0, 10.0))],
    },
    Case {
        name: "with_padding",
        group: Group::Floats,
        body: "<div id=o style=\"width:200px\"><div style=\"float:left;width:100px;height:100px\"></div><fieldset id=c style=\"display:block;margin:0;padding:0;border:0;padding:5px;height:20px\"></fieldset></div>",
        expected: &[
            ("o", (0.0, 0.0, 200.0, 30.0)),
            ("c", (100.0, 0.0, 100.0, 30.0)),
        ],
    },
    Case {
        name: "with_border",
        group: Group::Floats,
        body: "<div id=o style=\"width:200px\"><div style=\"float:left;width:100px;height:100px\"></div><fieldset id=c style=\"display:block;margin:0;padding:0;border:0;border:3px solid;height:20px\"></fieldset></div>",
        expected: &[
            ("o", (0.0, 0.0, 200.0, 26.0)),
            ("c", (100.0, 0.0, 100.0, 26.0)),
        ],
    },
];
