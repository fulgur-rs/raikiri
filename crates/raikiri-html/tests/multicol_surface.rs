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
