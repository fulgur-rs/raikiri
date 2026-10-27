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
    use raikiri_traits::PaintRect;
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
                // The area every clip in effect leaves. A column group's own
                // clip is open along the inline axis.
                let visible = clips.iter().copied().reduce(|a: PaintRect, b| {
                    let (x, y) = (a.x.max(b.x), a.y.max(b.y));
                    PaintRect::new(
                        x,
                        y,
                        (a.x + a.width).min(b.x + b.width) - x,
                        (a.y + a.height).min(b.y + b.height) - y,
                    )
                });
                actual.push((run.text, visible));
            }
            _ => {}
        }
    }
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
fn a_fragment_clip_stays_open_where_its_paragraph_begins_and_ends() {
    let document = lay_out(
        "<div class=mc><p>A<br>B<br>C<br>D</p><div class=span>E</div><p>F<br>G<br>H<br>I</p></div>",
        "",
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
                // Whether the clip leaves room past its group's top and bottom
                // edge, and where the group's edges lie.
                let edges = clips.last().map(|clip| {
                    (
                        clip.y < -1000.0,
                        clip.y + clip.height > 1000.0,
                        if clip.y < -1000.0 { None } else { Some(clip.y) },
                        if clip.y + clip.height > 1000.0 {
                            None
                        } else {
                            Some(clip.y + clip.height)
                        },
                    )
                });
                actual.push((run.text, edges));
            }
            _ => {}
        }
    }
    // The first fragment of a paragraph has nothing of it above it and the
    // last has nothing below it; the edges between its fragments stay cut so
    // that the lines of the neighbour do not show.
    let first = Some((true, false, None, Some(40.0)));
    let last = Some((false, true, Some(0.0), None));
    let later_first = Some((true, false, None, Some(100.0)));
    let later_last = Some((false, true, Some(60.0), None));
    assert_eq!(
        actual,
        [
            ("A", first),
            ("B", first),
            ("C", last),
            ("D", last),
            ("E", None),
            ("F", later_first),
            ("G", later_first),
            ("H", later_last),
            ("I", later_last),
        ]
    );
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

#[test]
fn a_spanner_with_no_lines_breaks_at_the_page_end_like_any_block() {
    // The spanner is 40px tall and starts at 40px on pages of 60px. It holds
    // no line, so the page ends inside it and the 20px that remain lie at the
    // top of the next page, where the second group starts below them. The
    // height may come from a block inside the spanner as well.
    for spanner in [
        "<div class=span style='height:40px'></div>",
        "<div class=span><div style='height:40px'></div></div>",
    ] {
        let document = lay_out(
            &format!("<div class=mc><p>A<br>B<br>C<br>D</p>{spanner}<p>F<br>G<br>H<br>I</p></div>"),
            "@page{size:180px 60px}",
        );
        assert_eq!(document.page_count(), 2, "{spanner}");
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
            &[
                ("F", (0.0, 36.0)),
                ("G", (0.0, 56.0)),
                ("H", (60.0, 36.0)),
                ("I", (60.0, 56.0)),
            ],
        );
    }
}

#[test]
fn a_spanner_whose_lines_end_above_the_page_end_continues_on_the_next_page() {
    // The spanner is 80px tall and starts at 40px on pages of 100px, so the
    // page ends 60px into it. Its only line ends at 20px, so the page ends in
    // the empty space below it, and the 20px that remain are at the top of the
    // next page.
    let document = lay_out(
        "<div class=mc><p>A<br>B<br>C<br>D</p><div class=span style='height:80px'>E</div><p>F<br>G<br>H<br>I</p></div>",
        "@page{size:180px 100px}",
    );
    assert_eq!(document.page_count(), 2);
    assert_page_runs(
        &document,
        0,
        &[
            ("A", (0.0, 16.0)),
            ("B", (0.0, 36.0)),
            ("C", (60.0, 16.0)),
            ("D", (60.0, 36.0)),
            ("E", (0.0, 56.0)),
        ],
    );
    assert_page_runs(
        &document,
        1,
        &[
            ("F", (0.0, 36.0)),
            ("G", (0.0, 56.0)),
            ("H", (60.0, 36.0)),
            ("I", (60.0, 56.0)),
        ],
    );
}

#[test]
fn a_spanner_with_content_at_the_page_end_moves_whole_to_the_next_page() {
    // The spanner is 40px tall and starts at 40px on pages of 70px, so a page
    // would end 30px into it. Each spanner has something there that the page
    // does not cut, so it moves whole to the top of the next page, and the
    // paragraph after the container starts below it.
    for spanner in [
        // A line crosses the page end.
        "<div class=span style='padding-top:20px;height:20px'>E</div>",
        // So does a line of a paragraph inside it.
        "<div class=span style='height:40px'><div style='height:20px'></div><p>E</p></div>",
        // A float crosses it although the only line ends above.
        "<div class=span style='height:40px'>E<span style='float:left;width:10px;height:40px'></span></div>",
        // A border and a padding lie there.
        "<div class=span style='border-top:40px solid'></div>",
        "<div class=span style='padding-bottom:40px'></div>",
        // The spanner may not break inside.
        "<div class=span style='height:40px;break-inside:avoid'></div>",
        // Nor may a box inside it.
        "<div class=span><div style='height:40px;break-inside:avoid'></div></div>",
    ] {
        let document = lay_out(
            &format!("<div class=mc><p>A<br>B<br>C<br>D</p>{spanner}</div><p>K</p>"),
            "@page{size:180px 70px}",
        );
        assert_eq!(document.page_count(), 2, "{spanner}");
        let runs = document.page(1).unwrap().text_runs();
        assert_eq!(
            runs.last().map(|run| (run.text, run.origin)),
            Some(("K", (0.0, 56.0))),
            "{spanner}"
        );
    }
}

#[test]
fn a_spanner_is_cut_only_where_every_page_end_it_crosses_allows() {
    // The first page is 100px tall and the next ones 50px. The spanner is 80px
    // tall and starts at 80px, so it crosses the page ends at 100px and 150px.
    // Cut at both, a plain spanner stays, and the paragraph after the
    // container starts at 160px, 10px into the third page. The border of the
    // other one would be cut at 150px, so it moves whole to the top of the
    // second page and the paragraph starts at 180px.
    for (spanner, expected) in [
        ("<div class=span style='height:80px'></div>", 26.0),
        (
            "<div class=span style='height:60px;border-bottom:20px solid'></div>",
            46.0,
        ),
    ] {
        let document = lay_out(
            &format!(
                "<div class=mc><p>A<br>B<br>C<br>D<br>E<br>F<br>G<br>H</p>{spanner}</div><p>K</p>"
            ),
            "@page{size:180px 50px}@page:first{size:180px 100px}",
        );
        assert_eq!(document.page_count(), 3, "{spanner}");
        assert_page_runs(&document, 2, &[("K", (0.0, expected))]);
    }
}

#[test]
fn a_cut_in_a_border_is_found_among_ranges_that_overlap() {
    // The pages are 120px tall and the spanner starts at 40px, so the page ends
    // 80px into it. The box inside it is 50px tall and has a 40px border below
    // that, which lies from 51px to 91px. A box in it that overflows its 50px
    // has a border from 61px to 66px, which ends before the page does. The cut
    // still lies in the first border, so the spanner moves whole to the top of
    // the next page and the paragraph after the container starts below it.
    let document = lay_out(
        "<div class=mc><p>A<br>B<br>C<br>D</p>\
         <div class=span style='height:100px'>\
         <div style='padding-top:1px;height:50px;border-bottom:40px solid'>\
         <div style='margin-top:55px;height:5px;border-bottom:5px solid'></div>\
         </div></div></div><p>K</p>",
        "@page{size:180px 120px}",
    );
    assert_eq!(document.page_count(), 2);
    assert_page_runs(&document, 1, &[("K", (0.0, 116.0))]);
}

#[test]
fn a_legend_is_a_block_that_leaves_the_columns_of_its_fieldset_in_place() {
    // A legend is a block, so a fieldset that holds one lays out its blocks and
    // its spanner as one with a block there does. As an inline box the legend
    // made the fieldset a paragraph of inline content, and the box positioned
    // inside the spanner took its static position from the end of the
    // fieldset, 120px down, instead of from the top of the spanner, 60px down.
    let kids = "<div>block1</div><div>block2</div><a></a><h3>spanner<a></a></h3><div>block3</div><div>block4</div><a></a>";
    let css = "fieldset{width:400px;overflow:hidden;position:absolute;margin:0;border:0;padding:0}\
               .t{column-count:3;column-rule:6px solid}\
               h3{column-span:all;margin:0;font-size:20px}\
               a{position:absolute;width:10px;height:10px;background:blue}div{margin:0}";
    let boxes = |first: &str| {
        let document = lay_out(&format!("<fieldset class=t>{first}{kids}</fieldset>"), css);
        document
            .page(0)
            .unwrap()
            .paint_order()
            .iter()
            .filter_map(|event| match event {
                PaintEvent::Box(fragment) if fragment.rect().width == 10.0 => {
                    Some((fragment.rect().x, fragment.rect().y))
                }
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    let expected = vec![(140.0, 60.0), (0.0, 60.0), (0.0, 120.0)];
    assert_eq!(boxes("<div>Legend</div>"), expected);
    assert_eq!(boxes("<legend>Legend</legend>"), expected);
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

#[test]
fn atomic_column_boxes_keep_their_group_before_a_fullwidth_spanner() {
    let document = lay_out(
        "<div class=mc><div class=box></div><div class=box></div><div class=span>A</div></div><p>B</p>",
        ".box{height:20px;background:blue}",
    );
    assert_eq!(document.page_count(), 1);
    assert_runs(&document, &[("A", (0.0, 36.0)), ("B", (0.0, 56.0))]);
    assert_eq!(rule_rects(&document), [(49.0, 0.0, 2.0, 20.0)]);
    let page = document.page(0).unwrap();
    let boxes: Vec<_> = page
        .fragments()
        .filter(|fragment| {
            page.dom().local_name(fragment.node()) == Some("div") && fragment.rect().width == 40.0
        })
        .map(|fragment| (fragment.rect().x, fragment.rect().y, fragment.rect().height))
        .collect();
    assert_eq!(boxes, [(0.0, 0.0, 20.0), (60.0, 0.0, 20.0)]);
}

#[test]
fn an_empty_column_group_does_not_add_height_or_rules_before_a_spanner() {
    let document = lay_out(
        "<div class=mc><p></p><p></p><div class=span>A</div></div><p>B</p>",
        "",
    );
    assert_eq!(document.page_count(), 1);
    assert_runs(&document, &[("A", (0.0, 16.0)), ("B", (0.0, 36.0))]);
    assert!(rule_rects(&document).is_empty());
}

#[test]
fn empty_paragraphs_do_not_consume_a_balanced_column_line() {
    let document = lay_out(
        "<div class=mc><p></p><p>A<br>B</p><div class=span>C</div></div><p>D</p>",
        "",
    );
    assert_eq!(document.page_count(), 1);
    assert_runs(
        &document,
        &[
            ("A", (0.0, 16.0)),
            ("B", (60.0, 16.0)),
            ("C", (0.0, 36.0)),
            ("D", (0.0, 56.0)),
        ],
    );
    assert_eq!(rule_rects(&document), [(49.0, 0.0, 2.0, 20.0)]);
}

#[test]
fn a_short_page_remainder_respects_paragraph_break_minima_before_the_spanner() {
    let document = lay_out(
        "<p>X</p><p>Y</p><div class=mc><p>A<br>B<br>C<br>D</p><div class=span>E</div></div><p>F</p>",
        "@page{size:180px 60px}.mc{orphans:2;widows:2}",
    );
    assert_eq!(document.page_count(), 3);
    assert_page_runs(&document, 0, &[("X", (0.0, 16.0)), ("Y", (0.0, 36.0))]);
    assert_page_runs(
        &document,
        1,
        &[
            ("A", (0.0, 16.0)),
            ("B", (0.0, 36.0)),
            ("C", (60.0, 16.0)),
            ("D", (60.0, 36.0)),
            ("E", (0.0, 56.0)),
        ],
    );
    assert_page_runs(&document, 2, &[("F", (0.0, 16.0))]);
}

#[test]
fn nested_columns_inside_a_spanner_retain_their_own_column_placements() {
    let document = lay_out(
        "<div class=mc><p>A<br>B</p><div class=span><div class=mc><p>C<br>D</p></div></div><p>E<br>F</p></div><p>G</p>",
        "",
    );
    assert_eq!(document.page_count(), 1);
    assert_runs(
        &document,
        &[
            ("A", (0.0, 16.0)),
            ("B", (60.0, 16.0)),
            ("C", (0.0, 36.0)),
            ("D", (60.0, 36.0)),
            ("E", (0.0, 56.0)),
            ("F", (60.0, 56.0)),
            ("G", (0.0, 76.0)),
        ],
    );
    let mut rules = rule_rects(&document);
    rules.sort_by(|a, b| a.1.total_cmp(&b.1));
    assert_eq!(
        rules,
        [
            (49.0, 0.0, 2.0, 20.0),
            (49.0, 20.0, 2.0, 20.0),
            (49.0, 40.0, 2.0, 20.0)
        ]
    );
}

#[test]
fn successive_spanning_owners_use_the_previous_owners_paginated_height() {
    let document = lay_out(
        "<div class=mc><p>A<br>B<br>C<br>D</p><div class=span>E</div><p>F<br>G<br>H<br>I</p></div><div class=mc><p>J<br>K<br>L<br>M</p><div class=span>N</div><p>O<br>P<br>Q<br>R</p></div><p>S</p>",
        "@page{size:180px 50px}",
    );
    assert_eq!(document.page_count(), 6);
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
        &[("E", (0.0, 16.0)), ("F", (0.0, 36.0)), ("G", (60.0, 36.0))],
    );
    assert_page_runs(
        &document,
        2,
        &[
            ("H", (0.0, 16.0)),
            ("I", (60.0, 16.0)),
            ("J", (0.0, 36.0)),
            ("K", (60.0, 36.0)),
        ],
    );
    assert_page_runs(
        &document,
        3,
        &[("L", (0.0, 16.0)), ("M", (60.0, 16.0)), ("N", (0.0, 36.0))],
    );
    assert_page_runs(
        &document,
        4,
        &[
            ("O", (0.0, 16.0)),
            ("P", (0.0, 36.0)),
            ("Q", (60.0, 16.0)),
            ("R", (60.0, 36.0)),
        ],
    );
    assert_page_runs(&document, 5, &[("S", (0.0, 16.0))]);
}

#[test]
fn nested_columns_keep_a_padded_wrapper_offset_inside_a_spanner() {
    let document = lay_out(
        "<div class=mc><p>A<br>B</p><div class=span><section><div class=inner><p>C<br>D</p></div></section></div><p>E<br>F</p></div><p>G</p>",
        "section{padding:10px 5px}.inner{column-count:2;column-gap:20px;column-rule:2px solid red}",
    );
    assert_eq!(document.page_count(), 1);
    assert_runs(
        &document,
        &[
            ("A", (0.0, 16.0)),
            ("B", (60.0, 16.0)),
            ("C", (5.0, 46.0)),
            ("D", (60.0, 46.0)),
            ("E", (0.0, 76.0)),
            ("F", (60.0, 76.0)),
            ("G", (0.0, 96.0)),
        ],
    );
}

#[test]
fn an_outer_column_fragment_keeps_its_following_child_after_span_pagination() {
    let document = lay_out(
        "<div class=outer><div class=mc><p>A<br>B<br>C<br>D</p><div class=span>E</div><p>F<br>G<br>H<br>I</p></div><p>J</p></div><p>K</p>",
        "@page{size:180px 50px}.outer{width:100px;column-count:1;column-gap:0}",
    );
    assert_eq!(document.page_count(), 4);
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
        &[("E", (0.0, 16.0)), ("F", (0.0, 36.0)), ("G", (60.0, 36.0))],
    );
    assert_page_runs(
        &document,
        2,
        &[("H", (0.0, 16.0)), ("I", (60.0, 16.0)), ("J", (0.0, 36.0))],
    );
    assert_page_runs(&document, 3, &[("K", (0.0, 16.0))]);
}

#[test]
fn a_line_taller_than_a_page_keeps_the_measured_group_without_a_page_loop() {
    let document = lay_out(
        "<div class=mc><p>A</p><p>B</p><div class=span>C</div></div><p>D</p>",
        "@page{size:180px 60px}.mc p{line-height:70px}",
    );
    assert_eq!(document.page_count(), 2);
    assert_page_runs(&document, 0, &[("A", (0.0, 41.0)), ("B", (60.0, 41.0))]);
    // The lines taller than the page reach onto the next page, which draws
    // them too, so its runs list them where they sit above it.
    assert_page_runs(
        &document,
        1,
        &[
            ("A", (0.0, -19.0)),
            ("B", (60.0, -19.0)),
            ("C", (0.0, 26.0)),
            ("D", (0.0, 46.0)),
        ],
    );
}

#[test]
fn a_fixed_height_spanning_owner_keeps_its_following_flow_at_the_authored_height() {
    let document = lay_out(
        "<div class=mc><div class=span>A</div><p>B</p></div><p>C</p>",
        ".mc{height:100px;column-fill:auto}",
    );
    assert_eq!(document.page_count(), 1);
    assert_runs(
        &document,
        &[("A", (0.0, 16.0)), ("B", (0.0, 36.0)), ("C", (0.0, 116.0))],
    );
    assert!(
        document
            .page(0)
            .unwrap()
            .fragments()
            .any(|fragment| fragment.rect().width == 100.0 && fragment.rect().height == 100.0)
    );
}

#[test]
fn a_max_height_spanning_owner_keeps_its_short_intrinsic_box() {
    let document = lay_out(
        "<div class=mc><div class=span>A</div><p>B</p></div><p>C</p>",
        ".mc{max-height:100px}",
    );
    assert_eq!(document.page_count(), 1);
    assert_runs(
        &document,
        &[("A", (0.0, 16.0)), ("B", (0.0, 36.0)), ("C", (0.0, 56.0))],
    );
    assert!(
        document
            .page(0)
            .unwrap()
            .fragments()
            .any(|fragment| fragment.rect().width == 100.0 && fragment.rect().height == 40.0)
    );
}

/// The rectangle of the first `div` of the page that lies `y` below its top.
fn div_rect_at(document: &DocumentLayout, y: f32) -> (f32, f32, f32, f32) {
    let page = document.page(0).unwrap();
    let rect = page
        .fragments()
        .find(|fragment| {
            page.dom().local_name(fragment.node()) == Some("div") && fragment.rect().y == y
        })
        .expect("div fragment")
        .rect();
    (rect.x, rect.y, rect.width, rect.height)
}

#[test]
fn a_spanner_may_be_any_block_level_box() {
    // `column-span: all` applies to every block-level box, not only to a plain
    // block (CSS Multi-column Layout 1, 6.1). Each display below spans both
    // columns and ends the rules between them, as a block does. A table is as
    // wide as its content, like any table.
    for (display, width) in [
        ("block", 100.0),
        ("flow-root", 100.0),
        ("flex", 100.0),
        ("grid", 100.0),
        ("table", 20.0),
    ] {
        let document = lay_out(
            &format!(
                "<div class=mc><p>A<br>B</p><div class=span style='display:{display}'>E</div><p>C<br>D</p></div>"
            ),
            "",
        );
        assert_eq!(document.page_count(), 1, "{display}");
        assert_runs(
            &document,
            &[
                ("A", (0.0, 16.0)),
                ("B", (60.0, 16.0)),
                ("E", (0.0, 36.0)),
                ("C", (0.0, 56.0)),
                ("D", (60.0, 56.0)),
            ],
        );
        assert_eq!(
            rule_rects(&document),
            [(49.0, 0.0, 2.0, 20.0), (49.0, 40.0, 2.0, 20.0)],
            "{display}"
        );
        assert_eq!(
            div_rect_at(&document, 20.0),
            (0.0, 20.0, width, 20.0),
            "{display}"
        );
    }
}

#[test]
fn the_auto_margins_of_a_spanner_centre_it_whatever_its_display() {
    // The spanner is 50px wide in the 100px of the container: 25px on each
    // side, with the lines around it and without them.
    for display in ["block", "flow-root", "flex", "grid"] {
        for lines in ["<p>A<br>B</p>{}<p>C<br>D</p>", "A<br>B<br>{}C<br>D"] {
            let spanner = format!(
                "<div class=span style='display:{display};width:50px;margin:0 auto'>E</div>"
            );
            let document = lay_out(
                &format!("<div class=mc>{}</div>", lines.replace("{}", &spanner)),
                "",
            );
            assert_eq!(
                div_rect_at(&document, 20.0),
                (25.0, 20.0, 50.0, 20.0),
                "{display} {lines}"
            );
        }
    }
}

#[test]
fn a_block_level_spanner_between_bare_lines_spans_the_columns() {
    // The lines are the content of the container itself, so the spanner is a
    // box of their paragraph.
    for display in ["flow-root", "flex", "grid", "table"] {
        let document = lay_out(
            &format!(
                "<div class=mc>A<br>B<br><div class=span style='display:{display}'>E</div>C<br>D</div>"
            ),
            "",
        );
        assert_eq!(document.page_count(), 1, "{display}");
        assert_runs(
            &document,
            &[
                ("A", (0.0, 16.0)),
                ("B", (60.0, 16.0)),
                ("E", (0.0, 36.0)),
                ("C", (0.0, 56.0)),
                ("D", (60.0, 56.0)),
            ],
        );
    }
}

#[test]
fn a_flow_root_spanner_contains_its_floats() {
    // The spanner is a formatting context of its own, so it is as tall as the
    // 40px float in it and the lines below it start under the float.
    let document = lay_out(
        "<div class=mc><p>A<br>B</p><div class=span style='display:flow-root'>\
         <div style='float:left;width:20px;height:40px'></div></div><p>C<br>D</p></div>",
        "",
    );
    assert_eq!(document.page_count(), 1);
    assert_runs(
        &document,
        &[
            ("A", (0.0, 16.0)),
            ("B", (60.0, 16.0)),
            ("C", (0.0, 76.0)),
            ("D", (60.0, 76.0)),
        ],
    );
}
