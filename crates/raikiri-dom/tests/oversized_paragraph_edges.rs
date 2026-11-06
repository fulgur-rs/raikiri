//! Oversized paragraph edges in single-item table cells and inline flex boxes.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults, PaintRect};

struct Rendered {
    glyphs: Vec<(String, usize, f32)>,
    boxes: Vec<(String, usize, PaintRect)>,
}

fn render(container: &str, lead: u32, edge: &str, tail: bool) -> Rendered {
    let paragraph = format!("<p id=p style='margin:0;{edge}'>z z z</p>");
    let contents = if container == "table" {
        format!(
            "<div id=holder style='display:table'><div style='display:table-row'>\
             <div id=cell style='display:table-cell'>{paragraph}</div></div></div>"
        )
    } else {
        format!("<div id=holder style='display:{container}'>{paragraph}</div>")
    };
    let tail = if tail {
        "<div id=tail style='height:10px;font:10px/10px Ahem'>q</div>"
    } else {
        ""
    };
    render_body(&format!(
        "<div style='height:{lead}px'></div>{contents}{tail}"
    ))
}

fn render_body(contents: &str) -> Rendered {
    let fonts = FontCollectionBuilder::new()
        .font_bytes(
            "Ahem",
            include_bytes!("data/text-autospace/Ahem.ttf").to_vec(),
        )
        .build()
        .unwrap();
    let resources = RenderResources::new().fonts(fonts);
    let html = format!(
        "<!DOCTYPE html><style>@page{{size:100px 100px;margin:0}}\
         html,body{{margin:0;font:20px/30px Ahem}}</style>{contents}"
    );
    let doc = parse_html_with_resources(html.as_bytes(), &resources).unwrap();
    let LayoutStatus::Completed(result) = layout(
        &doc,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new().resources(&resources),
    )
    .unwrap() else {
        panic!("incomplete layout")
    };
    let mut glyphs = Vec::new();
    let mut boxes = Vec::new();
    for (index, page) in result.pages().enumerate() {
        for run in page.text_runs() {
            let ascent = if run.text.contains('q') { 8.0 } else { 16.0 };
            glyphs.push((run.text.to_string(), index + 1, run.origin.1 - ascent));
        }
        for fragment in page.fragments() {
            if let Some(id) = page.dom().attr(fragment.node(), "id") {
                boxes.push((id.to_owned(), index + 1, fragment.rect()));
            }
        }
    }
    Rendered { glyphs, boxes }
}

fn check_single_item(container: &str) {
    // Chromium print, 100px square pages, Ahem 20px/30px. The break moves
    // the paragraph before its oversized edge only when it follows content.
    let mut actual = Vec::new();
    let mut expected = Vec::new();
    for lead in [0, 50] {
        for (edge, page, top) in [
            ("padding-top:125px", 2, 30.0),
            ("padding-top:250px", 3, 55.0),
            ("border-top:125px solid", 2, 30.0),
            ("padding-bottom:125px", 1, 5.0),
        ] {
            let output = render(container, lead, edge, false);
            let (_, actual_page, actual_top) = output
                .glyphs
                .iter()
                .find(|(text, _, _)| text.contains('z'))
                .unwrap();
            actual.push((lead, edge, (*actual_page, *actual_top)));
            expected.push((lead, edge, (page + usize::from(lead > 0), top)));
        }
    }
    assert_eq!(actual, expected, "{container}");
}

#[test]
fn table_cell_single_paragraph_keeps_oversized_edges_with_its_text() {
    check_single_item("table");
}

#[test]
fn inline_flex_single_paragraph_keeps_oversized_edges_with_its_text() {
    check_single_item("inline-flex");
}

#[test]
fn oversized_edge_moves_the_holder_fragments_and_following_flow_once() {
    for container in ["table", "inline-flex"] {
        let output = render(container, 50, "padding-top:125px;background:blue", true);
        let (_, actual_page, actual_top) = output
            .glyphs
            .iter()
            .find(|(text, _, _)| text.contains('q'))
            .unwrap();
        assert_eq!((*actual_page, *actual_top), (3, 55.0), "{container}");
        for id in if container == "table" {
            &["holder", "cell", "p"][..]
        } else {
            &["holder", "p"][..]
        } {
            let fragments = output
                .boxes
                .iter()
                .filter(|(name, _, rect)| name == id && rect.height > 0.01)
                .map(|(_, page, rect)| (*page, rect.y, rect.height))
                .collect::<Vec<_>>();
            assert_eq!(
                fragments,
                [(2, 0.0, 100.0), (3, 0.0, 55.0)],
                "{container} {id}"
            );
        }
    }
}

#[test]
fn a_relative_ancestor_does_not_turn_a_page_start_into_an_oversized_edge_break() {
    for contents in [
        "<div style='display:table'><div style='display:table-row'><div style='display:table-cell'><p style='margin:0;padding-top:125px'>z z z</p></div></div></div>",
        "<div style='display:inline-flex'><p style='margin:0;padding-top:125px'>z z z</p></div>",
    ] {
        let output = render_body(&format!(
            "<div style='position:relative;top:50px'>{contents}</div>"
        ));
        let (_, page, top) = output
            .glyphs
            .iter()
            .find(|(text, _, _)| text.contains('z'))
            .unwrap();
        assert_eq!((*page, *top), (2, 80.0));
    }
}
