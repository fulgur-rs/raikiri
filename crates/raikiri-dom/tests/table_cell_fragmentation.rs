//! Cell text and declared height across page boundaries.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

struct Rendered {
    lines: Vec<(String, usize, f32)>,
    boxes: Vec<(String, usize, f32, f32)>,
}

fn lines(body: &str) -> Vec<(String, usize, f32)> {
    render(body).lines
}

fn render(body: &str) -> Rendered {
    let fonts = FontCollectionBuilder::new()
        .font_bytes(
            "Ahem",
            include_bytes!("data/text-autospace/Ahem.ttf").to_vec(),
        )
        .build()
        .unwrap();
    let resources = RenderResources::new().fonts(fonts);
    let html = format!(
        "<!doctype html><style>@page{{size:100px 100px;margin:0}}\
         html,body{{margin:0;font:20px/30px Ahem}}</style>{body}"
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
    let mut lines = Vec::new();
    let mut boxes = Vec::new();
    for (page_index, page) in result.pages().enumerate() {
        for run in page.text_runs() {
            let ascent = if run.text.contains('Q') { 8.0 } else { 16.0 };
            lines.push((run.text.to_string(), page_index, run.origin.1 - ascent));
        }
        for fragment in page.fragments() {
            if let Some(id) = page.dom().attr(fragment.node(), "id") {
                let rect = fragment.rect();
                if rect.height > 0.01 {
                    boxes.push((id.to_owned(), page_index, rect.y, rect.height));
                }
            }
        }
    }
    Rendered { lines, boxes }
}

#[test]
fn a_declared_cell_height_keeps_its_text_after_a_fragmented_paragraph() {
    let output = lines(
        "<div style='height:30px'></div><div style='position:relative'>\
         <div style='width:20px'>z z z z</div>\
         <table style='border-spacing:0'><tbody><tr>\
         <td style='padding:0;height:45px'>T</td></tr></tbody></table></div>",
    );
    let text = output
        .iter()
        .filter(|(text, _, _)| text == "T")
        .map(|(_, page, y)| (*page, *y))
        .collect::<Vec<_>>();
    assert_eq!(text, [(1, 65.0)]);
}

fn table(cell_style: &str, row_style: &str, table_style: &str) -> String {
    format!(
        "<table style='border-spacing:0;{table_style}'><tbody>\
             <tr style='{row_style}'><td style='padding:0;{cell_style}'>T</td>\
             </tr></tbody></table><div style='font:10px/10px Ahem'>Q</div>"
    )
}

fn matching(output: &[(String, usize, f32)], text: &str) -> Vec<(usize, f32)> {
    output
        .iter()
        .filter(|(value, _, _)| value == text)
        .map(|(_, page, top)| (*page, *top))
        .collect()
}

#[test]
fn a_fragmented_cell_retains_height_but_drops_alignment_space_above_its_line() {
    let mut failures = Vec::new();
    for lead in [0, 30, 50] {
        for height in [30, 40, 45, 60, 100, 120] {
            for (align, factor) in [("top", 0.0), ("middle", 0.5), ("bottom", 1.0)] {
                let body = format!(
                    "<div style='height:{lead}px'></div><div style='position:relative'>\
                     <div style='width:20px'>z z z z</div>{}</div>",
                    table(&format!("height:{height}px;vertical-align:{align}"), "", "")
                );
                let output = lines(&body);
                let page = if lead == 50 { 2 } else { 1 };
                let top = 65.0 + if height == 40 { 10.0 * factor } else { 0.0 };
                let following = if height == 30 {
                    (page, 90.0)
                } else {
                    (page + 1, (height - 40) as f32)
                };
                if matching(&output, "T") != [(page, top)] || matching(&output, "Q") != [following]
                {
                    failures.push((lead, height, align, output));
                }
            }
        }
    }
    assert!(failures.is_empty(), "{failures:?}");
}

#[test]
fn actual_cell_padding_and_borders_still_precede_the_fitted_text() {
    for edge in ["padding:5px", "border:5px solid"] {
        let output = lines(&format!(
            "<div style='height:160px'></div>{}",
            table(&format!("height:45px;{edge}"), "", "")
        ));
        assert_eq!(matching(&output, "T"), [(1, 70.0)], "{edge}");
        assert_eq!(matching(&output, "Q"), [(2, 15.0)], "{edge}");
    }
}

#[test]
fn a_row_without_room_for_its_line_and_explicit_breaks_still_move_whole() {
    let output = lines(&format!(
        "<div style='height:180px'></div>{}",
        table("height:60px", "", "")
    ));
    assert_eq!(matching(&output, "T"), [(2, 20.0)]);
    assert_eq!(matching(&output, "Q"), [(2, 60.0)]);
    for target in 0..3 {
        for rule in ["break-inside:avoid", "break-before:page"] {
            let mut styles = ["height:45px".to_owned(), String::new(), String::new()];
            styles[target].push(';');
            styles[target].push_str(rule);
            let output = lines(&format!(
                "<div style='height:160px'></div>{}",
                table(&styles[0], &styles[1], &styles[2])
            ));
            let actual = matching(&output, "T");
            assert_eq!(actual.len(), 1, "{target} {rule}");
            assert_eq!(actual[0].0, 2, "{target} {rule}");
            assert!(
                (actual[0].1 - 13.0).abs() < 0.6,
                "{target} {rule}: {actual:?}"
            );
            assert_eq!(matching(&output, "Q"), [(2, 45.0)], "{target} {rule}");
        }
    }
}

#[test]
fn a_fitted_cell_keeps_its_border_box_origin_and_continuation_height() {
    let output = render(
        "<div style='height:160px'></div><table id=t style='border-spacing:0'>\
        <tr><td id=c style='padding:0;height:45px;background:blue'>T</td></tr>\
        </table><div style='font:10px/10px Ahem'>Q</div>",
    );
    assert_eq!(matching(&output.lines, "T"), [(1, 65.0)]);
    assert_eq!(matching(&output.lines, "Q"), [(2, 5.0)]);
    for id in ["t", "c"] {
        let boxes = output
            .boxes
            .iter()
            .filter(|(name, _, _, _)| name == id)
            .map(|(_, page, top, height)| (*page, *top, *height))
            .collect::<Vec<_>>();
        assert_eq!(boxes, [(1, 60.0, 40.0), (2, 0.0, 5.0)], "{id}");
    }
}
