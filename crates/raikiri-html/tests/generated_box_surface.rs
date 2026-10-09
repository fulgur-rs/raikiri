//! Generated inline decoration placements retain source identities and paint order.

use raikiri_html::{
    FontCollectionBuilder, GeneratedKind, LayoutOptions, LayoutStatus, PaintEvent, PaintRect,
    RenderResources, layout, parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

fn lay_out(body: &str, css: &str) -> raikiri_html::DocumentLayout {
    let fonts = FontCollectionBuilder::new()
        .font_bytes(
            "Ahem",
            include_bytes!("../../raikiri-dom/tests/data/text-autospace/Ahem.ttf").to_vec(),
        )
        .build()
        .unwrap();
    let resources = RenderResources::new().fonts(fonts);
    let html = format!(
        "<!doctype html><style>@page{{size:100px 60px;margin:0}}body{{margin:0;font:20px/20px Ahem}}{css}</style>{body}"
    );
    let parsed = parse_html_with_resources(html.as_bytes(), &resources).unwrap();
    let LayoutStatus::Completed(result) = layout(
        &parsed,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new().resources(&resources),
    )
    .unwrap() else {
        panic!("completed layout")
    };
    result
}

#[test]
fn before_after_decorations_precede_text_and_keep_arena_owners() {
    let document = lay_out(
        "<div>A</div>",
        "div::before{content:'B';background:red}div::after{content:'C';background:blue}",
    );
    let page = document.page(0).unwrap();
    let runs = page.text_runs();
    let events = page.paint_order_for_text_runs(&runs);
    let boxes: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            PaintEvent::GeneratedBox(piece) => Some(piece),
            _ => None,
        })
        .collect();
    assert_eq!(boxes.len(), 2);
    assert_eq!(boxes[0].kind, GeneratedKind::Before);
    assert_eq!(boxes[1].kind, GeneratedKind::After);
    assert_eq!(boxes[0].rect, PaintRect::new(0.0, 0.0, 20.0, 20.0));
    assert_eq!(boxes[1].rect, PaintRect::new(40.0, 0.0, 20.0, 20.0));
    assert_eq!(boxes[0].style.background_color.r, 255);
    assert_eq!(boxes[1].style.background_color.b, 255);
    for piece in boxes {
        assert!(page.computed(piece.owner).is_some());
        assert_eq!(piece.clip_owner, piece.owner);
        assert!(piece.has_start_edge && piece.has_end_edge);
    }
    let last_box = events
        .iter()
        .rposition(|event| matches!(event, PaintEvent::GeneratedBox(_)))
        .unwrap();
    let first_text = events
        .iter()
        .position(|event| matches!(event, PaintEvent::TextLine(_)))
        .unwrap();
    assert!(last_box < first_text);
    assert_eq!(runs.iter().map(|run| run.text).collect::<String>(), "BAC");
}

#[test]
fn block_generated_content_breaks_lines_without_replacing_text() {
    let document = lay_out(
        "<div>A</div>",
        "div::before{display:block;content:'B';background:red}div::after{display:block;content:'C';background:blue}",
    );
    let page = document.page(0).unwrap();
    let events = page.paint_order_for_text_runs(&page.text_runs());
    let boxes: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            PaintEvent::GeneratedBox(piece) => Some(piece),
            _ => None,
        })
        .collect();
    assert_eq!(boxes.len(), 2);
    assert_eq!(boxes[0].rect, PaintRect::new(0.0, 0.0, 20.0, 20.0));
    assert_eq!(boxes[1].rect, PaintRect::new(0.0, 40.0, 20.0, 20.0));
}

#[test]
fn nested_pseudo_backgrounds_stay_between_parent_and_later_sibling_boxes() {
    let document = lay_out(
        "<div><span>A</span><b>B</b></div>",
        "span{background:blue}span::before{content:'X';background:red}span::after{content:'Y';background:red}",
    );
    let page = document.page(0).unwrap();
    let events = page.paint_order_for_text_runs(&page.text_runs());
    let generated: Vec<_> = events
        .iter()
        .enumerate()
        .filter_map(|(index, event)| match event {
            PaintEvent::GeneratedBox(piece) => Some((index, piece)),
            _ => None,
        })
        .collect();
    assert_eq!(generated.len(), 2);
    let owner = generated[0].1.owner;
    let parent_box = events
        .iter()
        .position(|event| matches!(event, PaintEvent::Box(fragment) if fragment.node() == owner))
        .unwrap();
    assert!(parent_box < generated[0].0 && generated[0].0 < generated[1].0);
    assert!(
        events[generated[1].0 + 1..]
            .iter()
            .any(|event| matches!(event, PaintEvent::Box(_)))
    );
    assert_eq!(generated[0].1.rect.x, 0.0);
    assert_eq!(generated[1].1.rect.x, 40.0);
}

#[test]
fn wrapped_generated_boxes_own_only_their_page_and_inline_edges() {
    let document = lay_out(
        "<div style='width:20px'></div>",
        "div::before{content:'A B C D E';background:red}",
    );
    assert!(document.page_count() > 1);
    let pieces: Vec<_> = document
        .pages()
        .flat_map(|page| page.paint_order_for_text_runs(&page.text_runs()))
        .filter_map(|event| match event {
            PaintEvent::GeneratedBox(piece) => Some(piece),
            _ => None,
        })
        .collect();
    assert_eq!(pieces.len(), 5);
    assert!(pieces[0].has_start_edge);
    assert!(!pieces[0].has_end_edge);
    assert!(pieces[4].has_end_edge);
    assert!(!pieces[4].has_start_edge);
    let counts: Vec<_> = document
        .pages()
        .map(|page| {
            page.paint_order_for_text_runs(&page.text_runs())
                .iter()
                .filter(|event| matches!(event, PaintEvent::GeneratedBox(_)))
                .count()
        })
        .collect();
    assert_eq!(counts, vec![3, 2]);
}

#[test]
fn content_none_hidden_and_empty_pseudos_have_no_decoration() {
    for css in [
        "div::before{content:none;background:red}",
        "div::before{content:'';background:red}",
        "div::before{content:'X';visibility:hidden;background:red}",
        "div::before{content:'X';display:none;background:red}",
    ] {
        let document = lay_out("<div>A</div>", css);
        let page = document.page(0).unwrap();
        assert!(
            page.paint_order_for_text_runs(&page.text_runs())
                .iter()
                .all(|event| !matches!(event, PaintEvent::GeneratedBox(_)))
        );
    }
}

#[test]
fn fixed_generated_backgrounds_repeat_with_owner_opacity_and_overflow() {
    let document = lay_out(
        "<div>A</div><p style='height:100px'></p>",
        "div{position:fixed;left:0;top:0;opacity:.5;overflow:hidden}div::before{content:'X';background:red}",
    );
    assert!(document.page_count() > 1);
    for page in document.pages() {
        let events = page.paint_order_for_text_runs(&page.text_runs());
        let boxes: Vec<_> = events
            .iter()
            .filter_map(|event| match event {
                PaintEvent::GeneratedBox(piece) => Some(piece),
                _ => None,
            })
            .collect();
        assert_eq!(boxes.len(), 1);
        assert_eq!(boxes[0].rect, PaintRect::new(0.0, 0.0, 20.0, 20.0));
        let box_index = events
            .iter()
            .position(|event| matches!(event, PaintEvent::GeneratedBox(_)))
            .unwrap();
        assert!(
            events[..box_index]
                .iter()
                .any(|event| matches!(event, PaintEvent::PushOpacity(alpha) if *alpha == 0.5))
        );
        assert!(
            events[..box_index]
                .iter()
                .any(|event| matches!(event, PaintEvent::PushClip(_, _)))
        );
    }
}

#[test]
fn header_pseudo_boxes_repeat_only_on_the_tables_pages() {
    let document = lay_out(
        "<table><thead><tr><th>H</th></tr></thead><tbody><tr><td>A</td></tr><tr><td>B</td></tr></tbody></table><div style='height:70px'></div>",
        "table{border-spacing:0}th,td{padding:0;font:10px/10px Ahem;text-align:left}td{height:50px}th::before{content:'X';background:red}",
    );
    assert!(document.page_count() > 2);
    for page in document.pages() {
        let events = page.paint_order_for_text_runs(&page.text_runs());
        let pieces: Vec<_> = events
            .iter()
            .filter_map(|event| match event {
                PaintEvent::GeneratedBox(piece) => Some(piece),
                _ => None,
            })
            .collect();
        assert_eq!(pieces.len(), usize::from(page.index() < 2));
        if let Some(piece) = pieces.first() {
            assert_eq!(piece.rect, PaintRect::new(0.0, 0.0, 10.0, 10.0));
        }
    }
}

#[test]
fn unmodeled_inline_offsets_follow_the_existing_text_omission_policy() {
    let document = lay_out(
        "<div><span>A</span></div>",
        "span{position:relative;left:5px}span::before{content:'X';background:red}",
    );
    let page = document.page(0).unwrap();
    assert!(page.text_runs().is_empty());
    assert!(
        page.paint_order_for_text_runs(&page.text_runs())
            .iter()
            .all(|event| !matches!(event, PaintEvent::GeneratedBox(_)))
    );
}
