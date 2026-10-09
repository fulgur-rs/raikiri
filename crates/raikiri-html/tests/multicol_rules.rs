//! Multicolumn paint projections preserve the producer's column placement.

use raikiri_html::{
    ClipKind, DocumentLayout, FontCollectionBuilder, LayoutOptions, LayoutStatus, PaintEvent,
    RenderResources, layout, parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

fn lay_out(body: &str, css: &str) -> DocumentLayout {
    let fonts = FontCollectionBuilder::new()
        .font_bytes(
            "Ahem",
            include_bytes!("../../raikiri-dom/tests/data/text-autospace/Ahem.ttf").to_vec(),
        )
        .build()
        .unwrap();
    let resources = RenderResources::new().fonts(fonts);
    let html = format!(
        "<!doctype html><style>@page{{size:180px 160px;margin:0}}body{{margin:0;font:20px/20px Ahem}}p{{margin:0}}.mc{{width:100px;column-count:2;column-gap:20px;orphans:1;widows:1}}{css}</style>{body}"
    );
    let parsed = parse_html_with_resources(html.as_bytes(), &resources).unwrap();
    let LayoutStatus::Completed(document) = layout(
        &parsed,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new().resources(&resources),
    )
    .unwrap() else {
        panic!("completed layout")
    };
    document
}

fn rules(document: &DocumentLayout) -> Vec<(f32, f32, f32, f32)> {
    let page = document.page(0).unwrap();
    page.paint_order()
        .iter()
        .filter_map(|event| match event {
            PaintEvent::ColumnRule(rule) => {
                Some((rule.rect.x, rule.rect.y, rule.rect.width, rule.rect.height))
            }
            _ => None,
        })
        .collect()
}

#[test]
fn public_wide_rule_is_below_the_owners_outside_text_marker() {
    for overflow in ["", "overflow:hidden"] {
        let document = lay_out(
            "<div class=mc><p>A<br>B<br>C<br>D</p></div>",
            &format!(
                ".mc{{display:list-item;list-style-position:outside;margin-left:40px;height:40px;column-rule:200px solid red;{overflow}}}.mc::marker{{content:'X';color:blue}}"
            ),
        );
        let page = document.page(0).unwrap();
        let runs = page.text_runs();
        let marker = runs.iter().find(|run| run.text == "X").unwrap();
        assert_eq!(marker.origin, (16.0, 16.0));
        let events = page.paint_order_for_text_runs(&runs);
        let rule_index = events
            .iter()
            .position(|event| matches!(event, PaintEvent::ColumnRule(_)))
            .unwrap();
        let marker_index = events
            .iter()
            .position(|event| matches!(event, PaintEvent::TextLine(line) if *line == marker.line))
            .unwrap();
        assert!(rule_index < marker_index);
        let mut clips = 0;
        for event in &events {
            match event {
                PaintEvent::PushClip(_, _) => clips += 1,
                PaintEvent::PopClip => clips -= 1,
                PaintEvent::TextLine(line) if *line == marker.line => assert_eq!(clips, 0),
                _ => {}
            }
        }
        assert_eq!(clips, 0);
    }
}

#[test]
fn public_rules_are_literal_gap_rectangles_and_do_not_move_text() {
    let document = lay_out(
        "<div class=mc>A<br>B<br>C<br>D</div><p>E</p>",
        ".mc{column-rule:2px solid red}",
    );
    assert_eq!(rules(&document), [(49.0, 0.0, 2.0, 40.0)]);
    let page = document.page(0).unwrap();
    let runs = page.text_runs();
    let events = page.paint_order_for_text_runs(&runs);
    let index = events
        .iter()
        .position(|event| matches!(event, PaintEvent::ColumnRule(_)))
        .unwrap();
    assert!(
        events[..index]
            .iter()
            .any(|event| matches!(event, PaintEvent::Box(_)))
    );
    assert!(
        !events[..index]
            .iter()
            .any(|event| matches!(event, PaintEvent::TextLine(_)))
    );
    assert_eq!(
        runs.iter()
            .map(|run| (run.text, run.origin))
            .collect::<Vec<_>>(),
        [
            ("A", (0.0, 16.0)),
            ("B", (0.0, 36.0)),
            ("C", (60.0, 16.0)),
            ("D", (60.0, 36.0)),
            ("E", (0.0, 56.0)),
        ]
    );
}

#[test]
fn public_rules_use_content_insets_and_the_full_fixed_column_height() {
    let document = lay_out(
        "<div class=mc>A<br>B<br>C<br>D</div>",
        ".mc{height:60px;padding:4px 5px;border:1px solid black;column-rule:2px solid red}",
    );
    assert_eq!(rules(&document), [(55.0, 5.0, 2.0, 60.0)]);
}

#[test]
fn public_rules_do_not_adjoin_empty_columns() {
    let document = lay_out(
        "<div class=mc>A<br>B<br>C<br>D</div>",
        ".mc{width:160px;column-count:3;height:40px;column-rule:2px solid red}",
    );
    assert_eq!(rules(&document), [(49.0, 0.0, 2.0, 40.0)]);
    let document = lay_out(
        "<div class=mc>A</div>",
        ".mc{height:60px;column-rule:2px solid red}",
    );
    assert!(rules(&document).is_empty());
}

#[test]
fn relative_content_offsets_do_not_change_column_occupancy() {
    for body in [
        "<div class=mc><span style='display:inline-block;width:20px;height:20px;position:relative;left:60px;background:blue'></span></div>",
        "<div class=mc><div style='height:20px;position:relative;left:60px;background:blue'></div><div style='height:20px;background:lime'></div></div>",
    ] {
        let document = lay_out(body, ".mc{height:20px;column-rule:2px solid red}");
        assert!(rules(&document).is_empty(), "{body}");
    }
}

#[test]
fn foundational_relative_offsets_preserve_the_flow_column_slots() {
    for inset in [
        "left:60px",
        "right:-60px",
        "left:60%",
        "left:calc(30% + 30px)",
    ] {
        let body = format!(
            "<div class=mc><div style='height:40px;min-width:40px;position:relative;{inset};background:blue'><br></div><div style='height:40px;min-width:40px;background:lime'><br></div></div>"
        );
        let document = lay_out(&body, ".mc{height:20px;column-rule:2px solid red}");
        assert_eq!(rules(&document), [(49.0, 0.0, 2.0, 20.0)], "{inset}");
    }
}

#[test]
fn public_rule_slices_respect_page_margins_on_every_page() {
    let letters = (b'A'..=b'Z')
        .map(|letter| (letter as char).to_string())
        .collect::<Vec<_>>()
        .join("<br>");
    let document = lay_out(
        &format!("<div class=mc>{letters}</div>"),
        "@page{margin:20px}.mc{height:260px;column-rule:2px solid red}",
    );
    assert_eq!(document.page_count(), 3);
    for (page_index, height, origin) in [(0, 120.0, 20.0), (1, 120.0, -100.0), (2, 20.0, -220.0)] {
        let page = document.page(page_index).unwrap();
        let rules = page
            .paint_order()
            .into_iter()
            .filter_map(|event| match event {
                PaintEvent::ColumnRule(rule) => Some(rule),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(rules.len(), 1);
        assert_eq!(
            rules[0].rect,
            raikiri_traits::PaintRect::new(69.0, 20.0, 2.0, height)
        );
        assert_eq!(rules[0].pattern_origin, origin);
        assert_eq!(rules[0].pattern_height, 260.0);
    }
}

#[test]
fn public_rule_is_inside_owner_opacity_and_overflow_but_outside_its_columns() {
    let document = lay_out(
        "<div class=mc>A<br>B<br>C<br>D</div>",
        ".mc{height:40px;opacity:.5;overflow:hidden;column-rule:160px solid red}",
    );
    let page = document.page(0).unwrap();
    let mut opacity = Vec::new();
    let mut clips = Vec::new();
    let mut found = 0;
    for event in page.paint_order() {
        match event {
            PaintEvent::PushOpacity(alpha) => opacity.push(alpha),
            PaintEvent::PopOpacity => {
                opacity.pop().unwrap();
            }
            PaintEvent::PushClip(clip, kind) => clips.push((clip, kind)),
            PaintEvent::PopClip => {
                clips.pop().unwrap();
            }
            PaintEvent::ColumnRule(rule) => {
                found += 1;
                assert_eq!(opacity, [0.5]);
                assert_eq!(
                    rule.rect,
                    raikiri_traits::PaintRect::new(-30.0, 0.0, 160.0, 40.0)
                );
                assert!(
                    clips
                        .iter()
                        .any(|(clip, kind)| matches!(kind, ClipKind::Overflow)
                            && clip.rect == raikiri_traits::PaintRect::new(0.0, 0.0, 100.0, 40.0))
                );
                assert!(
                    !clips
                        .iter()
                        .any(|(_, kind)| matches!(kind, ClipKind::Fragmentainer))
                );
            }
            _ => {}
        }
    }
    assert_eq!(found, 1);
    assert!(clips.is_empty());
    assert!(opacity.is_empty());
}

#[test]
fn rule_slices_keep_the_original_pattern_across_pages() {
    let letters = (b'A'..=b'Z')
        .map(|letter| (letter as char).to_string())
        .collect::<Vec<_>>()
        .join("<br>");
    let document = lay_out(
        &format!("<div class=mc>{letters}</div>"),
        ".mc{height:260px;column-rule:2px dashed red}",
    );
    assert_eq!(document.page_count(), 2);
    for (page_index, height, pattern_origin) in [(0, 160.0, 0.0), (1, 100.0, -160.0)] {
        let page = document.page(page_index).unwrap();
        let rules = page
            .paint_order()
            .into_iter()
            .filter_map(|event| match event {
                PaintEvent::ColumnRule(rule) => Some(rule),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(rules.len(), 1);
        assert_eq!(
            rules[0].rect,
            raikiri_traits::PaintRect::new(49.0, 0.0, 2.0, height)
        );
        assert_eq!(rules[0].pattern_origin, pattern_origin);
        assert_eq!(rules[0].pattern_height, 260.0);
    }
}

#[test]
fn hidden_or_off_page_rules_do_not_emit_public_commands() {
    for style in ["visibility:hidden", "position:relative;top:160px"] {
        let document = lay_out(
            "<div class=mc>A<br>B<br>C<br>D</div>",
            &format!(".mc{{height:40px;column-rule:2px solid red;{style}}}"),
        );
        assert!(rules(&document).is_empty());
    }
}
