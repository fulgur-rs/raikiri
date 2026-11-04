//! Avoided containers and final row groups keep the closing edges of their
//! containing block with their content on 200px pages.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};
use std::collections::HashMap;

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/data/text-autospace/Ahem.ttf"
));
type Placement = (usize, f32, f32);

fn places(content: &str, outer_css: &str) -> HashMap<String, Placement> {
    places_with(content, outer_css, true, "")
}

fn places_with(
    content: &str,
    outer_css: &str,
    prefix: bool,
    extra_css: &str,
) -> HashMap<String, Placement> {
    let prefix = if prefix {
        "<div style='height:100px'>A</div><div style='height:60px'>B</div>"
    } else {
        ""
    };
    let html = format!(
        "<!doctype html><style>@page{{size:200px 200px;margin:0}}body{{margin:0;font:10px/10px Ahem}}td{{padding:0}}{extra_css}</style><div style='padding-bottom:30px;{outer_css}'>{prefix}{content}</div><div id=z style='height:20px'>Z</div>"
    );
    let fonts = FontCollectionBuilder::new()
        .font_bytes("Ahem", AHEM.to_vec())
        .build()
        .expect("fonts");
    let resources = RenderResources::new().fonts(fonts);
    let doc = parse_html_with_resources(html.as_bytes(), &resources).expect("parse");
    let LayoutStatus::Completed(result) = layout(
        &doc,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new().resources(&resources),
    )
    .expect("layout") else {
        panic!("incomplete layout")
    };
    let mut places = HashMap::new();
    for (index, page) in result.pages().enumerate() {
        for run in page.text_runs() {
            let origin = (run.origin.0, run.origin.1 - run.ascent);
            let top = run
                .transform
                .map_or(origin, |transform| transform.apply(origin))
                .1;
            places.entry(format!("paint:{}", run.text)).or_insert((
                index,
                top,
                run.ascent + run.descent,
            ));
        }
        let dom = page.dom();
        for fragment in page.fragments() {
            if let Some(id) = dom.attr(fragment.node(), "id") {
                let rect = fragment.rect();
                places
                    .entry(id.to_owned())
                    .or_insert((index, rect.y, rect.height));
            }
        }
    }
    places
}

fn row(id: &str, css: &str) -> String {
    format!(
        "<div style='display:table-row;{css}'><div id='{id}' style='display:table-cell;height:10px'>{id}</div></div>"
    )
}

fn table(groups: &str, css: &str) -> String {
    format!("<div style='display:table;border-spacing:0;{css}'>{groups}</div>")
}

fn group(rows: &str, css: &str) -> String {
    format!("<div style='display:table-row-group;{css}'>{rows}</div>")
}

fn check(places: &HashMap<String, Placement>, id: &str, expected: Placement) {
    let actual = places
        .get(id)
        .unwrap_or_else(|| panic!("missing {id}: {places:?}"));
    assert!(
        actual.0 == expected.0
            && (actual.1 - expected.1).abs() < 0.04
            && (actual.2 - expected.2).abs() < 0.04,
        "{id}: {actual:?}, expected {expected:?}; {places:?}"
    );
}

#[test]
fn avoided_column_flex_keeps_outer_closing_edges() {
    let p = places(
        "<div style='display:flex;flex-direction:column;break-inside:avoid'><div id=c style='height:10px'>c</div><div id=d style='height:10px'>d</div></div>",
        "",
    );
    check(&p, "c", (1, 0.0, 10.0));
    check(&p, "d", (1, 10.0, 10.0));
    check(&p, "z", (1, 50.0, 20.0));
}

#[test]
fn avoided_table_keeps_outer_closing_edges() {
    let p = places(
        &table(&(row("c", "") + &row("d", "")), "break-inside:avoid"),
        "",
    );
    check(&p, "c", (1, 0.0, 10.0));
    check(&p, "d", (1, 10.0, 10.0));
    check(&p, "z", (1, 50.0, 20.0));
}

#[test]
fn avoided_final_group_keeps_outer_closing_edges() {
    let p = places(
        &table(
            &group(&(row("c", "") + &row("d", "")), "break-inside:avoid"),
            "",
        ),
        "",
    );
    check(&p, "c", (1, 0.0, 10.0));
    check(&p, "d", (1, 10.0, 10.0));
    check(&p, "z", (1, 50.0, 20.0));
}

#[test]
fn only_final_avoided_group_moves_with_closing_edges() {
    let groups =
        group(&row("c", ""), "") + &group(&(row("d", "") + &row("e", "")), "break-inside:avoid");
    let p = places(&table(&groups, ""), "");
    check(&p, "c", (0, 160.0, 10.0));
    check(&p, "d", (1, 0.0, 10.0));
    check(&p, "e", (1, 10.0, 10.0));
    check(&p, "z", (1, 50.0, 20.0));
}

#[test]
fn ordinary_last_row_and_forced_row_keep_their_breaks() {
    for group_css in ["", "break-inside:avoid"] {
        let last_css = if group_css.is_empty() {
            ""
        } else {
            "break-before:page"
        };
        let p = places(
            &table(&group(&(row("c", "") + &row("d", last_css)), group_css), ""),
            "",
        );
        check(&p, "c", (0, 160.0, 10.0));
        check(&p, "d", (1, 0.0, 10.0));
        check(&p, "z", (1, 40.0, 20.0));
    }
}

#[test]
fn avoided_group_at_flow_start_stays_on_its_page() {
    let p = places_with(
        &table(
            &group(&(row("c", "") + &row("d", "")), "break-inside:avoid"),
            "",
        ),
        "",
        false,
        "",
    );
    check(&p, "c", (0, 0.0, 10.0));
    check(&p, "d", (0, 10.0, 10.0));
    check(&p, "z", (0, 50.0, 20.0));
}

#[test]
fn avoided_group_keeps_table_spacing_and_outer_border() {
    for (table_css, outer_css, c, d, z) in [
        ("border-spacing:2px", "", 2.0, 14.0, 56.0),
        (
            "",
            "padding-bottom:15px;border-bottom:15px solid",
            0.0,
            10.0,
            50.0,
        ),
    ] {
        let p = places(
            &table(
                &group(&(row("c", "") + &row("d", "")), "break-inside:avoid"),
                table_css,
            ),
            outer_css,
        );
        check(&p, "c", (1, c, 10.0));
        check(&p, "d", (1, d, 10.0));
        check(&p, "z", (1, z, 20.0));
    }
}

#[test]
fn avoided_relative_group_keeps_its_visual_offset() {
    let p = places(
        &table(
            &group(
                &(row("c", "") + &row("d", "")),
                "break-inside:avoid;position:relative;top:3px",
            ),
            "",
        ),
        "",
    );
    check(&p, "c", (1, 0.0, 10.0));
    check(&p, "d", (1, 10.0, 10.0));
    check(&p, "z", (1, 50.0, 20.0));
    check(&p, "paint:c", (1, 3.0, 10.0));
    check(&p, "paint:d", (1, 13.0, 10.0));
}

#[test]
fn final_group_splits_the_last_row_when_the_next_page_cannot_hold_its_tail() {
    let normal = table(
        &group(&(row("c", "") + &row("d", "")), "break-inside:avoid"),
        "",
    );
    // The 30px of padding after `d` fill the page they start, and `z` rides up
    // over them: the pages after the first are shorter than it, and the passes
    // that settle the page sizes (three of them) find only one more page each.
    let p = places_with(
        &normal,
        "margin-bottom:-30px",
        true,
        "@page{size:200px 30px}@page:first{size:200px 200px}",
    );
    check(&p, "c", (1, 0.0, 10.0));
    check(&p, "d", (2, 0.0, 10.0));
}

#[test]
fn avoided_boundary_inside_the_final_group_keeps_the_group_together() {
    for (first_css, last_css) in [("break-after:avoid", ""), ("", "break-before:avoid")] {
        let p = places(
            &table(
                &group(
                    &(row("c", first_css) + &row("d", last_css)),
                    "break-inside:avoid",
                ),
                "",
            ),
            "",
        );
        check(&p, "c", (1, 0.0, 10.0));
        check(&p, "d", (1, 10.0, 10.0));
        check(&p, "z", (1, 50.0, 20.0));
    }
}
