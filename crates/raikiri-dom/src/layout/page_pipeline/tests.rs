use super::*;
use taffy::Style;

// ── layout_single_page driver (Task 7) ──────────────────────

fn hello_world_doc() -> (Document, raikiri_style::CascadeResult) {
    // <html><head></head><body><p style="color:red">Hi</p></body></html>
    // Equivalent to parsing (built manually instead; future umbrella integration handles raikiri-html).
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
fn text_indent_calc_ch_adds_absolute_offset_to_measured_indent() {
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    fn prepared_indent(indent: &str) -> f32 {
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
        let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
        let style = format!("display:block;width:80px;font-size:16px;text-indent:{indent}");
        let p = doc.append_element(Some(body), "p", Style::default(), Some(style.as_str()));
        let text = doc.append_text(p, "00 00");
        let rules = build_rule_tree(&doc);
        let cascade = cascade(&doc, &rules).expect("cascade Ok");
        layout_single_page(&mut doc, &cascade, PageBox::A4, FontContext::new()).expect("layout Ok");
        match &doc.nodes[text].data {
            crate::node::NodeData::Text(text) => text.text_indent_px.expect("indent prepared"),
            _ => unreachable!("appended node is text"),
        }
    }

    let plain = prepared_indent("1ch");
    let calc = prepared_indent("calc(1ch + 7px)");
    assert!(
        (calc - plain - 7.0).abs() < 0.01,
        "plain ch={plain}, calc={calc}"
    );
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

/// Helper returning both glyph ends of a single-line block.
/// (first glyph x, last glyph x+advance).
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
    // text-align:justify + text-justify:none → do not spread.
    // (CSS Text 3 §6.2).
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

    // Attach <p> directly (fragment equivalent).
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
    // Check that the display bridge is active via layout_single_page:
    // assigning display:none to body must set taffy::Style.display
    // to Display::None.
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
    // An inline-block with width:auto shrinks to its content rather than
    // filling the containing block (shrink-to-fit). Check its geometry against
    // a plain block child that fills the available width:
    // an inline-block containing a 100px child is 100px wide, while an
    // inline-block with explicit width:300px stays 300px wide (neither fills the body).
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
    // Require byte-identical results across ten consecutive runs.
    // Check determinism on the same machine (cross-machine consistency needs
    // future font pinning).
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
            // Compare every taffy::Layout field byte for byte.
            // This catches floating-point subnormal / NaN drift first
            // (equivalent to the design doc §12.8 NonFiniteFloat check).
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
fn direct_absolute_auto_width_shrink_to_fit_empty_with_margin() {
    // CSS 2.1 §10.3.7: an absolutely positioned box with `width:auto` and
    // `left:auto` / `right:auto` shrink-wraps its content instead of filling
    // the containing block. An empty box with 10px borders on each side has
    // zero content width, so its border box is 20px wide regardless of the
    // 20px right margin or the 100px containing width.
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

    // Empty content shrink-wraps to zero; the border box holds only borders.
    let layout = doc.nodes[abs].unrounded_layout;
    assert!((layout.size.width - 20.0).abs() < 0.001);
}

#[test]
fn direct_absolute_auto_width_shrink_to_fit_block_child() {
    // CSS 2.1 §10.3.7 shrink-to-fit with a definite-content child: a
    // direct-body absolute box containing a 100px block must be 100px wide
    // in an 800px containing block, not stretched to the viewport width.
    // This is the WPT line-break-anywhere green-square case (70k-pixel
    // mismatch when the old fill override applied).
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let abs_pos = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("position:absolute; height:100px"),
    );
    let _child = doc.append_element(
        Some(abs_pos),
        "div",
        Style::default(),
        Some("width:100px;height:20px"),
    );
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");

    let mut page = PageBox::new();
    page.width = 800.0;
    page.height = 600.0;
    layout_single_page(&mut doc, &cascade, page, parley::FontContext::new()).expect("layout Ok");

    let layout = doc.nodes[abs_pos].unrounded_layout;
    assert!(
        (layout.size.width - 100.0).abs() < 0.5,
        "width:auto absolute must shrink-wrap its 100px child, got width={}",
        layout.size.width
    );
}

#[test]
fn nested_absolute_auto_width_matches_direct_body_shrink() {
    // The same 100px-child shrink must hold inside a relative wrapper: the
    // nested path always used taffy directly, so direct-body must match it.
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let outer = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("position:relative"),
    );
    let abs_pos = doc.append_element(
        Some(outer),
        "div",
        Style::default(),
        Some("position:absolute; height:100px"),
    );
    let _child = doc.append_element(
        Some(abs_pos),
        "div",
        Style::default(),
        Some("width:100px;height:20px"),
    );
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");

    let mut page = PageBox::new();
    page.width = 800.0;
    page.height = 600.0;
    layout_single_page(&mut doc, &cascade, page, parley::FontContext::new()).expect("layout Ok");

    let layout = doc.nodes[abs_pos].unrounded_layout;
    assert!(
        (layout.size.width - 100.0).abs() < 0.5,
        "nested width:auto absolute must shrink-wrap its 100px child, got width={}",
        layout.size.width
    );
}

mod page_fragments_tests;

// ── ifc roots stay off the parley passes ─────────────────────

#[test]
fn ifc_root_text_is_not_shaped_by_parley_and_the_root_is_not_a_flex_line() {
    use crate::layout::ifc::test_support::ahem_fonts;
    use crate::layout::test_support::{ahem_font_context, ahem_paragraph, page_box_800x600};
    let (mut doc, cascade, root) = ahem_paragraph("aaaa bbbb cccc", "width:50px");
    doc.enable_inline_formatting(ahem_fonts(), shodo::limits::Limits::default());
    layout_single_page(&mut doc, &cascade, page_box_800x600(), ahem_font_context())
        .expect("layout");
    let text = doc.nodes[root].children[0];
    assert!(doc.nodes[root].flags.contains(NodeFlags::IS_IFC_ROOT));
    assert!(!doc.nodes[root].flags.contains(NodeFlags::IS_INLINE_ROOT));
    // Parley cannot satisfy this: the text node was never shaped.
    assert!(doc.nodes[text].text_layout().is_none());
}

#[test]
fn without_the_switch_the_same_paragraph_is_shaped_by_parley() {
    use crate::layout::test_support::{ahem_font_context, ahem_paragraph, page_box_800x600};
    let (mut doc, cascade, root) = ahem_paragraph("aaaa bbbb cccc", "width:50px");
    layout_single_page(&mut doc, &cascade, page_box_800x600(), ahem_font_context())
        .expect("layout");
    let text = doc.nodes[root].children[0];
    assert!(!doc.nodes[root].flags.contains(NodeFlags::IS_IFC_ROOT));
    assert!(doc.nodes[text].text_layout().is_some());
}

// ── ifc roots in taffy ───────────────────────────────────────

use crate::layout::test_support::{
    ahem_font_context, ahem_paragraph, ahem_paragraph_beside_float, ahem_paragraph_beside_floats,
    ahem_paragraph_in_block_wrapper, ahem_paragraph_with_atomic, ahem_paragraph_with_float,
    ifc_ahem_fonts, line_start_x, line_text, page_box_800x600,
};

fn lay_out_with_switch(doc: &mut Document, cascade: &CascadeResult) {
    doc.enable_inline_formatting(ifc_ahem_fonts(), shodo::limits::Limits::default());
    layout_single_page(doc, cascade, page_box_800x600(), ahem_font_context()).expect("layout");
}

fn stored_lines(doc: &Document, root: usize) -> &crate::layout::ifc::root::IfcLines {
    doc.nodes[root]
        .ifc
        .as_ref()
        .and_then(|root| root.lines.as_ref())
        .expect("the root holds performed lines")
}

#[test]
fn ifc_root_height_is_lines_times_line_height() {
    let (mut doc, cascade, root) = ahem_paragraph("aaaa bbbb cccc", "width:50px");
    lay_out_with_switch(&mut doc, &cascade);
    let layout = doc.nodes[root].unrounded_layout;
    assert_eq!(layout.size.width, 50.0);
    // Three 10px lines, and the text node was never shaped by parley.
    assert_eq!(layout.size.height, 30.0);
    assert!(
        doc.nodes[doc.nodes[root].children[0]]
            .text_layout()
            .is_none()
    );
    assert_eq!(stored_lines(&doc, root).lines.len(), 3);
}

#[test]
fn padding_and_border_shrink_the_line_width() {
    let (mut doc, cascade, root) = ahem_paragraph(
        "aaaa bbbb",
        "box-sizing:border-box;width:70px;padding:0 10px;border:0 solid red;border-width:0 5px",
    );
    lay_out_with_switch(&mut doc, &cascade);
    // Content box: 70 - 2*10 - 2*5 = 40px, so "aaaa" and "bbbb" stack.
    let lines = stored_lines(&doc, root);
    assert_eq!(lines.width, 40.0);
    assert_eq!(lines.lines.len(), 2);
}

#[test]
fn an_explicit_width_root_breaks_at_the_box_width() {
    // The width is stretched or explicit, so it must not be clamped to the
    // max-content width of the text (90px).
    let (mut doc, cascade, root) = ahem_paragraph("aaaa bbbb", "width:200px");
    lay_out_with_switch(&mut doc, &cascade);
    assert_eq!(stored_lines(&doc, root).width, 200.0);
    assert_eq!(doc.nodes[root].unrounded_layout.size.width, 200.0);
}

#[test]
fn a_floated_root_shrinks_to_its_max_content() {
    // A `width:auto` float is laid out with no known width and a definite
    // available width: the shrink-to-fit branch.
    let (mut doc, cascade, root) = ahem_paragraph("aaaa bbbb", "float:left");
    lay_out_with_switch(&mut doc, &cascade);
    assert_eq!(doc.nodes[root].unrounded_layout.size.width, 90.0);
    assert_eq!(stored_lines(&doc, root).lines.len(), 1);
}

#[test]
fn an_inline_block_shrinks_to_the_max_content_of_its_ifc_root() {
    // inline-block -> block wrapper -> root: the inline-block is measured at
    // max-content, which reaches the root as `AvailableSpace::MaxContent`.
    let (mut doc, cascade, wrapper, root) =
        ahem_paragraph_in_block_wrapper("aaaa bbbb", "display:inline-block");
    lay_out_with_switch(&mut doc, &cascade);
    assert_eq!(doc.nodes[wrapper].unrounded_layout.size.width, 90.0);
    assert_eq!(doc.nodes[root].unrounded_layout.size.width, 90.0);
}

#[test]
fn the_last_performed_layout_decides_the_stored_lines() {
    // The inline-block probes its content at several widths before the final
    // pass; whichever pass ran last must be the one the root keeps.
    let (mut doc, cascade, _wrapper, root) =
        ahem_paragraph_in_block_wrapper("aaaa bbbb cccc", "display:inline-block;max-width:50px");
    lay_out_with_switch(&mut doc, &cascade);
    assert_eq!(
        stored_lines(&doc, root).width,
        doc.nodes[root].unrounded_layout.size.width
    );
}

#[test]
fn lines_survive_a_second_layout_pass() {
    let (mut doc, cascade, root) = ahem_paragraph("aaaa bbbb cccc", "width:50px");
    lay_out_with_switch(&mut doc, &cascade);
    // Same cascade: no generation change, so only the switch's own cache drop
    // keeps the measure callback running.
    layout_single_page(&mut doc, &cascade, page_box_800x600(), ahem_font_context())
        .expect("second layout");
    assert_eq!(stored_lines(&doc, root).lines.len(), 3);
}

#[test]
fn first_baseline_includes_the_top_padding_and_border() {
    use taffy::{
        AvailableSpace, LayoutInput, LayoutPartialTree, Line, NodeId, RequestedAxis, RunMode, Size,
        SizingMode,
    };
    let (mut doc, cascade, root) = ahem_paragraph(
        "aa",
        "width:100px;padding-top:6px;border-top-width:3px;border-top-style:solid",
    );
    lay_out_with_switch(&mut doc, &cascade);
    // Ask the root for its layout output again: the baselines are part of it.
    let output = doc.compute_child_layout(
        NodeId::from(root),
        LayoutInput {
            run_mode: RunMode::PerformLayout,
            sizing_mode: SizingMode::InherentSize,
            axis: RequestedAxis::Both,
            known_dimensions: Size {
                width: None,
                height: None,
            },
            known_dimensions_are_definite: Size {
                width: true,
                height: true,
            },
            parent_size: Size {
                width: Some(800.0),
                height: Some(600.0),
            },
            available_space: Size {
                width: AvailableSpace::Definite(800.0),
                height: AvailableSpace::MaxContent,
            },
            vertical_margins_are_collapsible: Line::FALSE,
        },
    );
    // Ahem ascent 8px + 6px padding + 3px border, from the border-box top.
    assert_eq!(output.baselines.first, Some(17.0));
}

#[test]
fn calc_min_width_is_resolved_on_an_ifc_root() {
    // The bridge passes `calc()` through for sizes and min/max sizes, not for
    // padding. A floated root shrinks to 90px, so the calc minimum decides.
    let (mut doc, cascade, root) =
        ahem_paragraph("aaaa bbbb", "float:left;min-width:calc(20% + 10px)");
    lay_out_with_switch(&mut doc, &cascade);
    // 20% of the 800px page plus 10px.
    assert_eq!(doc.nodes[root].unrounded_layout.size.width, 170.0);
    // The stored lines must be broken at the final content width too.
    assert_eq!(stored_lines(&doc, root).width, 170.0);
}

#[test]
fn a_floated_root_with_a_narrow_max_width_breaks_at_that_width() {
    // max-width is below the longest word, so the box shrinks to 40px and the
    // lines must be broken there: three lines, not the two that min-content
    // (60px) would give.
    let (mut doc, cascade, root) = ahem_paragraph("aaaaaa bb cc", "float:left;max-width:40px");
    lay_out_with_switch(&mut doc, &cascade);
    assert_eq!(doc.nodes[root].unrounded_layout.size.width, 40.0);
    assert_eq!(stored_lines(&doc, root).width, 40.0);
    assert_eq!(doc.nodes[root].unrounded_layout.size.height, 30.0);
}

#[test]
fn an_inline_block_baseline_follows_the_last_ifc_line() {
    use taffy::{
        AvailableSpace, LayoutInput, LayoutPartialTree, Line, NodeId, RequestedAxis, RunMode, Size,
        SizingMode,
    };
    let (mut doc, cascade, wrapper, _root) =
        ahem_paragraph_in_block_wrapper("aaaa bbbb cccc", "display:inline-block;width:50px");
    lay_out_with_switch(&mut doc, &cascade);
    let output = doc.compute_child_layout(
        NodeId::from(wrapper),
        LayoutInput {
            run_mode: RunMode::PerformLayout,
            sizing_mode: SizingMode::InherentSize,
            axis: RequestedAxis::Both,
            known_dimensions: Size::NONE,
            known_dimensions_are_definite: Size {
                width: true,
                height: true,
            },
            parent_size: Size {
                width: Some(800.0),
                height: Some(600.0),
            },
            available_space: Size {
                width: AvailableSpace::Definite(800.0),
                height: AvailableSpace::MaxContent,
            },
            vertical_margins_are_collapsible: Line::FALSE,
        },
    );
    // Three 10px lines: the last baseline is 20px + the 8px Ahem ascent.
    assert_eq!(output.baselines.first, Some(28.0));
}

// ── relayout for a page-specific width ───────────────────────

#[test]
fn relayout_keeps_an_authored_width_root_at_its_width() {
    use crate::layout::relayout_text_for_width;
    let (mut doc, cascade, root) = ahem_paragraph("aaaa bbbb cccc", "width:200px");
    lay_out_with_switch(&mut doc, &cascade);
    assert_eq!(stored_lines(&doc, root).lines.len(), 1);
    let height_before = doc.nodes[root].unrounded_layout.size.height;
    // parley keeps the authored 200px whatever the page width is.
    relayout_text_for_width(&mut doc, &cascade, 50.0, 50.0, ahem_font_context());
    assert_eq!(stored_lines(&doc, root).lines.len(), 1);
    relayout_text_for_width(&mut doc, &cascade, 800.0, 800.0, ahem_font_context());
    assert_eq!(stored_lines(&doc, root).lines.len(), 1);
    assert_eq!(doc.nodes[root].unrounded_layout.size.height, height_before);
}

#[test]
fn relayout_follows_the_page_width_for_an_auto_width_root() {
    use crate::layout::relayout_text_for_width;
    let (mut doc, cascade, root) = ahem_paragraph("aaaa bbbb cccc", "");
    lay_out_with_switch(&mut doc, &cascade);
    assert_eq!(stored_lines(&doc, root).lines.len(), 1);
    let height_before = doc.nodes[root].unrounded_layout.size.height;
    // No authored width anywhere above: parley re-shapes at `max_advance`,
    // narrower or wider than the width laid out at.
    relayout_text_for_width(&mut doc, &cascade, 50.0, 50.0, ahem_font_context());
    assert_eq!(stored_lines(&doc, root).lines.len(), 3);
    relayout_text_for_width(&mut doc, &cascade, 800.0, 800.0, ahem_font_context());
    assert_eq!(stored_lines(&doc, root).lines.len(), 1);
    // Only the lines change; the taffy box is left as it was.
    assert_eq!(doc.nodes[root].unrounded_layout.size.height, height_before);
}

#[test]
fn relayout_matches_the_parley_line_count() {
    use crate::layout::relayout_text_for_width;
    for (css, max_advance) in [("", 50.0_f32), ("width:200px", 50.0), ("", 800.0)] {
        let (mut off_doc, cascade, off_root) = ahem_paragraph("aaaa bbbb cccc", css);
        layout_single_page(
            &mut off_doc,
            &cascade,
            page_box_800x600(),
            ahem_font_context(),
        )
        .expect("layout");
        relayout_text_for_width(
            &mut off_doc,
            &cascade,
            max_advance,
            max_advance,
            ahem_font_context(),
        );
        let text = off_doc.nodes[off_root].children[0];
        let parley_lines = off_doc.nodes[text].text_layout().map(|layout| layout.len());

        let (mut on_doc, cascade, on_root) = ahem_paragraph("aaaa bbbb cccc", css);
        lay_out_with_switch(&mut on_doc, &cascade);
        relayout_text_for_width(
            &mut on_doc,
            &cascade,
            max_advance,
            max_advance,
            ahem_font_context(),
        );
        assert_eq!(
            Some(stored_lines(&on_doc, on_root).lines.len()),
            parley_lines,
            "{css} / {max_advance}"
        );
    }
}

// ── ifc roots beside floats ──────────────────────────────────

#[test]
fn a_paragraph_beside_a_left_float_starts_after_it() {
    let (mut doc, cascade, _float, root) =
        ahem_paragraph_beside_float("aaaa bbbb cccc", "float:left;width:30px;height:20px", "");
    lay_out_with_switch(&mut doc, &cascade);
    assert!(
        doc.nodes[root].is_ifc_root(),
        "the paragraph beside a float is an ifc root"
    );
    let lines = &stored_lines(&doc, root).lines;
    // Two lines fit beside the 20px float (70px wide), the third has the width.
    assert_eq!(
        lines.iter().map(line_text).collect::<Vec<_>>(),
        ["aaaa", "bbbb", "cccc"]
    );
    assert_eq!(
        lines.iter().map(line_start_x).collect::<Vec<_>>(),
        [Some(30.0), Some(30.0), Some(0.0)]
    );
    assert_eq!(doc.nodes[root].unrounded_layout.size.height, 30.0);
    assert!(stored_lines(&doc, root).beside_floats);
}

#[test]
fn a_paragraph_beside_a_right_float_keeps_its_start_and_loses_width() {
    let (mut doc, cascade, _float, root) =
        ahem_paragraph_beside_float("aaaa bbbb cccc", "float:right;width:30px;height:20px", "");
    lay_out_with_switch(&mut doc, &cascade);
    let lines = &stored_lines(&doc, root).lines;
    assert_eq!(
        lines.iter().map(line_text).collect::<Vec<_>>(),
        ["aaaa", "bbbb", "cccc"]
    );
    assert_eq!(
        lines.iter().map(line_start_x).collect::<Vec<_>>(),
        [Some(0.0); 3]
    );
}

#[test]
fn a_line_that_reaches_a_later_float_segment_is_narrowed_by_it() {
    // Two floats: a 30x10 one on the left at the top, then a 30x30 one on the
    // right that clears it, so it starts at y=10. The root has a 20px line
    // height. The first attempt at line 1 (assumed height 0) sees only the
    // left float: "aaa bb" (60px) fits in 70px. Its real height, 20px, reaches
    // into the right float's segment (y=10..40), so the line has to be laid
    // out again in 100 - 30 - 30 = 40px, where only "aaa" fits. A loop that
    // reads one segment, or never re-checks the space for the real height,
    // keeps "aaa bb".
    let (mut doc, cascade, _floats, root) = ahem_paragraph_beside_floats(
        "aaa bb cc",
        &[
            "float:left;width:30px;height:10px",
            "float:right;clear:left;width:30px;height:30px",
        ],
        "line-height:20px",
    );
    lay_out_with_switch(&mut doc, &cascade);
    let lines = &stored_lines(&doc, root).lines;
    assert_eq!(
        lines.iter().map(line_text).collect::<Vec<_>>(),
        ["aaa", "bb cc"]
    );
    // Line 2 (y=20) is inside the right float's segment only: 70px wide from 0.
    assert_eq!(
        lines.iter().map(line_start_x).collect::<Vec<_>>(),
        [Some(30.0), Some(0.0)]
    );
}

#[test]
fn relayout_keeps_the_lines_of_a_paragraph_beside_a_float() {
    use crate::layout::relayout_text_for_width;
    let (mut doc, cascade, _float, root) =
        ahem_paragraph_beside_float("aaaa bbbb cccc", "float:left;width:30px;height:20px", "");
    lay_out_with_switch(&mut doc, &cascade);
    let before: Vec<_> = stored_lines(&doc, root)
        .lines
        .iter()
        .map(line_text)
        .collect();
    assert_eq!(before, ["aaaa", "bbbb", "cccc"]);
    relayout_text_for_width(&mut doc, &cascade, 800.0, 800.0, ahem_font_context());
    // The shared float context is gone at this point; re-breaking at the full
    // width would ignore the float the lines were laid out beside.
    let after: Vec<_> = stored_lines(&doc, root)
        .lines
        .iter()
        .map(line_text)
        .collect();
    assert_eq!(after, before);
}

#[test]
fn a_paragraph_with_padding_beside_a_float_is_offset_inside_its_content_box() {
    let (mut doc, cascade, _float, root) = ahem_paragraph_beside_float(
        "aaaa bbbb",
        "float:left;width:30px;height:20px",
        "padding:0 10px;box-sizing:border-box",
    );
    lay_out_with_switch(&mut doc, &cascade);
    let lines = &stored_lines(&doc, root).lines;
    // The content box is 80px wide and starts 10px in; the float covers the
    // first 30px of the context, so 20px of the content box remain covered.
    assert_eq!(line_start_x(&lines[0]), Some(20.0));
}

#[test]
fn a_paragraph_with_top_padding_asks_for_space_below_its_border_top() {
    // The float is 20px tall and starts at the top of the wrapper. A root with
    // 15px of top padding has its content box at y=15, so the first line
    // (y=15..25) still overlaps the float (until y=20) and starts after it,
    // while the second line (y=25) is below it.
    let (mut doc, cascade, _float, root) = ahem_paragraph_beside_float(
        "aaaa bbbb",
        "float:left;width:30px;height:20px",
        "padding-top:15px",
    );
    lay_out_with_switch(&mut doc, &cascade);
    let lines = &stored_lines(&doc, root).lines;
    assert_eq!(
        lines.iter().map(line_start_x).collect::<Vec<_>>(),
        [Some(30.0), Some(0.0)]
    );
}

// ── floats inside ifc roots ──────────────────────────────────

#[test]
fn a_float_inside_the_paragraph_shortens_the_line_it_is_anchored_in() {
    let (mut doc, cascade, float, root) = ahem_paragraph_with_float(
        "aa",
        "float:left;width:30px;height:20px",
        " bbbb cccc dddd",
        "",
    );
    lay_out_with_switch(&mut doc, &cascade);
    assert!(
        doc.nodes[root].is_ifc_root(),
        "a paragraph with a float child is an ifc root"
    );
    let lines = &stored_lines(&doc, root).lines;
    // Line 1 holds "aa" and the anchor and is 70px wide beside the float:
    // "aa bbbb" (70px) fits exactly. Line 2 is still beside the float.
    // Line 3 (y=20) is below it.
    assert_eq!(
        lines.iter().map(line_text).collect::<Vec<_>>(),
        ["aa bbbb", "cccc", "dddd"]
    );
    assert_eq!(
        lines.iter().map(line_start_x).collect::<Vec<_>>(),
        [Some(30.0), Some(30.0), Some(0.0)]
    );
    // The float sits at the top left of the content box.
    let layout = doc.nodes[float].unrounded_layout;
    assert_eq!((layout.location.x, layout.location.y), (0.0, 0.0));
    assert_eq!((layout.size.width, layout.size.height), (30.0, 20.0));
}

#[test]
fn a_right_float_inside_the_paragraph_sits_at_the_right_edge() {
    let (mut doc, cascade, float, root) = ahem_paragraph_with_float(
        "aa",
        "float:right;width:30px;height:20px",
        " bbbb cccc dddd",
        "",
    );
    lay_out_with_switch(&mut doc, &cascade);
    let layout = doc.nodes[float].unrounded_layout;
    assert_eq!((layout.location.x, layout.location.y), (70.0, 0.0));
    let lines = &stored_lines(&doc, root).lines;
    assert_eq!(
        lines.iter().map(line_start_x).collect::<Vec<_>>(),
        [Some(0.0); 3]
    );
}

#[test]
fn a_float_that_would_shorten_its_own_line_moves_to_the_next_line() {
    // "aaaa bbbb" is 90px. The float (60px) is anchored right after "bbbb":
    // placing it would leave 40px and push "bbbb" (and with it the anchor) to
    // the next line, so the float is withdrawn and placed on the next line.
    let (mut doc, cascade, float, root) = ahem_paragraph_with_float(
        "aaaa bbbb",
        "float:left;width:60px;height:10px",
        " cccc",
        "",
    );
    lay_out_with_switch(&mut doc, &cascade);
    let lines = &stored_lines(&doc, root).lines;
    assert_eq!(line_text(&lines[0]), "aaaa bbbb");
    assert_eq!(line_start_x(&lines[0]), Some(0.0));
    // The float is placed at the top of the second line.
    let layout = doc.nodes[float].unrounded_layout;
    assert_eq!((layout.location.x, layout.location.y), (0.0, 10.0));
    assert_eq!(line_text(&lines[1]), "cccc");
    assert_eq!(line_start_x(&lines[1]), Some(60.0));
}

#[test]
fn an_intrinsic_width_includes_the_float() {
    // A shrink-to-fit root is measured for its content before the final pass.
    // The float (30px) sits on the first line with "aa bbbb" (70px), so the
    // max-content width is 100px; if the float's width is left out of the
    // intrinsic sizes the root is only 70px wide.
    let (mut doc, cascade, float, root) = ahem_paragraph_with_float(
        "aa",
        "float:left;width:30px;height:20px",
        " bbbb",
        "float:left;width:auto",
    );
    lay_out_with_switch(&mut doc, &cascade);
    let layout = doc.nodes[float].unrounded_layout;
    assert_eq!((layout.location.x, layout.location.y), (0.0, 0.0));
    let lines = &stored_lines(&doc, root).lines;
    assert_eq!(line_start_x(&lines[0]), Some(30.0));
    assert_eq!(doc.nodes[root].unrounded_layout.size.width, 100.0);
}

#[test]
fn a_float_inside_a_padded_paragraph_is_placed_inside_its_content_box() {
    // `apply_content_box_inset` is what keeps the paragraph's own floats out of
    // its padding: a left float sits at the content box's left edge.
    let (mut doc, cascade, float, _root) = ahem_paragraph_with_float(
        "aa",
        "float:left;width:30px;height:20px",
        " bbbb",
        "padding:0 10px;box-sizing:border-box",
    );
    lay_out_with_switch(&mut doc, &cascade);
    assert_eq!(doc.nodes[float].unrounded_layout.location.x, 10.0);
}

#[test]
fn relayout_keeps_the_lines_of_a_paragraph_that_has_boxes() {
    use crate::layout::relayout_text_for_width;
    let (mut doc, cascade, _float, root) = ahem_paragraph_with_float(
        "aa",
        "float:left;width:30px;height:20px",
        " bbbb cccc dddd",
        "",
    );
    lay_out_with_switch(&mut doc, &cascade);
    let before: Vec<_> = stored_lines(&doc, root)
        .lines
        .iter()
        .map(line_text)
        .collect();
    relayout_text_for_width(&mut doc, &cascade, 50.0, 50.0, ahem_font_context());
    let after: Vec<_> = stored_lines(&doc, root)
        .lines
        .iter()
        .map(line_text)
        .collect();
    assert_eq!(
        after, before,
        "a float fixes the positions the lines were broken for"
    );
}

#[test]
fn a_float_wider_than_the_rest_of_its_line_waits_for_the_next_line() {
    // Two 60px left floats anchored after "aa": the first takes 60px of the
    // 100px line, the second no longer fits beside it and is placed at the
    // start of the next line instead of shortening this one.
    let (mut doc, cascade, first, root) =
        ahem_paragraph_with_float("aa", "float:left;width:60px;height:10px", "", "");
    let second = doc.append_element(
        Some(root),
        "div",
        Style::default(),
        Some("display:block;float:left;width:60px;height:10px"),
    );
    doc.append_text(root, " bb");
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade_2 = raikiri_style::cascade(&doc, &rules).expect("cascade");
    drop(cascade);
    lay_out_with_switch(&mut doc, &cascade_2);
    let lines = &stored_lines(&doc, root).lines;
    assert_eq!(
        lines.iter().map(line_text).collect::<Vec<_>>(),
        ["aa", "bb"]
    );
    assert_eq!(
        lines.iter().map(line_start_x).collect::<Vec<_>>(),
        [Some(60.0), Some(60.0)]
    );
    let at = |id: usize| {
        let layout = doc.nodes[id].unrounded_layout;
        (layout.location.x, layout.location.y)
    };
    assert_eq!(at(first), (0.0, 0.0));
    assert_eq!(at(second), (0.0, 10.0));
}

#[test]
fn a_paragraph_inside_a_float_of_a_root_is_measured() {
    // The float holds a paragraph of its own. Laying the float out from the
    // outer root lays that paragraph out too, which needs the engine state:
    // the float gets the height of its two lines.
    let (mut doc, cascade, float, root) =
        ahem_paragraph_with_float("aa", "float:left;width:30px", " bbbb", "");
    let inner = doc.append_element(Some(float), "div", Style::default(), Some("display:block"));
    doc.append_text(inner, "ff gg");
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade_2 = raikiri_style::cascade(&doc, &rules).expect("cascade");
    drop(cascade);
    lay_out_with_switch(&mut doc, &cascade_2);
    assert!(doc.nodes[root].is_ifc_root());
    assert!(doc.nodes[inner].is_ifc_root());
    assert_eq!(stored_lines(&doc, inner).lines.len(), 2);
    assert_eq!(doc.nodes[float].unrounded_layout.size.height, 20.0);
}

#[test]
fn relayout_keeps_the_lines_of_a_paragraph_whose_float_is_below_them() {
    // The float is withdrawn from the only line and placed below it, so no
    // line is beside a float; the lines still belong with the float's place.
    use crate::layout::relayout_text_for_width;
    let (mut doc, cascade, _float, root) =
        ahem_paragraph_with_float("aaaa bbbb", "float:left;width:60px;height:10px", "", "");
    lay_out_with_switch(&mut doc, &cascade);
    assert!(!stored_lines(&doc, root).beside_floats);
    let before: Vec<_> = stored_lines(&doc, root)
        .lines
        .iter()
        .map(line_text)
        .collect();
    assert_eq!(before, ["aaaa bbbb"]);
    relayout_text_for_width(&mut doc, &cascade, 50.0, 50.0, ahem_font_context());
    let after: Vec<_> = stored_lines(&doc, root)
        .lines
        .iter()
        .map(line_text)
        .collect();
    assert_eq!(after, before);
}

#[test]
fn only_a_root_that_is_its_own_formatting_context_grows_around_its_floats() {
    // One 10px line and a 20px float. An in-flow root leaves the float
    // hanging below its line for the parent to collect; a floated root is a
    // formatting context of its own and contains it. (A later pass grows
    // auto-height ancestors of every float on both paths, so the node height
    // is not what tells the two apart; the measured content height is.)
    for (root_css, height) in [("", 10.0), ("float:left", 20.0)] {
        let (mut doc, cascade, _float, root) =
            ahem_paragraph_with_float("aa", "float:left;width:30px;height:20px", " bb", root_css);
        lay_out_with_switch(&mut doc, &cascade);
        assert!(doc.nodes[root].is_ifc_root(), "{root_css}");
        let lines = stored_lines(&doc, root);
        assert_eq!(lines.lines.len(), 1, "{root_css}");
        assert_eq!(lines.height, height, "{root_css}");
    }
}

// ── committed boxes of ifc roots ─────────────────────────────

#[test]
fn a_paragraph_that_is_a_formatting_context_contains_its_floats() {
    // The root is itself a block formatting context (it floats), so its height
    // reaches the bottom of the tallest float even when the text is shorter.
    let (mut doc, cascade, _float, root) = ahem_paragraph_with_float(
        "aa",
        "float:left;width:30px;height:50px",
        " bb",
        "float:left;width:100px",
    );
    lay_out_with_switch(&mut doc, &cascade);
    assert!(doc.nodes[root].is_ifc_root());
    assert_eq!(doc.nodes[root].unrounded_layout.size.height, 50.0);
}

#[test]
fn an_in_flow_paragraph_does_not_grow_for_its_floats() {
    let (mut doc, cascade, _float, root) =
        ahem_paragraph_with_float("aa", "float:left;width:30px;height:50px", " bb", "");
    lay_out_with_switch(&mut doc, &cascade);
    assert!(doc.nodes[root].is_ifc_root());
    // One 10px line; the float hangs below the content, which is what a block
    // that is not a formatting context does.
    assert_eq!(stored_lines(&doc, root).height, 10.0);
    // `realign_text_after_layout` later grows every auto-height ancestor of a
    // float to the float's bottom, on either path; the float starts at the
    // top of the paragraph, so that is 50px.
    assert_eq!(doc.nodes[root].unrounded_layout.size.height, 50.0);
}

#[test]
fn a_committed_float_layout_carries_its_box_model() {
    let (mut doc, cascade, float, _root) = ahem_paragraph_with_float(
        "aa",
        "float:left;width:30px;height:20px;margin-left:5px;padding-top:2px;box-sizing:content-box",
        " bb",
        "",
    );
    lay_out_with_switch(&mut doc, &cascade);
    let layout = doc.nodes[float].unrounded_layout;
    // The border box starts after the 5px margin and is 22px tall.
    assert_eq!((layout.location.x, layout.location.y), (5.0, 0.0));
    assert_eq!(layout.size.height, 22.0);
    assert_eq!(layout.padding.top, 2.0);
    assert_eq!(layout.margin.left, 5.0);
}

#[test]
fn a_percentage_margin_resolves_against_the_paragraph_width() {
    // The root is 100px wide under an 800px body: 10% is 10px, not 80px.
    let (mut doc, cascade, float, _root) = ahem_paragraph_with_float(
        "aa",
        "float:left;width:30px;height:20px;margin-left:10%",
        " bb",
        "",
    );
    lay_out_with_switch(&mut doc, &cascade);
    let layout = doc.nodes[float].unrounded_layout;
    assert_eq!(layout.margin.left, 10.0);
    assert_eq!(layout.location.x, 10.0);
}

#[test]
fn a_padded_formatting_context_does_not_count_its_top_edge_twice() {
    // The context measures floats from the border-box top, the box height from
    // the content-box top: a 15px top padding and a 50px float make a box that
    // is 15 + 50 = 65px tall, not 15 + 65.
    let (mut doc, cascade, _float, root) = ahem_paragraph_with_float(
        "aa",
        "float:left;width:30px;height:50px",
        " bb",
        "float:left;width:100px;padding-top:15px",
    );
    lay_out_with_switch(&mut doc, &cascade);
    assert_eq!(doc.nodes[root].unrounded_layout.size.height, 65.0);
}

// ── float placement rules the shadow record has to follow ────

/// `ahem_paragraph_with_float` with more floats after the first one: each
/// entry of `more` is `(float_css, text_after)`. Returns the floats in order.
fn ahem_paragraph_with_floats(
    before: &str,
    first: (&str, &str),
    more: &[(&str, &str)],
    root_css: &str,
) -> (Document, CascadeResult, Vec<usize>, usize) {
    let (mut doc, _cascade, float, root) =
        ahem_paragraph_with_float(before, first.0, first.1, root_css);
    let mut floats = vec![float];
    for (css, after) in more {
        floats.push(doc.append_element(
            Some(root),
            "div",
            Style::default(),
            Some(&format!("display:block;{css}")),
        ));
        if !after.is_empty() {
            doc.append_text(root, *after);
        }
    }
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
    (doc, cascade, floats, root)
}

fn origin(doc: &Document, id: usize) -> (f32, f32) {
    let layout = doc.nodes[id].unrounded_layout;
    (layout.location.x, layout.location.y)
}

#[test]
fn a_float_after_a_deferred_float_is_deferred_too() {
    // CSS 2.1 9.5.1 rule 5: a float is not placed above an earlier one. The
    // 60px float does not stay on "aaaa bbbb" and goes to the next line, so
    // the 5px float after it must follow it there instead of taking line 1.
    let (mut doc, cascade, floats, root) = ahem_paragraph_with_floats(
        "aaaa bbbb",
        ("float:left;width:60px;height:10px", ""),
        &[("float:left;width:5px;height:10px", " cc")],
        "",
    );
    lay_out_with_switch(&mut doc, &cascade);
    let lines = &stored_lines(&doc, root).lines;
    assert_eq!(line_text(&lines[0]), "aaaa bbbb");
    assert_eq!(line_start_x(&lines[0]), Some(0.0));
    assert_eq!(origin(&doc, floats[0]), (0.0, 10.0));
    assert_eq!(origin(&doc, floats[1]), (60.0, 10.0));
}

#[test]
fn a_float_that_clears_a_float_of_the_same_line_does_not_narrow_it() {
    // The second float clears the first one, so it sits below it and takes no
    // room from the line both are anchored in.
    let (mut doc, cascade, floats, root) = ahem_paragraph_with_floats(
        "aa",
        ("float:left;width:20px;height:10px", ""),
        &[("float:left;clear:left;width:20px;height:10px", " bb")],
        "",
    );
    lay_out_with_switch(&mut doc, &cascade);
    let lines = &stored_lines(&doc, root).lines;
    assert_eq!(line_start_x(&lines[0]), Some(20.0));
    assert_eq!(origin(&doc, floats[0]), (0.0, 0.0));
    assert_eq!(origin(&doc, floats[1]), (0.0, 10.0));
}

#[test]
fn a_float_is_not_placed_above_an_earlier_cleared_float() {
    // Line 1 holds a 30x30 float and one that clears it (placed at y=30).
    // The 10px float on line 2 (y=10) may not go above that one, so it does
    // not narrow line 2 either.
    let (mut doc, cascade, floats, root) = ahem_paragraph_with_floats(
        "aa",
        ("float:left;width:30px;height:30px", ""),
        &[
            ("float:left;clear:left;width:30px;height:10px", " bbbb cc"),
            ("float:left;width:10px;height:10px", " dd"),
        ],
        "",
    );
    lay_out_with_switch(&mut doc, &cascade);
    let lines = &stored_lines(&doc, root).lines;
    assert_eq!(
        lines.iter().map(line_text).collect::<Vec<_>>(),
        ["aa bbbb", "cc dd"]
    );
    assert_eq!(line_start_x(&lines[1]), Some(30.0));
    assert_eq!(origin(&doc, floats[1]), (0.0, 30.0));
    assert_eq!(origin(&doc, floats[2]).1, 30.0);
}

#[test]
fn a_line_too_narrow_beside_a_float_moves_below_it() {
    // CSS 2.1 9.5: a line box that does not fit next to a float is shifted
    // down. Only 20px are left beside the 80px float, too little for "aaaa".
    let (mut doc, cascade, _float, root) =
        ahem_paragraph_beside_float("aaaa bbbb", "float:left;width:80px;height:20px", "");
    lay_out_with_switch(&mut doc, &cascade);
    let lines = &stored_lines(&doc, root).lines;
    assert_eq!(
        lines.iter().map(line_text).collect::<Vec<_>>(),
        ["aaaa bbbb"]
    );
    assert_eq!(lines[0].block_offset(), 20.0);
    assert_eq!(line_start_x(&lines[0]), Some(0.0));
}

#[test]
fn a_float_wider_than_the_paragraph_stays_at_the_top_and_the_text_goes_below() {
    let (mut doc, cascade, float, root) =
        ahem_paragraph_with_float("", "float:left;width:150px;height:20px", "aa", "");
    lay_out_with_switch(&mut doc, &cascade);
    assert!(doc.nodes[root].is_ifc_root());
    assert_eq!(origin(&doc, float), (0.0, 0.0));
    let lines = &stored_lines(&doc, root).lines;
    assert_eq!(line_text(&lines[0]), "aa");
    assert_eq!(lines[0].block_offset(), 20.0);
}

#[test]
fn a_cleared_float_does_not_shorten_the_line_above_its_clearance() {
    // The 30x30 float of line 1 still reaches line 2 (y=10). The float of
    // line 2 clears it, so it goes below it (y=30) and must not take room
    // from line 2, which starts after the first float only.
    let (mut doc, cascade, floats, root) = ahem_paragraph_with_floats(
        "aa",
        ("float:left;width:30px;height:30px", " bbbb cc"),
        &[("float:left;clear:left;width:20px;height:10px", " dd")],
        "",
    );
    lay_out_with_switch(&mut doc, &cascade);
    let lines = &stored_lines(&doc, root).lines;
    assert_eq!(
        lines.iter().map(line_text).collect::<Vec<_>>(),
        ["aa bbbb", "cc dd"]
    );
    assert_eq!(line_start_x(&lines[1]), Some(30.0));
    assert_eq!(origin(&doc, floats[1]), (0.0, 30.0));
}

// ── atomic inlines in ifc roots ──────────────────────────────

#[test]
fn an_inline_block_is_measured_before_the_lines_are_broken() {
    // "aa " (30px) + a 30x30 inline-block + " bb". The atomic's baseline is its
    // bottom edge (it has no text), so it rises 30px above the baseline and
    // the line is 30 + 2 (the strut's descent) = 32px tall.
    let (mut doc, cascade, _atomic, root) =
        ahem_paragraph_with_atomic("aa ", "width:30px;height:30px", " bb", "");
    lay_out_with_switch(&mut doc, &cascade);
    assert!(doc.nodes[root].is_ifc_root());
    let lines = &stored_lines(&doc, root).lines;
    assert_eq!(lines.len(), 1);
    assert_eq!(doc.nodes[root].unrounded_layout.size.height, 32.0);
}

#[test]
fn a_wide_atomic_moves_to_the_next_line() {
    let (mut doc, cascade, _atomic, root) =
        ahem_paragraph_with_atomic("aa ", "width:90px;height:10px", " bb", "");
    lay_out_with_switch(&mut doc, &cascade);
    // "aa " leaves 70px, the atomic needs 90: it starts the second line. After
    // it only 10px are left, so " bb" (30px) starts a third line. The line
    // with the atomic has no text (the placeholder is stripped).
    let lines = &stored_lines(&doc, root).lines;
    assert_eq!(
        lines.iter().map(line_text).collect::<Vec<_>>(),
        ["aa", "", "bb"]
    );
}

#[test]
fn a_shrink_to_fit_paragraph_with_an_inline_block_is_as_wide_as_its_content() {
    // max-content: "aa " (30) + the 30px atomic + "bb" (20) = 80.
    let (mut doc, cascade, _atomic, root) = ahem_paragraph_with_atomic(
        "aa ",
        "width:30px;height:10px",
        "bb",
        "float:left;width:auto",
    );
    lay_out_with_switch(&mut doc, &cascade);
    assert!(doc.nodes[root].is_ifc_root());
    assert_eq!(doc.nodes[root].unrounded_layout.size.width, 80.0);
}

#[test]
fn an_inline_block_holding_a_paragraph_is_measured() {
    // The atomic holds a block holding a block with text; the innermost block
    // is an ifc root of its own (its parent is a block container and its
    // siblings are blocks). The state of the engine must survive measuring it
    // from inside the outer paragraph.
    let (mut doc, _cascade, root) = ahem_paragraph("aa ", "width:100px");
    let atomic_box = doc.append_element(
        Some(root),
        "span",
        taffy::Style::default(),
        Some("display:inline-block"),
    );
    let middle = doc.append_element(
        Some(atomic_box),
        "div",
        taffy::Style::default(),
        Some("display:block"),
    );
    let inner = doc.append_element(
        Some(middle),
        "div",
        taffy::Style::default(),
        Some("display:block"),
    );
    doc.append_text(inner, "bb cc");
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
    lay_out_with_switch(&mut doc, &cascade);
    assert!(doc.nodes[root].is_ifc_root());
    assert!(
        doc.nodes[inner].is_ifc_root(),
        "the paragraph inside the atomic is a root of its own"
    );
    // The inner paragraph was laid out while the outer one was measured; with
    // the engine state taken it would store no lines at all.
    assert_eq!(stored_lines(&doc, inner).lines.len(), 1);
}

#[test]
fn an_inline_block_baseline_counts_its_top_margin() {
    // The atomic holds one line of "ii" (baseline 8px below its border-box
    // top) and has a 3px top margin, so its baseline is 11px below its margin
    // box top. The margin box (13px) rises 11px above the line's baseline and
    // reaches 2px below it: the line is 13px tall with the baseline at 11.
    let (mut doc, _cascade, root) = ahem_paragraph("aa ", "width:100px");
    let atomic = doc.append_element(
        Some(root),
        "span",
        taffy::Style::default(),
        Some("display:inline-block;width:20px;margin-top:3px"),
    );
    doc.append_text(atomic, "ii");
    doc.append_text(root, " bb");
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
    lay_out_with_switch(&mut doc, &cascade);
    assert!(doc.nodes[root].is_ifc_root());
    let lines = &stored_lines(&doc, root).lines;
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].block_size(), 13.0);
    assert_eq!(
        lines[0].baseline(shodo::geometry::BaselineKind::Alphabetic),
        11.0
    );
}

#[test]
fn a_clipping_inline_block_sits_on_its_bottom_margin_edge() {
    // An inline-block that is a scroll container has no baseline (CSS 2.1
    // 10.8.1): its 10px box rises 10px above the line's baseline instead of
    // lining its text up with the surrounding text, so the line is 10 + 2
    // (the strut's descent) = 12px tall.
    let (mut doc, _cascade, root) = ahem_paragraph("aa ", "width:100px");
    let atomic = doc.append_element(
        Some(root),
        "span",
        taffy::Style::default(),
        Some("display:inline-block;width:20px;height:10px;overflow:hidden"),
    );
    doc.append_text(atomic, "ii");
    doc.append_text(root, " bb");
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
    lay_out_with_switch(&mut doc, &cascade);
    assert!(doc.nodes[root].is_ifc_root());
    let lines = &stored_lines(&doc, root).lines;
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].block_size(), 12.0);
}

#[test]
fn an_inline_block_is_placed_at_its_fragment() {
    // The atomic follows "aa " (30px) and rises 30px above the baseline: its
    // top is the line's top (y=0).
    let (mut doc, cascade, atomic, _root) =
        ahem_paragraph_with_atomic("aa ", "width:30px;height:30px", " bb", "");
    lay_out_with_switch(&mut doc, &cascade);
    let layout = doc.nodes[atomic].unrounded_layout;
    assert_eq!((layout.location.x, layout.location.y), (30.0, 0.0));
    assert_eq!((layout.size.width, layout.size.height), (30.0, 30.0));
}

#[test]
fn an_inline_block_with_text_sits_on_its_last_baseline() {
    // The atomic holds one line of text, so its baseline is the text's (8px
    // from its top) and it lines up with the surrounding text: same line
    // height, top at y=0.
    let (mut doc, _cascade, root) = ahem_paragraph("aa ", "width:100px");
    let atomic = doc.append_element(
        Some(root),
        "span",
        taffy::Style::default(),
        Some("display:inline-block;width:20px"),
    );
    doc.append_text(atomic, "ii");
    doc.append_text(root, " bb");
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
    lay_out_with_switch(&mut doc, &cascade);
    assert!(doc.nodes[root].is_ifc_root());
    let layout = doc.nodes[atomic].unrounded_layout;
    assert_eq!((layout.location.x, layout.location.y), (30.0, 0.0));
    assert_eq!(doc.nodes[root].unrounded_layout.size.height, 10.0);
}

#[test]
fn an_atomic_margin_moves_its_border_box_inside_the_margin_box() {
    let (mut doc, cascade, atomic, _root) = ahem_paragraph_with_atomic(
        "aa ",
        "width:30px;height:10px;margin-left:5px;margin-top:3px",
        "",
        "",
    );
    lay_out_with_switch(&mut doc, &cascade);
    let layout = doc.nodes[atomic].unrounded_layout;
    // The margin box starts after "aa " (30px); the border box is 5px in.
    assert_eq!(layout.location.x, 35.0);
    assert_eq!(layout.margin.left, 5.0);
    // The atomic has no text, so its baseline is the margin box's bottom edge
    // and the margin box (3 + 10 = 13px) rises 13px above the baseline: the
    // margin box starts at the line top and the border box 3px below it.
    assert_eq!(layout.location.y, 3.0);
}

#[test]
fn atomics_on_later_lines_are_placed_on_their_own_line() {
    let (mut doc, cascade, atomic, root) =
        ahem_paragraph_with_atomic("aaaaaaaa ", "width:30px;height:10px", "", "");
    lay_out_with_switch(&mut doc, &cascade);
    // "aaaaaaaa " is 90px; the atomic (30px) starts the second line.
    let layout = doc.nodes[atomic].unrounded_layout;
    assert_eq!((layout.location.x, layout.location.y), (0.0, 10.0));
    assert_eq!(stored_lines(&doc, root).lines.len(), 2);
}

#[test]
fn an_atomic_is_offset_by_the_content_box_of_its_paragraph() {
    let (mut doc, cascade, atomic, _root) = ahem_paragraph_with_atomic(
        "aa ",
        "width:30px;height:10px",
        "",
        "padding:4px 6px;box-sizing:border-box",
    );
    lay_out_with_switch(&mut doc, &cascade);
    let layout = doc.nodes[atomic].unrounded_layout;
    assert_eq!((layout.location.x, layout.location.y), (36.0, 4.0));
}

#[test]
fn an_atomic_beside_a_float_starts_after_it() {
    // The float covers the first 30px of the line, so "aa " starts at 30 and
    // the atomic follows it at 60.
    let (mut doc, _cascade, _float, root) =
        ahem_paragraph_with_float("aa ", "float:left;width:30px;height:30px", "", "");
    let atomic = doc.append_element(
        Some(root),
        "span",
        taffy::Style::default(),
        Some("display:inline-block;width:30px;height:10px"),
    );
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
    lay_out_with_switch(&mut doc, &cascade);
    assert!(doc.nodes[root].is_ifc_root());
    let layout = doc.nodes[atomic].unrounded_layout;
    assert_eq!(layout.location.x, 60.0);
}

#[test]
fn an_image_is_placed_like_an_inline_block() {
    // A 10x10 image after "aa " sits on the baseline with its bottom edge: it
    // rises 10px, 2px above the strut's ascent, so the line is 12px tall and
    // the image's top is the line's top.
    let (mut doc, _cascade, root) = ahem_paragraph("aa ", "width:100px");
    let image = doc.append_element(
        Some(root),
        "img",
        taffy::Style::default(),
        Some("display:inline;width:10px;height:10px"),
    );
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
    lay_out_with_switch(&mut doc, &cascade);
    assert!(doc.nodes[root].is_ifc_root());
    let layout = doc.nodes[image].unrounded_layout;
    assert_eq!((layout.location.x, layout.location.y), (30.0, 0.0));
    assert_eq!((layout.size.width, layout.size.height), (10.0, 10.0));
    assert_eq!(doc.nodes[root].unrounded_layout.size.height, 12.0);
}

// ── block children of ifc roots ──────────────────────────────

/// `root` holds `before`, a block div of the given css, then `after`.
fn paragraph_with_block(
    before: &str,
    block_css: &str,
    after: &str,
    root_css: &str,
) -> (Document, CascadeResult, usize, usize) {
    let (mut doc, _cascade, root) = ahem_paragraph(before, &format!("width:100px;{root_css}"));
    let block = doc.append_element(
        Some(root),
        "div",
        taffy::Style::default(),
        Some(&format!("display:block;{block_css}")),
    );
    doc.append_text(root, after);
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
    (doc, cascade, block, root)
}

#[test]
fn a_block_child_splits_the_lines_around_it() {
    let (mut doc, cascade, block, root) = paragraph_with_block("aa", "height:20px", "bb", "");
    lay_out_with_switch(&mut doc, &cascade);
    assert!(doc.nodes[root].is_ifc_root());
    let lines = &stored_lines(&doc, root).lines;
    assert_eq!(
        lines.iter().map(line_text).collect::<Vec<_>>(),
        ["aa", "bb"]
    );
    assert_eq!(
        lines.iter().map(|l| l.block_offset()).collect::<Vec<_>>(),
        [0.0, 30.0]
    );
    let layout = doc.nodes[block].unrounded_layout;
    assert_eq!((layout.location.x, layout.location.y), (0.0, 10.0));
    assert_eq!((layout.size.width, layout.size.height), (100.0, 20.0));
    assert_eq!(doc.nodes[root].unrounded_layout.size.height, 40.0);
}

#[test]
fn a_leading_block_child_starts_the_paragraph() {
    let (mut doc, cascade, block, root) = paragraph_with_block("", "height:20px", "bb", "");
    lay_out_with_switch(&mut doc, &cascade);
    assert!(doc.nodes[root].is_ifc_root());
    assert_eq!(doc.nodes[block].unrounded_layout.location.y, 0.0);
    assert_eq!(doc.nodes[root].unrounded_layout.size.height, 30.0);
}

#[test]
fn a_block_child_is_as_wide_as_the_content_box_less_its_margins() {
    let (mut doc, cascade, block, _root) = paragraph_with_block(
        "aa",
        "height:10px;margin-left:10px;margin-right:20px",
        "bb",
        "",
    );
    lay_out_with_switch(&mut doc, &cascade);
    let layout = doc.nodes[block].unrounded_layout;
    assert_eq!((layout.location.x, layout.size.width), (10.0, 70.0));
}

#[test]
fn a_block_child_is_offset_by_the_content_box_of_its_paragraph() {
    let (mut doc, cascade, block, root) = paragraph_with_block(
        "aa",
        "height:10px",
        "bb",
        "padding:4px 6px;box-sizing:border-box",
    );
    lay_out_with_switch(&mut doc, &cascade);
    assert!(doc.nodes[root].is_ifc_root());
    let layout = doc.nodes[block].unrounded_layout;
    // The content box starts at (6, 4); the block follows the first line.
    // (Its width is the root's authored 100px, which a pre-layout pass copies
    // into every auto-width block under an authored-width ancestor on both
    // paths; the block child keeps the width taffy's style gives it.)
    assert_eq!((layout.location.x, layout.location.y), (6.0, 14.0));
    assert_eq!(layout.size.width, 100.0);
}

#[test]
fn text_inside_a_block_child_is_laid_out_inside_the_block() {
    // The block is a box of the outer root and the root of its own text: it
    // is one 10px line tall ("bb cc dd" is 80px in the 100px block).
    let (mut doc, _cascade, block, root) = paragraph_with_block("aa", "", "cc", "");
    doc.append_text(block, "bb cc dd");
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
    lay_out_with_switch(&mut doc, &cascade);
    assert!(doc.nodes[root].is_ifc_root());
    assert!(doc.nodes[block].is_ifc_root());
    let layout = doc.nodes[block].unrounded_layout;
    assert_eq!((layout.location.x, layout.location.y), (0.0, 10.0));
    assert_eq!(layout.size.height, 10.0);
    let lines = &stored_lines(&doc, root).lines;
    assert_eq!(
        lines.iter().map(line_text).collect::<Vec<_>>(),
        ["aa", "cc"]
    );
    assert_eq!(
        lines.iter().map(|l| l.block_offset()).collect::<Vec<_>>(),
        [0.0, 20.0]
    );
    assert_eq!(doc.nodes[root].unrounded_layout.size.height, 30.0);
}

#[test]
fn a_float_anchored_right_before_a_block_child_is_placed_above_it() {
    // No text precedes the block, so the float's anchor ends no line: the
    // float is still placed, at the top, and the block starts there too.
    let (mut doc, _cascade, root) = ahem_paragraph("", "width:100px");
    let float = doc.append_element(
        Some(root),
        "div",
        taffy::Style::default(),
        Some("display:block;float:left;width:30px;height:10px"),
    );
    let block = doc.append_element(
        Some(root),
        "div",
        taffy::Style::default(),
        Some("display:block;height:20px"),
    );
    doc.append_text(root, "bb");
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
    lay_out_with_switch(&mut doc, &cascade);
    assert!(doc.nodes[root].is_ifc_root());
    let layout = doc.nodes[float].unrounded_layout;
    assert_eq!((layout.location.x, layout.location.y), (0.0, 0.0));
    assert_eq!((layout.size.width, layout.size.height), (30.0, 10.0));
    assert_eq!(doc.nodes[block].unrounded_layout.location.y, 0.0);
}

#[test]
fn a_shrink_to_fit_paragraph_is_as_wide_as_its_widest_block_child() {
    // max-content: the lines are 20px wide, the block child 60px.
    let (mut doc, cascade, _block, root) = paragraph_with_block(
        "aa",
        "width:60px;height:10px",
        "bb",
        "float:left;width:auto",
    );
    lay_out_with_switch(&mut doc, &cascade);
    assert!(doc.nodes[root].is_ifc_root());
    assert_eq!(doc.nodes[root].unrounded_layout.size.width, 60.0);
}

#[test]
fn a_paragraph_that_contains_its_floats_contains_those_of_its_block_children() {
    // The root floats, so it is a formatting context of its own. The block
    // child holds only a 30x50 float, which starts below "aa" (y=10) and ends
    // at 60; "bb" goes beside it. (The node height grows to the float in a
    // later pass on both paths; the measured content height is what counts.)
    let (mut doc, _cascade, block, root) = paragraph_with_block("aa", "", "bb", "float:left");
    doc.append_element(
        Some(block),
        "div",
        taffy::Style::default(),
        Some("display:block;float:left;width:30px;height:50px"),
    );
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
    lay_out_with_switch(&mut doc, &cascade);
    assert!(doc.nodes[root].is_ifc_root());
    let lines = stored_lines(&doc, root);
    assert_eq!(
        lines.lines.iter().map(line_start_x).collect::<Vec<_>>(),
        [Some(0.0), Some(30.0)]
    );
    assert_eq!(lines.height, 60.0);
}

// ── re-break rules for roots with boxes ──────────────────────

/// `root` (no authored width, so the re-break follows the page width) holds
/// "aa ", a child of the given element css, then `after`.
fn unsized_paragraph_with_child(child_css: &str, after: &str) -> (Document, CascadeResult, usize) {
    let (mut doc, _cascade, root) = ahem_paragraph("aa ", "");
    doc.append_element(Some(root), "span", taffy::Style::default(), Some(child_css));
    doc.append_text(root, after);
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
    (doc, cascade, root)
}

#[test]
fn relayout_keeps_the_lines_of_a_paragraph_with_an_atomic() {
    use crate::layout::relayout_text_for_width;
    let (mut doc, cascade, root) =
        unsized_paragraph_with_child("display:inline-block;width:30px;height:10px", " bbbb cccc");
    lay_out_with_switch(&mut doc, &cascade);
    assert!(doc.nodes[root].is_ifc_root());
    let before: Vec<_> = stored_lines(&doc, root)
        .lines
        .iter()
        .map(line_text)
        .collect();
    relayout_text_for_width(&mut doc, &cascade, 50.0, 50.0, ahem_font_context());
    let after: Vec<_> = stored_lines(&doc, root)
        .lines
        .iter()
        .map(line_text)
        .collect();
    assert_eq!(
        after, before,
        "an atomic's position is tied to the lines it was placed in"
    );
}

#[test]
fn relayout_keeps_the_lines_of_a_paragraph_with_a_block_child() {
    use crate::layout::relayout_text_for_width;
    let (mut doc, cascade, root) =
        unsized_paragraph_with_child("display:block;height:20px", "bbbb cccc");
    lay_out_with_switch(&mut doc, &cascade);
    assert!(doc.nodes[root].is_ifc_root());
    let before: Vec<_> = stored_lines(&doc, root)
        .lines
        .iter()
        .map(line_text)
        .collect();
    relayout_text_for_width(&mut doc, &cascade, 50.0, 50.0, ahem_font_context());
    let after: Vec<_> = stored_lines(&doc, root)
        .lines
        .iter()
        .map(line_text)
        .collect();
    assert_eq!(after, before);
}

#[test]
fn relayout_still_follows_the_page_width_for_a_plain_paragraph() {
    use crate::layout::relayout_text_for_width;
    let (mut doc, cascade, root) = ahem_paragraph("aaaa bbbb cccc", "");
    lay_out_with_switch(&mut doc, &cascade);
    relayout_text_for_width(&mut doc, &cascade, 50.0, 50.0, ahem_font_context());
    assert_eq!(stored_lines(&doc, root).lines.len(), 3);
}

#[test]
fn a_block_child_keeps_its_own_width() {
    for (css, width) in [
        ("width:40px;height:10px", 40.0),
        ("max-width:50%;height:10px", 50.0),
        ("width:10px;min-width:30px;height:10px", 30.0),
        (
            "width:40px;padding-left:5px;box-sizing:border-box;height:10px",
            40.0,
        ),
        ("width:40px;padding-left:5px;height:10px", 45.0),
    ] {
        let (mut doc, cascade, block, root) = paragraph_with_block("aa", css, "bb", "");
        lay_out_with_switch(&mut doc, &cascade);
        assert!(doc.nodes[root].is_ifc_root(), "{css}");
        let layout = doc.nodes[block].unrounded_layout;
        assert_eq!(
            (layout.location.x, layout.size.width),
            (0.0, width),
            "{css}"
        );
    }
}

#[test]
fn a_right_float_inside_a_narrow_block_child_sits_at_the_block_edge() {
    // Each block is 40px wide, so its 10px right float is 30px in.
    for css in ["width:40px", "max-width:40px", "width:10px;min-width:40px"] {
        let (mut doc, _cascade, block, root) = paragraph_with_block("aa", css, "bb", "");
        let float = doc.append_element(
            Some(block),
            "div",
            taffy::Style::default(),
            Some("display:block;float:right;width:10px;height:10px"),
        );
        doc.mark_in_document_flags();
        let rules = raikiri_style::build_rule_tree(&doc);
        let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
        lay_out_with_switch(&mut doc, &cascade);
        assert!(doc.nodes[root].is_ifc_root(), "{css}");
        assert_eq!(doc.nodes[float].unrounded_layout.location.x, 30.0, "{css}");
    }
}

#[test]
fn a_float_root_with_its_own_width_breaks_a_long_word_at_that_width() {
    // `overflow-wrap: break-word` does not lower the min-content width, so a
    // shrink-to-fit clamp would lay the word out on one 240px line; the
    // float's own `width` fixes the line width instead (CSS 2.1 10.3.5 applies
    // shrink-to-fit only to an `auto` width).
    for css in [
        "float:left;width:100px;overflow-wrap:break-word",
        "float:left;width:100px;word-break:break-word",
        "float:left;width:12.5%;overflow-wrap:break-word",
        "float:left;width:calc(50px + 6.25%);overflow-wrap:break-word",
    ] {
        let (mut doc, cascade, root) = ahem_paragraph("FillerFillerFillerFiller", css);
        lay_out_with_switch(&mut doc, &cascade);
        assert!(doc.nodes[root].is_ifc_root(), "{css}");
        assert_eq!(doc.nodes[root].unrounded_layout.size.width, 100.0, "{css}");
        assert_eq!(stored_lines(&doc, root).lines.len(), 3, "{css}");
        assert_eq!(doc.nodes[root].unrounded_layout.size.height, 30.0, "{css}");
    }
}

#[test]
fn a_float_root_without_a_width_still_shrinks_to_fit() {
    let (mut doc, cascade, root) = ahem_paragraph("aaaa bb", "float:left");
    lay_out_with_switch(&mut doc, &cascade);
    assert!(doc.nodes[root].is_ifc_root());
    assert_eq!(doc.nodes[root].unrounded_layout.size.width, 70.0);
    assert_eq!(stored_lines(&doc, root).lines.len(), 1);
}

// ── boxes of inline elements in ifc roots ────────────────────

const SPAN_EDGES: &str = "padding:0 3px;border-width:0 2px;border-style:solid;margin:0 4px";

#[test]
fn an_inline_element_has_the_same_border_box_with_and_without_the_switch() {
    use crate::layout::test_support::{absolute_rect, ahem_paragraph_with};
    let make = || {
        let mut span_id = 0;
        let (doc, cascade, _root) = ahem_paragraph_with("width:200px", |doc, root| {
            doc.append_text(root, "aa");
            let inner = doc.append_element(
                Some(root),
                "span",
                taffy::Style::default(),
                Some(format!("display:inline;{SPAN_EDGES}").as_str()),
            );
            doc.append_text(inner, "bb");
            doc.append_text(root, "cc");
            span_id = inner;
        });
        (doc, cascade, span_id)
    };
    let (mut off_doc, off_cascade, off_span) = make();
    layout_single_page(
        &mut off_doc,
        &off_cascade,
        page_box_800x600(),
        ahem_font_context(),
    )
    .expect("layout");
    let (mut on_doc, on_cascade, on_span) = make();
    lay_out_with_switch(&mut on_doc, &on_cascade);
    assert!(on_doc.nodes[on_doc.parent_of(on_span).expect("root")].is_ifc_root());

    let off = absolute_rect(&off_doc, off_span);
    assert_eq!(
        off,
        (24.0, 0.0, 30.0, 10.0),
        "the parley path boxes the span like F"
    );
    assert_eq!(absolute_rect(&on_doc, on_span), off);
}

#[test]
fn a_padded_root_places_the_inline_element_inside_its_content_box() {
    use crate::layout::test_support::{absolute_rect, ahem_paragraph_with};
    let make = || {
        let mut span_id = 0;
        let (doc, cascade, _root) = ahem_paragraph_with(
            "width:200px;padding:0 5px 0 7px;box-sizing:content-box",
            |doc, root| {
                doc.append_text(root, "aa");
                let inner = doc.append_element(
                    Some(root),
                    "span",
                    taffy::Style::default(),
                    Some("display:inline;padding:0 3px"),
                );
                doc.append_text(inner, "bb");
                span_id = inner;
            },
        );
        (doc, cascade, span_id)
    };
    let (mut off_doc, off_cascade, off_span) = make();
    layout_single_page(
        &mut off_doc,
        &off_cascade,
        page_box_800x600(),
        ahem_font_context(),
    )
    .expect("layout");
    let (mut on_doc, on_cascade, on_span) = make();
    lay_out_with_switch(&mut on_doc, &on_cascade);
    assert!(on_doc.nodes[on_doc.parent_of(on_span).expect("root")].is_ifc_root());
    // 7px left padding, "aa" 20px, then the span's box.
    assert_eq!(absolute_rect(&on_doc, on_span), (27.0, 0.0, 26.0, 10.0));
    assert_eq!(
        absolute_rect(&on_doc, on_span),
        absolute_rect(&off_doc, off_span)
    );
}

#[test]
fn a_root_with_a_top_border_places_the_inline_element_below_it() {
    use crate::layout::test_support::{absolute_rect, ahem_paragraph_with};
    let mut span_id = 0;
    let (mut doc, cascade, root) = ahem_paragraph_with(
        "width:200px;border-width:6px 0 0 0;border-style:solid;padding-top:1px",
        |doc, root| {
            let inner = doc.append_element(
                Some(root),
                "span",
                taffy::Style::default(),
                Some("display:inline"),
            );
            doc.append_text(inner, "bb");
            span_id = inner;
        },
    );
    lay_out_with_switch(&mut doc, &cascade);
    assert!(doc.nodes[root].is_ifc_root());
    // The content box starts 7px below the root's border-box top.
    let (_, root_y, _, _) = absolute_rect(&doc, root);
    assert_eq!(
        absolute_rect(&doc, span_id),
        (0.0, root_y + 7.0, 20.0, 10.0)
    );
}

#[test]
fn a_wrapping_inline_element_has_the_bounding_box_of_its_pieces() {
    use crate::layout::test_support::{absolute_rect, ahem_paragraph_with};
    let mut span_id = 0;
    let (mut doc, cascade, _root) = ahem_paragraph_with("width:60px", |doc, root| {
        let inner = doc.append_element(
            Some(root),
            "span",
            taffy::Style::default(),
            Some(format!("display:inline;{SPAN_EDGES}").as_str()),
        );
        doc.append_text(inner, "aaaa bbbb");
        span_id = inner;
    });
    lay_out_with_switch(&mut doc, &cascade);
    // The span wraps after "aaaa ": pieces x 4..49 (line 1) and x 0..45
    // (line 2); the bounding box is x 0..49, y 0..20.
    assert_eq!(absolute_rect(&doc, span_id), (0.0, 0.0, 49.0, 20.0));
}

#[test]
fn nested_inline_elements_accumulate_to_their_own_boxes() {
    use crate::layout::test_support::{absolute_rect, ahem_paragraph_with};
    let (mut outer_id, mut inner_id) = (0, 0);
    let (mut doc, cascade, _root) = ahem_paragraph_with("width:200px", |doc, root| {
        doc.append_text(root, "a");
        let outer = doc.append_element(
            Some(root),
            "span",
            taffy::Style::default(),
            Some("display:inline;padding:0 1px"),
        );
        doc.append_text(outer, "b");
        let inner = doc.append_element(
            Some(outer),
            "span",
            taffy::Style::default(),
            Some("display:inline;padding:0 2px"),
        );
        doc.append_text(inner, "c");
        outer_id = outer;
        inner_id = inner;
    });
    lay_out_with_switch(&mut doc, &cascade);
    assert_eq!(absolute_rect(&doc, outer_id), (10.0, 0.0, 26.0, 10.0));
    assert_eq!(absolute_rect(&doc, inner_id), (21.0, 0.0, 14.0, 10.0));
}

#[test]
fn an_empty_inline_element_has_a_zero_width_box_on_its_line() {
    use crate::layout::test_support::{absolute_rect, ahem_paragraph_with};
    // An inline element without content still has a content area on the
    // line: 0 wide, ascent + descent (10px) tall, after "aa".
    let mut span_id = 0;
    let (mut doc, cascade, _root) = ahem_paragraph_with("width:200px", |doc, root| {
        doc.append_text(root, "aa");
        span_id = doc.append_element(
            Some(root),
            "span",
            taffy::Style::default(),
            Some("display:inline"),
        );
    });
    lay_out_with_switch(&mut doc, &cascade);
    assert_eq!(absolute_rect(&doc, span_id), (20.0, 0.0, 0.0, 10.0));
}

#[test]
fn an_inline_element_without_a_piece_gets_an_empty_layout() {
    use crate::layout::test_support::ahem_paragraph_with;
    let mut span_id = 0;
    let (mut doc, cascade, root) =
        ahem_paragraph_with("width:200px;padding:5px 0 0 7px", |doc, root| {
            doc.append_text(root, "aa");
            span_id = doc.append_element(
                Some(root),
                "span",
                taffy::Style::default(),
                Some("display:none;padding:0 3px"),
            );
            doc.append_text(span_id, "bb");
        });
    lay_out_with_switch(&mut doc, &cascade);
    assert!(doc.nodes[root].is_ifc_root());
    let layout = doc.nodes[span_id].unrounded_layout;
    assert_eq!(
        (
            layout.location.x,
            layout.location.y,
            layout.size.width,
            layout.size.height
        ),
        (0.0, 0.0, 0.0, 0.0)
    );
}

#[test]
fn a_second_layout_does_not_keep_the_stale_box_of_a_removed_element() {
    use crate::layout::test_support::ahem_paragraph_with;
    let mut span_id = 0;
    let (mut doc, cascade, _root) = ahem_paragraph_with("width:200px", |doc, root| {
        doc.append_text(root, "aa");
        let inner = doc.append_element(
            Some(root),
            "span",
            taffy::Style::default(),
            Some("display:inline;padding:0 3px"),
        );
        doc.append_text(inner, "bb");
        span_id = inner;
    });
    lay_out_with_switch(&mut doc, &cascade);
    assert!(doc.nodes[span_id].unrounded_layout.size.width > 0.0);
    // Hide the element and lay out again: it has no piece any more.
    doc.set_element_inline_style(span_id, Some("display:none".into()));
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
    lay_out_with_switch(&mut doc, &cascade);
    assert_eq!(doc.nodes[span_id].unrounded_layout.size.width, 0.0);
}

#[test]
fn a_wide_padding_does_not_make_the_layout_check_zero_the_element() {
    use crate::layout::test_support::{absolute_rect, ahem_paragraph_with};
    // The padding (2 x 40) is wider than the bounding box of the pieces,
    // which carry one edge each; the layout check must not read that as a
    // negative content box and zero the element.
    let mut span_id = 0;
    let (mut doc, cascade, _root) = ahem_paragraph_with("width:60px", |doc, root| {
        let inner = doc.append_element(
            Some(root),
            "span",
            taffy::Style::default(),
            Some("display:inline;padding:0 40px"),
        );
        doc.append_text(inner, "a b");
        span_id = inner;
    });
    lay_out_with_switch(&mut doc, &cascade);
    assert!(
        absolute_rect(&doc, span_id).2 > 0.0,
        "the element kept its box"
    );
}

#[test]
fn relayout_records_the_boxes_again_for_the_new_lines() {
    use crate::layout::relayout_text_for_width;
    use crate::layout::test_support::{absolute_rect, ahem_paragraph_with};
    let mut span_id = 0;
    // No authored width: relayout follows the page width. The content box
    // starts 7px right of and 3px below the root's border-box corner.
    let (mut doc, cascade, root) = ahem_paragraph_with(
        "padding-left:7px;border-width:3px 0 0 0;border-style:solid",
        |doc, root| {
            let inner = doc.append_element(
                Some(root),
                "span",
                taffy::Style::default(),
                Some("display:inline"),
            );
            doc.append_text(inner, "aaaa bbbb");
            span_id = inner;
        },
    );
    lay_out_with_switch(&mut doc, &cascade);
    assert!(doc.nodes[root].is_ifc_root());
    assert_eq!(
        absolute_rect(&doc, span_id),
        (7.0, 3.0, 90.0, 10.0),
        "one line"
    );
    relayout_text_for_width(&mut doc, &cascade, 50.0, 50.0, ahem_font_context());
    assert_eq!(
        absolute_rect(&doc, span_id),
        (7.0, 3.0, 40.0, 20.0),
        "two lines at 50px"
    );
}

/// Pages of a paragraph whose inline element asks for a page break before it,
/// with the switch on or off.
fn pages_with_a_breaking_inline(switch: bool) -> (usize, bool) {
    use crate::layout::test_support::ahem_paragraph_with;
    // At 30px the span lands on the third line, 20px down the page.
    let (mut doc, cascade, root) = ahem_paragraph_with("width:30px", |doc, root| {
        doc.append_text(root, "aa aa ");
        let inner = doc.append_element(
            Some(root),
            "span",
            taffy::Style::default(),
            Some("display:inline;break-before:page"),
        );
        doc.append_text(inner, "bb");
        doc.append_text(root, " cc");
    });
    if switch {
        doc.enable_inline_formatting(ifc_ahem_fonts(), shodo::limits::Limits::default());
    }
    let slices =
        layout_pages(&mut doc, &cascade, page_box_800x600(), ahem_font_context()).expect("pages");
    (slices.len(), doc.nodes[root].is_ifc_root())
}

#[test]
fn an_inline_element_in_an_ifc_paragraph_is_not_a_page_break_candidate() {
    // `break-before` applies to block-level boxes (CSS Fragmentation 3 3.1);
    // the paragraph's lines are not split around an inline element, as on
    // the parley path.
    assert_eq!(pages_with_a_breaking_inline(false), (1, false));
    assert_eq!(pages_with_a_breaking_inline(true), (1, true));
}

#[test]
fn a_relative_inline_element_is_located_with_its_offset() {
    use crate::layout::test_support::{absolute_rect, ahem_paragraph_with};
    let mut span_id = 0;
    let (mut doc, cascade, root) = ahem_paragraph_with("width:200px", |doc, root| {
        doc.append_text(root, "aa");
        let inner = doc.append_element(
            Some(root),
            "span",
            taffy::Style::default(),
            Some("display:inline;position:relative;left:5px;top:2px"),
        );
        doc.append_text(inner, "bb");
        span_id = inner;
    });
    lay_out_with_switch(&mut doc, &cascade);
    assert!(doc.nodes[root].is_ifc_root());
    // Unshifted "bb" box: x 20..40, y 0..10; shifted by (5, 2).
    assert_eq!(absolute_rect(&doc, span_id), (25.0, 2.0, 20.0, 10.0));
}

#[test]
fn a_relative_inline_is_located_like_the_parley_path() {
    use crate::layout::test_support::{absolute_rect, ahem_paragraph_with};
    let make = || {
        let mut span_id = 0;
        let (doc, cascade, _root) = ahem_paragraph_with("width:200px", |doc, root| {
            doc.append_text(root, "aa");
            let inner = doc.append_element(
                Some(root),
                "span",
                taffy::Style::default(),
                Some("display:inline;position:relative;right:4px;bottom:3px"),
            );
            doc.append_text(inner, "bb");
            span_id = inner;
        });
        (doc, cascade, span_id)
    };
    let (mut off_doc, off_cascade, off_span) = make();
    layout_single_page(
        &mut off_doc,
        &off_cascade,
        page_box_800x600(),
        ahem_font_context(),
    )
    .expect("layout");
    let (mut on_doc, on_cascade, on_span) = make();
    lay_out_with_switch(&mut on_doc, &on_cascade);
    assert!(on_doc.nodes[on_doc.parent_of(on_span).expect("root")].is_ifc_root());
    let off = absolute_rect(&off_doc, off_span);
    assert_eq!(
        off,
        (16.0, -3.0, 20.0, 10.0),
        "the parley path shifts the span"
    );
    assert_eq!(absolute_rect(&on_doc, on_span), off);
}

#[test]
fn a_child_of_a_relative_inline_moves_with_it() {
    use crate::layout::test_support::{absolute_rect, ahem_paragraph_with};
    let (mut outer_id, mut inner_id) = (0, 0);
    let (mut doc, cascade, _root) = ahem_paragraph_with("width:200px", |doc, root| {
        let outer = doc.append_element(
            Some(root),
            "span",
            taffy::Style::default(),
            Some("display:inline;position:relative;left:5px"),
        );
        doc.append_text(outer, "a");
        let inner = doc.append_element(
            Some(outer),
            "span",
            taffy::Style::default(),
            Some("display:inline"),
        );
        doc.append_text(inner, "b");
        outer_id = outer;
        inner_id = inner;
    });
    lay_out_with_switch(&mut doc, &cascade);
    assert_eq!(absolute_rect(&doc, outer_id), (5.0, 0.0, 20.0, 10.0));
    // The child is not itself relative, but sits inside the shifted parent.
    assert_eq!(absolute_rect(&doc, inner_id), (15.0, 0.0, 10.0, 10.0));
}

#[test]
fn an_element_inside_a_contents_element_is_located_from_the_nearest_box() {
    use crate::layout::test_support::{absolute_rect, ahem_paragraph_with};
    let (mut wrapper_id, mut inner_id) = (0, 0);
    let (mut doc, cascade, root) = ahem_paragraph_with("width:200px", |doc, root| {
        doc.append_text(root, "aa");
        let wrapper = doc.append_element(
            Some(root),
            "span",
            taffy::Style::default(),
            Some("display:contents"),
        );
        let inner = doc.append_element(
            Some(wrapper),
            "span",
            taffy::Style::default(),
            Some("display:inline;padding:0 3px"),
        );
        doc.append_text(inner, "bb");
        wrapper_id = wrapper;
        inner_id = inner;
    });
    lay_out_with_switch(&mut doc, &cascade);
    assert!(doc.nodes[root].is_ifc_root());
    // The contents element has no box; the span after "aa" is x 20..46.
    assert_eq!(absolute_rect(&doc, wrapper_id), (0.0, 0.0, 0.0, 0.0));
    assert_eq!(absolute_rect(&doc, inner_id), (20.0, 0.0, 26.0, 10.0));
}

#[test]
fn a_block_inside_a_contents_child_of_a_paragraph_still_breaks_the_page() {
    use crate::layout::test_support::ahem_paragraph_with;
    let pages = |switch: bool| {
        let (mut doc, cascade, _root) = ahem_paragraph_with("width:200px", |doc, root| {
            doc.append_text(root, "aa");
            let wrapper = doc.append_element(
                Some(root),
                "span",
                taffy::Style::default(),
                Some("display:contents"),
            );
            let block = doc.append_element(
                Some(wrapper),
                "div",
                taffy::Style::default(),
                Some("display:block;break-before:page"),
            );
            doc.append_text(block, "bb");
        });
        if switch {
            doc.enable_inline_formatting(ifc_ahem_fonts(), shodo::limits::Limits::default());
        }
        layout_pages(&mut doc, &cascade, page_box_800x600(), ahem_font_context())
            .expect("pages")
            .len()
    };
    assert_eq!(pages(false), 2);
    assert_eq!(pages(true), pages(false));
}

// ── roots under other layout algorithms ─────────────────────

use crate::layout::test_support::{
    ahem_paragraph_in, body_with_text_and_block_child, flex_container_with_inline_item,
    paragraph_with_inline_block, paragraph_with_only_an_empty_span, paragraph_with_only_atomic,
    paragraph_with_only_br,
};

/// Lay `doc` out with the inline engine on (`ifc`) or on the parley path.
fn lay_out(doc: &mut Document, cascade: &raikiri_style::CascadeResult, ifc: bool) {
    if ifc {
        lay_out_with_switch(doc, cascade);
    } else {
        layout_single_page(doc, cascade, page_box_800x600(), ahem_font_context()).expect("layout");
    }
}

/// Border-box size of the paragraph of [`ahem_paragraph_in`].
fn laid_out_sizes(parent_css: &str, css: &str, text: &str, ifc: bool) -> (f32, f32) {
    let (mut doc, cascade, root) = ahem_paragraph_in(parent_css, css, text);
    lay_out(&mut doc, &cascade, ifc);
    let layout = doc.nodes[root].unrounded_layout;
    (layout.size.width, layout.size.height)
}

/// Offset of the outer line's baseline from the top of the inline-block of
/// [`paragraph_with_inline_block`]: where the inline-block's own baseline sits.
fn inline_block_baseline(text: &str, width: f32, ifc: bool) -> f32 {
    let (mut doc, cascade, root, inline_block) = paragraph_with_inline_block(text, width);
    lay_out(&mut doc, &cascade, ifc);
    let line_baseline = if ifc {
        assert!(doc.nodes[root].is_ifc_root());
        let line = &stored_lines(&doc, root).lines[0];
        line.block_offset() + line.baseline(shodo::geometry::BaselineKind::Alphabetic)
    } else {
        let x = doc.nodes[root].children[0];
        let layout = doc.nodes[x].text_layout().expect("parley lines");
        doc.nodes[x].unrounded_layout.location.y
            + layout.lines().next().expect("a line").metrics().baseline
    };
    line_baseline - doc.nodes[inline_block].unrounded_layout.location.y
}

#[test]
fn a_flex_item_becomes_an_ifc_root() {
    let (mut doc, cascade, root) = ahem_paragraph_in("display:flex", "", "aaaa bbbb");
    lay_out_with_switch(&mut doc, &cascade);
    assert!(doc.nodes[root].is_ifc_root());
}

#[test]
fn an_inline_flex_item_becomes_an_ifc_root() {
    // The item is `display:inline` in the cascade; only taffy blockifies it.
    let (mut doc, cascade, span) = flex_container_with_inline_item("aaaa");
    lay_out_with_switch(&mut doc, &cascade);
    assert!(doc.nodes[span].is_ifc_root());
    assert_eq!(doc.nodes[span].unrounded_layout.size.width, 40.0); // 4 em of Ahem 10px
}

#[test]
fn a_block_child_of_a_paragraph_with_text_is_its_own_root() {
    // <body>aaaa <div>bbbb cccc</div></body>: the div is a box of body's paragraph
    // and the root of its own text.
    let (mut doc, cascade, body, div) = body_with_text_and_block_child("aaaa ", "bbbb cccc");
    lay_out_with_switch(&mut doc, &cascade);
    assert!(doc.nodes[body].is_ifc_root());
    assert!(doc.nodes[div].is_ifc_root());
}

#[test]
fn a_flex_item_is_laid_out_like_the_parley_path() {
    // Ahem 10px: "aaaa bbbb" is 90px wide in one line, 40px per word when wrapped.
    for (parent_css, css) in [
        ("display:flex", ""),
        ("display:flex", "width:50px"),
        ("display:flex;flex-direction:column", ""),
        ("display:flex;align-items:flex-start", ""),
    ] {
        assert_eq!(
            laid_out_sizes(parent_css, css, "aaaa bbbb", true),
            laid_out_sizes(parent_css, css, "aaaa bbbb", false),
            "{parent_css} / {css}"
        );
    }
    // Hand-computed, not an oracle: a flex row item shrinks to its max-content.
    assert_eq!(
        laid_out_sizes("display:flex", "", "aaaa bbbb", true),
        (90.0, 10.0)
    );
}

#[test]
fn a_paragraph_of_only_an_atomic_becomes_a_root() {
    let (mut doc, cascade, root) =
        paragraph_with_only_atomic("display:inline-block;width:20px;height:10px");
    lay_out_with_switch(&mut doc, &cascade);
    assert!(doc.nodes[root].is_ifc_root());
}

#[test]
fn a_paragraph_of_only_br_elements_becomes_a_root() {
    let (mut doc, cascade, root) = paragraph_with_only_br(2);
    lay_out_with_switch(&mut doc, &cascade);
    assert!(doc.nodes[root].is_ifc_root());
    assert_eq!(doc.nodes[root].unrounded_layout.size.height, 20.0); // two 10px lines
}

#[test]
fn a_paragraph_of_only_an_empty_inline_with_edges_becomes_a_root() {
    let (mut doc, cascade, root) =
        paragraph_with_only_an_empty_span("padding:4px;border:1px solid");
    lay_out_with_switch(&mut doc, &cascade);
    assert!(doc.nodes[root].is_ifc_root());
}

#[test]
fn an_inline_block_baseline_is_its_last_line() {
    // Two lines of 10px text: the inline-block's baseline sits on line 2.
    let off = inline_block_baseline("aaaa bbbb", 40.0, false);
    let on = inline_block_baseline("aaaa bbbb", 40.0, true);
    assert_eq!(on, off);
    assert_eq!(on, 18.0); // 10 (line 1) + 8 (ascent of line 2)
}

#[test]
fn parley_gives_a_paragraph_of_only_br_elements_no_height() {
    // CSS 2.1 9.4.2: each `<br>` ends a line box that holds a forced break, so
    // two of them make two 10px lines; the parley path lays them out as an
    // empty block. The inline engine is right (see the test above).
    let (mut doc, cascade, root) = paragraph_with_only_br(2);
    lay_out(&mut doc, &cascade, false);
    assert!(!doc.nodes[root].is_ifc_root());
    assert_eq!(doc.nodes[root].unrounded_layout.size.height, 0.0);
}

#[test]
fn a_paragraph_of_only_an_atomic_has_the_strut_descent_below_it() {
    // A 20x10 inline-block without lines sits on the baseline with its bottom
    // margin edge (CSS 2.1 10.8.1); the strut adds Ahem's 2px descent below
    // the baseline, so the line is 12px tall. The parley path gives 10px.
    let css = "display:inline-block;width:20px;height:10px";
    let (mut doc, cascade, root) = paragraph_with_only_atomic(css);
    lay_out_with_switch(&mut doc, &cascade);
    assert_eq!(doc.nodes[root].unrounded_layout.size.height, 12.0);
    let (mut doc, cascade, root) = paragraph_with_only_atomic(css);
    lay_out(&mut doc, &cascade, false);
    assert_eq!(doc.nodes[root].unrounded_layout.size.height, 10.0);
}

#[test]
fn a_paragraph_of_only_an_empty_inline_with_edges_is_one_line_tall() {
    for ifc in [false, true] {
        let (mut doc, cascade, root) =
            paragraph_with_only_an_empty_span("padding:4px;border:1px solid");
        lay_out(&mut doc, &cascade, ifc);
        assert_eq!(doc.nodes[root].unrounded_layout.size.height, 10.0, "{ifc}");
    }
}

#[test]
fn a_grid_item_becomes_an_ifc_root() {
    let (mut doc, cascade, root) = ahem_paragraph_in("display:grid", "", "aaaa bbbb");
    lay_out_with_switch(&mut doc, &cascade);
    assert!(doc.nodes[root].is_ifc_root());
    assert_eq!(
        laid_out_sizes("display:grid", "", "aaaa bbbb", true),
        laid_out_sizes("display:grid", "", "aaaa bbbb", false)
    );
    // Hand-computed: the single grid column stretches over the 800px page.
    assert_eq!(
        laid_out_sizes("display:grid", "", "aaaa bbbb", true),
        (800.0, 10.0)
    );
}

#[test]
fn an_inline_block_with_text_becomes_an_ifc_root() {
    let (mut doc, cascade, root) = ahem_paragraph_in("", "display:inline-block", "aaaa bbbb");
    lay_out_with_switch(&mut doc, &cascade);
    assert!(doc.nodes[root].is_ifc_root());
    let on = laid_out_sizes("", "display:inline-block", "aaaa bbbb", true);
    assert_eq!(
        on,
        laid_out_sizes("", "display:inline-block", "aaaa bbbb", false)
    );
    // Hand-computed: shrink-to-fit is the max-content width of "aaaa bbbb".
    assert_eq!(on, (90.0, 10.0));
}

#[test]
fn a_list_item_becomes_an_ifc_root() {
    let css = "display:list-item;list-style-type:none;width:50px";
    let (mut doc, cascade, root) = ahem_paragraph_in("", css, "aaaa bbbb");
    lay_out_with_switch(&mut doc, &cascade);
    assert!(doc.nodes[root].is_ifc_root());
    let on = laid_out_sizes("", css, "aaaa bbbb", true);
    assert_eq!(on, laid_out_sizes("", css, "aaaa bbbb", false));
    // Hand-computed: two 40px words do not fit 50px together.
    assert_eq!(on, (50.0, 20.0));
}

/// `location.y` of two flex items aligned by their baselines: Ahem at 10px and
/// at 20px, with a line height of 1.
fn flex_baseline_item_ys(ifc: bool) -> (f32, f32) {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), Some("display:block"));
    let body = doc.append_element(Some(html), "body", Style::default(), Some("display:block"));
    let flex = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:flex;align-items:baseline;font-family:Ahem;line-height:1"),
    );
    let small = doc.append_element(Some(flex), "div", Style::default(), Some("font-size:10px"));
    doc.append_text(small, "aa");
    let large = doc.append_element(Some(flex), "div", Style::default(), Some("font-size:20px"));
    doc.append_text(large, "bb");
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
    lay_out(&mut doc, &cascade, ifc);
    if ifc {
        assert!(doc.nodes[small].is_ifc_root() && doc.nodes[large].is_ifc_root());
    }
    (
        doc.nodes[small].unrounded_layout.location.y,
        doc.nodes[large].unrounded_layout.location.y,
    )
}

#[test]
fn a_flex_item_baseline_matches() {
    let on = flex_baseline_item_ys(true);
    assert_eq!(on, flex_baseline_item_ys(false));
    // Hand-computed: the 20px item's ascent is 16 and the 10px item's is 8,
    // so the small item sits 8px lower.
    assert_eq!(on, (8.0, 0.0));
}

#[test]
fn an_empty_inline_block_keeps_its_margin_box_baseline() {
    // "x" then an empty 20x20 inline-block: without lines it sits on the
    // baseline with its bottom margin edge, so the line baseline is 20px down
    // and the strut descent makes the line 22px tall.
    for ifc in [false, true] {
        let (mut doc, cascade, root, inline_block) = paragraph_with_inline_block("", 20.0);
        doc.set_element_inline_style(
            inline_block,
            Some("display:inline-block;width:20px;height:20px".into()),
        );
        doc.mark_in_document_flags();
        let rules = raikiri_style::build_rule_tree(&doc);
        let cascade_again = raikiri_style::cascade(&doc, &rules).expect("cascade");
        let _ = cascade;
        lay_out(&mut doc, &cascade_again, ifc);
        assert!(!doc.nodes[inline_block].is_ifc_root());
        assert_eq!(
            doc.nodes[inline_block].unrounded_layout.location.y, 0.0,
            "{ifc}"
        );
        if ifc {
            assert_eq!(doc.nodes[root].unrounded_layout.size.height, 22.0);
        }
    }
}

#[test]
fn bare_text_next_to_an_element_item_gets_its_own_root() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), Some("display:block"));
    let body = doc.append_element(Some(html), "body", Style::default(), Some("display:block"));
    let flex = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:flex;font-family:Ahem;font-size:10px;line-height:10px"),
    );
    let bare = doc.append_text(flex, "aa");
    let item = doc.append_element(Some(flex), "div", Style::default(), Some("display:block"));
    doc.append_text(item, "bbbb");
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
    lay_out_with_switch(&mut doc, &cascade);
    assert!(!doc.nodes[flex].is_ifc_root());
    assert!(doc.nodes[item].is_ifc_root());
    // The bare text is an anonymous item and the root of its own paragraph.
    assert!(doc.nodes[bare].is_ifc_root());
    assert!(doc.nodes[bare].text_layout().is_none());
    // Hand-computed: the item follows the 20px anonymous item.
    assert_eq!(doc.nodes[item].unrounded_layout.location.x, 20.0);
    assert_eq!(doc.nodes[item].unrounded_layout.size.width, 40.0);
}

// ── text node roots ─────────────────────────────────────────

use crate::layout::test_support::ahem_paragraph_in_text_only;

/// Border-box size of the bare text of [`ahem_paragraph_in_text_only`].
fn bare_text_size(parent_css: &str, ifc: bool) -> (f32, f32) {
    let (mut doc, cascade, parent) = ahem_paragraph_in_text_only(parent_css, "aaaa bbbb");
    lay_out(&mut doc, &cascade, ifc);
    let text = doc.nodes[parent].children[0];
    let layout = doc.nodes[text].unrounded_layout;
    (layout.size.width, layout.size.height)
}

#[test]
fn bare_text_in_a_flex_container_becomes_a_root() {
    let (mut doc, cascade, flex) = ahem_paragraph_in_text_only("display:flex", "aaaa bbbb");
    lay_out_with_switch(&mut doc, &cascade);
    let text = doc.nodes[flex].children[0];
    assert!(doc.nodes[text].is_ifc_root());
}

#[test]
fn bare_text_in_a_flex_container_has_the_parley_size() {
    for parent_css in [
        "display:flex",
        "display:flex;flex-direction:column",
        "display:grid",
    ] {
        assert_eq!(
            bare_text_size(parent_css, true),
            bare_text_size(parent_css, false),
            "{parent_css}"
        );
    }
    // Hand-computed: a flex row item shrinks to "aaaa bbbb" (90px), one line;
    // in a 50px container the two words wrap.
    assert_eq!(bare_text_size("display:flex", true), (90.0, 10.0));
    assert_eq!(
        bare_text_size("display:flex;width:50px", true),
        (50.0, 20.0)
    );
}

#[test]
fn parley_gives_a_shrunk_anonymous_flex_item_its_longest_line_as_width() {
    // The anonymous item of "aaaa bbbb" in a 50px flex row shrinks from its
    // 90px max-content to 50px, above its 40px min-content (CSS Flexbox 1,
    // 9.7), and its two lines are laid out in that 50px box. The parley path
    // reports the width of its longest line instead.
    assert_eq!(
        bare_text_size("display:flex;width:50px", false),
        (40.0, 20.0)
    );
}

#[test]
fn a_text_node_root_answers_the_line_and_baseline_readers() {
    let (mut doc, cascade, flex) = ahem_paragraph_in_text_only("display:flex", "aaaa bbbb");
    lay_out_with_switch(&mut doc, &cascade);
    let text = doc.nodes[flex].children[0];
    let lines = doc.ifc_text_lines(text).expect("lines");
    assert_eq!(lines.root, text);
    assert_eq!(lines.lines.len(), 1);
    // Hand-computed: Ahem 10px, line-height 10px, ascent 8.
    assert_eq!(
        crate::taffy_impl::first_inline_baseline(&doc, text),
        Some(8.0)
    );
}

#[test]
fn whitespace_only_text_in_a_flex_container_is_not_a_root() {
    // Collapsible white space makes no anonymous item (CSS Flexbox 1, 4).
    let (mut doc, cascade, flex) = ahem_paragraph_in_text_only("display:flex", " \n\t ");
    lay_out_with_switch(&mut doc, &cascade);
    let text = doc.nodes[flex].children[0];
    assert!(!doc.nodes[text].is_ifc_root());
}

#[test]
fn a_flex_row_places_bare_text_after_its_sibling_item() {
    let (mut doc, cascade, flex) = ahem_paragraph_in_text_only("display:flex", "");
    let item = doc.append_element(
        Some(flex),
        "div",
        Style::default(),
        Some("display:block;width:30px;height:10px"),
    );
    let text = doc.append_text(flex, "aaaa");
    let _ = item;
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade_again = raikiri_style::cascade(&doc, &rules).expect("cascade");
    let _ = cascade;
    lay_out_with_switch(&mut doc, &cascade_again);
    assert!(doc.nodes[text].is_ifc_root());
    let layout = doc.nodes[text].unrounded_layout;
    // Hand-computed: the 30px item first, then the 40px anonymous item.
    assert_eq!((layout.location.x, layout.size.width), (30.0, 40.0));
}

#[test]
fn bare_text_takes_the_text_align_of_its_container() {
    // The anonymous item inherits `text-align` from the container: in a 50px
    // flex row, "aaaa" (40px) is centered in the 50px item.
    let (mut doc, cascade, flex) =
        ahem_paragraph_in_text_only("display:flex;width:50px;text-align:center", "aaaa bbbb");
    lay_out_with_switch(&mut doc, &cascade);
    let text = doc.nodes[flex].children[0];
    let lines = &stored_lines(&doc, text).lines;
    assert_eq!(line_start_x(&lines[0]), Some(5.0));
}
