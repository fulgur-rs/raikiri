use super::*;
use taffy::Style;

// ── layout_single_page driver (Task 7) ──────────────────────

fn hello_world_doc() -> (Document, raikiri_style::CascadeResult) {
    // <html><head></head><body><p style="color:red">Hi</p></body></html>
    // 相当 (parser の代わりに手動構築、raikiri-html 統合は将来 umbrella が担当)
    use raikiri_style::{build_rule_tree, cascade};
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let _head = doc.append_element(Some(html), "head", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let p = doc.append_element(Some(body), "p", Style::default(), Some("color:red"));
    let _text = doc.append_text(p, "Hi");
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");
    (doc, cr)
}

#[test]
fn layout_pages_exercises_used_margin_fallbacks_and_page_insets() {
    use raikiri_style::{Origin, build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut direct_doc = Document::new();
    let html =
        direct_doc.append_element(Some(0), "html", Style::default(), Some("margin-top:auto"));
    let body = direct_doc.append_element(
        Some(html),
        "body",
        Style::default(),
        Some("background-color:red"),
    );
    direct_doc.append_text(body, "direct body text");
    let direct_rules = build_rule_tree(&direct_doc);
    let direct_cascade = cascade(&direct_doc, &direct_rules).expect("cascade Ok");
    let mut direct_page = PageBox::new();
    direct_page.width = 100.0;
    direct_page.height = 100.0;
    assert!(
        !layout_pages(
            &mut direct_doc,
            &direct_cascade,
            direct_page,
            FontContext::new(),
        )
        .expect("direct pagination Ok")
        .is_empty()
    );

    let mut block_doc = Document::new();
    let html = block_doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = block_doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    block_doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("height:20px;margin-bottom:auto"),
    );
    let mut block_rules = build_rule_tree(&block_doc);
    block_rules.add_stylesheet("@page { padding:10px; }", Origin::Author);
    let block_cascade = cascade(&block_doc, &block_rules).expect("cascade Ok");
    let mut block_page = PageBox::new();
    block_page.width = 100.0;
    block_page.height = 100.0;
    assert!(
        !layout_pages(
            &mut block_doc,
            &block_cascade,
            block_page,
            FontContext::new(),
        )
        .expect("block pagination Ok")
        .is_empty()
    );
}

#[test]
fn relayout_nested_multicol_children_packs_block_children_into_columns() {
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let container = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:block;column-count:2;column-gap:20px;width:200px"),
    );
    let a = doc.append_element(
        Some(container),
        "div",
        Style::default(),
        Some("display:block;height:30px"),
    );
    let b = doc.append_element(
        Some(container),
        "div",
        Style::default(),
        Some("display:block;height:30px"),
    );
    let c = doc.append_element(
        Some(container),
        "div",
        Style::default(),
        Some("display:block;height:30px"),
    );
    let d = doc.append_element(
        Some(container),
        "div",
        Style::default(),
        Some("display:block;height:30px"),
    );

    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");
    let mut page = PageBox::new();
    page.width = 300.0;
    page.height = 200.0;
    layout_single_page(&mut doc, &cascade, page, parley::FontContext::new()).expect("layout Ok");

    // Each column balances to a 60px target (4 * 30px total / 2
    // columns); the third child overflows the first column's budget and
    // starts the second column.
    assert!(
        doc.fragment_tree
            .break_tokens
            .iter()
            .any(|token| token.node_id == container && token.child_index == 2),
        "a column break must be recorded before the third child (index 2)"
    );
    let fragmentainer_of = |node: usize| {
        doc.fragment_tree
            .fragments
            .iter()
            .rev()
            .find(|fragment| fragment.node_id == node && fragment.line_start.is_none())
            .map(|fragment| fragment.fragmentainer)
    };
    assert_eq!(fragmentainer_of(a), Some(0));
    assert_eq!(fragmentainer_of(b), Some(0));
    assert_eq!(fragmentainer_of(c), Some(1));
    assert_eq!(fragmentainer_of(d), Some(1));

    // width 200 / 2 columns with a 20px gap: (200 - 20) / 2 = 90; column
    // 1 starts at 90 + 20 = 110.
    assert!((doc.nodes[a].unrounded_layout.location.x - 0.0).abs() < 0.01);
    assert!((doc.nodes[c].unrounded_layout.location.x - 110.0).abs() < 0.01);
    assert!((doc.nodes[a].unrounded_layout.location.y - 0.0).abs() < 0.01);
    assert!((doc.nodes[b].unrounded_layout.location.y - 30.0).abs() < 0.01);
    assert!((doc.nodes[container].unrounded_layout.size.height - 60.0).abs() < 0.01);
}

#[test]
fn relayout_nested_multicol_children_honors_break_before_avoid() {
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let container = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:block;column-count:2;column-gap:20px;width:200px"),
    );
    let a = doc.append_element(
        Some(container),
        "div",
        Style::default(),
        Some("display:block;height:30px"),
    );
    let b = doc.append_element(
        Some(container),
        "div",
        Style::default(),
        Some("display:block;height:30px"),
    );
    let c = doc.append_element(
        Some(container),
        "div",
        Style::default(),
        Some("display:block;height:30px;break-before:avoid"),
    );
    let d = doc.append_element(
        Some(container),
        "div",
        Style::default(),
        Some("display:block;height:30px"),
    );

    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");
    let mut page = PageBox::new();
    page.width = 300.0;
    page.height = 200.0;
    layout_single_page(&mut doc, &cascade, page, parley::FontContext::new()).expect("layout Ok");

    // `break-before:avoid` on the third child forbids the break that
    // would otherwise land there, so it stays in column 0 and the
    // fourth child is pushed into column 1 instead.
    assert!(
        doc.fragment_tree
            .break_tokens
            .iter()
            .any(|token| token.node_id == container && token.child_index == 3),
        "the break must move to the fourth child (index 3)"
    );
    let fragmentainer_of = |node: usize| {
        doc.fragment_tree
            .fragments
            .iter()
            .rev()
            .find(|fragment| fragment.node_id == node && fragment.line_start.is_none())
            .map(|fragment| fragment.fragmentainer)
    };
    assert_eq!(fragmentainer_of(a), Some(0));
    assert_eq!(fragmentainer_of(b), Some(0));
    assert_eq!(fragmentainer_of(c), Some(0));
    assert_eq!(fragmentainer_of(d), Some(1));
    assert!((doc.nodes[c].unrounded_layout.location.y - 60.0).abs() < 0.01);
    assert!((doc.nodes[container].unrounded_layout.size.height - 90.0).abs() < 0.01);
}

#[test]
fn establish_minimal_line_boxes_aligns_inline_block_text_baselines() {
    use parley::FontContext;
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let p = doc.append_element(Some(body), "p", Style::default(), Some("display:block"));
    let small = doc.append_element(
        Some(p),
        "span",
        Style::default(),
        Some("display:inline-block; font-size:10px; line-height:10px; vertical-align:8px"),
    );
    let small_text = doc.append_text(small, "A");
    let large = doc.append_element(
        Some(p),
        "span",
        Style::default(),
        Some("display:inline-block; font-size:20px; line-height:20px"),
    );
    let large_text = doc.append_text(large, "B");
    let lowered = doc.append_element(
        Some(p),
        "span",
        Style::default(),
        Some("display:inline-block; font-size:12px; line-height:12px; vertical-align:-4px"),
    );
    let _lowered_text = doc.append_text(lowered, "C");

    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");
    layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("layout Ok");

    let text_baseline = |element: usize, text: usize| {
        doc.nodes[element].unrounded_layout.location.y
            + doc.nodes[text].unrounded_layout.location.y
            + doc.nodes[text]
                .text_layout()
                .expect("text shaped")
                .lines()
                .next()
                .expect("one line")
                .metrics()
                .baseline
    };
    assert_eq!(
        doc.nodes[p].style.padding.top,
        LengthPercentage::length(8.0)
    );
    assert_eq!(
        doc.nodes[p].style.padding.bottom,
        LengthPercentage::length(4.0)
    );
    let small_baseline = text_baseline(small, small_text);
    let large_baseline = text_baseline(large, large_text);
    // cov:ignore: panic-message literal only executed on assertion failure.
    assert!(
        (small_baseline - large_baseline).abs() < 1e-3,
        "inline-block text baselines must align across siblings, got small={small_baseline} large={large_baseline}"
    );
}

fn inline_image_positions(
    parent_width: u32,
    image_specs: &[(u32, &str)],
    spaces_between: bool,
) -> Vec<(f32, f32, f32)> {
    use parley::FontContext;
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let p_style = format!("display:block;width:{parent_width}px;font-size:20px;line-height:20px");
    let p = doc.append_element(Some(body), "p", Style::default(), Some(&p_style));
    let mut images = Vec::with_capacity(image_specs.len());
    for (index, (height, extra_style)) in image_specs.iter().enumerate() {
        if spaces_between && index > 0 {
            doc.append_text(p, " ");
        }
        let image_style = format!("display:inline;width:8px;height:{height}px;{extra_style}");
        images.push(doc.append_element(Some(p), "img", Style::default(), Some(&image_style)));
    }

    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");
    layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("layout Ok");
    assert_eq!(
        doc.nodes[p].style.align_items,
        Some(TaffyAlignItems::BASELINE)
    );

    images
        .into_iter()
        .map(|image| {
            let layout = doc.nodes[image].unrounded_layout;
            (layout.location.x, layout.location.y, layout.size.height)
        })
        .collect()
}

#[test]
fn establish_minimal_line_boxes_keep_relative_image_offsets_out_of_shared_baseline() {
    let first_relative =
        inline_image_positions(100, &[(8, "position:relative;top:5px"), (4, "")], true);
    let bottom = |index: usize| first_relative[index].1 + first_relative[index].2;
    assert!(
        (bottom(0) - bottom(1) - 5.0).abs() < 1e-3,
        "a relative inset on the first image must not move its baseline sibling"
    );

    let second_relative =
        inline_image_positions(100, &[(8, ""), (4, "position:relative;top:5px")], true);
    let bottom = |index: usize| second_relative[index].1 + second_relative[index].2;
    assert!(
        (bottom(1) - bottom(0) - 5.0).abs() < 1e-3,
        "a relative inset on a later image must remain local to that image"
    );
}

#[test]
fn establish_minimal_line_boxes_keep_relative_horizontal_offsets_per_image() {
    let normal = inline_image_positions(100, &[(8, ""), (4, "")], true);
    let first_left =
        inline_image_positions(100, &[(8, "position:relative;left:5px"), (4, "")], true);
    let first_right =
        inline_image_positions(100, &[(8, "position:relative;right:5px"), (4, "")], true);
    let second_left =
        inline_image_positions(100, &[(8, ""), (4, "position:relative;left:5px")], true);
    let second_right =
        inline_image_positions(100, &[(8, ""), (4, "position:relative;right:5px")], true);
    let epsilon = 1e-3;

    assert!((first_left[0].0 - normal[0].0 - 5.0).abs() < epsilon);
    assert!((first_left[1].0 - normal[1].0).abs() < epsilon);
    assert!((first_right[0].0 - normal[0].0 + 5.0).abs() < epsilon);
    assert!((first_right[1].0 - normal[1].0).abs() < epsilon);
    assert!((second_left[0].0 - normal[0].0).abs() < epsilon);
    assert!((second_left[1].0 - normal[1].0 - 5.0).abs() < epsilon);
    assert!((second_right[0].0 - normal[0].0).abs() < epsilon);
    assert!((second_right[1].0 - normal[1].0 + 5.0).abs() < epsilon);
}

#[test]
fn establish_minimal_line_boxes_keep_bottom_relative_offset_after_manual_wrap() {
    let images = inline_image_positions(
        16,
        &[
            (8, "vertical-align:bottom"),
            (4, "vertical-align:bottom"),
            (6, "vertical-align:bottom"),
            (3, "vertical-align:bottom;position:relative;top:5px"),
        ],
        false,
    );
    let bottom = |index: usize| images[index].1 + images[index].2;
    assert!(
        (bottom(0) - bottom(1)).abs() < 1e-3,
        "the first bottom-aligned image row must share its line edge"
    );
    assert!(
        (bottom(3) - bottom(2) - 5.0).abs() < 1e-3,
        "the later row must preserve only the last image's relative inset"
    );
}

fn assert_inline_image_bottom_matches_text_baseline(text_content: &str, white_space: &str) {
    use parley::FontContext;
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let p_style =
        format!("display:block;font-size:20px;line-height:20px;white-space:{white_space}");
    let p = doc.append_element(Some(body), "p", Style::default(), Some(&p_style));
    // Comments are ignored by the inline bridge and exercise its non-box path.
    doc.append_comment(Some(p), "baseline helper should ignore comments");
    let text = doc.append_text(p, text_content);
    let image = doc.append_element(
        Some(p),
        "img",
        Style::default(),
        Some("display:inline;width:8px;height:8px"),
    );

    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");
    layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("layout Ok");

    assert_eq!(doc.nodes[p].style.display, Display::Flex);
    assert_eq!(
        doc.nodes[p].style.align_items,
        Some(TaffyAlignItems::BASELINE)
    );
    let text_baseline = doc.nodes[text].unrounded_layout.location.y
        + doc.nodes[text]
            .text_layout()
            .expect("text shaped")
            .lines()
            .next()
            .expect("one line")
            .metrics()
            .baseline;
    let image_bottom = doc.nodes[image].unrounded_layout.location.y
        + doc.nodes[image].unrounded_layout.size.height;
    assert!(
        (image_bottom - text_baseline).abs() < 1e-3,
        "inline image bottom must meet the text baseline, got image_bottom={image_bottom} text_baseline={text_baseline}"
    );
    if white_space == "pre" {
        let text_right = doc.nodes[text].unrounded_layout.location.x
            + doc.nodes[text].unrounded_layout.size.width;
        assert!(
            doc.nodes[image].unrounded_layout.location.x >= text_right - 1e-3,
            "preserved leading space must advance the image, got image_x={} text_right={text_right}",
            doc.nodes[image].unrounded_layout.location.x
        );
    }
}

#[test]
fn establish_minimal_line_boxes_aligns_inline_image_bottom_to_visible_text_baseline() {
    assert_inline_image_bottom_matches_text_baseline("A", "normal");
}

#[test]
fn establish_minimal_line_boxes_aligns_replaced_bottoms_after_nbsp_text() {
    use parley::FontContext;
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let p = doc.append_element(
        Some(body),
        "p",
        Style::default(),
        Some("display:block;font-size:20px;line-height:20px;white-space:normal"),
    );
    // A normal-mode NBSP is preserved as an advance-only item; it must not let
    // the legacy top-alignment pass undo the replaced-box baseline fallback.
    doc.append_text(p, "\u{00A0}");
    let tall_image = doc.append_element(
        Some(p),
        "img",
        Style::default(),
        Some("display:inline;width:8px;height:8px"),
    );
    let short_image = doc.append_element(
        Some(p),
        "img",
        Style::default(),
        Some("display:inline;width:8px;height:4px"),
    );

    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");
    layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("layout Ok");

    assert_eq!(
        doc.nodes[p].style.align_items,
        Some(TaffyAlignItems::BASELINE)
    );
    let tall_bottom = doc.nodes[tall_image].unrounded_layout.location.y
        + doc.nodes[tall_image].unrounded_layout.size.height;
    let short_bottom = doc.nodes[short_image].unrounded_layout.location.y
        + doc.nodes[short_image].unrounded_layout.size.height;
    assert!(
        (tall_bottom - short_bottom).abs() < 1e-3,
        "baseline fallback must align replaced bottoms after NBSP, got tall={tall_bottom} short={short_bottom}"
    );
}

#[test]
fn establish_minimal_line_boxes_aligns_replaced_bottoms_across_collapsible_space() {
    use parley::FontContext;
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let p = doc.append_element(
        Some(body),
        "p",
        Style::default(),
        Some("display:block;font-size:20px;line-height:20px;white-space:normal"),
    );
    let tall_image = doc.append_element(
        Some(p),
        "img",
        Style::default(),
        Some("display:inline;width:8px;height:8px"),
    );
    doc.append_text(p, " ");
    let short_image = doc.append_element(
        Some(p),
        "img",
        Style::default(),
        Some("display:inline;width:8px;height:4px"),
    );

    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");
    layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("layout Ok");

    assert_eq!(
        doc.nodes[p].style.align_items,
        Some(TaffyAlignItems::BASELINE)
    );
    let tall_bottom = doc.nodes[tall_image].unrounded_layout.location.y
        + doc.nodes[tall_image].unrounded_layout.size.height;
    let short_bottom = doc.nodes[short_image].unrounded_layout.location.y
        + doc.nodes[short_image].unrounded_layout.size.height;
    assert!(
        (tall_bottom - short_bottom).abs() < 1e-3,
        "normal-space inline images must retain Taffy's shared baseline, got tall={tall_bottom} short={short_bottom}"
    );
}

#[test]
fn establish_minimal_line_boxes_recomputes_baselines_after_manual_wrap() {
    use parley::FontContext;
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let p = doc.append_element(
        Some(body),
        "p",
        Style::default(),
        Some("display:block;width:16px;font-size:20px;line-height:20px"),
    );
    let first_image = doc.append_element(
        Some(p),
        "img",
        Style::default(),
        Some("display:inline;width:8px;height:8px"),
    );
    let second_image = doc.append_element(
        Some(p),
        "img",
        Style::default(),
        Some("display:inline;width:8px;height:4px"),
    );
    let third_image = doc.append_element(
        Some(p),
        "img",
        Style::default(),
        Some("display:inline;width:8px;height:6px"),
    );
    let fourth_image = doc.append_element(
        Some(p),
        "img",
        Style::default(),
        Some("display:inline;width:8px;height:3px"),
    );

    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");
    layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("layout Ok");

    assert_eq!(
        doc.nodes[p].style.align_items,
        Some(TaffyAlignItems::BASELINE)
    );
    let bottom = |image: usize| {
        doc.nodes[image].unrounded_layout.location.y + doc.nodes[image].unrounded_layout.size.height
    };
    let first_row_baseline = bottom(first_image);
    let second_row_baseline = bottom(second_image);
    let third_row_baseline = bottom(third_image);
    let fourth_row_baseline = bottom(fourth_image);
    assert!(
        (first_row_baseline - second_row_baseline).abs() < 1e-3,
        "the first manually wrapped row must share its image baseline"
    );
    assert!(
        (third_row_baseline - fourth_row_baseline).abs() < 1e-3,
        "the second manually wrapped row must recompute its own image baseline"
    );
    assert!(
        (doc.nodes[third_image].unrounded_layout.location.y
            - doc.nodes[first_image].unrounded_layout.location.y
            - 20.0)
            .abs()
            < 1e-3,
        "the second row must start one line-height after the first, without stale Taffy offset"
    );
}

#[test]
fn establish_minimal_line_boxes_preserves_inline_image_baseline_with_preformatted_space() {
    assert_inline_image_bottom_matches_text_baseline(" ", "pre");
}

#[test]
fn establish_minimal_line_boxes_aligns_inline_image_with_wrapped_text_baseline() {
    use parley::FontContext;
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let p = doc.append_element(
        Some(body),
        "p",
        Style::default(),
        Some("display:block;font-size:20px;line-height:20px"),
    );
    let wrapper = doc.append_element(Some(p), "span", Style::default(), Some("display:inline"));
    let text = doc.append_text(wrapper, "A");
    let image = doc.append_element(
        Some(p),
        "img",
        Style::default(),
        Some("display:inline;width:8px;height:8px"),
    );

    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");
    layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("layout Ok");

    assert_eq!(
        doc.nodes[p].style.align_items,
        Some(TaffyAlignItems::BASELINE)
    );
    let text_baseline = doc.nodes[wrapper].unrounded_layout.location.y
        + doc.nodes[text].unrounded_layout.location.y
        + doc.nodes[text]
            .text_layout()
            .expect("text shaped")
            .lines()
            .next()
            .expect("one line")
            .metrics()
            .baseline;
    let image_bottom = doc.nodes[image].unrounded_layout.location.y
        + doc.nodes[image].unrounded_layout.size.height;
    assert!(
        (image_bottom - text_baseline).abs() < 1e-3,
        "inline image bottom must meet the wrapped text baseline, got image_bottom={image_bottom} text_baseline={text_baseline}"
    );
}

#[test]
fn establish_minimal_line_boxes_keep_ordinary_inline_text_top_aligned() {
    use parley::FontContext;
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let p = doc.append_element(Some(body), "p", Style::default(), Some("display:block"));
    doc.append_text(p, "A");
    let wrapper = doc.append_element(Some(p), "span", Style::default(), Some("display:inline"));
    let raised = doc.append_element(
        Some(wrapper),
        "span",
        Style::default(),
        Some("display:inline; vertical-align:super"),
    );
    doc.append_text(raised, "B");
    doc.append_text(p, "C");

    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");
    layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("layout Ok");

    assert_eq!(doc.nodes[p].style.display, Display::Flex);
    assert_eq!(
        doc.nodes[p].style.align_items,
        Some(TaffyAlignItems::FLEX_START)
    );
    assert_eq!(
        doc.nodes[p].style.padding.top,
        LengthPercentage::length(16.0 / 3.0)
    );
    assert_eq!(
        doc.nodes[p].style.padding.bottom,
        LengthPercentage::length(0.0)
    );
}

#[test]
fn inline_block_baseline_uses_last_in_flow_text_line() {
    use parley::FontContext;
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let p = doc.append_element(Some(body), "p", Style::default(), Some("display:block"));
    let multiline = doc.append_element(
        Some(p),
        "span",
        Style::default(),
        Some("display:inline-block; width:20px; font-size:10px; line-height:10px"),
    );
    let first_text = doc.append_text(multiline, "A");
    doc.append_element(Some(multiline), "br", Style::default(), None::<&str>);
    let last_text = doc.append_text(multiline, "B");
    let hidden = doc.append_element(
        Some(multiline),
        "span",
        Style::default(),
        Some("display:none"),
    );
    doc.append_text(hidden, "hidden");
    let single_line = doc.append_element(
        Some(p),
        "span",
        Style::default(),
        Some("display:inline-block; font-size:20px; line-height:20px"),
    );
    let sibling_text = doc.append_text(single_line, "C");

    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");
    layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("layout Ok");

    let first_y = doc.nodes[first_text].unrounded_layout.location.y;
    let last_y = doc.nodes[last_text].unrounded_layout.location.y;
    assert!(last_y > first_y, "the br must put B on a later line");
    let multiline_baseline = doc.nodes[multiline].unrounded_layout.location.y
        + last_y
        + doc.nodes[last_text]
            .text_layout()
            .expect("last text shaped")
            .lines()
            .next()
            .expect("one line in B text node")
            .metrics()
            .baseline;
    let sibling_baseline = doc.nodes[single_line].unrounded_layout.location.y
        + doc.nodes[sibling_text].unrounded_layout.location.y
        + doc.nodes[sibling_text]
            .text_layout()
            .expect("sibling text shaped")
            .lines()
            .next()
            .expect("one sibling line")
            .metrics()
            .baseline;
    assert!(
        (multiline_baseline - sibling_baseline).abs() < 1e-3,
        "inline-block last text baseline must align with sibling: {multiline_baseline} vs {sibling_baseline}" // cov:ignore: panic-message literal only runs if the assertion fails.
    );
}

#[test]
fn establish_minimal_line_boxes_honors_vertical_align_top_and_bottom() {
    use parley::FontContext;
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let p = doc.append_element(Some(body), "p", Style::default(), Some("display:block"));
    let top = doc.append_element(
        Some(p),
        "span",
        Style::default(),
        Some("display:inline-block; width:10px; height:30px; vertical-align:top"),
    );
    let bottom = doc.append_element(
        Some(p),
        "span",
        Style::default(),
        Some("display:inline-block; width:10px; height:10px; vertical-align:bottom"),
    );
    let wrapper = doc.append_element(
        Some(p),
        "span",
        Style::default(),
        Some("display:inline; padding-top:20px"),
    );
    let _wrapper_text = doc.append_text(wrapper, "A");
    let intermediary = doc.append_element(
        Some(wrapper),
        "span",
        Style::default(),
        Some("display:inline"),
    );
    let _nested_top = doc.append_element(
        Some(intermediary),
        "span",
        Style::default(),
        Some("display:inline-block; width:10px; height:30px; vertical-align:top"),
    );
    let plain_wrapper =
        doc.append_element(Some(p), "span", Style::default(), Some("display:inline"));
    let _plain_text = doc.append_text(plain_wrapper, "B");

    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");
    layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("layout Ok");

    let parent_y = doc.nodes[p].unrounded_layout.location.y;
    let parent_bottom = parent_y + doc.nodes[p].unrounded_layout.size.height;
    let top_layout = doc.nodes[top].unrounded_layout;
    let bottom_layout = doc.nodes[bottom].unrounded_layout;
    assert!((top_layout.location.y - parent_y).abs() < 1e-3);
    assert!((bottom_layout.location.y + bottom_layout.size.height - parent_bottom).abs() < 1e-3);
    assert!((top_layout.size.height - 30.0).abs() < 1e-3);
    assert!((bottom_layout.size.height - 10.0).abs() < 1e-3);
    assert_eq!(
        doc.nodes[wrapper].style.padding.top,
        LengthPercentage::length(0.0)
    );
}

#[test]
fn terminal_preserved_newline_does_not_create_an_empty_line_box() {
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let block = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:inline-block;font:24px monospace;white-space:pre-wrap;letter-spacing:10px"),
    );
    let content_span = doc.append_element(Some(block), "span", Style::default(), None::<&str>);
    let content = doc.append_text(content_span, "1. a");
    let newline_span = doc.append_element(Some(block), "span", Style::default(), None::<&str>);
    let newline = doc.append_text(newline_span, "\n");

    let _separator = doc.append_element(Some(body), "div", Style::default(), Some("display:block"));
    let direct_block = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:inline-block;font:24px monospace;white-space:pre-wrap;letter-spacing:10px"),
    );
    let direct_content_span =
        doc.append_element(Some(direct_block), "span", Style::default(), None::<&str>);
    let direct_content = doc.append_text(direct_content_span, "2. b");
    let direct_newline = doc.append_text(direct_block, "\n");

    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");
    layout_single_page(&mut doc, &cascade, PageBox::A4, FontContext::new()).expect("layout Ok");

    assert_eq!(
        doc.nodes[content]
            .text_layout()
            .expect("content shaped")
            .len(),
        1
    );
    assert!(doc.nodes[newline].text_layout().is_none());
    assert!(doc.nodes[block].unrounded_layout.size.height < 40.0);
    assert_eq!(
        doc.nodes[direct_content]
            .text_layout()
            .expect("direct content shaped")
            .len(),
        1
    );
    assert!(doc.nodes[direct_newline].text_layout().is_none());
    assert!(doc.nodes[direct_block].unrounded_layout.size.height < 40.0);
}

#[test]
fn text_indent_offsets_empty_inline_block_by_content_box_percentage() {
    use parley::FontContext;
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let block = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:block;box-sizing:border-box;width:120px;padding-right:10px;text-indent:50%"),
    );
    let inline_block = doc.append_element(
        Some(block),
        "span",
        Style::default(),
        Some("display:inline-block;width:10px;height:10px"),
    );
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");
    layout_single_page(&mut doc, &cascade, PageBox::A4, FontContext::new()).expect("layout Ok");

    assert!((doc.nodes[block].unrounded_layout.content_box_width() - 110.0).abs() < 0.01);
    assert!((doc.nodes[inline_block].unrounded_layout.location.x - 55.0).abs() < 0.01);
}

#[test]
fn modifier_text_indent_ch_changes_taffy_height_before_layout() {
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;
    use std::path::PathBuf;

    let fonts_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("target")
        .join("wpt")
        .join("fonts");
    // cov:ignore: this integration fixture is intentionally skippable when shared WPT assets are absent.
    if !fonts_dir.join("Ahem.ttf").exists() {
        eprintln!(
            "skipping modifier text-indent ch test: Ahem.ttf is required under {}",
            fonts_dir.display()
        );
        return;
    }

    fn measure(fonts_dir: &std::path::Path, value: &str) -> (usize, f32, f32, bool, bool, bool) {
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
        let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
        let block = doc.append_element(
                Some(body),
                "p",
                Style::default(),
                Some(&format!(
                    "display:block;width:80px;font-family:Ahem;font-size:16px;line-height:20px;word-break:break-all;text-indent:{value}"
                )),
            );
        let text = doc.append_text(block, "0000000000");
        let rules = build_rule_tree(&doc);
        let cascade = cascade(&doc, &rules).expect("cascade Ok");
        let fonts =
            crate::fonts::build_wpt_font_ctx(fonts_dir).expect("bundled WPT fonts should register");
        layout_single_page(&mut doc, &cascade, PageBox::A4, fonts).expect("layout Ok");
        let layout = doc.nodes[text].text_layout().expect("text shaped");
        let crate::node::NodeData::Text(text_data) = &doc.nodes[text].data else {
            unreachable!("text node remains text after layout"); // cov:ignore: layout preserves text nodes after shaping.
        };
        (
            layout.len(),
            layout.height(),
            doc.nodes[block].unrounded_layout.size.height,
            text_data.text_indent_hanging,
            text_data.text_indent_each_line,
            text_data.text_indent_rebreak,
        )
    }

    let hanging = measure(&fonts_dir, "2ch hanging");
    let each_line = measure(&fonts_dir, "2ch each-line");
    // Parley's `each_line` option covers the first and hard-break lines;
    // soft-wrap continuation lines still use its normal scope semantics.
    assert_eq!(hanging.0, 3);
    assert_eq!(each_line.0, 3);
    assert!((hanging.1 - hanging.2).abs() < 0.01);
    assert!((each_line.1 - each_line.2).abs() < 0.01);
    assert!(hanging.2 > 20.0);
    assert!(each_line.2 > 20.0);
    assert!(hanging.3 && !hanging.4 && hanging.5);
    assert!(!each_line.3 && each_line.4 && each_line.5);
}

#[test]
fn width_ch_and_text_indent_ch_change_taffy_height_before_layout() {
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;
    use std::path::PathBuf;

    let fonts_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("target")
        .join("wpt")
        .join("fonts");
    // cov:ignore: this integration fixture is intentionally skippable when shared WPT assets are absent.
    if !fonts_dir.join("Ahem.ttf").exists() {
        eprintln!(
            "skipping width/text-indent ch test: Ahem.ttf is required under {}",
            fonts_dir.display()
        );
        return;
    }

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), Some("margin:0"));
    let block = doc.append_element(
            Some(body),
            "p",
            Style::default(),
            Some(
                "display:block;width:2ch;font-family:Ahem;font-size:16px;line-height:20px;word-break:break-all;text-indent:1ch",
            ),
        );
    let text = doc.append_text(block, "00");
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");
    let fonts =
        crate::fonts::build_wpt_font_ctx(&fonts_dir).expect("bundled WPT fonts should register");
    layout_single_page(&mut doc, &cascade, PageBox::A4, fonts).expect("layout Ok");

    let layout = doc.nodes[text].text_layout().expect("text shaped");
    assert_eq!(layout.len(), 2, "one ch of indent leaves one ch per line");
    assert!((layout.height() - 40.0).abs() < 0.01);
    let block_layout = doc.nodes[block].unrounded_layout;
    assert!((block_layout.size.width - 32.0).abs() < 0.01);
    assert!((block_layout.size.height - 40.0).abs() < 0.01);
    assert!(matches!(
        &doc.nodes[text].data,
        crate::node::NodeData::Text(text)
            if text.text_indent_px.is_some() && text.text_indent_rebreak
    ));
}

#[test]
fn relayout_text_for_width_rebuilds_pre_taffy_indent_state() {
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let p = doc.append_element(
        Some(body),
        "p",
        Style::default(),
        Some("display:block;width:80px;font-size:16px;text-indent:1ch"),
    );
    let text = doc.append_text(p, "00 00");
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");
    layout_single_page(&mut doc, &cascade, PageBox::A4, FontContext::new()).expect("layout Ok");
    relayout_text_for_width(&mut doc, &cascade, 80.0, 80.0, FontContext::new());
    assert!(doc.nodes[text].text_layout().is_some());
    assert!(matches!(
        &doc.nodes[text].data,
        crate::node::NodeData::Text(text) if text.text_indent_px.is_some()
    ));
}

#[test]
fn ch_box_values_reach_taffy_before_percentage_children() {
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;
    use std::path::PathBuf;

    let fonts_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("target")
        .join("wpt")
        .join("fonts");
    // cov:ignore: this integration fixture is intentionally skippable when shared WPT assets are absent.
    if !fonts_dir.join("Ahem.ttf").exists() {
        eprintln!(
            "skipping ch box test: Ahem.ttf is required under {}",
            fonts_dir.display()
        );
        return;
    }

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), Some("margin:0"));
    let parent = doc.append_element(
            Some(body),
            "div",
            Style::default(),
            Some("display:block;font-family:Ahem;font-size:16px;width:1ch;height:2ch;padding-top:1ch;padding-right:2ch;padding-bottom:1ch;padding-left:1ch;margin-top:1ch;margin-right:3ch;margin-bottom:1ch;margin-left:1ch"),
        );
    let child = doc.append_element(
        Some(parent),
        "div",
        Style::default(),
        Some("display:block;width:100%;height:1px"),
    );
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");
    let fonts =
        crate::fonts::build_wpt_font_ctx(&fonts_dir).expect("bundled WPT fonts should register");
    layout_single_page(&mut doc, &cascade, PageBox::A4, fonts).expect("layout Ok");

    let parent_layout = doc.nodes[parent].unrounded_layout;
    let child_layout = doc.nodes[child].unrounded_layout;
    assert!((parent_layout.size.width - 64.0).abs() < 0.01);
    assert!((parent_layout.size.height - 64.0).abs() < 0.01);
    let used_style = &doc.nodes[parent].style;
    assert_eq!(used_style.padding.top.into_raw().value(), 16.0);
    assert_eq!(used_style.padding.right.into_raw().value(), 32.0);
    assert_eq!(used_style.padding.bottom.into_raw().value(), 16.0);
    assert_eq!(used_style.padding.left.into_raw().value(), 16.0);
    assert_eq!(used_style.margin.top.into_raw().value(), 16.0);
    assert_eq!(used_style.margin.right.into_raw().value(), 48.0);
    assert_eq!(used_style.margin.bottom.into_raw().value(), 16.0);
    assert_eq!(used_style.margin.left.into_raw().value(), 16.0);
    assert!((parent_layout.location.x - 16.0).abs() < 0.01);
    assert!((child_layout.size.width - 16.0).abs() < 0.01);
}

/// single-line block の glyph 両端を返す helper
/// (first glyph x, last glyph x+advance)。
fn single_line_ends(doc: &Document, t: usize) -> (f32, f32) {
    use parley::PositionedLayoutItem;

    let layout = doc.nodes[t].text_layout().expect("text shaped");
    assert_eq!(layout.len(), 1, "fixture must stay single-line");
    let mut first = None;
    let mut last = (0.0, 0.0);
    for line in layout.lines() {
        for it in line.items() {
            if let PositionedLayoutItem::GlyphRun(gr) = it {
                for g in gr.positioned_glyphs() {
                    if first.is_none() {
                        first = Some(g.x);
                    }
                    last = (g.x, g.advance);
                }
            }
        }
    }
    (first.expect("glyph"), last.0 + last.1)
}

#[test]
fn text_justify_none_disables_justification() {
    // text-align:justify + text-justify:none → spread しない
    // (CSS Text 3 §6.2)。
    use parley::FontContext;
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let div = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display: block; width: 200px; text-align: justify; text-justify: none"),
    );
    let t = doc.append_text(div, "aa bb cc");

    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");
    layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("layout Ok");

    let (first_x, last_end) = single_line_ends(&doc, t);
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert!(
        first_x.abs() < 2.0 && last_end < 150.0,
        "unjustified single line must not spread: first={} last_end={}",
        first_x,
        last_end
    );
}

#[test]
fn word_break_break_all_rebreaks_narrow_container_text() {
    // Text is initially shaped against the page width. `word-break` must
    // still take effect when the containing block is narrower than that
    // preshape width.
    use parley::FontContext;
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let div = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display: block; width: 1px; word-break: break-all"),
    );
    let t = doc.append_text(div, "ab");

    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");
    layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("layout Ok");

    let layout = doc.nodes[t].text_layout().expect("text shaped");
    assert_eq!(layout.len(), 2, "break-all text must wrap in a 1px block");
}

#[test]
fn break_spaces_rebreaks_narrow_container_text() {
    // `break-spaces` preserves spaces but still needs the containing block
    // width during the post-layout rebreak (the initial shape uses page width).
    use parley::FontContext;
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let div = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display: block; width: 1px; white-space: break-spaces"),
    );
    let t = doc.append_text(div, "a b");

    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");
    layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("layout Ok");

    let layout = doc.nodes[t].text_layout().expect("text shaped");
    assert_eq!(
        layout.len(),
        2,
        "break-spaces text must wrap in a 1px block"
    );
}

#[test]
fn text_wrap_nowrap_keeps_single_line_in_narrow_container() {
    // `text-wrap: nowrap` suppresses soft wrapping (CSS Text 4 §5,
    // Long text in a narrow block stays one line.
    use parley::FontContext;
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let div = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display: block; width: 60px; text-wrap: nowrap"),
    );
    let t = doc.append_text(div, "aaaa bbbb cccc dddd eeee ffff");

    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");
    layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("layout Ok");

    let layout = doc.nodes[t].text_layout().expect("text shaped");
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(layout.len(), 1, "nowrap text must not soft-wrap");
}

#[test]
fn establish_minimal_line_boxes_br_forces_second_line_end_to_end() {
    // End-to-end check (full `layout_single_page` pipeline, matching
    // `establish_minimal_line_boxes_lays_out_children_side_by_side_not_stacked`'s
    // style) for the concrete geometry `<br>` must produce: the content
    // after `<br>` lands on a lower line than the content before it,
    // and the `<br>` itself contributes no visible height — the line
    // it alone occupies (taffy's line-packing algorithm assigns it one
    // because its flex_basis:100% never fits next to prior content)
    // must have cross size 0, so it does not introduce a phantom blank
    // line between the two real ones.
    use parley::FontContext;
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let p = doc.append_element(Some(body), "p", Style::default(), Some("display: block"));
    let a = doc.append_text(p, "AAAA");
    let br = doc.append_element(Some(p), "br", Style::default(), None::<&str>);
    let c = doc.append_text(p, "CCCC");

    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");
    layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("layout Ok");

    let a_loc = doc.nodes[a].unrounded_layout;
    let br_loc = doc.nodes[br].unrounded_layout;
    let c_loc = doc.nodes[c].unrounded_layout;
    let p_loc = doc.nodes[p].unrounded_layout;

    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert!(
        c_loc.location.y >= a_loc.location.y + a_loc.size.height - 1e-3,
        "the text after <br> must start at or below the first line's \
             bottom edge (forced break into a second line), got \
             a.y={} a.h={} c.y={}",
        a_loc.location.y,
        a_loc.size.height,
        c_loc.location.y
    );
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        br_loc.size.height, 0.0,
        "<br> is a childless leaf with no text layout, so its own \
             measured cross size must be 0 — the load-bearing fact that \
             keeps the line it alone occupies from adding visible height"
    );
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert!(
        (p_loc.size.height - (a_loc.size.height + c_loc.size.height)).abs() < 1e-3,
        "the container's total height must equal exactly the sum of \
             the two real lines' heights — no phantom third (blank) line \
             from the line <br> alone occupies, got p.h={} a.h={} c.h={}",
        p_loc.size.height,
        a_loc.size.height,
        c_loc.size.height
    );
}

#[test]
fn establish_minimal_line_boxes_consecutive_br_adds_no_blank_line_either() {
    // Same mechanism as the leading-<br> Non-goal
    // (`establish_minimal_line_boxes_leading_br_does_not_add_leading_blank_line`),
    // checked for the *second* <br> in a `<br><br>` run instead of the
    // first: after the first <br> takes the whole of its own (now
    // empty) line, the second <br> is checked against a line with 0
    // remaining space — still its line's first item (the first <br>
    // already moved on), so the same "an empty line accepts its first
    // item" exception applies to it too, and it likewise measures
    // 0-height. Confirms the doc's claim explicitly, rather than
    // leaving it as an un-pinned assertion about a case distinct from
    // the leading-<br> one.
    use parley::FontContext;
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let p = doc.append_element(Some(body), "p", Style::default(), Some("display: block"));
    let a = doc.append_text(p, "AAAA");
    let br1 = doc.append_element(Some(p), "br", Style::default(), None::<&str>);
    let br2 = doc.append_element(Some(p), "br", Style::default(), None::<&str>);
    let b = doc.append_text(p, "BBBB");

    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");
    layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("layout Ok");

    let a_loc = doc.nodes[a].unrounded_layout;
    let br1_loc = doc.nodes[br1].unrounded_layout;
    let br2_loc = doc.nodes[br2].unrounded_layout;
    let b_loc = doc.nodes[b].unrounded_layout;
    let p_loc = doc.nodes[p].unrounded_layout;

    assert_eq!(br1_loc.size.height, 0.0);
    assert_eq!(br2_loc.size.height, 0.0);
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert!(
        b_loc.location.y >= a_loc.location.y + a_loc.size.height - 1e-3,
        "got a.y={} a.h={} b.y={}",
        a_loc.location.y,
        a_loc.size.height,
        b_loc.location.y
    );
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert!(
        (p_loc.size.height - (a_loc.size.height + b_loc.size.height)).abs() < 1e-3,
        "a consecutive <br><br> must not add a visible blank line \
             between AAAA and BBBB — got p.h={} a.h={} b.h={}",
        p_loc.size.height,
        a_loc.size.height,
        b_loc.size.height
    );
}

#[test]
fn layout_single_page_without_body_returns_error() {
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::{LayoutError, PageBox};

    // <p> 直接 attach (fragment 相当)
    let mut doc = Document::new();
    let _p = doc.append_element(Some(0), "p", Style::default(), None::<&str>);
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).unwrap();

    match layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()) {
        Err(LayoutError::Internal { message }) => {
            assert!(
                message.contains("body"),
                "error message should mention <body>, got '{}'",
                message
            );
        }
        other => panic!("expected LayoutError::Internal, got {:?}", other),
    }
}

#[test]
fn layout_single_page_bridges_display_none() {
    // layout_single_page 経由で display bridge が active
    // であることを確認 — body に display:none を指定すると taffy::Style.display
    // が Display::None になる。
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let _head = doc.append_element(Some(html), "head", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), Some("display:none"));
    let p = doc.append_element(Some(body), "p", Style::default(), Some("color:red"));
    let _text = doc.append_text(p, "Hi");
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");

    layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("layout Ok");
    assert_eq!(doc.nodes[body].style.display, Display::None);
}

#[test]
fn inline_block_width_auto_shrink_wraps_to_content() {
    // width:auto の inline-block は containing block いっぱいに広がらず
    // content に shrink-wrap する (shrink-to-fit)。block の子として
    // fill される plain block との差を geometry で check する:
    // 100px の child を持つ inline-block は幅 100 に、明示 width:300px の
    // inline-block は 300 のままになる (どちらも body 幅 fill ではない)。
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let _head = doc.append_element(Some(html), "head", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    // Block sibling first so body keeps mixed children (no minimal
    // line-box rerouting) and lays the inline-blocks out as block
    // children with a definite available width.
    let _p = doc.append_element(Some(body), "p", Style::default(), None::<&str>);
    let ib = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:inline-block"),
    );
    let _child = doc.append_element(
        Some(ib),
        "div",
        Style::default(),
        Some("width:100px;height:20px"),
    );
    let ib_fixed = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:inline-block;width:300px"),
    );
    let _fixed_child = doc.append_element(
        Some(ib_fixed),
        "div",
        Style::default(),
        Some("width:100px;height:20px"),
    );
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");

    layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("layout Ok");

    let auto_size = doc.nodes[ib].unrounded_layout.size;
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert!(
        (auto_size.width - 100.0).abs() < 0.5,
        "width:auto inline-block must shrink-wrap its 100px child, got width={}",
        auto_size.width
    );
    let fixed_size = doc.nodes[ib_fixed].unrounded_layout.size;
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert!(
        (fixed_size.width - 300.0).abs() < 0.5,
        "explicit-width inline-block must keep its specified width, got width={}",
        fixed_size.width
    );
}

#[test]
fn flex_items_follow_order_modified_source_order() {
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let _head = doc.append_element(Some(html), "head", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let flex = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:flex;width:160px;height:20px"),
    );
    let a = doc.append_element(
        Some(flex),
        "div",
        Style::default(),
        Some("order:2;width:40px;height:20px"),
    );
    let absolute = doc.append_element(
        Some(flex),
        "div",
        Style::default(),
        Some("position:absolute;order:-100;width:10px;height:10px"),
    );
    let b = doc.append_element(
        Some(flex),
        "div",
        Style::default(),
        Some("order:-1;width:40px;height:20px"),
    );
    let c = doc.append_element(
        Some(flex),
        "div",
        Style::default(),
        Some("order:2;width:40px;height:20px"),
    );
    let d = doc.append_element(
        Some(flex),
        "div",
        Style::default(),
        Some("width:40px;height:20px"),
    );
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");

    layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("layout Ok");

    assert_eq!(doc.nodes[flex].children, vec![a, absolute, b, c, d]);
    assert_eq!(
        doc.nodes[flex].order_modified_children.as_ref(),
        &[b, absolute, d, a, c]
    );
    let x = |id: usize| doc.nodes[id].unrounded_layout.location.x;
    assert!((x(d) - x(b) - 40.0).abs() < 0.5);
    assert!((x(a) - x(d) - 40.0).abs() < 0.5);
    assert!((x(c) - x(a) - 40.0).abs() < 0.5);
}

#[test]
fn relayout_invalidates_cache_when_order_modified_view_returns_to_dom_order() {
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let flex = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:flex;width:60px;height:20px"),
    );
    let first = doc.append_element(
        Some(flex),
        "div",
        Style::default(),
        Some("order:1;width:30px;height:20px"),
    );
    let second = doc.append_element(
        Some(flex),
        "div",
        Style::default(),
        Some("order:0;width:30px;height:20px"),
    );

    let rules = build_rule_tree(&doc);
    let cascade_result = cascade(&doc, &rules).expect("initial cascade Ok");
    layout_single_page(&mut doc, &cascade_result, PageBox::A4, FontContext::new())
        .expect("initial layout Ok");
    assert_eq!(
        doc.nodes[flex].order_modified_children.as_ref(),
        &[second, first]
    );
    assert!(
        doc.nodes[second].unrounded_layout.location.x
            < doc.nodes[first].unrounded_layout.location.x
    );

    // Keep the same Document and page size, but remove the nonzero order.
    // The derived child view becomes empty, which must invalidate cached
    // flex positions rather than reusing the prior reversed placement.
    doc.set_element_inline_style(first, Some("order:0;width:30px;height:20px".into()));
    doc.set_element_inline_style(second, Some("order:0;width:30px;height:20px".into()));
    let rules = build_rule_tree(&doc);
    let updated_cascade = cascade(&doc, &rules).expect("updated cascade Ok");
    layout_single_page(&mut doc, &updated_cascade, PageBox::A4, FontContext::new())
        .expect("updated layout Ok");

    assert!(doc.nodes[flex].order_modified_children.is_empty());
    assert!(
        doc.nodes[first].unrounded_layout.location.x
            < doc.nodes[second].unrounded_layout.location.x
    );
}

#[test]
fn order_is_ignored_for_non_flex_grid_children() {
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let _head = doc.append_element(Some(html), "head", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let block = doc.append_element(Some(body), "div", Style::default(), None::<&str>);
    let a = doc.append_element(
        Some(block),
        "div",
        Style::default(),
        Some("order:2;height:20px"),
    );
    let b = doc.append_element(
        Some(block),
        "div",
        Style::default(),
        Some("order:-1;height:20px"),
    );
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");

    layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("layout Ok");

    assert!(doc.nodes[block].order_modified_children.is_empty());
    // cov:ignore: panic-message text is only executed when the assertion fails.
    assert!(
        doc.nodes[b].unrounded_layout.location.y > doc.nodes[a].unrounded_layout.location.y,
        "ordinary block children must retain source order despite CSS order"
    );
}

#[test]
fn nested_normal_flow_descendant_clears_ancestor_level_float() {
    // `LayoutBlockContainer::compute_block_child_layout`'s override in
    // `taffy_impl` only matters when a float and content that reacts to
    // it are at *different* nesting depths — direct siblings share one
    // `compute_block_layout` invocation (and hence one taffy
    // `BlockFormattingContext`) regardless of the override. This
    // fixture puts the float and a `clear:left` box two levels apart
    // (`float_sibling` is `container`'s child, `probe` is `container`'s
    // grandchild via the intervening `wrapper`), so the assertion only
    // holds if `wrapper`'s own recursive layout call continues
    // `container`'s `BlockFormattingContext` rather than starting a
    // fresh, float-blind one for `wrapper`'s own children (CSS2 §9.5.2
    // <https://www.w3.org/TR/CSS2/visuren.html#propdef-clear>: "clear"
    // requires the box's top border edge be below any earlier float in
    // the same block formatting context — not just floats that are its
    // own direct siblings).
    //
    // `wrapper` is left with no explicit width so it stretch-fits to
    // `container`'s full inner width, matching the BFC root's width —
    // avoiding a taffy `block_layout` limitation (a same-BFC child
    // narrower than its BFC root can get float insets computed against
    // the root's width instead of its own) that is orthogonal to what
    // this test pins.
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let _head = doc.append_element(Some(html), "head", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let container = doc.append_element(Some(body), "div", Style::default(), Some("width:200px"));
    let float_sibling = doc.append_element(
        Some(container),
        "div",
        Style::default(),
        Some("float:left;width:60px;height:40px"),
    );
    let wrapper = doc.append_element(Some(container), "div", Style::default(), None::<&str>);
    let probe = doc.append_element(
        Some(wrapper),
        "div",
        Style::default(),
        Some("clear:left;width:50px;height:10px"),
    );

    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");

    layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("layout Ok");

    let float_loc = doc.nodes[float_sibling].unrounded_layout.location;
    let float_size = doc.nodes[float_sibling].unrounded_layout.size;
    let wrapper_loc = doc.nodes[wrapper].unrounded_layout.location;
    let probe_loc = doc.nodes[probe].unrounded_layout.location;

    // `location` is parent-relative in taffy, so translate `probe`'s y
    // into `container`'s coordinate space by walking up one level.
    let probe_y_in_container = wrapper_loc.y + probe_loc.y;
    let float_bottom = float_loc.y + float_size.height;

    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert!(
        probe_y_in_container + 0.5 >= float_bottom,
        "clear:left box two BFC levels below the float must sit at or below the float's bottom edge, got float_bottom={float_bottom}, probe_y_in_container={probe_y_in_container}"
    );
}

#[test]
fn layout_single_page_deterministic_across_10_runs() {
    // 10 回連続実行で byte-identical であることを acceptance 条件とする。
    // 同一マシン上の determinism を check (cross-machine は将来 font
    // pinning に置き換わる)。
    use raikiri_traits::PageBox;

    fn one_run() -> Vec<taffy::Layout> {
        let (mut doc, cr) = hello_world_doc();
        layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("layout Ok");
        doc.nodes.iter().map(|n| n.unrounded_layout).collect()
    }

    let baseline = one_run();
    for i in 1..10 {
        let run = one_run();
        assert_eq!(
            baseline.len(),
            run.len(),
            "run {i}: layout node count changed"
        );
        for (j, (b, r)) in baseline.iter().zip(run.iter()).enumerate() {
            // taffy::Layout の全 field を byte-identical で比較。
            // 浮動小数点の subnormal / NaN drift があると here が最も先に
            // 反応する (design doc §12.8 の NonFiniteFloat 検討の check 相当)
            assert_eq!(
                b.size.width, r.size.width,
                "run {i} node {j}: size.width differs (baseline={} run={})",
                b.size.width, r.size.width
            );
            assert_eq!(b.size.height, r.size.height);
            assert_eq!(b.location.x, r.location.x);
            assert_eq!(b.location.y, r.location.y);
        }
    }
}

#[test]
fn grid_auto_placement_uses_order_modified_source_order() {
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let _head = doc.append_element(Some(html), "head", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let grid = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:grid;width:200px;grid-template-columns:100px 100px;grid-auto-rows:20px"),
    );
    let a = doc.append_element(Some(grid), "div", Style::default(), Some("order:2"));
    let b = doc.append_element(Some(grid), "div", Style::default(), Some("order:-1"));
    let c = doc.append_element(Some(grid), "div", Style::default(), Some("order:2"));
    let d = doc.append_element(Some(grid), "div", Style::default(), None::<&str>);
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");

    layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("layout Ok");

    assert_eq!(doc.nodes[grid].children, vec![a, b, c, d]);
    assert_eq!(
        doc.nodes[grid].order_modified_children.as_ref(),
        &[b, d, a, c]
    );
    let loc = |id: usize| doc.nodes[id].unrounded_layout.location;
    assert!((loc(d).x - loc(b).x - 100.0).abs() < 0.5);
    assert!((loc(a).y - loc(b).y - 20.0).abs() < 0.5);
    assert!((loc(c).x - loc(a).x - 100.0).abs() < 0.5);
    assert!((loc(c).y - loc(d).y - 20.0).abs() < 0.5);
}

#[test]
fn grid_template_columns_named_line_after_repeat_resolves_to_the_correct_taffy_grid_line() {
    // Regression test for the interleaving contract between
    // raikiri-style's `GridTrackList::line_names` (one entry per
    // `<line-names>?` production, i.e. `components.len() + 1` entries —
    // CSS Grid 1 §7.2.1 `<track-list>` grammar) and taffy's
    // `NamedLineResolver`, which advances an internal line counter once
    // per name-list entry and additionally steps across an unrolled
    // `repeat()`'s own tracks when it consumes one. A child placed with
    // `grid-column-start: z`, where `z` is named right after a
    // `repeat(2, [b] 50px)` block, must resolve to the grid line
    // following the 100px + 50px + 50px tracks that precede it — this
    // can only be told apart from an off-by-one in that bookkeeping by
    // checking where taffy's own grid algorithm actually places the
    // child, not by asserting on the bridged `taffy::Style` value.
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let _head = doc.append_element(Some(html), "head", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let grid_container = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:grid;grid-template-columns:[a] 100px repeat(2, [b] 50px) [z] 100px"),
    );
    let cell_first = doc.append_element(
        Some(grid_container),
        "div",
        Style::default(),
        Some("grid-column:1;grid-row:1;height:20px"),
    );
    let cell_z = doc.append_element(
        Some(grid_container),
        "div",
        Style::default(),
        Some("grid-column:z;grid-row:1;height:20px"),
    );
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");

    layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("layout Ok");

    let first_loc = doc.nodes[cell_first].unrounded_layout.location;
    let z_loc = doc.nodes[cell_z].unrounded_layout.location;
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert!(
        (z_loc.x - first_loc.x - 200.0).abs() < 0.5,
        "line `z` follows a 100px track and a repeat(2, 50px) block \
             (100px total), so it should sit 200px to the right of line 1, \
             got first.x={}, z.x={}",
        first_loc.x,
        z_loc.x
    );
}

#[test]
fn grid_auto_abspos_static_position_uses_all_parent_padding_sides() {
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let grid = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:grid;width:100px;height:100px;padding:10px"),
    );
    let child = doc.append_element(
        Some(grid),
        "div",
        Style::default(),
        Some("position:absolute;width:20px;height:20px"),
    );
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");
    layout_single_page(&mut doc, &cascade, PageBox::A4, FontContext::new()).expect("layout Ok");
    let layout = doc.nodes[child].unrounded_layout;
    assert!((layout.location.x - 10.0).abs() < 0.01);
    assert!((layout.location.y - 10.0).abs() < 0.01);
}

#[test]
fn layout_page_fragments_emits_one_page_with_deterministic_items() {
    let (mut doc, cascade) = hello_world_doc();
    let pages = layout_page_fragments(&mut doc, &cascade, PageBox::A4, FontContext::new())
        .expect("page fragment layout should succeed");

    assert_eq!(pages.len(), 1);
    assert_eq!(pages[0].page_index, 0);
    assert_eq!(pages[0].page_box, PageBox::A4);
    assert!(!pages[0].is_empty());
    assert!(
        pages[0]
            .items
            .windows(2)
            .all(|items| items[0].node_id <= items[1].node_id)
    );
    assert!(
        pages[0]
            .items
            .iter()
            .any(|item| item.kind == PageFragmentKind::Text)
    );
}

#[test]
fn layout_page_fragments_preserves_forced_page_break() {
    use raikiri_style::{build_rule_tree, cascade};

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let first = doc.append_element(Some(body), "div", Style::default(), Some("height:10px"));
    doc.append_text(first, "first");
    let second = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("break-before:page;height:10px"),
    );
    doc.append_text(second, "second");
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");
    let mut page = PageBox::new();
    page.width = 100.0;
    page.height = 60.0;

    let pages = layout_page_fragments(&mut doc, &cascade, page, FontContext::new())
        .expect("page fragment layout should succeed");
    assert!(pages.len() >= 2, "forced page break must create page 1+");
    assert!(
        pages[1]
            .items
            .iter()
            .any(|item| item.node_id.0 == second as u64)
    );
}

#[test]
fn page_fragment_projection_handles_empty_missing_body_and_invalid_slice_inputs() {
    use raikiri_style::{build_rule_tree, cascade};

    let document = Document::new();
    let rules = build_rule_tree(&document);
    let cascade_result = cascade(&document, &rules).expect("cascade Ok");
    assert!(page_fragments_from_slices(&document, &cascade_result, PageBox::A4, &[]).is_empty());

    let slice = PageSlice {
        page_index: 0,
        content_origin_y: 0.0,
        page_name: None,
    };
    let pages = page_fragments_from_slices(
        &document,
        &cascade_result,
        PageBox::A4,
        std::slice::from_ref(&slice),
    );
    assert_eq!(pages.len(), 1);
    assert!(pages[0].is_empty());

    let reversed_slices = [
        PageSlice {
            page_index: 1,
            content_origin_y: 100.0,
            page_name: None,
        },
        slice.clone(),
    ];
    let pages =
        page_fragments_from_slices(&document, &cascade_result, PageBox::A4, &reversed_slices);
    assert_eq!(
        pages.iter().map(|page| page.page_index).collect::<Vec<_>>(),
        vec![0, 1]
    );

    let mut document = Document::new();
    let html = document.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = document.append_element(Some(html), "body", Style::default(), None::<&str>);
    let comment = document.append_comment(Some(body), "comment");
    document.nodes[comment].set_in_document(true);
    document.nodes[body].children.push(usize::MAX);
    let rules = build_rule_tree(&document);
    let cascade = cascade(&document, &rules).expect("cascade Ok");
    let pages = page_fragments_from_slices(
        &document,
        &cascade,
        PageBox::A4,
        std::slice::from_ref(&slice),
    );
    assert_eq!(pages.len(), 1);
    assert!(
        pages[0]
            .items
            .iter()
            .all(|item| item.node_id.0 != comment as u64)
    );

    let invalid_slice = PageSlice {
        page_index: 0,
        content_origin_y: f32::NAN,
        page_name: None,
    };
    let pages = page_fragments_from_slices(
        &document,
        &cascade,
        PageBox::A4,
        std::slice::from_ref(&invalid_slice),
    );
    assert_eq!(pages.len(), 1);
    assert!(pages[0].is_empty());

    document.nodes[body].unrounded_layout.location.x = f32::NAN;
    document.nodes[body].unrounded_layout.size.width = f32::NAN;
    document.nodes[body].unrounded_layout.size.height = f32::NAN;
    let pages = page_fragments_from_slices(
        &document,
        &cascade,
        PageBox::A4,
        std::slice::from_ref(&slice),
    );
    assert!(pages[0].is_empty());
}

#[test]
fn layout_page_fragments_classifies_replaced_and_skips_non_rendered_nodes() {
    use raikiri_style::{build_rule_tree, cascade};

    let mut document = Document::new();
    let html = document.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = document.append_element(Some(html), "body", Style::default(), None::<&str>);
    let template = document.append_element(Some(body), "template", Style::default(), None::<&str>);
    document.append_text(template, "not rendered");
    document.append_comment(Some(body), "comment");
    document.append_element(Some(body), "div", Style::default(), None::<&str>);
    let image = document.append_element(
        Some(body),
        "img",
        Style::default(),
        Some("width:10px;height:10px"),
    );
    let rules = build_rule_tree(&document);
    let cascade = cascade(&document, &rules).expect("cascade Ok");

    let pages = layout_page_fragments(&mut document, &cascade, PageBox::A4, FontContext::new())
        .expect("page fragment layout should succeed");
    assert!(
        pages[0]
            .items
            .iter()
            .any(|item| item.node_id.0 == image as u64 && item.kind == PageFragmentKind::Replaced)
    );
    assert!(
        !pages[0]
            .items
            .iter()
            .any(|item| item.node_id.0 == template as u64)
    );
}

#[test]
fn page_fragment_geometry_table_groups_fragments_by_node_id() {
    let mut first_page = PageFragment {
        page_index: 0,
        ..Default::default()
    };
    first_page.items.push(PageFragmentItem::new(
        NodeId::new(9),
        PageFragmentRect::new(0.0, 0.0, 10.0, 4.0),
        PageFragmentKind::Box,
        0,
        2,
        false,
    ));
    let mut second_page = PageFragment {
        page_index: 1,
        ..Default::default()
    };
    second_page.items.push(PageFragmentItem::new(
        NodeId::new(9),
        PageFragmentRect::new(0.0, 0.0, 10.0, 4.0),
        PageFragmentKind::Box,
        1,
        2,
        false,
    ));
    let pages = vec![second_page, first_page];
    let table = page_fragment_geometry_table(&pages);
    let geometry = table.get(&NodeId::new(9)).expect("node geometry");
    assert_eq!(geometry.node_id, NodeId::new(9));
    assert!(geometry.is_split());
    assert_eq!(geometry.fragments.len(), 2);
    assert_eq!(geometry.fragments[0].page_index, 0);
    assert_eq!(geometry.fragments[1].page_index, 1);
    assert_eq!(geometry.fragments[1].fragment_index, 1);
}

#[test]
fn propagated_start_page_name_uses_resolved_grid_child_order() {
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(
        Some(html),
        "body",
        Style::default(),
        Some("display:grid;grid-template-columns:100px"),
    );
    doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:block;order:1;page:a;height:10px"),
    );
    doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:block;order:0;page:b;height:10px"),
    );
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");
    layout_single_page(&mut doc, &cascade, PageBox::A4, FontContext::new()).expect("layout Ok");
    assert_eq!(
        propagated_start_page_name(&doc, &cascade, body, None),
        (true, Some("b".to_owned()))
    );
}

#[test]
fn layout_pages_uses_order_modified_flex_sequence_for_forced_breaks() {
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let flex = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:flex;flex-direction:column;width:40px;height:20px"),
    );
    let break_later_in_visual_order = doc.append_element(
        Some(flex),
        "div",
        Style::default(),
        Some("order:1;break-before:page;height:10px"),
    );
    let first_in_visual_order = doc.append_element(
        Some(flex),
        "div",
        Style::default(),
        Some("order:0;height:10px"),
    );

    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");
    let mut page = PageBox::new();
    page.width = 100.0;
    page.height = 100.0;
    let slices = layout_pages(&mut doc, &cascade, page, FontContext::new()).expect("pagination Ok");

    assert!(slices.len() >= 2, "the forced break must create page 1+");
    let first_y = doc.nodes[first_in_visual_order].unrounded_layout.location.y;
    let later_y = doc.nodes[break_later_in_visual_order]
        .unrounded_layout
        .location
        .y;
    // cov:ignore: panic-message text is only executed when the assertion fails.
    assert!(
        first_y.abs() < 0.01,
        "the visual first item stays on page 0: y={first_y}"
    );
    // cov:ignore: panic-message text is only executed when the assertion fails.
    assert!(
        later_y > first_y + 50.0,
        "the forced-break item moves later: y={later_y}"
    );
    assert_eq!(
        doc.nodes[flex].layout_children(),
        &[first_in_visual_order, break_later_in_visual_order]
    );
}

#[test]
fn layout_pages_uses_order_modified_flex_last_item_for_overflow() {
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let flex = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:flex;flex-direction:column;width:40px"),
    );
    let visual_last = doc.append_element(
        Some(flex),
        "div",
        Style::default(),
        Some("order:1;height:70px"),
    );
    let visual_first = doc.append_element(
        Some(flex),
        "div",
        Style::default(),
        Some("order:0;height:40px"),
    );

    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");
    let mut page = PageBox::new();
    page.width = 100.0;
    page.height = 100.0;
    let _slices =
        layout_pages(&mut doc, &cascade, page, FontContext::new()).expect("pagination Ok");

    let first_y = doc.nodes[visual_first].unrounded_layout.location.y;
    let last_y = doc.nodes[visual_last].unrounded_layout.location.y;
    // cov:ignore: panic-message text is only executed when the assertion fails.
    assert!(
        first_y.abs() < 0.01,
        "the visual first item stays at y=0: {first_y}"
    );
    // cov:ignore: panic-message text is only executed when the assertion fails.
    assert!(
        last_y < 60.0,
        "the visual last item may continue across the page edge: y={last_y}"
    );
    assert_eq!(
        doc.nodes[flex].layout_children(),
        &[visual_first, visual_last]
    );
}

#[test]
fn layout_pages_uses_column_reverse_visual_order_for_forced_breaks() {
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let flex = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:flex;flex-direction:column-reverse;width:40px;height:20px"),
    );
    let visual_top = doc.append_element(
        Some(flex),
        "div",
        Style::default(),
        Some("order:1;height:10px"),
    );
    let visual_bottom_forced_break = doc.append_element(
        Some(flex),
        "div",
        Style::default(),
        Some("order:0;break-before:page;height:10px"),
    );

    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");
    let mut page = PageBox::new();
    page.width = 100.0;
    page.height = 100.0;
    let slices = layout_pages(&mut doc, &cascade, page, FontContext::new()).expect("pagination Ok");

    assert!(slices.len() >= 2, "the forced break must create page 1+");
    assert_eq!(
        doc.nodes[flex].layout_children(),
        &[visual_bottom_forced_break, visual_top]
    );
    let top_y = doc.nodes[visual_top].unrounded_layout.location.y;
    let bottom_y = doc.nodes[visual_bottom_forced_break]
        .unrounded_layout
        .location
        .y;
    // cov:ignore: panic-message text is only executed when the assertion fails.
    assert!(
        top_y.abs() < 0.01,
        "the visual top stays on page 0: {top_y}"
    );
    // cov:ignore: panic-message text is only executed when the assertion fails.
    assert!(
        bottom_y >= 100.0,
        "the lower item moves to page 1+: {bottom_y}"
    );
}

#[test]
fn layout_pages_keeps_column_reverse_visual_last_item_at_page_edge() {
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let flex = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:flex;flex-direction:column-reverse;width:40px"),
    );
    let visual_top = doc.append_element(
        Some(flex),
        "div",
        Style::default(),
        Some("order:1;height:40px"),
    );
    let visual_bottom = doc.append_element(
        Some(flex),
        "div",
        Style::default(),
        Some("order:0;height:70px"),
    );

    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");
    let mut page = PageBox::new();
    page.width = 100.0;
    page.height = 100.0;
    let _slices =
        layout_pages(&mut doc, &cascade, page, FontContext::new()).expect("pagination Ok");

    assert_eq!(
        doc.nodes[flex].layout_children(),
        &[visual_bottom, visual_top]
    );
    let top_y = doc.nodes[visual_top].unrounded_layout.location.y;
    let bottom_y = doc.nodes[visual_bottom].unrounded_layout.location.y;
    // cov:ignore: panic-message text is only executed when the assertion fails.
    assert!(
        top_y.abs() < 0.01,
        "visual first item starts at y=0: {top_y}"
    );
    // cov:ignore: panic-message text is only executed when the assertion fails.
    assert!(
        bottom_y < 60.0 && bottom_y + 70.0 > 100.0,
        "visual last item may continue across the page edge: y={bottom_y}"
    );
}

#[test]
fn layout_pages_orders_grid_rows_by_resolved_placement_before_forced_break() {
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let grid = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:grid;grid-template-columns:40px;grid-template-rows:60px 60px"),
    );
    let row_two = doc.append_element(
        Some(grid),
        "div",
        Style::default(),
        Some("grid-column:1;grid-row:2;order:0;break-before:page;height:60px"),
    );
    let _comment = doc.append_comment(Some(grid), "not a grid item");
    let row_one = doc.append_element(
        Some(grid),
        "div",
        Style::default(),
        Some("grid-column:1;grid-row:1;order:1;height:60px"),
    );

    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");
    let mut page = PageBox::new();
    page.width = 100.0;
    page.height = 100.0;
    let slices = layout_pages(&mut doc, &cascade, page, FontContext::new()).expect("pagination Ok");

    assert!(slices.len() >= 2, "the forced break must create page 1+");
    assert_eq!(
        doc.nodes[grid].layout_children(),
        &[row_two, _comment, row_one]
    );
    let row_one_y = doc.nodes[row_one].unrounded_layout.location.y;
    let row_two_y = doc.nodes[row_two].unrounded_layout.location.y;
    // cov:ignore: panic-message text is only executed when the assertion fails.
    assert!(
        row_one_y.abs() < 0.01,
        "resolved row 1 stays on page 0: {row_one_y}"
    );
    // cov:ignore: panic-message text is only executed when the assertion fails.
    assert!(
        row_two_y >= 100.0,
        "resolved row 2 moves after the break: {row_two_y}"
    );
}

#[test]
fn layout_pages_processes_single_column_grid_in_placed_row_order() {
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let grid = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:grid;grid-template-columns:40px;grid-template-rows:60px 60px"),
    );
    let first_row = doc.append_element(
        Some(grid),
        "div",
        Style::default(),
        Some("grid-column:1;grid-row:1;order:1;height:60px"),
    );
    let second_row = doc.append_element(
        Some(grid),
        "div",
        Style::default(),
        Some("grid-column:1;grid-row:2;order:0;height:60px"),
    );

    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");
    let mut page = PageBox::new();
    page.width = 100.0;
    page.height = 100.0;
    let slices = layout_pages(&mut doc, &cascade, page, FontContext::new()).expect("pagination Ok");

    // cov:ignore: panic-message text is only executed when the assertion fails.
    assert!(
        slices.len() >= 2,
        "the second grid row should continue to page 1+"
    );
    assert_eq!(doc.nodes[grid].layout_children(), &[second_row, first_row]);
    let first_y = doc.nodes[first_row].unrounded_layout.location.y;
    let second_y = doc.nodes[second_row].unrounded_layout.location.y;
    // cov:ignore: panic-message text is only executed when the assertion fails.
    assert!(
        first_y.abs() < 0.01,
        "explicit row 1 stays at y=0: {first_y}"
    );
    // cov:ignore: panic-message text is only executed when the assertion fails.
    assert!(
        second_y > first_y + 50.0,
        "explicit row 2 moves after row 1: y={second_y}"
    );
}

#[test]
fn layout_pages_coalesces_zero_height_named_boxes_at_same_position() {
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:block;page:first;height:0px"),
    );
    doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:block;page:last;height:0px"),
    );

    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");
    let slices =
        layout_pages(&mut doc, &cascade, PageBox::A4, FontContext::new()).expect("pagination Ok");

    assert_eq!(slices.len(), 1);
    assert_eq!(slices[0].page_name.as_deref(), Some("last"));
}

#[test]
fn layout_pages_processes_mixed_auto_and_explicit_grid_rows_by_resolved_placement() {
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let grid = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:grid;grid-template-columns:40px;grid-template-rows:60px 60px"),
    );
    let auto_row_one = doc.append_element(
        Some(grid),
        "div",
        Style::default(),
        Some("order:1;height:60px"),
    );
    let explicit_row_two = doc.append_element(
        Some(grid),
        "div",
        Style::default(),
        Some("grid-column:1;grid-row:2;order:0;height:60px"),
    );

    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");
    let mut page = PageBox::new();
    page.width = 100.0;
    page.height = 100.0;
    let slices = layout_pages(&mut doc, &cascade, page, FontContext::new()).expect("pagination Ok");

    // cov:ignore: panic-message text is only executed when the assertion fails.
    assert!(
        slices.len() >= 2,
        "the second grid row should continue to page 1+"
    );
    assert_eq!(
        doc.nodes[grid].layout_children(),
        &[explicit_row_two, auto_row_one]
    );
    let row_one_y = doc.nodes[auto_row_one].unrounded_layout.location.y;
    let row_two_y = doc.nodes[explicit_row_two].unrounded_layout.location.y;
    // cov:ignore: panic-message text is only executed when the assertion fails.
    assert!(
        row_one_y.abs() < 0.01,
        "auto row 1 stays at y=0: {row_one_y}"
    );
    // cov:ignore: panic-message text is only executed when the assertion fails.
    assert!(
        row_two_y > row_one_y + 50.0,
        "explicit row 2 follows row 1: {row_two_y}"
    );
}

#[test]
fn layout_pages_orders_direct_grid_text_before_later_forced_break() {
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let grid = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:grid;grid-template-columns:100px;grid-template-rows:60px 60px"),
    );
    let first_row_text = doc.append_text(grid, "first row text");
    let later_row_forced_break = doc.append_element(
        Some(grid),
        "div",
        Style::default(),
        Some("grid-column:1;grid-row:2;order:-1;break-before:page;height:60px"),
    );

    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");
    let mut page = PageBox::new();
    page.width = 100.0;
    page.height = 100.0;
    let slices = layout_pages(&mut doc, &cascade, page, FontContext::new()).expect("pagination Ok");

    assert!(slices.len() >= 2, "the forced break must create page 1+");
    assert_eq!(
        doc.nodes[grid].layout_children(),
        &[later_row_forced_break, first_row_text]
    );
    let text_y = doc.nodes[first_row_text].unrounded_layout.location.y;
    let later_y = doc.nodes[later_row_forced_break]
        .unrounded_layout
        .location
        .y;
    assert!(text_y.abs() < 0.01, "row 1 text stays on page 0: {text_y}");
    // cov:ignore: panic-message text is only executed when the assertion fails.
    assert!(
        later_y >= 100.0,
        "row 2 moves after the forced break: {later_y}"
    );
}

#[test]
fn layout_pages_orders_negative_explicit_grid_rows_by_placement() {
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let grid = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:grid;grid-template-columns:40px;grid-template-rows:60px 60px"),
    );
    let first_row = doc.append_element(
        Some(grid),
        "div",
        Style::default(),
        Some("grid-column:1;grid-row:-3;order:1;height:60px"),
    );
    let second_row = doc.append_element(
        Some(grid),
        "div",
        Style::default(),
        Some("grid-column:1;grid-row:-2;order:0;height:60px"),
    );

    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");
    let mut page = PageBox::new();
    page.width = 100.0;
    page.height = 100.0;
    let slices = layout_pages(&mut doc, &cascade, page, FontContext::new()).expect("pagination Ok");

    // cov:ignore: panic-message text is only executed when the assertion fails.
    assert!(
        slices.len() >= 2,
        "the second grid row should continue to page 1+"
    );
    assert_eq!(doc.nodes[grid].layout_children(), &[second_row, first_row]);
    let first_y = doc.nodes[first_row].unrounded_layout.location.y;
    let second_y = doc.nodes[second_row].unrounded_layout.location.y;
    // cov:ignore: panic-message text is only executed when the assertion fails.
    assert!(
        first_y.abs() < 0.01,
        "negative row line -3 is first: {first_y}"
    );
    // cov:ignore: panic-message text is only executed when the assertion fails.
    assert!(
        second_y > first_y + 50.0,
        "negative row line -2 follows: {second_y}"
    );
}

#[test]
fn layout_pages_uses_resolved_rows_for_reversed_grid_lines_and_order() {
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let grid = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:grid;grid-template-columns:40px;grid-template-rows:60px 60px"),
    );
    let reversed_row_lines = doc.append_element(
        Some(grid),
        "div",
        Style::default(),
        Some("grid-column:1;grid-row:2 / 1;height:60px"),
    );
    let later_row_forced_break = doc.append_element(
        Some(grid),
        "div",
        Style::default(),
        Some("grid-column:1;grid-row:2 / 3;order:-1;break-before:page;height:60px"),
    );

    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");
    let mut page = PageBox::new();
    page.width = 100.0;
    page.height = 100.0;
    let slices = layout_pages(&mut doc, &cascade, page, FontContext::new()).expect("pagination Ok");

    assert!(slices.len() >= 2, "the forced break must create page 1+");
    assert_eq!(
        doc.nodes[grid].layout_children(),
        &[later_row_forced_break, reversed_row_lines]
    );
    let first_row_y = doc.nodes[reversed_row_lines].unrounded_layout.location.y;
    let second_row_y = doc.nodes[later_row_forced_break]
        .unrounded_layout
        .location
        .y;
    // cov:ignore: panic-message text is only executed when the assertion fails.
    assert!(
        first_row_y.abs() < 0.01,
        "reversed lines resolve to row 1 and stay before the forced break: {first_row_y}"
    );
    // cov:ignore: panic-message text is only executed when the assertion fails.
    assert!(
        second_row_y >= 100.0,
        "row 2's break-before moves only that item to page 1+: {second_row_y}"
    );
}

#[test]
fn layout_pages_orders_relative_grid_items_by_resolved_placement() {
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let grid = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:grid;grid-template-columns:40px;grid-template-rows:20px 20px"),
    );
    let relative_first_row = doc.append_element(
        Some(grid),
        "div",
        Style::default(),
        Some("grid-column:1;grid-row:1;position:relative;top:60px;height:20px"),
    );
    let page_break_after_second_row = doc.append_element(
        Some(grid),
        "div",
        Style::default(),
        Some("grid-column:1;grid-row:2;break-after:page;height:20px"),
    );
    let following = doc.append_element(Some(body), "div", Style::default(), Some("height:10px"));

    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");
    let mut page = PageBox::new();
    page.width = 100.0;
    page.height = 100.0;
    let slices = layout_pages(&mut doc, &cascade, page, FontContext::new()).expect("pagination Ok");

    assert!(slices.len() >= 2, "the forced break must create page 1+");
    let first_row_y = doc.nodes[relative_first_row].unrounded_layout.location.y;
    let second_row_y = doc.nodes[page_break_after_second_row]
        .unrounded_layout
        .location
        .y;
    let following_y = doc.nodes[following].unrounded_layout.location.y;
    // cov:ignore: panic-message text is only executed when the assertion fails.
    assert!(
        (first_row_y - 60.0).abs() < 0.01,
        "row 1 keeps its relative visual offset without being page-shifted: y={first_row_y}"
    );
    // cov:ignore: panic-message text is only executed when the assertion fails.
    assert!(
        (second_row_y - 20.0).abs() < 0.01,
        "row 2 remains in its placed row: y={second_row_y}"
    );
    assert!(following_y > second_row_y + 50.0);
}

#[test]
fn layout_pages_places_footnote_at_page_bottom_without_flow_footprint() {
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let lead = doc.append_element(Some(body), "div", Style::default(), Some("height:20px"));
    let note = doc.append_element(
        Some(body),
        "aside",
        Style::default(),
        Some("float:footnote;height:10px;width:50px"),
    );
    doc.append_text(note, "note");
    let following = doc.append_element(Some(body), "div", Style::default(), Some("height:10px"));

    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");
    assert!(matches!(cascade.computed[note].float, FloatValue::Footnote));
    let mut page = PageBox::new();
    page.width = 100.0;
    page.height = 50.0;
    let slices = layout_pages(&mut doc, &cascade, page, FontContext::new()).expect("pagination Ok");

    assert_eq!(slices.len(), 1);
    let note_y = doc.nodes[note].unrounded_layout.location.y;
    let following_y = doc.nodes[following].unrounded_layout.location.y;
    assert!((note_y - 40.0).abs() < 0.01, "note y={note_y}");
    assert!(
        following_y < note_y,
        "following flow content should not be pushed below the footnote: following y={following_y}, note y={note_y}"
    );
    assert!(doc.nodes[lead].unrounded_layout.size.height > 0.0);
}

#[test]
fn layout_single_page_resolves_direct_absolute_auto_width_with_margin() {
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let abs = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("position:absolute; margin-right:20px; border:10px solid black"),
    );
    let _auto_margin_abs = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("position:absolute; margin-left:auto; margin-right:auto"),
    );
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");

    let mut page = PageBox::new();
    page.width = 100.0;
    page.height = 100.0;
    layout_single_page(&mut doc, &cascade, page, parley::FontContext::new()).expect("layout Ok");

    // Content-box width = 100 - 20 (margin) - 20 (horizontal border),
    // while the resulting border box is 80px wide.
    let layout = doc.nodes[abs].unrounded_layout;
    assert!((layout.size.width - 80.0).abs() < 0.001);
}

mod page_fragments_tests;
