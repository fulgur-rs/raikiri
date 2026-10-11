//! Multicolumn paint projections preserve the producer's column placement.

use raikiri_html::{
    ClipKind, DocumentLayout, FontCollectionBuilder, LayoutOptions, LayoutStatus, PaintEvent,
    RenderResources, WarningKind, layout, parse_html_with_resources,
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

fn text_origins(document: &DocumentLayout) -> Vec<(String, (f32, f32))> {
    document
        .page(0)
        .unwrap()
        .text_runs()
        .iter()
        .map(|run| (run.text.to_owned(), run.origin))
        .collect()
}

#[test]
fn ordinary_paragraph_is_the_unfragmented_control() {
    let document = lay_out("<div>A<br>B<br>C<br>D</div>", "");
    assert_eq!(
        text_origins(&document),
        [
            ("A".into(), (0.0, 16.0)),
            ("B".into(), (0.0, 36.0)),
            ("C".into(), (0.0, 56.0)),
            ("D".into(), (0.0, 76.0)),
        ]
    );
}

#[test]
fn an_anonymous_table_cell_preserves_its_text_paint_order() {
    let document = lay_out("<div style='display:table'>A</div>", "");
    let page = document.page(0).unwrap();
    let runs = page.text_runs();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].text, "A");
    assert_eq!(
        page.paint_order_for_text_runs(&runs)
            .iter()
            .filter(|event| matches!(event, PaintEvent::TextLine(_)))
            .count(),
        1
    );
}

#[test]
fn a_replaced_image_uses_its_own_rounded_clip_after_its_box() {
    let document = lay_out(
        "<img src='data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVQIHWP4z8DwHwAFgAI/ScLbtAAAAABJRU5ErkJggg=='>",
        "img{display:block;width:80px;height:80px;border-radius:50%;overflow:hidden}",
    );
    let page = document.page(0).unwrap();
    let dom = page.dom();
    let runs = page.text_runs();
    let mut clips = Vec::new();
    let mut replaced_clip = None;
    for event in page.paint_order_for_text_runs(&runs) {
        match event {
            PaintEvent::PushClip(clip, _) => clips.push(clip),
            PaintEvent::PopClip => {
                clips.pop().unwrap();
            }
            PaintEvent::Box(fragment) if dom.local_name(fragment.node()) == Some("img") => {
                assert!(clips.is_empty())
            }
            PaintEvent::Replaced(_) => replaced_clip = clips.last().copied(),
            _ => {}
        }
    }
    let clip = replaced_clip.expect("the image's own clip");
    assert_eq!(
        clip.rect,
        raikiri_traits::PaintRect::new(0.0, 0.0, 80.0, 80.0)
    );
    assert_eq!(clip.corner_radii, Some([[40.0, 40.0]; 4]));
    assert!(clips.is_empty());
}

#[test]
fn one_paragraph_places_each_line_in_its_balanced_column() {
    let document = lay_out("<div class=mc>A<br>B<br>C<br>D</div>", "");
    assert_eq!(
        text_origins(&document),
        [
            ("A".into(), (0.0, 16.0)),
            ("B".into(), (0.0, 36.0)),
            ("C".into(), (60.0, 16.0)),
            ("D".into(), (60.0, 36.0)),
        ]
    );
    let page = document.page(0).unwrap();
    let runs = page.text_runs();
    let events = page.paint_order_for_text_runs(&runs);
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, PaintEvent::TextLine(_)))
            .count(),
        4
    );
    // These plain line ranges have no producer fragmentainer clip. Visible
    // inline overflow must not acquire a blanket column-width clip.
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, PaintEvent::PushClip(_, ClipKind::Fragmentainer)))
    );
}

#[test]
fn separate_paragraphs_follow_their_column_box_fragments() {
    let document = lay_out(
        "<div class=mc><p>A</p><p>B</p><p>C</p><p>D</p></div><p>E</p>",
        "",
    );
    assert_eq!(
        text_origins(&document),
        [
            ("A".into(), (0.0, 16.0)),
            ("B".into(), (0.0, 36.0)),
            ("C".into(), (60.0, 16.0)),
            ("D".into(), (60.0, 36.0)),
            ("E".into(), (0.0, 56.0)),
        ]
    );
}

#[test]
fn three_columns_keep_source_line_order_and_distinct_positions() {
    let document = lay_out(
        "<div class=mc>A<br>B<br>C<br>D<br>E<br>F</div>",
        ".mc{width:160px;column-count:3}",
    );
    assert_eq!(
        text_origins(&document),
        [
            ("A".into(), (0.0, 16.0)),
            ("B".into(), (0.0, 36.0)),
            ("C".into(), (60.0, 16.0)),
            ("D".into(), (60.0, 36.0)),
            ("E".into(), (120.0, 16.0)),
            ("F".into(), (120.0, 36.0)),
        ]
    );
}

#[test]
fn a_child_paragraph_split_across_columns_keeps_each_line_once() {
    let document = lay_out("<div class=mc><p>A<br>B<br>C<br>D</p></div><p>E</p>", "");
    assert_eq!(
        text_origins(&document),
        [
            ("A".into(), (0.0, 16.0)),
            ("B".into(), (0.0, 36.0)),
            ("C".into(), (60.0, 16.0)),
            ("D".into(), (60.0, 36.0)),
            ("E".into(), (0.0, 56.0)),
        ]
    );
}

#[test]
fn container_insets_preserve_public_columns_boxes_and_following_flow() {
    for (css, width, column_width, right, height) in [
        (
            ".mc{padding:4px 5px;border:1px solid black}",
            112.0,
            40.0,
            66.0,
            50.0,
        ),
        (
            ".mc{box-sizing:border-box;padding:4px 5px;border:1px solid black}",
            100.0,
            34.0,
            60.0,
            50.0,
        ),
        (
            ".mc{min-height:60px;padding:4px 5px;border:1px solid black}",
            112.0,
            40.0,
            66.0,
            70.0,
        ),
    ] {
        let document = lay_out("<div class=mc><p>A<br>B<br>C<br>D</p></div><p>E</p>", css);
        assert_eq!(
            text_origins(&document),
            [
                ("A".into(), (6.0, 21.0)),
                ("B".into(), (6.0, 41.0)),
                ("C".into(), (right, 21.0)),
                ("D".into(), (right, 41.0)),
                ("E".into(), (0.0, height + 16.0)),
            ]
        );
        let page = document.page(0).unwrap();
        let runs = page.text_runs();
        let root = runs[0].line.root;
        let columns: Vec<_> = page
            .fragments()
            .filter(|f| f.node() == root)
            .map(|f| (f.rect(), f.fragmentainer()))
            .collect();
        assert_eq!(
            columns,
            [
                (
                    raikiri_traits::PaintRect::new(6.0, 5.0, column_width, 40.0),
                    0
                ),
                (
                    raikiri_traits::PaintRect::new(right, 5.0, column_width, 40.0),
                    1
                ),
            ]
        );
        let container: Vec<_> = page
            .fragments()
            .filter(|f| page.dom().local_name(f.node()) == Some("div"))
            .map(|f| f.rect())
            .collect();
        assert_eq!(
            container,
            [raikiri_traits::PaintRect::new(0.0, 0.0, width, height)]
        );
        assert_eq!(
            page.paint_order_for_text_runs(&runs)
                .iter()
                .filter(|e| matches!(e, PaintEvent::TextLine(_)))
                .count(),
            5
        );
    }
}

#[test]
fn a_split_child_keeps_both_boxes_and_source_fragment_ordinals() {
    let document = lay_out(
        "<div class=mc><p>A<br>B<br>C<br>D</p></div><p>E</p>",
        "p{background:lime}",
    );
    let page = document.page(0).unwrap();
    let runs = page.text_runs();
    let root = runs.iter().find(|run| run.text == "A").unwrap().line.root;
    let boxes: Vec<_> = page
        .fragments()
        .filter(|fragment| fragment.node() == root)
        .map(|fragment| {
            (
                fragment.rect(),
                fragment.fragment_index(),
                fragment.is_last_fragment(),
            )
        })
        .collect();
    assert_eq!(
        boxes,
        [
            (
                raikiri_traits::PaintRect::new(0.0, 0.0, 40.0, 40.0),
                0,
                Some(false)
            ),
            (
                raikiri_traits::PaintRect::new(60.0, 0.0, 40.0, 40.0),
                1,
                Some(true)
            ),
        ]
    );
    let columns: Vec<_> = page
        .fragments()
        .filter(|fragment| fragment.node() == root)
        .map(|fragment| fragment.fragmentainer())
        .collect();
    assert_eq!(columns, [0, 1]);
}

#[test]
fn generated_inline_boxes_and_text_follow_the_split_paragraph() {
    let document = lay_out(
        "<div class=mc><p>A<br>B<br>C<br>D</p></div>",
        ".mc p::before{content:'X';background:red}.mc p::after{content:'Y';background:blue}",
    );
    assert_eq!(
        text_origins(&document),
        [
            ("X".into(), (0.0, 16.0)),
            ("A".into(), (20.0, 16.0)),
            ("B".into(), (0.0, 36.0)),
            ("C".into(), (60.0, 16.0)),
            ("D".into(), (60.0, 36.0)),
            ("Y".into(), (80.0, 36.0)),
        ]
    );
    let page = document.page(0).unwrap();
    let runs = page.text_runs();
    let events = page.paint_order_for_text_runs(&runs);
    let rects: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            PaintEvent::GeneratedBox(fragment) => Some(fragment.rect),
            _ => None,
        })
        .collect();
    assert_eq!(
        rects,
        [
            raikiri_traits::PaintRect::new(0.0, 0.0, 20.0, 20.0),
            raikiri_traits::PaintRect::new(80.0, 20.0, 20.0, 20.0)
        ]
    );
}

#[test]
fn a_wide_image_preserves_visible_inline_overflow() {
    let document = lay_out(
        "<div class=mc><p><img src='data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aWQAAAABJRU5ErkJggg==' width=80 height=20></p><p>B</p></div>",
        "img{display:block}",
    );
    let page = document.page(0).unwrap();
    let runs = page.text_runs();
    let events = page.paint_order_for_text_runs(&runs);
    let images: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            PaintEvent::Replaced(fragment) => Some(fragment.rect()),
            _ => None,
        })
        .collect();
    assert_eq!(
        images,
        [raikiri_traits::PaintRect::new(0.0, 0.0, 80.0, 20.0)]
    );
    assert_eq!(text_origins(&document), [("B".into(), (60.0, 16.0))]);
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, PaintEvent::PushClip(_, ClipKind::Fragmentainer)))
    );
}

#[test]
fn ordinary_inline_decorations_have_one_piece_per_column_line() {
    let document = lay_out(
        "<div class=mc><p><span>A<br>B<br>C<br>D</span></p></div>",
        "span{background:lime}",
    );
    let page = document.page(0).unwrap();
    let dom = page.dom();
    let rects: Vec<_> = page
        .fragments()
        .filter(|fragment| dom.local_name(fragment.node()) == Some("span"))
        .map(|fragment| fragment.rect())
        .collect();
    assert_eq!(
        rects,
        [
            raikiri_traits::PaintRect::new(0.0, 0.0, 20.0, 20.0),
            raikiri_traits::PaintRect::new(0.0, 20.0, 20.0, 20.0),
            raikiri_traits::PaintRect::new(60.0, 0.0, 20.0, 20.0),
            raikiri_traits::PaintRect::new(60.0, 20.0, 20.0, 20.0),
        ]
    );
}

#[test]
fn a_split_paragraph_overflow_clip_belongs_to_each_column_piece() {
    let document = lay_out(
        "<div class=mc><p>A<br>B<br>C<br>D</p></div>",
        "p{overflow:hidden}",
    );
    let page = document.page(0).unwrap();
    let runs = page.text_runs();
    let events = page.paint_order_for_text_runs(&runs);
    let mut clips = Vec::new();
    let mut line_clips = Vec::new();
    for event in events {
        match event {
            PaintEvent::PushClip(clip, _) => clips.push(clip),
            PaintEvent::PopClip => {
                clips.pop().unwrap();
            }
            PaintEvent::TextLine(line) => {
                line_clips.push((line.index, clips.last().map(|clip| clip.rect)))
            }
            _ => {}
        }
    }
    let left = Some(raikiri_traits::PaintRect::new(0.0, 0.0, 40.0, 40.0));
    let right = Some(raikiri_traits::PaintRect::new(60.0, 0.0, 40.0, 40.0));
    assert_eq!(line_clips, [(0, left), (1, left), (2, right), (3, right)]);
}

#[test]
fn a_span_present_in_only_one_column_has_no_box_in_the_other() {
    let document = lay_out(
        "<div class=mc><p><span>A<br>B</span><br>C<br>D</p></div>",
        "span{background:lime}",
    );
    let page = document.page(0).unwrap();
    let dom = page.dom();
    let rects: Vec<_> = page
        .fragments()
        .filter(|fragment| dom.local_name(fragment.node()) == Some("span"))
        .map(|fragment| fragment.rect())
        .collect();
    assert_eq!(
        rects,
        [
            raikiri_traits::PaintRect::new(0.0, 0.0, 20.0, 20.0),
            raikiri_traits::PaintRect::new(0.0, 20.0, 20.0, 20.0),
        ]
    );
}

#[test]
fn direct_multicol_inline_pieces_preserve_their_column_numbers() {
    let document = lay_out(
        "<div class=mc><span>A<br>B<br>C<br>D</span></div>",
        "span{background:lime}",
    );
    let page = document.page(0).unwrap();
    let dom = page.dom();
    let columns: Vec<_> = page
        .fragments()
        .filter(|fragment| dom.local_name(fragment.node()) == Some("span"))
        .map(|fragment| fragment.fragmentainer())
        .collect();
    assert_eq!(columns, [0, 0, 1, 1]);
}

#[test]
fn one_text_source_has_distinct_column_rectangles_and_line_ranges() {
    let document = lay_out("<div class=mc>AA BB CC DD</div>", "");
    let page = document.page(0).unwrap();
    let runs = page.text_runs();
    let raikiri_html::RunSource::Text(owner) = runs[0].source else {
        panic!("text source")
    };
    let fragments: Vec<_> = page
        .fragments()
        .filter(|fragment| fragment.node() == owner)
        .map(|fragment| (fragment.rect(), fragment.line_range()))
        .collect();
    assert_eq!(
        fragments,
        [
            (
                raikiri_traits::PaintRect::new(0.0, 0.0, 40.0, 40.0),
                Some(0..2)
            ),
            (
                raikiri_traits::PaintRect::new(60.0, 0.0, 40.0, 40.0),
                Some(2..4)
            ),
        ]
    );
}

#[test]
fn fixed_height_plain_wrapper_keeps_public_column_lines_once() {
    let document = lay_out(
        "<div class=mc><div><p>A<br>B<br>C<br>D</p></div></div><p>E</p>",
        ".mc{height:40px}",
    );
    assert_eq!(
        text_origins(&document),
        [
            ("A".into(), (0.0, 16.0)),
            ("B".into(), (0.0, 36.0)),
            ("C".into(), (60.0, 16.0)),
            ("D".into(), (60.0, 36.0)),
            ("E".into(), (0.0, 56.0)),
        ]
    );
}

#[test]
fn a_constrained_wrapper_width_keeps_its_public_continuation() {
    for sizing in [
        "width:80px",
        "min-width:80px",
        "width:200%",
        "width:calc(200%)",
    ] {
        let document = lay_out(
            &format!(
                "<div class=mc><div style='{sizing}'><p>A<br>B<br>C<br>D</p></div></div><p>E</p>"
            ),
            ".mc{height:40px}",
        );
        assert_eq!(
            text_origins(&document),
            [
                ("A".into(), (0.0, 16.0)),
                ("B".into(), (0.0, 36.0)),
                ("C".into(), (60.0, 16.0)),
                ("D".into(), (60.0, 36.0)),
                ("E".into(), (0.0, 56.0)),
            ]
        );
        let page = document.page(0).unwrap();
        let dom = page.dom();
        let rects: Vec<_> = page
            .fragments()
            .filter(|fragment| dom.local_name(fragment.node()) == Some("p"))
            .map(|fragment| fragment.rect())
            .collect();
        assert_eq!(
            rects,
            [
                raikiri_traits::PaintRect::new(0.0, 0.0, 80.0, 40.0),
                raikiri_traits::PaintRect::new(60.0, 0.0, 80.0, 40.0),
                raikiri_traits::PaintRect::new(0.0, 40.0, 180.0, 20.0),
            ]
        );
    }
}

#[test]
fn a_constrained_wrapper_margin_uses_only_the_first_column_budget() {
    let document = lay_out(
        "<div class=mc><div style='overflow:hidden'><p style='margin-top:20px'>A<br>B<br>C<br>D</p></div></div><p>E</p>",
        ".mc{height:60px}",
    );
    assert_eq!(
        text_origins(&document),
        [
            ("A".into(), (0.0, 36.0)),
            ("B".into(), (0.0, 56.0)),
            ("C".into(), (60.0, 16.0)),
            ("D".into(), (60.0, 36.0)),
            ("E".into(), (0.0, 76.0)),
        ]
    );
}

#[test]
fn a_constrained_wrapper_resolves_auto_margins_at_the_column_width() {
    let document = lay_out(
        "<div class=mc><div style='width:20px;margin-left:auto;margin-right:auto'><p>A<br>B<br>C<br>D</p></div></div><p>E</p>",
        ".mc{height:40px}",
    );
    assert_eq!(
        text_origins(&document),
        [
            ("A".into(), (10.0, 16.0)),
            ("B".into(), (10.0, 36.0)),
            ("C".into(), (70.0, 16.0)),
            ("D".into(), (70.0, 36.0)),
            ("E".into(), (0.0, 56.0)),
        ]
    );
}

#[test]
fn variable_height_wrapper_lines_keep_public_origins_and_feasible_break_minima() {
    let document = lay_out(
        "<div class=mc><div style='width:80px'><p style='orphans:1;widows:3'><span style='line-height:20px'>A</span><br><span style='line-height:5px'>B</span><br><span style='line-height:10px'>C</span><br><span style='line-height:15px'>D</span><br><span style='line-height:20px'>E</span><br><span style='line-height:10px'>F</span><br><span style='line-height:5px'>G</span><br><span style='line-height:5px'>H</span></p></div></div><p>I</p>",
        "body{font:5px/5px Ahem}.mc{height:40px;width:160px;column-count:3}",
    );
    assert_eq!(
        text_origins(&document),
        [
            ("A".into(), (0.0, 11.5)),
            ("B".into(), (60.0, 4.0)),
            ("C".into(), (60.0, 11.5)),
            ("D".into(), (60.0, 24.0)),
            ("E".into(), (120.0, 11.5)),
            ("F".into(), (120.0, 26.5)),
            ("G".into(), (120.0, 34.0)),
            ("H".into(), (120.0, 39.0)),
            ("I".into(), (0.0, 44.0)),
        ]
    );
    let page = document.page(0).unwrap();
    let dom = page.dom();
    let paragraphs: Vec<_> = page
        .fragments()
        .filter(|fragment| dom.local_name(fragment.node()) == Some("p"))
        .map(|fragment| (fragment.fragmentainer(), fragment.rect()))
        .collect();
    assert_eq!(
        paragraphs,
        [
            (0, raikiri_traits::PaintRect::new(0.0, 0.0, 80.0, 40.0)),
            (1, raikiri_traits::PaintRect::new(60.0, 0.0, 80.0, 40.0)),
            (2, raikiri_traits::PaintRect::new(120.0, 0.0, 80.0, 40.0)),
            (0, raikiri_traits::PaintRect::new(0.0, 40.0, 180.0, 5.0)),
        ]
    );
}

#[test]
fn fixed_height_deep_plain_wrapper_preserves_three_columns_and_origin() {
    let document = lay_out(
        "<p>X</p><div class=mc><div><div><p>A<br>B<br>C<br>D<br>E<br>F</p></div></div></div><p>G</p>",
        ".mc{width:160px;height:40px;column-count:3}",
    );
    assert_eq!(
        text_origins(&document),
        [
            ("X".into(), (0.0, 16.0)),
            ("A".into(), (0.0, 36.0)),
            ("B".into(), (0.0, 56.0)),
            ("C".into(), (60.0, 36.0)),
            ("D".into(), (60.0, 56.0)),
            ("E".into(), (120.0, 36.0)),
            ("F".into(), (120.0, 56.0)),
            ("G".into(), (0.0, 76.0)),
        ]
    );
}

#[test]
fn deep_plain_wrapper_clips_use_the_padded_parent_and_column_origins() {
    let document = lay_out(
        "<p>X</p><div class=mc><div><div><p>A<br>B<br>C<br>D<br>E<br>F</p></div></div></div><p>G</p>",
        ".mc{width:160px;height:40px;column-count:3;padding:4px 5px;border:1px solid black}.mc p{overflow:hidden}",
    );
    assert_eq!(
        text_origins(&document),
        [
            ("X".into(), (0.0, 16.0)),
            ("A".into(), (6.0, 41.0)),
            ("B".into(), (6.0, 61.0)),
            ("C".into(), (66.0, 41.0)),
            ("D".into(), (66.0, 61.0)),
            ("E".into(), (126.0, 41.0)),
            ("F".into(), (126.0, 61.0)),
            ("G".into(), (0.0, 86.0)),
        ]
    );
    let page = document.page(0).unwrap();
    let runs = page.text_runs();
    let mut clips = Vec::new();
    let mut line_clips = Vec::new();
    for event in page.paint_order_for_text_runs(&runs) {
        match event {
            PaintEvent::PushClip(clip, _) => clips.push(clip),
            PaintEvent::PopClip => {
                clips.pop().unwrap();
            }
            PaintEvent::TextLine(line) => {
                line_clips.push((line.index, clips.last().map(|clip| clip.rect)));
            }
            _ => {}
        }
    }
    let left = Some(raikiri_traits::PaintRect::new(6.0, 25.0, 40.0, 40.0));
    let middle = Some(raikiri_traits::PaintRect::new(66.0, 25.0, 40.0, 40.0));
    let right = Some(raikiri_traits::PaintRect::new(126.0, 25.0, 40.0, 40.0));
    assert_eq!(
        line_clips,
        [
            (0, None),
            (0, left),
            (1, left),
            (2, middle),
            (3, middle),
            (4, right),
            (5, right),
            (0, None),
        ]
    );
    assert!(clips.is_empty());
}

#[test]
fn fixed_height_border_box_wrapper_preserves_following_flow() {
    let document = lay_out(
        "<div class=mc><div><p>A<br>B<br>C<br>D</p></div></div><p>E</p>",
        ".mc{box-sizing:border-box;height:50px;padding:4px 5px;border:1px solid black}",
    );
    assert_eq!(
        text_origins(&document),
        [
            ("A".into(), (6.0, 21.0)),
            ("B".into(), (6.0, 41.0)),
            ("C".into(), (60.0, 21.0)),
            ("D".into(), (60.0, 41.0)),
            ("E".into(), (0.0, 66.0)),
        ]
    );
}

#[test]
fn a_short_plain_wrapper_with_a_huge_column_count_projects_once() {
    let document = lay_out(
        "<div class=mc><div><p>A</p></div></div><p>E</p>",
        ".mc{height:40px;column-count:1000000000;column-gap:0}",
    );
    assert_eq!(
        text_origins(&document),
        [("A".into(), (0.0, 16.0)), ("E".into(), (0.0, 56.0))]
    );
    assert!(document.page(0).unwrap().fragments().count() < 16);
}

#[test]
fn single_column_rtl_wrapper_preserves_ordinary_block_flow() {
    let document = lay_out(
        "<div class=mc><div><p>A<br>B<br>C<br>D</p></div></div><p>E</p>",
        ".mc{height:40px;column-count:1;direction:rtl}.mc p{direction:ltr}",
    );
    assert_eq!(
        text_origins(&document),
        [
            ("A".into(), (0.0, 16.0)),
            ("B".into(), (0.0, 36.0)),
            ("C".into(), (0.0, 56.0)),
            ("D".into(), (0.0, 76.0)),
            ("E".into(), (0.0, 56.0)),
        ]
    );
}

#[test]
fn multiple_paragraphs_continue_in_the_last_used_column() {
    let document = lay_out(
        "<div class=mc><p>A<br>B<br>C<br>D</p><p>E<br>F</p></div><p>G</p>",
        "",
    );
    assert_eq!(
        text_origins(&document),
        [
            ("A".into(), (0.0, 16.0)),
            ("B".into(), (0.0, 36.0)),
            ("C".into(), (0.0, 56.0)),
            ("D".into(), (60.0, 16.0)),
            ("E".into(), (60.0, 36.0)),
            ("F".into(), (60.0, 56.0)),
            ("G".into(), (0.0, 76.0)),
        ]
    );
}

#[test]
fn multiple_paragraphs_preserve_two_line_break_minima() {
    let document = lay_out(
        "<div class=mc><p>A<br>B<br>C<br>D<br>E<br>F</p><p>G<br>H</p></div><p>I</p>",
        ".mc{orphans:2;widows:2}",
    );
    assert_eq!(
        text_origins(&document),
        [
            ("A".into(), (0.0, 16.0)),
            ("B".into(), (0.0, 36.0)),
            ("C".into(), (0.0, 56.0)),
            ("D".into(), (0.0, 76.0)),
            ("E".into(), (60.0, 16.0)),
            ("F".into(), (60.0, 36.0)),
            ("G".into(), (60.0, 56.0)),
            ("H".into(), (60.0, 76.0)),
            ("I".into(), (0.0, 96.0)),
        ]
    );
}

#[test]
fn a_paragraph_split_below_earlier_content_has_column_local_origins() {
    let document = lay_out(
        "<div class=mc><p>A<br>B</p><p>C<br>D</p><p>E<br>F</p></div><p>G</p>",
        ".mc p{margin-bottom:8px}.mc p+p{margin-top:12px}",
    );
    assert_eq!(
        text_origins(&document),
        [
            ("A".into(), (0.0, 16.0)),
            ("B".into(), (0.0, 36.0)),
            ("C".into(), (0.0, 68.0)),
            ("D".into(), (60.0, 16.0)),
            ("E".into(), (60.0, 48.0)),
            ("F".into(), (60.0, 68.0)),
            ("G".into(), (0.0, 96.0)),
        ]
    );
    let page = document.page(0).unwrap();
    let runs = page.text_runs();
    assert!(
        !page
            .paint_order_for_text_runs(&runs)
            .iter()
            .any(|event| { matches!(event, PaintEvent::PushClip(_, ClipKind::Fragmentainer)) })
    );
}

#[test]
fn paragraph_specific_minima_preserve_all_lines_and_following_flow() {
    let document = lay_out(
        "<div class=mc><p style='orphans:3;widows:3'>A<br>B<br>C<br>D<br>E<br>F</p><p>G<br>H</p></div><p>I</p>",
        "",
    );
    assert_eq!(
        text_origins(&document),
        [
            ("A".into(), (0.0, 16.0)),
            ("B".into(), (0.0, 36.0)),
            ("C".into(), (0.0, 56.0)),
            ("D".into(), (60.0, 16.0)),
            ("E".into(), (60.0, 36.0)),
            ("F".into(), (60.0, 56.0)),
            ("G".into(), (60.0, 76.0)),
            ("H".into(), (60.0, 96.0)),
            ("I".into(), (0.0, 116.0)),
        ]
    );
}

#[test]
fn an_overflow_clip_follows_each_group_paragraph_piece() {
    let document = lay_out(
        "<div class=mc><p>A<br>B</p><p>C<br>D</p><p>E<br>F</p></div><p>G</p>",
        ".mc p{margin-bottom:8px;overflow:hidden}.mc p+p{margin-top:12px}",
    );
    let page = document.page(0).unwrap();
    let mut clips = Vec::new();
    let mut line_clips = Vec::new();
    let runs = page.text_runs();
    for event in page.paint_order_for_text_runs(&runs) {
        match event {
            PaintEvent::PushClip(clip, _) => clips.push(clip),
            PaintEvent::PopClip => {
                clips.pop().unwrap();
            }
            PaintEvent::TextLine(_) => line_clips.push(clips.last().map(|clip| clip.rect)),
            _ => {}
        }
    }
    use raikiri_traits::PaintRect;
    assert_eq!(
        line_clips,
        [
            Some(PaintRect::new(0.0, 0.0, 40.0, 40.0)),
            Some(PaintRect::new(0.0, 0.0, 40.0, 40.0)),
            Some(PaintRect::new(0.0, 52.0, 40.0, 28.0)),
            Some(PaintRect::new(60.0, 0.0, 40.0, 20.0)),
            Some(PaintRect::new(60.0, 32.0, 40.0, 40.0)),
            Some(PaintRect::new(60.0, 32.0, 40.0, 40.0)),
            None,
        ]
    );
    assert!(clips.is_empty());
}

#[test]
fn an_empty_paragraph_keeps_group_continuations() {
    let document = lay_out(
        "<div class=mc><p>A<br>B<br>C<br>D</p><p></p><p>E<br>F</p></div><p>G</p>",
        "",
    );
    assert_eq!(
        text_origins(&document),
        [
            ("A".into(), (0.0, 16.0)),
            ("B".into(), (0.0, 36.0)),
            ("C".into(), (0.0, 56.0)),
            ("D".into(), (60.0, 16.0)),
            ("E".into(), (60.0, 36.0)),
            ("F".into(), (60.0, 56.0)),
            ("G".into(), (0.0, 76.0)),
        ]
    );
}

#[test]
fn one_nonempty_paragraph_balances_beside_empty_blocks() {
    for body in [
        "<div class=mc><p>A<br>B<br>C<br>D</p><p></p></div><p>E</p>",
        "<div class=mc><p></p><p>A<br>B<br>C<br>D</p></div><p>E</p>",
        "<div class=mc><p>A<br>B<br>C<br>D</p><p> </p></div><p>E</p>",
        "<div class=mc><p> </p><p>A<br>B<br>C<br>D</p></div><p>E</p>",
    ] {
        let document = lay_out(body, "");
        assert_eq!(
            text_origins(&document),
            [
                ("A".into(), (0.0, 16.0)),
                ("B".into(), (0.0, 36.0)),
                ("C".into(), (60.0, 16.0)),
                ("D".into(), (60.0, 36.0)),
                ("E".into(), (0.0, 56.0)),
            ]
        );
    }
}

#[test]
fn a_leading_margin_keeps_content_in_the_first_column() {
    let document = lay_out(
        "<div class=mc><p style='margin-top:100px'>A</p><p>B</p></div><p>C</p>",
        "",
    );
    assert_eq!(
        text_origins(&document),
        [
            ("A".into(), (0.0, 116.0)),
            ("B".into(), (60.0, 16.0)),
            ("C".into(), (0.0, 136.0)),
        ]
    );
}

#[test]
fn zero_line_height_keeps_each_source_and_following_flow() {
    let document = lay_out(
        "<div class=mc><p class=zero>A</p><p>B</p></div><p>C</p>",
        ".mc .zero{line-height:0}",
    );
    assert_eq!(
        text_origins(&document),
        [
            ("A".into(), (0.0, 6.0)),
            ("B".into(), (0.0, 16.0)),
            ("C".into(), (0.0, 36.0)),
        ]
    );
}

#[test]
fn empty_paragraph_margin_chains_keep_both_signed_extrema() {
    let document = lay_out(
        "<div class=mc><p>A<br>B</p><p class=empty></p><p class=later>C<br>D</p><p class=last>E<br>F</p></div><p>G</p>",
        ".mc p{margin-bottom:8px}.mc .empty{margin-top:12px;margin-bottom:-4px}.mc .later{margin-top:20px}.mc .last{margin-top:12px}",
    );
    assert_eq!(
        text_origins(&document),
        [
            ("A".into(), (0.0, 16.0)),
            ("B".into(), (0.0, 36.0)),
            ("C".into(), (0.0, 72.0)),
            ("D".into(), (60.0, 16.0)),
            ("E".into(), (60.0, 48.0)),
            ("F".into(), (60.0, 68.0)),
            ("G".into(), (0.0, 96.0)),
        ]
    );
}

#[test]
fn inline_boxes_and_text_bounds_follow_mid_column_continuations() {
    let document = lay_out(
        "<div class=mc><p>A<br>B</p><p><span style='background:lime'>C<br>D</span></p><p>E<br>F</p></div><p>G</p>",
        ".mc p{margin-bottom:8px}.mc p+p{margin-top:12px}",
    );
    let page = document.page(0).unwrap();
    let dom = page.dom();
    let rects: Vec<_> = page
        .fragments()
        .filter(|fragment| dom.local_name(fragment.node()) == Some("span"))
        .map(|fragment| (fragment.rect(), fragment.fragmentainer()))
        .collect();
    assert_eq!(
        rects,
        [
            (raikiri_traits::PaintRect::new(0.0, 52.0, 20.0, 20.0), 0),
            (raikiri_traits::PaintRect::new(60.0, 0.0, 20.0, 20.0), 1),
        ]
    );
    let runs = page.text_runs();
    for (letter, x, y) in [("C", 0.0, 52.0), ("D", 60.0, 0.0)] {
        let run = runs.iter().find(|run| run.text == letter).unwrap();
        let raikiri_html::RunSource::Text(owner) = run.source else {
            panic!("text source")
        };
        assert!(page.fragments().any(|fragment| fragment.node() == owner
            && fragment.rect() == raikiri_traits::PaintRect::new(x, y, 40.0, 20.0)));
    }
}

#[test]
fn nonfinal_paragraph_boxes_fill_the_remaining_column_extent() {
    let document = lay_out(
        "<div class=mc><p style='background:lime;orphans:3;widows:3'>A<br>B<br>C<br>D<br>E<br>F</p><p>G<br>H</p></div><p>I</p>",
        "",
    );
    let page = document.page(0).unwrap();
    let runs = page.text_runs();
    let root = runs[0].line.root;
    let rects: Vec<_> = page
        .fragments()
        .filter(|fragment| fragment.node() == root)
        .map(|fragment| fragment.rect())
        .collect();
    assert_eq!(
        rects,
        [
            raikiri_traits::PaintRect::new(0.0, 0.0, 40.0, 100.0),
            raikiri_traits::PaintRect::new(60.0, 0.0, 40.0, 60.0)
        ]
    );
}

fn omitted_text_runs(document: &DocumentLayout) -> usize {
    document
        .warnings()
        .iter()
        .filter(|warning| matches!(warning.kind, WarningKind::TextRunsOmitted))
        .count()
}

#[test]
fn a_nested_container_balances_its_paragraph_into_its_own_columns() {
    // The outer container measures the inner one at its own width before
    // placing it in a 70px column; only the final layout is reported.
    let document = lay_out(
        "<div class=mc><div class=inner><p>A<br>B<br>C<br>D</p></div></div><p>E</p>",
        ".mc{width:160px}.inner{columns:2;column-gap:10px}",
    );
    assert_eq!(omitted_text_runs(&document), 0);
    assert_eq!(
        text_origins(&document),
        [
            ("A".into(), (0.0, 16.0)),
            ("B".into(), (0.0, 36.0)),
            ("C".into(), (40.0, 16.0)),
            ("D".into(), (40.0, 36.0)),
            ("E".into(), (0.0, 56.0)),
        ]
    );
}

#[test]
fn nested_container_paragraphs_keep_their_own_column_placements() {
    let document = lay_out(
        "<div class=mc><p>W</p><div class=inner><p>I</p><p>N</p></div></div>",
        ".mc{width:160px;height:40px;column-fill:auto}.inner{columns:2;column-gap:10px}",
    );
    assert_eq!(omitted_text_runs(&document), 0);
    assert_eq!(
        text_origins(&document),
        [
            ("W".into(), (0.0, 16.0)),
            ("I".into(), (0.0, 36.0)),
            ("N".into(), (40.0, 36.0)),
        ]
    );
}

#[test]
fn a_nested_container_taller_than_its_outer_column_continues_in_the_next_one() {
    let document = lay_out(
        "<div class=mc><div class=inner><p>A<br>B<br>C<br>D<br>E<br>F<br>G<br>H</p></div></div>",
        ".mc{width:160px;height:40px;column-fill:auto}.inner{columns:2;column-gap:10px}",
    );
    assert_eq!(omitted_text_runs(&document), 0);
    assert_eq!(
        text_origins(&document),
        [
            ("A".into(), (0.0, 16.0)),
            ("B".into(), (0.0, 36.0)),
            ("C".into(), (40.0, 16.0)),
            ("D".into(), (40.0, 36.0)),
            ("E".into(), (90.0, 16.0)),
            ("F".into(), (90.0, 36.0)),
            ("G".into(), (130.0, 16.0)),
            ("H".into(), (130.0, 36.0)),
        ]
    );
}

#[test]
fn a_nested_container_starting_lower_continues_and_its_next_sibling_follows_it() {
    let document = lay_out(
        "<div class=mc><p>W</p><div class=inner><p>A<br>B<br>C<br>D</p></div><p>Z</p></div>",
        ".mc{width:160px;height:40px;column-fill:auto}.inner{columns:2;column-gap:10px}",
    );
    assert_eq!(omitted_text_runs(&document), 0);
    assert_eq!(
        text_origins(&document),
        [
            ("W".into(), (0.0, 16.0)),
            ("A".into(), (0.0, 36.0)),
            ("B".into(), (40.0, 36.0)),
            ("C".into(), (90.0, 16.0)),
            ("D".into(), (130.0, 16.0)),
            ("Z".into(), (90.0, 36.0)),
        ]
    );
}

#[test]
fn a_nested_container_with_no_room_left_starts_in_the_next_outer_column() {
    let document = lay_out(
        "<div class=mc><p style=height:30px>W</p><div class=inner><p>A<br>B<br>C<br>D<br>E<br>F</p></div></div>",
        ".mc{width:250px;height:40px;column-count:3;column-fill:auto}.inner{columns:2;column-gap:10px}",
    );
    assert_eq!(omitted_text_runs(&document), 0);
    assert_eq!(
        text_origins(&document),
        [
            ("W".into(), (0.0, 16.0)),
            ("A".into(), (90.0, 16.0)),
            ("B".into(), (90.0, 36.0)),
            ("C".into(), (130.0, 16.0)),
            ("D".into(), (130.0, 36.0)),
            ("E".into(), (180.0, 16.0)),
            ("F".into(), (220.0, 16.0)),
        ]
    );
}

#[test]
fn a_continued_nested_container_paints_its_box_in_each_row() {
    let document = lay_out(
        "<div class=mc><p>W</p><div class=inner id=i><p>A<br>B<br>C<br>D<br>E<br>F</p></div></div>",
        ".mc{width:160px;height:60px;column-fill:auto}.inner{columns:2;column-gap:10px;background:green;padding-bottom:5px}",
    );
    let page = document.page(0).unwrap();
    let dom = page.dom();
    let boxes: Vec<_> = page
        .paint_order()
        .iter()
        .filter_map(|event| match event {
            PaintEvent::Box(fragment) if dom.attr(fragment.node(), "id") == Some("i") => {
                let rect = fragment.rect();
                Some((rect.x, rect.y, rect.width, rect.height))
            }
            _ => None,
        })
        .collect();
    // The first box reaches the end of the first outer column; the second
    // starts at the top of the next one and ends with the bottom padding.
    assert_eq!(boxes, [(0.0, 20.0, 70.0, 40.0), (90.0, 0.0, 70.0, 25.0)]);
    assert_eq!(
        text_origins(&document)[1..],
        [
            ("A".into(), (0.0, 36.0)),
            ("B".into(), (0.0, 56.0)),
            ("C".into(), (40.0, 36.0)),
            ("D".into(), (40.0, 56.0)),
            ("E".into(), (90.0, 16.0)),
            ("F".into(), (130.0, 16.0)),
        ]
    );
}

#[test]
fn a_nested_container_that_avoids_breaks_inside_moves_to_the_next_outer_column() {
    let document = lay_out(
        "<div class=mc><p>W</p><div class=inner><p>A<br>B<br>C<br>D</p></div></div>",
        ".mc{width:160px;height:40px;column-fill:auto}.inner{columns:2;column-gap:10px;break-inside:avoid}",
    );
    assert_eq!(
        text_origins(&document),
        [
            ("W".into(), (0.0, 16.0)),
            ("A".into(), (90.0, 16.0)),
            ("B".into(), (90.0, 36.0)),
            ("C".into(), (130.0, 16.0)),
            ("D".into(), (130.0, 36.0)),
        ]
    );
}

#[test]
fn balance_all_balances_every_row_of_a_continued_nested_container() {
    let document = lay_out(
        "<div class=mc><div class=inner><p>A<br>B<br>C<br>D<br>E<br>F<br>G</p></div></div>",
        ".mc{width:160px;height:60px;column-fill:auto}.inner{columns:2;column-gap:10px;column-fill:balance-all}",
    );
    // The first row keeps the three lines per column the outer column holds;
    // the last row balances the one remaining line.
    assert_eq!(
        text_origins(&document),
        [
            ("A".into(), (0.0, 16.0)),
            ("B".into(), (0.0, 36.0)),
            ("C".into(), (0.0, 56.0)),
            ("D".into(), (40.0, 16.0)),
            ("E".into(), (40.0, 36.0)),
            ("F".into(), (40.0, 56.0)),
            ("G".into(), (90.0, 16.0)),
        ]
    );
}
