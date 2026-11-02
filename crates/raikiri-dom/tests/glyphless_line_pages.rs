//! Lines without glyphs keep their line boxes intact at page boundaries.

use raikiri_html::{
    LayoutOptions, LayoutStatus, RenderResources, layout, parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

type Placements = std::collections::BTreeMap<String, Vec<(usize, f32, f32)>>;

fn paginate(body: &str, css: &str) -> (usize, Placements) {
    let html = format!(
        "<!DOCTYPE html><style>@page{{size:480px 288px;margin:48px}}\
         body{{margin:0;line-height:10px}}ul{{margin:0;padding:0}}{css}</style>\
         {body}"
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
        panic!("expected complete layout");
    };
    let mut placements = Placements::new();
    let mut pages = 0;
    for (page_index, page) in result.pages().enumerate() {
        pages += 1;
        for fragment in page.fragments() {
            let Some(id) = page.dom().attr(fragment.node(), "id") else {
                continue;
            };
            let rect = fragment.rect();
            placements
                .entry(id.into())
                .or_default()
                .push((page_index, rect.y - 48.0, rect.height));
        }
    }
    (pages, placements)
}

fn assert_twenty_one_lines(tag: &str, content: &str, css: &str) {
    let children = (0..21)
        .map(|i| format!("<{tag} id=i{i}>{content}</{tag}>"))
        .collect::<String>();
    let body = if tag == "li" {
        format!("<ul>{children}</ul>")
    } else {
        children
    };
    let (pages, placements) = paginate(&body, css);
    assert_eq!(pages, 2);
    for index in 0..21 {
        let fragments = &placements[&format!("i{index}")];
        let (page, y) = if index < 19 {
            (0, index as f32 * 10.0)
        } else {
            (1, (index - 19) as f32 * 10.0)
        };
        assert_eq!(
            fragments.len(),
            1,
            "{tag}/{content}, i{index}: {fragments:?}"
        );
        let actual = fragments[0];
        assert!(
            actual.0 == page && (actual.1 - y).abs() < 0.04 && (actual.2 - 10.0).abs() < 0.04,
            "{tag}/{content}, i{index}: {actual:?}, expected ({page}, {y}, 10)"
        );
    }
}

#[test]
fn list_items_with_only_br_move_the_whole_line() {
    assert_twenty_one_lines("li", "<br>", "");
}

#[test]
fn empty_list_items_move_the_whole_line() {
    assert_twenty_one_lines("li", "", "");
}

#[test]
fn blocks_with_only_br_move_the_whole_line() {
    assert_twenty_one_lines("div", "<br>", "");
}

#[test]
fn lines_with_glyphs_keep_their_existing_page_placement() {
    assert_twenty_one_lines("li", "x", "");
}

#[test]
fn marker_position_and_absence_do_not_change_br_line_placement() {
    assert_twenty_one_lines("li", "<br>", "li{list-style:none}");
    assert_twenty_one_lines("li", "", "li{list-style-position:inside}");
    assert_twenty_one_lines("li", " ", "");
}

fn line_after_190px(content: &str, css: &str) -> (usize, Placements) {
    paginate(
        &format!(
            "<div style='height:190px'></div><div id=target>{content}</div><div id=after style='height:1px'></div>"
        ),
        css,
    )
}

#[test]
fn a_box_without_a_line_remains_fragmentable() {
    let (pages, placements) = line_after_190px("", "#target{height:10px}");
    assert_eq!(pages, 2);
    assert_eq!(placements["target"], [(0, 190.0, 2.0), (1, 0.0, 8.0)]);
}

#[test]
fn a_zero_height_br_does_not_add_a_page() {
    let (pages, placements) = line_after_190px("<br>", "#target{line-height:0}");
    assert_eq!(pages, 1);
    assert_eq!(placements["after"], [(0, 190.0, 1.0)]);
}

#[test]
fn a_line_taller_than_a_page_does_not_move_to_a_blank_page() {
    let css = "#target{line-height:250px}";
    let (pages, placements) = paginate("<div id=target><br></div>", css);
    assert_eq!(pages, 2);
    assert_eq!(placements["target"].first(), Some(&(0, 0.0, 192.0)));
    assert_eq!(line_after_190px("<br>", css), line_after_190px("x", css));
}

#[test]
fn padding_border_and_margin_follow_the_rules_for_a_line_with_glyphs() {
    let css = "#target{border:2px solid;padding-top:3px;padding-bottom:200px;margin-bottom:7px}";
    let glyphless = line_after_190px("<br>", css);
    let glyphs = line_after_190px("x", css);
    assert_eq!(glyphless, glyphs);
    assert_eq!(glyphless.1["target"][0].0, 1);
    assert!(glyphless.1["target"].len() > 1);
}

#[test]
fn an_oversized_first_piece_is_not_moved_just_because_its_line_fits() {
    for css in [
        "#target{padding-top:200px}",
        "#target{border-top:5px solid;padding-top:180px}",
    ] {
        assert_eq!(
            line_after_190px("<br>", css),
            line_after_190px("x", css),
            "{css}"
        );
    }
}

#[test]
fn a_short_box_keeps_the_same_clamped_line_end_as_text() {
    let css = "#target{height:5px}";
    let glyphless = line_after_190px("<br>", css);
    assert_eq!(glyphless, line_after_190px("x", css));
    assert_eq!(glyphless.1["target"], [(1, 0.0, 5.0)]);
}

#[test]
fn an_oversized_line_in_a_short_box_keeps_the_text_placement() {
    let css = "#target{height:5px;line-height:250px}";
    assert_eq!(line_after_190px("<br>", css), line_after_190px("x", css));
}

#[test]
fn an_explicit_avoid_keeps_its_oversized_box_behavior() {
    let css = "#target{line-height:250px;break-inside:avoid}";
    assert_eq!(line_after_190px("<br>", css), line_after_190px("x", css));
}

#[test]
fn a_glyphless_line_can_move_when_the_next_page_is_taller() {
    let css = "@page{size:480px 400px}@page:first{size:480px 288px}#target{line-height:250px}";
    let (pages, placements) = line_after_190px("<br>", css);
    assert_eq!(pages, 2);
    assert_eq!(placements["target"], [(1, 0.0, 250.0)]);
    assert_eq!(placements["after"], [(1, 250.0, 1.0)]);
}

#[test]
fn a_nested_block_moves_its_glyphless_line() {
    let (pages, placements) = paginate(
        "<section><div style='height:190px'></div><div id=target><span><br></span></div></section>",
        "",
    );
    assert_eq!(pages, 2);
    assert_eq!(placements["target"], [(1, 0.0, 10.0)]);
}
