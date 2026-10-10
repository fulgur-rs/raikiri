//! Margin-box geometry, decoration and generated content resolved from
//! `@page` rules, on a 400x300 page with 40px page margins.

use super::*;

const AHEM: &str = "font-family: Ahem; font-size: 10px;";

fn boxes_for(css: &str) -> Vec<MarginBox> {
    let (doc, cascade) = page_cascade_fixture(css);
    layout_page(&doc, &cascade, small_page())
}

fn boxes_on_page(css: &str, context: MarginBoxPageContext) -> Vec<MarginBox> {
    let (doc, cascade) = page_cascade_fixture(css);
    page_margin_boxes(&doc, &cascade, &cascade.page, small_page(), context)
}

fn rects(boxes: &[MarginBox]) -> Vec<(PageMarginBoxSlot, [f32; 4])> {
    boxes
        .iter()
        .map(|margin_box| {
            let rect = margin_box.rect;
            (
                margin_box.slot,
                [rect.x, rect.y, rect.width, rect.height].map(|v| (v * 100.0).round() / 100.0),
            )
        })
        .collect()
}

fn single_content(css: &str) -> String {
    let boxes = boxes_for(css);
    let [margin_box] = boxes.as_slice() else {
        panic!("one box: {boxes:?}");
    };
    margin_box.content.clone()
}

#[test]
fn auto_side_boxes_share_a_column_like_the_ac_track() {
    let boxes = boxes_for(&format!(
        "@page {{ margin: 40px; {AHEM} \
           @left-top {{ content: 'a' }} @left-middle {{ content: 'b' }} \
           @left-bottom {{ content: 'c' }} }}"
    ));
    assert_eq!(
        rects(&boxes),
        [
            (PageMarginBoxSlot::LeftTop, [0.0, 40.0, 40.0, 73.33]),
            (PageMarginBoxSlot::LeftMiddle, [0.0, 113.33, 40.0, 73.33]),
            (PageMarginBoxSlot::LeftBottom, [0.0, 186.67, 40.0, 73.33]),
        ]
    );
}

#[test]
fn fixed_height_side_boxes_center_in_their_thirds() {
    let boxes = boxes_for(
        "@page { margin: 40px; \
           @right-top { content: ''; height: 20px } \
           @right-middle { content: ''; height: 20px } \
           @right-bottom { content: ''; height: 20px } }",
    );
    assert_eq!(
        rects(&boxes),
        [
            (PageMarginBoxSlot::RightTop, [360.0, 66.67, 40.0, 20.0]),
            (PageMarginBoxSlot::RightMiddle, [360.0, 140.0, 40.0, 20.0]),
            (PageMarginBoxSlot::RightBottom, [360.0, 213.33, 40.0, 20.0]),
        ]
    );
}

#[test]
fn a_lone_fixed_height_middle_box_centers_in_the_column() {
    let boxes = boxes_for("@page { margin: 40px; @right-middle { content: ''; height: 20px } }");
    assert_eq!(
        rects(&boxes),
        [(PageMarginBoxSlot::RightMiddle, [360.0, 140.0, 40.0, 20.0])]
    );
}

#[test]
fn empty_side_boxes_split_the_column_evenly() {
    let boxes = boxes_for(
        "@page { margin: 40px; @right-top { content: '' } @right-bottom { content: '' } }",
    );
    assert_eq!(
        rects(&boxes),
        [
            (PageMarginBoxSlot::RightTop, [360.0, 40.0, 40.0, 110.0]),
            (PageMarginBoxSlot::RightBottom, [360.0, 150.0, 40.0, 110.0]),
        ]
    );
}

#[test]
fn fixed_width_left_boxes_hug_the_page_area_unless_centered() {
    let boxes = boxes_for(
        "@page { margin: 40px; \
           @left-top { content: ''; width: 20px; height: 20px } \
           @left-bottom { content: ''; width: 20px; height: 20px; margin: auto } }",
    );
    assert_eq!(
        rects(&boxes),
        [
            (PageMarginBoxSlot::LeftTop, [20.0, 40.0, 20.0, 20.0]),
            (PageMarginBoxSlot::LeftBottom, [10.0, 60.0, 20.0, 20.0]),
        ]
    );
}

#[test]
fn corner_boxes_fill_or_align_in_their_cells() {
    let boxes = boxes_for(
        "@page { margin: 40px; \
           @top-left-corner { content: ''; width: 20px; height: 20px } \
           @top-right-corner { content: ''; width: 20px; height: 20px; margin: auto } \
           @bottom-left-corner { content: '' } \
           @bottom-right-corner { content: ''; height: 20px } }",
    );
    assert_eq!(
        rects(&boxes),
        [
            (PageMarginBoxSlot::TopLeftCorner, [20.0, 20.0, 20.0, 20.0]),
            (PageMarginBoxSlot::TopRightCorner, [370.0, 10.0, 20.0, 20.0]),
            (
                PageMarginBoxSlot::BottomRightCorner,
                [360.0, 260.0, 40.0, 20.0]
            ),
            (
                PageMarginBoxSlot::BottomLeftCorner,
                [0.0, 260.0, 40.0, 40.0]
            ),
        ]
    );
}

#[test]
fn three_auto_top_boxes_keep_the_center_box_centered() {
    let boxes = boxes_for(&format!(
        "@page {{ margin: 40px; {AHEM} \
           @top-left {{ content: 'a' }} @top-center {{ content: 'bb' }} \
           @top-right {{ content: 'a' }} }}"
    ));
    assert_eq!(
        rects(&boxes),
        [
            (PageMarginBoxSlot::TopLeft, [40.0, 0.0, 80.0, 40.0]),
            (PageMarginBoxSlot::TopCenter, [120.0, 0.0, 160.0, 40.0]),
            (PageMarginBoxSlot::TopRight, [280.0, 0.0, 80.0, 40.0]),
        ]
    );
}

#[test]
fn an_empty_center_box_anchors_the_side_boxes_to_half_the_row() {
    let boxes = boxes_for(&format!(
        "@page {{ margin: 40px; {AHEM} \
           @top-left {{ content: 'abc' }} @top-center {{ content: '' }} }}"
    ));
    assert_eq!(
        rects(&boxes),
        [(PageMarginBoxSlot::TopLeft, [40.0, 0.0, 160.0, 40.0])]
    );
}

#[test]
fn fixed_height_edge_boxes_sit_against_the_page_area_or_center() {
    let boxes = boxes_for(
        "@page { margin: 40px; \
           @top-center { content: ''; height: 20px } \
           @bottom-center { content: ''; height: 20px; margin: auto } }",
    );
    assert_eq!(
        rects(&boxes),
        [
            (PageMarginBoxSlot::TopCenter, [40.0, 20.0, 320.0, 20.0]),
            (PageMarginBoxSlot::BottomCenter, [40.0, 270.0, 320.0, 20.0]),
        ]
    );
}

#[test]
fn lengths_resolve_every_unit_to_css_px() {
    for (length, px) in [
        (Length::Px(5.0), 5.0),
        (Length::Percent(50.0), 100.0),
        (Length::Em(2.0), 20.0),
        (Length::Rem(2.0), 20.0),
        (Length::Ex(2.0), 10.0),
        (Length::Ch(2.0), 10.0),
        (Length::Ic(2.0), 20.0),
        (Length::Lh(2.0), 20.0),
        (Length::Pt(72.0), 96.0),
        (Length::Cm(2.54), 96.0),
        (Length::Mm(25.4), 96.0),
        (Length::Q(101.6), 96.0),
        (Length::In(1.0), 96.0),
        (Length::Pc(6.0), 96.0),
        (Length::Px(-5.0), 0.0),
        (Length::Px(f32::NAN), 0.0),
    ] {
        let resolved = length_to_px(length, 200.0, 10.0);
        assert!((resolved - px).abs() < 1e-3, "{length:?} -> {resolved}");
    }
    assert_eq!(length_or_auto_to_px(LengthOrAuto::Auto, 200.0, 10.0), None);
    let calc = |px, percent| {
        LengthOrAuto::Calc(raikiri_style::property::CalcLengthPercentage { px, percent })
    };
    assert_eq!(
        length_or_auto_to_px(calc(10.0, 10.0), 200.0, 10.0),
        Some(30.0)
    );
    assert_eq!(
        length_or_auto_to_px(calc(f32::NAN, f32::NAN), 200.0, 10.0),
        Some(0.0)
    );
    assert_eq!(
        length_or_auto_to_px(calc(-50.0, 0.0), 200.0, 10.0),
        Some(0.0)
    );
    assert_eq!(margin_box_length(&calc(10.0, 10.0), 200.0, 10.0), 30.0);
    assert_eq!(
        margin_box_length(&calc(f32::INFINITY, 0.0), 200.0, 10.0),
        0.0
    );
    assert_eq!(margin_box_length(&LengthOrAuto::Auto, 200.0, 10.0), 0.0);
}

#[test]
fn box_sizes_paddings_and_margins_accept_calc_and_longhands() {
    let boxes = boxes_for(
        "@page { margin: 40px; \
           @top-left { content: ''; width: 10%; height: 20px; \
                       padding: 1px 2px 3px 4px; padding-left: 5px; \
                       margin: 0; margin-left: 2px; margin-top: 1px } }",
    );
    let [margin_box] = boxes.as_slice() else {
        panic!("one box: {boxes:?}");
    };
    // 10% of the 320px strip, plus 7px of horizontal padding.
    assert_eq!(margin_box.rect.width, 39.0);
    assert_eq!(margin_box.rect.x, 42.0);
    // 20px plus 4px of vertical padding, against the page area.
    assert_eq!(margin_box.rect.height, 24.0);
    assert_eq!(margin_box.rect.y, 40.0 - 25.0 + 1.0);
    assert_eq!(
        (
            margin_box.padding.top,
            margin_box.padding.right,
            margin_box.padding.bottom,
            margin_box.padding.left
        ),
        (1.0, 2.0, 3.0, 5.0)
    );
}

#[test]
fn border_longhands_override_the_shorthand_and_skip_unsupported_styles() {
    let boxes = boxes_for(
        "@page { margin: 40px; color: blue; \
           @top-left { content: ''; border: 1px solid red; \
                       border-top-width: 3px; border-right-style: dashed; \
                       border-bottom-color: currentcolor; border-left-color: lime } }",
    );
    let [margin_box] = boxes.as_slice() else {
        panic!("one box: {boxes:?}");
    };
    let [top, right, bottom, left] = margin_box.borders;
    assert_eq!(
        top.map(|border| (border.width, border.color.r)),
        Some((3.0, 255))
    );
    assert!(right.is_none(), "dashed borders are not drawn");
    assert_eq!(bottom.map(|border| border.color.b), Some(255));
    assert_eq!(left.map(|border| border.color.g), Some(255));
    assert_eq!(margin_box.color.b, 255, "the page color is inherited");
}

#[test]
fn margin_boxes_inherit_page_fonts_and_alignment() {
    let boxes = boxes_for(
        "@page { margin: 40px; font-family: Ahem; font-size: 20px; text-align: center; \
           vertical-align: text-bottom; \
           @top-left { content: 'ab'; font-size: 50% } }",
    );
    let [margin_box] = boxes.as_slice() else {
        panic!("one box: {boxes:?}");
    };
    let text = margin_box.text.as_ref().expect("the box has text");
    let (x, y) = text.origin();
    assert_eq!(text.shaped().width(), 20.0);
    // Centered on the 320px strip and pushed to the bottom of the 40px row.
    assert_eq!(x, 40.0);
    assert_eq!(y, 30.0);
    let glyphs = glyph_positions(text);
    assert_eq!(glyphs[0].0, 40.0 + 150.0);
}

#[test]
fn every_text_align_keyword_reaches_the_spec() {
    for (keyword, expected) in [
        ("left", StandaloneAlign::Left),
        ("right", StandaloneAlign::Right),
        ("center", StandaloneAlign::Center),
        ("end", StandaloneAlign::End),
        ("justify", StandaloneAlign::Justify),
        ("start", StandaloneAlign::Start),
    ] {
        let (doc, cascade) = page_cascade_fixture(&format!(
            "@page {{ @top-left {{ content: 'a'; text-align: {keyword}; vertical-align: text-top }} }}"
        ));
        let rule = cascade.page.margin_boxes().first().unwrap();
        let spec = margin_box_spec(
            &doc,
            &cascade,
            &cascade.page,
            rule,
            100.0,
            40.0,
            MarginBoxPageContext::new(0, 1, false),
        )
        .unwrap();
        assert_eq!(spec.alignment, expected, "{keyword}");
        assert_eq!(spec.vertical_align, MarginTextVerticalAlign::Top);
    }
}

#[test]
fn page_counters_follow_page_resets_and_increments() {
    let content = |page_rule: &str, box_rule: &str, context: MarginBoxPageContext| {
        let boxes = boxes_on_page(
            &format!(
                "@page {{ margin: 40px; {page_rule}; \
                   @top-left {{ content: counter(page) '|' counters(chapter, '.') '|' \
                                counter(pages); {box_rule} }} }}"
            ),
            context,
        );
        boxes
            .first()
            .unwrap_or_else(|| panic!("no box for {page_rule:?} {box_rule:?}"))
            .content
            .clone()
    };
    let third = MarginBoxPageContext::new(2, 6, false);
    assert_eq!(content("", "", third), "3|0|6");
    assert_eq!(content("counter-reset: page 10", "", third), "11|0|6");
    assert_eq!(
        content("counter-increment: page 2 chapter", "", third),
        "6|3|6"
    );
    assert_eq!(content("counter-reset: none", "", third), "2|0|6");
    assert_eq!(content("counter-increment: page 3", "", third), "6|0|6");
    assert_eq!(
        content(
            "counter-increment: page 0",
            "",
            MarginBoxPageContext::new(4, 6, false).with_paired_page_increment(Some(2)),
        ),
        "4|0|6"
    );
    assert_eq!(
        content(
            "",
            "counter-reset: chapter 4; counter-increment: chapter",
            third
        ),
        "3|5|6"
    );
    assert_eq!(
        content("", "counter-reset: inherit; counter-increment: page", third),
        "4|0|6"
    );
}

#[test]
fn margin_box_quotes_come_from_the_box_the_page_or_the_root() {
    let content = "content: open-quote 'q' close-quote no-open-quote no-close-quote";
    assert_eq!(
        single_content(&format!(
            "@page {{ margin: 40px; @top-left {{ {content}; quotes: '<' '>' }} }}"
        )),
        "<q>"
    );
    assert_eq!(
        single_content(&format!(
            "@page {{ margin: 40px; quotes: none; @top-left {{ {content} }} }}"
        )),
        "q"
    );
    assert_eq!(
        single_content(&format!(
            "@page {{ margin: 40px; @top-left {{ {content} }} }}"
        )),
        "“q”"
    );
    assert_eq!(
        single_content(&format!(
            "html {{ quotes: '[' ']' }} @page {{ margin: 40px; @top-left {{ {content} }} }}"
        )),
        "[q]"
    );
    assert_eq!(
        single_content(&format!(
            "html {{ quotes: none }} @page {{ margin: 40px; @top-left {{ {content} }} }}"
        )),
        "q"
    );
}

#[test]
fn named_strings_and_running_elements_resolve_to_document_text() {
    let mut doc = engine_document();
    let html = doc.append_element(Some(0), "html", Style::default(), Some("display:block"));
    let head = doc.append_element(Some(html), "head", Style::default(), Some("display:none"));
    let style = doc.append_element(Some(head), "style", Style::default(), None::<&str>);
    doc.append_text(
        style,
        "h1 { string-set: title 'Ch. ' content() } \
         div { position: running(hdr) } \
         @page { margin: 40px; \
           @top-left { content: string(title) } \
           @top-right { content: element(hdr) } \
           @bottom-left { content: string(missing) element(missing) attr(id) } }",
    );
    let body = doc.append_element(Some(html), "body", Style::default(), Some("display:block"));
    let h1 = doc.append_element(Some(body), "h1", Style::default(), None::<&str>);
    doc.append_text(h1, "  One \n Two ");
    let hdr = doc.append_element(Some(body), "div", Style::default(), None::<&str>);
    doc.append_text(hdr, "Running head");
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");
    let contents: Vec<_> = layout_page(&doc, &cascade, small_page())
        .into_iter()
        .map(|margin_box| (margin_box.slot, margin_box.content))
        .collect();
    assert_eq!(
        contents,
        [
            (PageMarginBoxSlot::TopLeft, "Ch. One Two".to_owned()),
            (PageMarginBoxSlot::TopRight, "Running head".to_owned()),
            (PageMarginBoxSlot::BottomLeft, String::new()),
        ]
    );
}

#[test]
fn a_lime_content_image_is_placed_after_the_text() {
    let boxes = boxes_for(&format!(
        "@page {{ margin: 40px; @top-left {{ content: 'ab' url(green.png); {AHEM} }} }}"
    ));
    let [margin_box] = boxes.as_slice() else {
        panic!("one box: {boxes:?}");
    };
    assert_eq!(margin_box.content, "ab");
    assert_eq!(
        margin_box.lime_content_image,
        Some(PaintRect::new(60.0, 0.0, 100.0, 40.0))
    );
}
