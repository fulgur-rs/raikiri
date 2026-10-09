//! Full-width spanners interrupt independently balanced column groups.

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
        "<!doctype html><style>@page{{size:180px 160px;margin:0}}body{{margin:0;font:20px/20px Ahem}}p{{margin:0}}.mc{{width:100px;column-count:2;column-gap:20px;orphans:1;widows:1;column-rule:2px solid red}}.span{{column-span:all}}{css}</style>{body}"
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

#[test]
fn fullwidth_spanner_balances_both_column_groups_and_interrupts_rules() {
    let document = lay_out(
        "<div class=mc><p>A<br>B<br>C<br>D</p><div class=span>E</div><p>F<br>G<br>H<br>I</p></div><p>J</p>",
        "",
    );
    assert_eq!(document.page_count(), 1);
    let page = document.page(0).unwrap();
    let runs = page.text_runs();
    let rules: Vec<_> = page
        .paint_order_for_text_runs(&runs)
        .iter()
        .filter_map(|event| {
            if let PaintEvent::ColumnRule(rule) = event {
                Some((rule.rect.x, rule.rect.y, rule.rect.width, rule.rect.height))
            } else {
                None
            }
        })
        .collect();
    assert_eq!(rules, [(49.0, 0.0, 2.0, 40.0), (49.0, 60.0, 2.0, 40.0)]);
    for (text, origin) in [
        ("A", (0.0, 16.0)),
        ("B", (0.0, 36.0)),
        ("C", (60.0, 16.0)),
        ("D", (60.0, 36.0)),
        ("E", (0.0, 56.0)),
        ("F", (0.0, 76.0)),
        ("G", (0.0, 96.0)),
        ("H", (60.0, 76.0)),
        ("I", (60.0, 96.0)),
        ("J", (0.0, 116.0)),
    ] {
        let matching: Vec<_> = runs.iter().filter(|run| run.text == text).collect();
        assert_eq!(matching.len(), 1, "{text}");
        assert_eq!(matching[0].origin, origin, "{text}");
    }
    let spanner = page
        .fragments()
        .find(|fragment| {
            page.dom().local_name(fragment.node()) == Some("div") && fragment.rect().y == 40.0
        })
        .expect("spanner fragment");
    assert_eq!(spanner.rect().width, 100.0);
    let mut clips = Vec::new();
    for event in page.paint_order_for_text_runs(&runs) {
        match event {
            PaintEvent::PushClip(_, kind) => clips.push(kind),
            PaintEvent::PopClip => {
                clips.pop().unwrap();
            }
            PaintEvent::Box(fragment) if fragment.node() == spanner.node() => {
                assert!(!clips.contains(&ClipKind::Fragmentainer));
            }
            _ => {}
        }
    }
    assert!(clips.is_empty());
}

fn assert_runs(document: &DocumentLayout, expected: &[(&str, (f32, f32))]) {
    let page = document.page(0).unwrap();
    let runs = page.text_runs();
    assert_eq!(
        runs.len(),
        expected.len(),
        "{:?}",
        runs.iter()
            .map(|run| (run.text, run.origin))
            .collect::<Vec<_>>()
    );
    for &(text, origin) in expected {
        let matching: Vec<_> = runs.iter().filter(|run| run.text == text).collect();
        assert_eq!(matching.len(), 1, "{text}");
        assert_eq!(matching[0].origin, origin, "{text}");
    }
}

fn rule_rects(document: &DocumentLayout) -> Vec<(f32, f32, f32, f32)> {
    document
        .page(0)
        .unwrap()
        .paint_order()
        .iter()
        .filter_map(|event| {
            if let PaintEvent::ColumnRule(rule) = event {
                Some((rule.rect.x, rule.rect.y, rule.rect.width, rule.rect.height))
            } else {
                None
            }
        })
        .collect()
}

#[test]
fn leading_trailing_and_consecutive_spanners_have_no_empty_group_rules() {
    let document = lay_out(
        "<div class=mc><div class=span>A</div><div class=span>B</div><p>C<br>D</p><div class=span>E</div></div><p>F</p>",
        "",
    );
    assert_runs(
        &document,
        &[
            ("A", (0.0, 16.0)),
            ("B", (0.0, 36.0)),
            ("C", (0.0, 56.0)),
            ("D", (60.0, 56.0)),
            ("E", (0.0, 76.0)),
            ("F", (0.0, 96.0)),
        ],
    );
    assert_eq!(rule_rects(&document), [(49.0, 40.0, 2.0, 20.0)]);
}

#[test]
fn auto_fill_balances_before_a_spanner_but_preserves_unbalanced_final_group() {
    let document = lay_out(
        "<div class=mc><p>A<br>B<br>C<br>D</p><div class=span>E</div><p>F<br>G<br>H<br>I</p></div><p>J</p>",
        ".mc{column-fill:auto}",
    );
    assert_runs(
        &document,
        &[
            ("A", (0.0, 16.0)),
            ("B", (0.0, 36.0)),
            ("C", (60.0, 16.0)),
            ("D", (60.0, 36.0)),
            ("E", (0.0, 56.0)),
            ("F", (0.0, 76.0)),
            ("G", (0.0, 96.0)),
            ("H", (0.0, 116.0)),
            ("I", (0.0, 136.0)),
            ("J", (0.0, 156.0)),
        ],
    );
    assert_eq!(rule_rects(&document), [(49.0, 0.0, 2.0, 40.0)]);
}

#[test]
fn consecutive_spanner_margins_collapse_and_next_column_group_keeps_separation() {
    let document = lay_out(
        "<div class=mc><div class='span first'>A</div><div class='span second'>B</div><p>C<br>D</p></div><p>E</p>",
        ".first{margin-bottom:10px}.second{margin-top:15px;margin-bottom:5px}",
    );
    assert_runs(
        &document,
        &[
            ("A", (0.0, 16.0)),
            ("B", (0.0, 51.0)),
            ("C", (0.0, 76.0)),
            ("D", (60.0, 76.0)),
            ("E", (0.0, 96.0)),
        ],
    );
    assert_eq!(rule_rects(&document), [(49.0, 60.0, 2.0, 20.0)]);
}

#[test]
fn source_whitespace_does_not_change_single_paragraph_group_balancing() {
    let document = lay_out(
        "<div class=mc>\n <p>A<br>B<br>C<br>D</p>\n <div class=span>E</div>\n <p>F<br>G<br>H<br>I</p>\n </div><p>J</p>",
        "",
    );
    assert_runs(
        &document,
        &[
            ("A", (0.0, 16.0)),
            ("B", (0.0, 36.0)),
            ("C", (60.0, 16.0)),
            ("D", (60.0, 36.0)),
            ("E", (0.0, 56.0)),
            ("F", (0.0, 76.0)),
            ("G", (0.0, 96.0)),
            ("H", (60.0, 76.0)),
            ("I", (60.0, 96.0)),
            ("J", (0.0, 116.0)),
        ],
    );
    assert_eq!(
        rule_rects(&document),
        [(49.0, 0.0, 2.0, 40.0), (49.0, 60.0, 2.0, 40.0)]
    );
}

#[test]
fn overflow_clips_follow_each_column_group_and_leave_the_spanner_fullwidth() {
    let document = lay_out(
        "<div class=mc><p>A<br>B<br>C<br>D</p><div class=span>E</div><p>F<br>G<br>H<br>I</p></div><p>J</p>",
        ".mc p{overflow:hidden}.span{overflow:hidden}",
    );
    let page = document.page(0).unwrap();
    let runs = page.text_runs();
    let mut clips = Vec::new();
    let mut actual = Vec::new();
    for event in page.paint_order_for_text_runs(&runs) {
        match event {
            PaintEvent::PushClip(clip, _) => clips.push(clip.rect),
            PaintEvent::PopClip => {
                clips.pop().unwrap();
            }
            PaintEvent::TextLine(line) => {
                let run = runs.iter().find(|run| run.line == line).unwrap();
                actual.push((run.text, clips.last().copied()));
            }
            _ => {}
        }
    }
    use raikiri_traits::PaintRect;
    let left = Some(PaintRect::new(0.0, 0.0, 40.0, 40.0));
    let right = Some(PaintRect::new(60.0, 0.0, 40.0, 40.0));
    let bottom_left = Some(PaintRect::new(0.0, 60.0, 40.0, 40.0));
    let bottom_right = Some(PaintRect::new(60.0, 60.0, 40.0, 40.0));
    assert_eq!(
        actual,
        [
            ("A", left),
            ("B", left),
            ("C", right),
            ("D", right),
            ("E", Some(PaintRect::new(0.0, 40.0, 100.0, 20.0))),
            ("F", bottom_left),
            ("G", bottom_left),
            ("H", bottom_right),
            ("I", bottom_right),
            ("J", None),
        ]
    );
    assert!(clips.is_empty());
}

#[test]
fn padding_and_decorated_spanner_preserve_one_owner_background_and_group_insets() {
    let document = lay_out(
        "<div class=mc><p>A<br>B<br>C<br>D</p><div class=span>E</div><p>F<br>G<br>H<br>I</p></div><p>J</p>",
        ".mc{padding:4px 5px;border:1px solid black;background:lime}.span{padding:2px 3px;border:1px solid blue;margin:5px 0}",
    );
    assert_runs(
        &document,
        &[
            ("A", (6.0, 21.0)),
            ("B", (6.0, 41.0)),
            ("C", (66.0, 21.0)),
            ("D", (66.0, 41.0)),
            ("E", (10.0, 69.0)),
            ("F", (6.0, 97.0)),
            ("G", (6.0, 117.0)),
            ("H", (66.0, 97.0)),
            ("I", (66.0, 117.0)),
            ("J", (0.0, 142.0)),
        ],
    );
    assert_eq!(
        rule_rects(&document),
        [(55.0, 5.0, 2.0, 40.0), (55.0, 81.0, 2.0, 40.0)]
    );
    let page = document.page(0).unwrap();
    let owner: Vec<_> = page
        .fragments()
        .filter(|fragment| {
            page.dom().local_name(fragment.node()) == Some("div") && fragment.rect().y == 0.0
        })
        .collect();
    assert_eq!(owner.len(), 1);
    assert_eq!(
        owner[0].rect(),
        raikiri_traits::PaintRect::new(0.0, 0.0, 112.0, 126.0)
    );
}

#[test]
fn short_page_keeps_spanner_and_group_positions_across_page_slices() {
    let document = lay_out(
        "<div class=mc><p>A<br>B<br>C<br>D</p><div class=span>E</div><p>F<br>G<br>H<br>I</p></div><p>J</p>",
        "@page{size:180px 60px}",
    );
    assert_eq!(document.page_count(), 2);
    for (page_number, expected) in [
        (
            0,
            vec![
                ("A", (0.0, 16.0)),
                ("B", (0.0, 36.0)),
                ("C", (60.0, 16.0)),
                ("D", (60.0, 36.0)),
                ("E", (0.0, 56.0)),
            ],
        ),
        (
            1,
            vec![
                ("F", (0.0, 16.0)),
                ("G", (0.0, 36.0)),
                ("H", (60.0, 16.0)),
                ("I", (60.0, 36.0)),
                ("J", (0.0, 56.0)),
            ],
        ),
    ] {
        let page = document.page(page_number).unwrap();
        let runs = page.text_runs();
        assert_eq!(
            runs.iter()
                .map(|run| (run.text, run.origin))
                .collect::<Vec<_>>(),
            expected
        );
        let rules: Vec<_> = page
            .paint_order()
            .iter()
            .filter_map(|event| match event {
                PaintEvent::ColumnRule(rule) => Some((
                    rule.rect.x,
                    rule.rect.y,
                    rule.rect.height,
                    rule.pattern_origin,
                    rule.pattern_height,
                )),
                _ => None,
            })
            .collect();
        assert_eq!(rules, [(49.0, 0.0, 40.0, 0.0, 40.0)]);
    }
}

#[test]
fn a_column_group_crossing_pages_fills_each_page_before_balancing_the_last() {
    let document = lay_out(
        "<div class=mc><p>A<br>B<br>C<br>D<br>E<br>F<br>G<br>H</p><div class=span>I</div><p>J<br>K</p></div><p>L</p>",
        "@page{size:180px 60px}",
    );
    assert_eq!(document.page_count(), 3);
    for (page_number, expected) in [
        (
            0,
            vec![
                ("A", (0.0, 16.0)),
                ("B", (0.0, 36.0)),
                ("C", (0.0, 56.0)),
                ("D", (60.0, 16.0)),
                ("E", (60.0, 36.0)),
                ("F", (60.0, 56.0)),
            ],
        ),
        (
            1,
            vec![
                ("G", (0.0, 16.0)),
                ("H", (60.0, 16.0)),
                ("I", (0.0, 36.0)),
                ("J", (0.0, 56.0)),
                ("K", (60.0, 56.0)),
            ],
        ),
        (2, vec![("L", (0.0, 16.0))]),
    ] {
        let runs = document.page(page_number).unwrap().text_runs();
        assert_eq!(
            runs.iter()
                .map(|run| (run.text, run.origin))
                .collect::<Vec<_>>(),
            expected
        );
    }
}

#[test]
fn a_spanner_that_cannot_fit_moves_before_the_following_group_is_paginated() {
    let document = lay_out(
        "<div class=mc><p>A<br>B<br>C<br>D</p><div class=span>E</div><p>F<br>G<br>H<br>I</p></div><p>J</p>",
        "@page{size:180px 50px}",
    );
    assert_eq!(document.page_count(), 3);
    for (number, expected) in [
        (
            0,
            vec![
                ("A", (0.0, 16.0)),
                ("B", (0.0, 36.0)),
                ("C", (60.0, 16.0)),
                ("D", (60.0, 36.0)),
            ],
        ),
        (
            1,
            vec![("E", (0.0, 16.0)), ("F", (0.0, 36.0)), ("G", (60.0, 36.0))],
        ),
        (
            2,
            vec![("H", (0.0, 16.0)), ("I", (60.0, 16.0)), ("J", (0.0, 36.0))],
        ),
    ] {
        let runs = document.page(number).unwrap().text_runs();
        assert_eq!(
            runs.iter()
                .map(|run| (run.text, run.origin))
                .collect::<Vec<_>>(),
            expected
        );
    }
}

fn assert_page_runs(document: &DocumentLayout, number: u32, expected: &[(&str, (f32, f32))]) {
    let runs = document.page(number).unwrap().text_runs();
    assert_eq!(
        runs.iter()
            .map(|run| (run.text, run.origin))
            .collect::<Vec<_>>(),
        expected
    );
}

#[test]
fn both_groups_resume_in_later_pages_without_repeating_their_first_columns() {
    let document = lay_out(
        "<div class=mc><p>A<br>B<br>C<br>D<br>E<br>F<br>G<br>H</p><div class=span>I</div><p>J<br>K<br>L<br>M<br>N<br>O<br>P<br>Q</p></div><p>R</p>",
        "@page{size:180px 60px}",
    );
    assert_eq!(document.page_count(), 4);
    assert_page_runs(
        &document,
        0,
        &[
            ("A", (0.0, 16.0)),
            ("B", (0.0, 36.0)),
            ("C", (0.0, 56.0)),
            ("D", (60.0, 16.0)),
            ("E", (60.0, 36.0)),
            ("F", (60.0, 56.0)),
        ],
    );
    assert_page_runs(
        &document,
        1,
        &[
            ("G", (0.0, 16.0)),
            ("H", (60.0, 16.0)),
            ("I", (0.0, 36.0)),
            ("J", (0.0, 56.0)),
            ("K", (60.0, 56.0)),
        ],
    );
    assert_page_runs(
        &document,
        2,
        &[
            ("L", (0.0, 16.0)),
            ("M", (0.0, 36.0)),
            ("N", (0.0, 56.0)),
            ("O", (60.0, 16.0)),
            ("P", (60.0, 36.0)),
            ("Q", (60.0, 56.0)),
        ],
    );
    assert_page_runs(&document, 3, &[("R", (0.0, 16.0))]);
    for (number, expected) in [
        (0, vec![(49.0, 0.0, 2.0, 60.0)]),
        (1, vec![(49.0, 0.0, 2.0, 20.0), (49.0, 40.0, 2.0, 20.0)]),
        (2, vec![(49.0, 0.0, 2.0, 60.0)]),
        (3, vec![]),
    ] {
        let page = document.page(number).unwrap();
        let rules: Vec<_> = page
            .paint_order()
            .iter()
            .filter_map(|event| match event {
                PaintEvent::ColumnRule(rule) => {
                    Some((rule.rect.x, rule.rect.y, rule.rect.width, rule.rect.height))
                }
                _ => None,
            })
            .collect();
        assert_eq!(rules, expected);
    }
}

#[test]
fn a_group_uses_the_remaining_page_height_and_extends_following_ancestor_flow() {
    let document = lay_out(
        "<p>X</p><section><div class=mc><p>A<br>B<br>C<br>D</p><p>E<br>F<br>G<br>H</p><div class=span>I</div><p>J<br>K</p></div><p>L</p></section><p>Z</p>",
        "@page{size:180px 60px}",
    );
    assert_eq!(document.page_count(), 3);
    assert_page_runs(
        &document,
        0,
        &[
            ("X", (0.0, 16.0)),
            ("A", (0.0, 36.0)),
            ("B", (0.0, 56.0)),
            ("C", (60.0, 36.0)),
            ("D", (60.0, 56.0)),
        ],
    );
    assert_page_runs(
        &document,
        1,
        &[
            ("E", (0.0, 16.0)),
            ("F", (0.0, 36.0)),
            ("G", (60.0, 16.0)),
            ("H", (60.0, 36.0)),
            ("I", (0.0, 56.0)),
        ],
    );
    assert_page_runs(
        &document,
        2,
        &[
            ("J", (0.0, 16.0)),
            ("K", (60.0, 16.0)),
            ("L", (0.0, 36.0)),
            ("Z", (0.0, 56.0)),
        ],
    );
}

#[test]
fn paragraph_inline_margins_do_not_move_the_column_clip_or_rules() {
    let document = lay_out(
        "<div class=mc><p>A<br>B<br>C<br>D</p><div class=span>E</div><p>F<br>G<br>H<br>I</p></div><p>J</p>",
        ".mc p{margin-left:5px;margin-right:5px}",
    );
    assert_runs(
        &document,
        &[
            ("A", (5.0, 16.0)),
            ("B", (5.0, 36.0)),
            ("C", (65.0, 16.0)),
            ("D", (65.0, 36.0)),
            ("E", (0.0, 56.0)),
            ("F", (5.0, 76.0)),
            ("G", (5.0, 96.0)),
            ("H", (65.0, 76.0)),
            ("I", (65.0, 96.0)),
            ("J", (0.0, 116.0)),
        ],
    );
    assert_eq!(
        rule_rects(&document),
        [(49.0, 0.0, 2.0, 40.0), (49.0, 60.0, 2.0, 40.0)]
    );
}

#[test]
fn absolute_owners_and_ancestors_do_not_extend_normal_following_flow() {
    for css in [
        ".mc{position:absolute;top:0}",
        "section{position:absolute;top:0}",
    ] {
        let document = lay_out(
            "<section><div class=mc><p>A<br>B<br>C<br>D<br>E<br>F<br>G<br>H</p><div class=span>I</div><p>J<br>K</p></div></section><p>Z</p>",
            &format!("@page{{size:180px 50px}}{css}"),
        );
        let page = document.page(0).unwrap();
        let runs = page.text_runs();
        let following: Vec<_> = runs.iter().filter(|run| run.text == "Z").collect();
        assert_eq!(following.len(), 1, "{css}");
        assert_eq!(following[0].origin, (0.0, 16.0), "{css}");
    }
}

#[test]
fn bare_ifc_text_follows_the_paginated_owner_height_before_outer_following_flow() {
    let document = lay_out(
        "<section><div class=mc><p>A<br>B<br>C<br>D</p><div class=span>E</div><p>F<br>G<br>H<br>I</p></div>L</section><p>Z</p>",
        "@page{size:180px 60px}.span{height:30px}",
    );
    assert_eq!(document.page_count(), 3);
    assert_page_runs(
        &document,
        0,
        &[
            ("A", (0.0, 16.0)),
            ("B", (0.0, 36.0)),
            ("C", (60.0, 16.0)),
            ("D", (60.0, 36.0)),
        ],
    );
    assert_page_runs(
        &document,
        1,
        &[("E", (0.0, 16.0)), ("F", (0.0, 46.0)), ("G", (60.0, 46.0))],
    );
    // Mixed IFC roots already enumerate their own text before block children.
    // This control checks once-only text and its physical following position.
    let page = document.page(2).unwrap();
    let runs = page.text_runs();
    assert_eq!(runs.len(), 4);
    for (text, origin) in [
        ("H", (0.0, 16.0)),
        ("I", (60.0, 16.0)),
        ("L", (0.0, 36.0)),
        ("Z", (0.0, 56.0)),
    ] {
        let matching: Vec<_> = runs.iter().filter(|run| run.text == text).collect();
        assert_eq!(matching.len(), 1, "{text}");
        assert_eq!(matching[0].origin, origin, "{text}");
    }
}
