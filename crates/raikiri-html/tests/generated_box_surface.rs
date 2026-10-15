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
fn transformed_inline_offsets_follow_the_text_omission_policy() {
    // Outside columns, the decoration keeps the line's coordinates inside
    // the transform, like its text.
    let document = lay_out(
        "<div><span>A</span></div>",
        "div{transform:rotate(5deg)}span::before{content:'X';background:red}",
    );
    let page = document.page(0).unwrap();
    let runs = page.text_runs();
    assert!(runs.iter().all(|run| run.transform.is_some()));
    let events = page.paint_order_for_text_runs(&runs);
    let open = events
        .iter()
        .position(|event| matches!(event, PaintEvent::PushTransform(_)))
        .expect("the turned box");
    let piece = events
        .iter()
        .position(|event| matches!(event, PaintEvent::GeneratedBox(_)))
        .expect("the decoration");
    assert!(open < piece);
    // Inside columns, a turned box has neither runs nor decorations.
    let document = lay_out(
        "<div style=\"columns:2;height:40px\"><div class=\"t\"><span>A</span></div></div>",
        ".t{transform:rotate(5deg)}span::before{content:'X';background:red}",
    );
    let page = document.page(0).unwrap();
    assert!(page.text_runs().is_empty());
    assert!(
        page.paint_order_for_text_runs(&page.text_runs())
            .iter()
            .all(|event| !matches!(event, PaintEvent::GeneratedBox(_)))
    );
}

#[test]
fn first_letter_decorations_are_generated_boxes_after_before() {
    let document = lay_out(
        "<div><p>AB</p></div>",
        "p{margin:0}p::before{content:'X';background:blue}\
         div::first-letter{background:red;box-shadow:2px 2px green}",
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
    let letter = boxes[1];
    assert_eq!(letter.kind, GeneratedKind::FirstLetter);
    // The first letter comes from the `::before` text, so it wraps `X`.
    assert_eq!(letter.rect, PaintRect::new(0.0, 0.0, 20.0, 20.0));
    assert_eq!(letter.style.background_color.r, 255);
    assert_eq!(letter.style.box_shadow.len(), 1);
    // The letter belongs to the `div`, an ancestor of the paragraph.
    assert_ne!(letter.owner, boxes[0].owner);
    assert!(page.computed(letter.owner).is_some());
    assert!(letter.has_start_edge && letter.has_end_edge);
}

#[test]
fn first_letter_without_decoration_style_keeps_its_box_identity() {
    let document = lay_out("<p>AB</p>", "p{margin:0}p::first-letter{color:red}");
    let page = document.page(0).unwrap();
    let runs = page.text_runs();
    let events = page.paint_order_for_text_runs(&runs);
    let letters: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            PaintEvent::GeneratedBox(piece) if piece.kind == GeneratedKind::FirstLetter => {
                Some(piece)
            }
            _ => None,
        })
        .collect();
    assert_eq!(letters.len(), 1);
    assert_eq!(letters[0].rect, PaintRect::new(0.0, 0.0, 20.0, 20.0));
    assert_eq!(letters[0].style.color.r, 255);
}

fn first_letter_boxes(events: &[PaintEvent<'_>]) -> Vec<(usize, PaintRect)> {
    events
        .iter()
        .enumerate()
        .filter_map(|(index, event)| match event {
            PaintEvent::GeneratedBox(piece) if piece.kind == GeneratedKind::FirstLetter => {
                Some((index, piece.rect))
            }
            _ => None,
        })
        .collect()
}

/// The first letter moves with the relatively positioned inline element its
/// letter comes from, as its glyphs do.
#[test]
fn a_first_letter_box_moves_with_its_relative_inline_source() {
    let document = lay_out(
        "<p><span>AB</span></p>",
        "p{margin:0}span{position:relative;left:10px;top:5px}p::first-letter{background:red}",
    );
    let page = document.page(0).unwrap();
    let events = page.paint_order_for_text_runs(&page.text_runs());
    let letters = first_letter_boxes(&events);
    assert_eq!(letters.len(), 1);
    assert_eq!(letters[0].1, PaintRect::new(10.0, 5.0, 20.0, 20.0));
}

/// A first letter taken from an inline opacity group is composited with the
/// group, as its glyphs are.
#[test]
fn a_first_letter_box_stays_inside_its_inline_opacity_group() {
    let document = lay_out(
        "<p><span>AB</span></p>",
        "p{margin:0}span{opacity:.5}p::first-letter{background:red}",
    );
    let page = document.page(0).unwrap();
    let events = page.paint_order_for_text_runs(&page.text_runs());
    let letters = first_letter_boxes(&events);
    assert_eq!(letters.len(), 1);
    let push = events
        .iter()
        .position(|event| matches!(event, PaintEvent::PushOpacity(alpha) if *alpha == 0.5))
        .unwrap();
    let pop = events
        .iter()
        .rposition(|event| matches!(event, PaintEvent::PopOpacity))
        .unwrap();
    assert!(push < letters[0].0 && letters[0].0 < pop);
}

/// A first letter taken from an inline element is painted above that
/// element's background, as its box nests inside the element's.
#[test]
fn a_first_letter_box_follows_the_box_of_its_inline_source() {
    let document = lay_out(
        "<p><span>AB</span></p>",
        "p{margin:0}span{background:blue}p::first-letter{background:red}",
    );
    let page = document.page(0).unwrap();
    let events = page.paint_order_for_text_runs(&page.text_runs());
    let letters = first_letter_boxes(&events);
    assert_eq!(letters.len(), 1);
    // The span's box is the last box listed.
    let span = events
        .iter()
        .rposition(|event| matches!(event, PaintEvent::Box(_)))
        .unwrap();
    assert!(span < letters[0].0);
}

/// A first letter taken from `::before` text is painted below that
/// pseudo-element's images, as a line's inline boxes are painted before its
/// replaced content.
#[test]
fn a_first_letter_box_precedes_the_images_of_its_before_source() {
    let document = lay_out(
        "<p>B</p>",
        "p{margin:0}p::before{content:'A' url(data:image/svg+xml,%3Csvg%20xmlns=%22http://www.w3.org/2000/svg%22%20width=%2210%22%20height=%2210%22/%3E)}\
         p::first-letter{background:red}",
    );
    let page = document.page(0).unwrap();
    let events = page.paint_order_for_text_runs(&page.text_runs());
    let letters = first_letter_boxes(&events);
    assert_eq!(letters.len(), 1);
    let image = events
        .iter()
        .position(|event| matches!(event, PaintEvent::GeneratedImage(_)))
        .expect("the image is listed");
    assert!(letters[0].0 < image);
}

/// A first letter that an ancestor's `::first-letter` takes from a
/// descendant block is clipped by that block, which holds its letters.
#[test]
fn a_first_letter_box_is_clipped_by_the_block_holding_its_letters() {
    let document = lay_out(
        "<div><p>AB</p></div>",
        "p{margin:0;overflow:hidden}div::first-letter{background:red}",
    );
    let page = document.page(0).unwrap();
    let events = page.paint_order_for_text_runs(&page.text_runs());
    let letters: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            PaintEvent::GeneratedBox(piece) if piece.kind == GeneratedKind::FirstLetter => {
                Some(piece)
            }
            _ => None,
        })
        .collect();
    assert_eq!(letters.len(), 1);
    let letter = letters[0];
    assert_ne!(letter.clip_owner, letter.owner);
    assert_eq!(
        page.computed(letter.clip_owner).unwrap().overflow.x,
        raikiri_html::computed::OverflowValue::Hidden
    );
}

/// A first letter taken from `::after` text is painted above that
/// pseudo-element's background, as its box nests inside it.
#[test]
fn a_first_letter_box_follows_the_after_box_it_comes_from() {
    let document = lay_out(
        "<p><span></span></p>",
        "p{margin:0}span::after{content:'A';background:blue}p::first-letter{background:red}",
    );
    let page = document.page(0).unwrap();
    let events = page.paint_order_for_text_runs(&page.text_runs());
    let letters = first_letter_boxes(&events);
    assert_eq!(letters.len(), 1);
    let after = events
        .iter()
        .position(|event| {
            matches!(event, PaintEvent::GeneratedBox(piece) if piece.kind == GeneratedKind::After)
        })
        .unwrap();
    assert!(after < letters[0].0);
}

/// A relatively positioned `::before` moves its own text and box, as a
/// relatively positioned inline element does (CSS 2.1 §9.4.3).
#[test]
fn relative_pseudo_elements_move_their_text_and_box() {
    let document = lay_out(
        "<div>A</div>",
        "div::before{content:'B';position:relative;left:20px;top:5px;background:red}",
    );
    let page = document.page(0).unwrap();
    let runs = page.text_runs();
    let generated = runs
        .iter()
        .find(|run| run.text == "B")
        .expect("generated run");
    let own = runs.iter().find(|run| run.text == "A").expect("text run");
    assert_eq!(generated.origin.0, 20.0);
    assert_eq!(generated.origin.1, own.origin.1 + 5.0);
    assert_eq!(own.origin.0, 20.0);
    let events = page.paint_order_for_text_runs(&runs);
    let piece = events
        .iter()
        .find_map(|event| match event {
            PaintEvent::GeneratedBox(piece) => Some(piece),
            _ => None,
        })
        .unwrap();
    assert_eq!(piece.rect, PaintRect::new(20.0, 5.0, 20.0, 20.0));
}

/// One paint event summarized by what a painter composites: the opacity
/// groups, the generated boxes and the text each line event draws.
fn composited(page: &raikiri_html::Page<'_>) -> Vec<String> {
    let runs = page.text_runs();
    let mut drawn = vec![0; runs.len()];
    let mut order = Vec::new();
    for event in page.paint_order_for_text_runs(&runs) {
        match event {
            PaintEvent::PushOpacity(alpha) => order.push(format!("push {alpha}")),
            PaintEvent::PopOpacity => order.push("pop".into()),
            PaintEvent::GeneratedBox(piece) => order.push(format!("box {:?}", piece.kind)),
            PaintEvent::GeneratedImage(_) => order.push("image".into()),
            PaintEvent::TextLine(line) | PaintEvent::GroupTextLine(line, _) => {
                let group = match event {
                    PaintEvent::GroupTextLine(_, group) => Some(group),
                    _ => None,
                };
                let mut text = String::new();
                for (index, run) in runs.iter().enumerate() {
                    if run.line == line && page.text_run_opacity_group(run) == group {
                        drawn[index] += 1;
                        text.push_str(run.text);
                    }
                }
                order.push(format!("text {text}"));
            }
            _ => {}
        }
    }
    assert!(drawn.iter().all(|&count| count == 1), "{drawn:?}");
    order
}

/// A `::before` with opacity below one composites its decoration and its
/// text as one group (CSS Color 4 §3.3), after the rest of its paragraph.
#[test]
fn a_translucent_before_is_an_opacity_group() {
    let document = lay_out(
        "<div>A</div>",
        "div::before{content:'B';background:red;opacity:.5}div::after{content:'C';background:blue}",
    );
    let page = document.page(0).unwrap();
    assert_eq!(
        composited(&page),
        [
            "box After",
            "text AC",
            "push 0.5",
            "box Before",
            "text B",
            "pop"
        ]
    );
    let runs = page.text_runs();
    let generated = runs.iter().find(|run| run.text == "B").unwrap();
    let group = page.text_run_opacity_group(generated).unwrap();
    assert!(page.dom().attr(group, "id").is_none());
}

/// The images of a translucent `::after` stay inside its group, above its
/// decoration.
#[test]
fn a_translucent_after_holds_its_images() {
    let document = lay_out(
        "<div>A</div>",
        "div::after{content:'C' url(data:image/svg+xml,%3Csvg%20xmlns=%22http://www.w3.org/2000/svg%22%20width=%2210%22%20height=%2210%22/%3E);\
         background:blue;opacity:.25}",
    );
    let page = document.page(0).unwrap();
    assert_eq!(
        composited(&page),
        ["text A", "push 0.25", "box After", "image", "text C", "pop"]
    );
}

/// A translucent `::after` groups only the page that holds it; the
/// paragraph's earlier page paints no group.
#[test]
fn a_translucent_after_groups_only_its_page() {
    let document = lay_out(
        "<p>AAAA BBBB CCCC DDDD</p>",
        // `widows: 1` keeps the natural break after the third line.
        "p{margin:0;widows:1}p::after{content:'E';opacity:.5}",
    );
    assert_eq!(
        composited(&document.page(0).unwrap()),
        ["text AAAA ", "text BBBB ", "text CCCC "]
    );
    assert_eq!(
        composited(&document.page(1).unwrap()),
        ["text DDDD", "push 0.5", "box After", "text E", "pop"]
    );
}

/// A translucent `::after` composites after a translucent element that
/// comes before it in tree order.
#[test]
fn a_translucent_after_follows_an_earlier_inline_group() {
    let document = lay_out(
        "<p>A<span>B</span>C</p>",
        "p{margin:0}span{opacity:.5}p::after{content:'D';opacity:.25}",
    );
    assert_eq!(
        composited(&document.page(0).unwrap()),
        [
            "text AC",
            "push 0.5",
            "text B",
            "pop",
            "push 0.25",
            "box After",
            "text D",
            "pop"
        ]
    );
}

/// A translucent `::first-letter` composites its background and its letter
/// as one group, as the built-in painter does.
#[test]
fn a_translucent_first_letter_is_an_opacity_group() {
    let document = lay_out(
        "<p>AB</p>",
        "p{margin:0}p::first-letter{background:red;opacity:.5}",
    );
    let page = document.page(0).unwrap();
    assert_eq!(
        composited(&page),
        ["text B", "push 0.5", "box FirstLetter", "text A", "pop"]
    );
}

/// A translucent `::first-letter` is nested in the inline group its letter
/// comes from.
#[test]
fn a_translucent_first_letter_nests_in_its_source_group() {
    let document = lay_out(
        "<p><span>AB</span>C</p>",
        "p{margin:0}span{opacity:.5}p::first-letter{opacity:.25}",
    );
    let page = document.page(0).unwrap();
    assert_eq!(
        composited(&page),
        [
            "text C",
            "push 0.5",
            "text B",
            "push 0.25",
            "box FirstLetter",
            "text A",
            "pop",
            "pop"
        ]
    );
}

/// A first letter taken from a translucent `::before` is painted inside that
/// group, and a translucent one nests there as a group of its own.
#[test]
fn first_letters_of_a_translucent_before_stay_in_its_group() {
    let document = lay_out(
        "<p>C</p>",
        "p{margin:0}p::before{content:'AB';background:blue;opacity:.5}\
         p::first-letter{background:red}",
    );
    let page = document.page(0).unwrap();
    assert_eq!(
        composited(&page),
        [
            "text C",
            "push 0.5",
            "box Before",
            "box FirstLetter",
            "text AB",
            "pop"
        ]
    );
    let document = lay_out(
        "<p>C</p>",
        "p{margin:0}p::before{content:'AB';background:blue;opacity:.5}\
         p::first-letter{background:red;opacity:.25}",
    );
    let page = document.page(0).unwrap();
    assert_eq!(
        composited(&page),
        [
            "text C",
            "push 0.5",
            "box Before",
            "text B",
            "push 0.25",
            "box FirstLetter",
            "text A",
            "pop",
            "pop"
        ]
    );
}

/// A translucent `::after` follows the groups of its element's descendants
/// in tree order, and a positioned one keeps its z-index level.
#[test]
fn translucent_pseudo_groups_follow_tree_order_and_z_index() {
    let document = lay_out(
        "<p>A<span>B</span></p>",
        "p{margin:0}span{opacity:.5}p::after{content:'C';opacity:.25}",
    );
    let page = document.page(0).unwrap();
    assert_eq!(
        composited(&page),
        [
            "text A",
            "push 0.5",
            "text B",
            "pop",
            "push 0.25",
            "box After",
            "text C",
            "pop"
        ]
    );
    let document = lay_out(
        "<p>A<span>B</span></p>",
        "p{margin:0}span{opacity:.5;position:relative;z-index:2}\
         p::after{content:'C';opacity:.25;position:relative;z-index:1}",
    );
    let page = document.page(0).unwrap();
    assert_eq!(
        composited(&page),
        [
            "text A",
            "push 0.25",
            "box After",
            "text C",
            "pop",
            "push 0.5",
            "text B",
            "pop"
        ]
    );
}

/// Without supplied runs, a translucent `::before` still groups its
/// decoration and images.
#[test]
fn a_translucent_before_groups_its_boxes_without_runs() {
    let document = lay_out(
        "<div>A</div>",
        "div::before{content:'B' url(data:image/svg+xml,%3Csvg%20xmlns=%22http://www.w3.org/2000/svg%22%20width=%2210%22%20height=%2210%22/%3E);\
         background:red;opacity:.5}",
    );
    let page = document.page(0).unwrap();
    let order: Vec<String> = page
        .paint_order()
        .iter()
        .filter_map(|event| match event {
            PaintEvent::PushOpacity(alpha) => Some(format!("push {alpha}")),
            PaintEvent::PopOpacity => Some("pop".into()),
            PaintEvent::GeneratedBox(piece) => Some(format!("box {:?}", piece.kind)),
            PaintEvent::GeneratedImage(_) => Some("image".into()),
            PaintEvent::Text(_) => Some("text".into()),
            _ => None,
        })
        .collect();
    assert_eq!(order, ["text", "push 0.5", "box Before", "image", "pop"]);
}

/// The translucent first letter of a block nests inside the translucent
/// first letter of the block around it, which wraps the same letters.
#[test]
fn nested_translucent_first_letters_nest_their_groups() {
    let document = lay_out(
        "<div><p>AB</p></div>",
        "p{margin:0}div::first-letter{background:red;opacity:.5}\
         p::first-letter{background:blue;opacity:.25}",
    );
    let page = document.page(0).unwrap();
    assert_eq!(
        composited(&page),
        [
            "text B",
            "push 0.5",
            "box FirstLetter",
            "push 0.25",
            "box FirstLetter",
            "text A",
            "pop",
            "pop"
        ]
    );
}

/// The fragments of a `::first-letter` keep the opacity of the style they
/// retain for the text they wrap: fragments resolving `opacity` to
/// different values composite apart, and one resolving it to 1 is no group.
#[test]
fn first_letter_fragments_composite_with_their_retained_opacity() {
    let document = lay_out(
        "<p><span id=a>'</span><span id=b>AB</span></p>",
        "p{margin:0;--o:.5}#a{--o:0}#b{--o:1}p::first-letter{background:red;opacity:var(--o)}",
    );
    let page = document.page(0).unwrap();
    assert_eq!(
        composited(&page),
        [
            "box FirstLetter",
            "text AB",
            "push 0",
            "box FirstLetter",
            "text '",
            "pop"
        ]
    );
    let document = lay_out(
        "<p><span id=a>'</span><span id=b>AB</span></p>",
        "p{margin:0;--o:.5}#a{--o:.25}#b{--o:.75}p::first-letter{background:red;opacity:var(--o)}",
    );
    let page = document.page(0).unwrap();
    assert_eq!(
        composited(&page),
        [
            "text B",
            "push 0.25",
            "box FirstLetter",
            "text '",
            "pop",
            "push 0.75",
            "box FirstLetter",
            "text A",
            "pop"
        ]
    );
    let runs = page.text_runs();
    let group = |text: &str| {
        page.text_run_opacity_group(runs.iter().find(|run| run.text == text).unwrap())
            .unwrap()
    };
    assert_ne!(group("'"), group("A"));
}

/// A first letter whose letters are partly inside a translucent inline
/// element composites that part inside the element's group and the rest
/// apart, each letter drawn once.
#[test]
fn first_letter_split_by_an_inline_group_composites_in_both() {
    let document = lay_out(
        "<p><span id=a>'</span>AB</p>",
        "p{margin:0}#a{opacity:.5}p::first-letter{background:red;opacity:.25}",
    );
    let page = document.page(0).unwrap();
    assert_eq!(
        composited(&page),
        [
            "text B",
            "push 0.25",
            "box FirstLetter",
            "text A",
            "pop",
            "push 0.5",
            "push 0.25",
            "box FirstLetter",
            "text '",
            "pop",
            "pop"
        ]
    );
}

/// Fragments of one first letter in the same group compose once, even when
/// a fragment of another group lies between them.
#[test]
fn first_letter_fragments_of_one_group_compose_once() {
    let document = lay_out(
        "<p>'<span>A</span>'B</p>",
        "p{margin:0;opacity:.9}span{display:contents;opacity:.5}\
         p::first-letter{background:red;opacity:inherit}",
    );
    let page = document.page(0).unwrap();
    assert_eq!(
        composited(&page),
        [
            "push 0.9",
            "text B",
            "push 0.9",
            "box FirstLetter",
            "box FirstLetter",
            "text ''",
            "pop",
            "push 0.5",
            "box FirstLetter",
            "text A",
            "pop",
            "pop"
        ]
    );
}

/// The events of `page` summarized like [`composited`], with the transforms
/// around them.
fn composited_with_transforms(page: &raikiri_html::Page<'_>) -> Vec<String> {
    let runs = page.text_runs();
    let mut order = Vec::new();
    for event in page.paint_order_for_text_runs(&runs) {
        match event {
            PaintEvent::PushTransform(transform) => order.push(format!(
                "turn {} {} {} {} {} {}",
                transform.a, transform.b, transform.c, transform.d, transform.e, transform.f
            )),
            PaintEvent::PopTransform => order.push("unturn".into()),
            PaintEvent::PushOpacity(alpha) => order.push(format!("push {alpha}")),
            PaintEvent::PopOpacity => order.push("pop".into()),
            PaintEvent::GeneratedBox(piece) => order.push(format!("box {:?}", piece.kind)),
            PaintEvent::TextLine(line) | PaintEvent::GroupTextLine(line, _) => {
                let group = match event {
                    PaintEvent::GroupTextLine(_, group) => Some(group),
                    _ => None,
                };
                let text: String = runs
                    .iter()
                    .filter(|run| run.line == line && page.text_run_opacity_group(run) == group)
                    .map(|run| run.text)
                    .collect();
                order.push(format!("text {text}"));
            }
            _ => {}
        }
    }
    order
}

/// An atomic `::before` with a transform turns its box and its text as one
/// group about its own box (CSS Transforms 1 §6), after the rest of its
/// paragraph, and its runs carry that transform.
#[test]
fn a_transformed_atomic_before_turns_its_box_and_text() {
    let document = lay_out(
        "<div>A</div>",
        "div::before{content:'B';display:inline-block;background:red;transform:translateX(30px)}",
    );
    let page = document.page(0).unwrap();
    assert_eq!(
        composited_with_transforms(&page),
        [
            "text A",
            "turn 1 0 0 1 30 0",
            "box Before",
            "text B",
            "unturn"
        ]
    );
    let runs = page.text_runs();
    let generated = runs.iter().find(|run| run.text == "B").unwrap();
    assert_eq!(
        generated.transform,
        Some(raikiri_html::PaintTransform::translate(30.0, 0.0))
    );
    let own = runs.iter().find(|run| run.text == "A").unwrap();
    assert_eq!(own.transform, None);
}

/// The transform turns about the pseudo-element's transform origin, and it
/// nests inside the group of its opacity.
#[test]
fn a_transformed_translucent_after_turns_about_its_origin() {
    let document = lay_out(
        "<div>A</div>",
        "div::after{content:'C';display:inline-block;opacity:.5;\
         transform:scale(2);transform-origin:0 0}",
    );
    let page = document.page(0).unwrap();
    // The box is at x 20: scaling about its corner moves points by 20 to
    // the left of it.
    assert_eq!(
        composited_with_transforms(&page),
        [
            "text A",
            "turn 2 0 0 2 -20 0",
            "push 0.5",
            "box After",
            "text C",
            "pop",
            "unturn"
        ]
    );
}

/// An inline `::before` is not transformable, and a singular transform
/// paints nothing of the pseudo-element.
#[test]
fn inline_and_singular_pseudo_transforms() {
    let document = lay_out(
        "<div>A</div>",
        "div::before{content:'B';background:red;transform:translateX(30px)}",
    );
    let page = document.page(0).unwrap();
    assert_eq!(composited_with_transforms(&page), ["box Before", "text BA"]);
    let document = lay_out(
        "<div>A</div>",
        "div::before{content:'B';display:inline-block;background:red;transform:scale(0)}",
    );
    let page = document.page(0).unwrap();
    assert_eq!(composited_with_transforms(&page), ["text A"]);
}

/// A `::before` that an image alone in its `content` replaces is
/// transformable even when it is inline, and turns its image.
#[test]
fn a_replaced_inline_before_turns_its_image() {
    let document = lay_out(
        "<div>A</div>",
        "div::before{content:url(data:image/svg+xml,%3Csvg%20xmlns=%22http://www.w3.org/2000/svg%22%20width=%2210%22%20height=%2210%22/%3E);\
         background:red;transform:translateX(30px)}",
    );
    let page = document.page(0).unwrap();
    let events: Vec<String> = page
        .paint_order_for_text_runs(&page.text_runs())
        .iter()
        .filter_map(|event| match event {
            PaintEvent::PushTransform(transform) => {
                Some(format!("turn {} {}", transform.e, transform.f))
            }
            PaintEvent::PopTransform => Some("unturn".into()),
            PaintEvent::GeneratedBox(piece) => Some(format!("box {:?}", piece.kind)),
            PaintEvent::GeneratedImage(_) => Some("image".into()),
            _ => None,
        })
        .collect();
    assert_eq!(events, ["turn 30 0", "box Before", "image", "unturn"]);
}
