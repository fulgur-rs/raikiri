use super::*;
use taffy::Style;

#[test]
fn apply_computed_to_style_bridges_display_to_taffy() {
    // display bridge active — DisplayValue → taffy::Display
    // mapping が正しく行われていることを確認する regression check。
    //
    // bridge_margin が dispatch に加わったが
    // margin unspecified の element では initial `Sides::all(Length::Px(0.0))`
    // が cascade で入る → taffy `LengthPercentageAuto::length(0.0)` に translate、
    // これは `taffy::Style::default().margin` (all `Length(0.0)`) と一致するため
    // 既存 assertion は無変更で通ることを確認する check にもなる。
    //
    // bridge_padding も dispatch に
    // 加わったが同様に padding unspecified の element では initial
    // `Sides::all(Length::Px(0.0))` → taffy `LengthPercentage::length(0.0)`
    // が入り、`taffy::Style::default().padding` と一致するため padding assertion
    // も無変更で通る pin。
    //
    // bridge_size (width) が dispatch に
    // 加わったが width unspecified の element は initial `LengthOrAuto::Auto`
    // → `Dimension::auto()` に translate、これは `taffy::Style::default().size`
    // (`Size::auto()`) の width と一致 (height は default 保持のまま追加予定)。
    // 既存 `size == default_style.size` 相当 assertion は変化なく通る。
    use raikiri_style::{build_rule_tree, cascade};

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), Some("display:none"));
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");

    apply_computed_to_style(&mut doc, &cr);
    assert_eq!(doc.nodes[body].style.display, Display::None);

    let default_style = <taffy::Style as Default>::default();
    assert_eq!(doc.nodes[body].style.size, default_style.size);
    assert_eq!(doc.nodes[body].style.margin, default_style.margin);
    assert_eq!(doc.nodes[body].style.padding, default_style.padding);
}

#[test]
fn apply_computed_to_style_tracks_logical_min_block_provenance() {
    use raikiri_style::{build_rule_tree, cascade};

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let avoided = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("min-block-size: 40px; break-inside: avoid"),
    );
    let auto = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("min-block-size: 40px; break-inside: auto"),
    );
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");

    apply_computed_to_style(&mut doc, &cr);
    assert!(doc.nodes[avoided].has_logical_min_block_size);
    assert!(!doc.nodes[auto].has_logical_min_block_size);
    assert_eq!(
        doc.nodes[avoided].style.min_size.height,
        LengthPercentageAuto::length(40.0)
    );
}

#[test]
fn prepare_multicol_layout_splits_direct_text_lines_across_columns() {
    use raikiri_style::{build_rule_tree, cascade};

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let container = doc.append_element(
            Some(body),
            "div",
            Style::default(),
            Some(
                "column-count:2;column-gap:20px;width:200px;font-size:10px;line-height:10px;white-space:pre-line",
            ),
        );
    let text = doc.append_text(container, "a\nb\nc\nd");

    doc.mark_in_document_flags();
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");
    apply_computed_to_style(&mut doc, &cr);
    let mut fonts = FontContext::new();
    let mut layout_cx = LayoutContext::<()>::new();
    preshape_text(&mut doc, &cr, &mut fonts, &mut layout_cx, 200.0, 200.0);
    assert_eq!(
        doc.nodes[text].text_layout().expect("text shaped").len(),
        4,
        "sanity: four pre-line segments produce four lines"
    );

    prepare_multicol_layout(&mut doc, &cr, 200.0);

    // width 200 / 2 columns with a 20px gap: (200 - 20) / 2 = 90.
    let fragments = doc.nodes[text]
        .multicol_fragments()
        .expect("direct text under a multicol container gets explicit line fragments")
        .to_vec();
    assert_eq!(fragments.len(), 2);
    assert_eq!(fragments[0].line_start, 0);
    assert_eq!(fragments[0].line_end, 2);
    assert!((fragments[0].x - 0.0).abs() < 0.001);
    assert_eq!(fragments[1].line_start, 2);
    assert_eq!(fragments[1].line_end, 4);
    assert!((fragments[1].x - 110.0).abs() < 0.001);
    assert_eq!(doc.nodes[text].style.size.width, Dimension::length(90.0));
    // 2 lines per column * 10px line-height.
    assert_eq!(
        doc.nodes[container].style.size.height,
        Dimension::length(20.0)
    );
}

#[test]
fn prepare_multicol_layout_projects_oversized_direct_br_children_as_wrapped_flex() {
    use raikiri_style::{build_rule_tree, cascade};

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let container = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("column-count:2;column-gap:20px;width:200px;height:20px"),
    );
    let tall_child = doc.append_element(
        Some(container),
        "div",
        Style::default(),
        Some("height:30px"),
    );
    doc.append_element(Some(tall_child), "br", Style::default(), None::<&str>);
    let auto_height_child = doc.append_element(
        Some(container),
        "div",
        Style::default(),
        Some("font-size:10px;line-height:10px"),
    );
    doc.append_element(
        Some(auto_height_child),
        "br",
        Style::default(),
        None::<&str>,
    );

    doc.mark_in_document_flags();
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");
    apply_computed_to_style(&mut doc, &cr);

    prepare_multicol_layout(&mut doc, &cr, 200.0);

    // A fixed-height auto-fill multicol with a direct child taller than
    // the container, and a direct `<br>` in every child, is projected as
    // a row-wrapping flex flow so each child owns one column's inline
    // slot while its block overflow remains visible.
    assert_eq!(doc.nodes[container].style.display, Display::Flex);
    assert_eq!(
        doc.nodes[container].style.flex_direction,
        TaffyFlexDirection::Row
    );
    assert_eq!(doc.nodes[container].style.flex_wrap, TaffyFlexWrap::Wrap);
    assert_eq!(
        doc.nodes[container].style.gap.width,
        LengthPercentage::length(20.0)
    );
    assert_eq!(
        doc.nodes[container].style.gap.height,
        LengthPercentage::length(0.0)
    );

    // width 200 / 2 columns with a 20px gap: (200 - 20) / 2 = 90.
    for child in [tall_child, auto_height_child] {
        assert_eq!(doc.nodes[child].style.flex_grow, 0.0);
        assert_eq!(doc.nodes[child].style.flex_shrink, 0.0);
        assert_eq!(doc.nodes[child].style.size.width, Dimension::length(90.0));
        assert_eq!(
            doc.nodes[child].style.min_size.width,
            LengthPercentageAuto::length(0.0)
        );
        assert_eq!(doc.nodes[child].style.flex_basis, Dimension::length(90.0));
    }
    // An explicit height is preserved rather than overwritten by the
    // `<br>` line-height projection...
    assert_eq!(
        doc.nodes[tall_child].style.size.height,
        Dimension::length(30.0)
    );
    // ...while a child with no explicit height gets one line's worth of
    // height from its own computed line-height.
    assert_eq!(
        doc.nodes[auto_height_child].style.size.height,
        Dimension::length(10.0)
    );
}

#[test]
fn apply_computed_to_style_bridges_direction_to_taffy() {
    use raikiri_style::{build_rule_tree, cascade};

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), Some("direction:rtl"));
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");

    apply_computed_to_style(&mut doc, &cr);

    assert_eq!(doc.nodes[body].style.direction, taffy::Direction::Rtl);
}

#[test]
fn collapse_block_in_inline_margins_ignores_whitespace_between_blocks() {
    use raikiri_style::{build_rule_tree, cascade};

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let inline = doc.append_element(Some(body), "div", Style::default(), Some("display:inline"));
    let before = doc.append_element(
        Some(inline),
        "div",
        Style::default(),
        Some("display:block; margin:16px 0"),
    );
    doc.append_text(inline, "\n   ");
    let after = doc.append_element(
        Some(inline),
        "div",
        Style::default(),
        Some("display:block; margin:16px 0"),
    );
    doc.mark_in_document_flags();
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");
    apply_computed_to_style(&mut doc, &cr);

    collapse_block_in_inline_margins(&mut doc, inline, &cr);

    assert_eq!(
        doc.nodes[before].style.margin.bottom,
        LengthPercentageAuto::length(16.0)
    );
    assert_eq!(
        doc.nodes[after].style.margin.top,
        LengthPercentageAuto::length(0.0)
    );
}

#[test]
fn establish_minimal_line_boxes_upgrades_qualifying_container_to_flex_row() {
    use raikiri_style::{build_rule_tree, cascade};

    // `build_rule_tree` deliberately excludes UA CSS (its own doc:
    // "UA CSS は含めない" — UA is injected via `raikiri_html::parse`
    // during real HTML parsing, which this hand-built-arena test
    // fixture bypasses). So every element's own display is declared
    // explicitly via inline style below, rather than relying on a
    // `p { display: block }` / `b { display: inline }` UA default
    // that would not actually be present in this fixture.
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let p = doc.append_element(Some(body), "p", Style::default(), Some("display: block"));
    let _a = doc.append_text(p, "A ");
    let b = doc.append_element(Some(p), "b", Style::default(), Some("display: inline"));
    let _b_text = doc.append_text(b, "B");
    let _c = doc.append_text(p, " C");

    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");
    apply_computed_to_style(&mut doc, &cr);

    assert_eq!(doc.nodes[p].style.display, Display::Flex);
    assert_eq!(doc.nodes[p].style.flex_direction, TaffyFlexDirection::Row);
    assert_eq!(doc.nodes[p].style.flex_wrap, TaffyFlexWrap::NoWrap);
    assert_eq!(
        doc.nodes[p].style.align_items,
        Some(TaffyAlignItems::BASELINE)
    );
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert!(
        doc.nodes[p].flags.contains(NodeFlags::IS_INLINE_ROOT),
        "qualifying container must have IS_INLINE_ROOT set"
    );
    // Participating children get flex_grow/flex_shrink pinned to 0 so
    // they neither grow nor compress narrower than their own shaped
    // content (see this pass's doc, "flex-shrink hazard").
    for &c in &doc.nodes[p].children {
        assert_eq!(doc.nodes[c].style.flex_grow, 0.0);
        assert_eq!(doc.nodes[c].style.flex_shrink, 0.0);
    }

    // `b` is a plain `inline` element (not `block`/`inline-block`), so
    // it does not itself qualify regardless of its own child count —
    // covered separately by
    // `establish_minimal_line_boxes_excludes_plain_inline_container`.
    // Here it just needs to remain block-container-shaped (i.e. its
    // own single Text child is laid out exactly as before this pass).
    assert_eq!(doc.nodes[b].style.display, Display::Block);
    assert!(!doc.nodes[b].flags.contains(NodeFlags::IS_INLINE_ROOT));
}

#[test]
fn establish_minimal_line_boxes_leaves_single_inline_child_container_on_block_path() {
    use raikiri_style::{build_rule_tree, cascade};

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let p = doc.append_element(Some(body), "p", Style::default(), Some("display: block"));
    let _a = doc.append_text(p, "A");

    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");
    apply_computed_to_style(&mut doc, &cr);

    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        doc.nodes[p].style.display,
        Display::Block,
        "a single inline-level child is geometrically degenerate \
             (nothing to place beside it) — must be left on the plain \
             block path, not upgraded to a 1-item flex row"
    );
    assert!(!doc.nodes[p].flags.contains(NodeFlags::IS_INLINE_ROOT));
}

#[test]
fn establish_minimal_line_boxes_leaves_mixed_block_and_inline_content_untouched() {
    use raikiri_style::{build_rule_tree, cascade};

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let p = doc.append_element(Some(body), "p", Style::default(), Some("display: block"));
    let _text = doc.append_text(p, "A");
    let _div = doc.append_element(Some(p), "div", Style::default(), Some("display: block"));

    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");
    apply_computed_to_style(&mut doc, &cr);

    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        doc.nodes[p].style.display,
        Display::Block,
        "a block-level sibling among inline-level content must \
             disqualify minimal-line-box treatment (mixed content is out \
             of scope for this pass)"
    );
    assert!(!doc.nodes[p].flags.contains(NodeFlags::IS_INLINE_ROOT));
}

#[test]
fn establish_minimal_line_boxes_hidden_sibling_does_not_count_toward_threshold() {
    use raikiri_style::{build_rule_tree, cascade};

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let p = doc.append_element(Some(body), "p", Style::default(), Some("display: block"));
    let _a = doc.append_text(p, "A");
    let _hidden = doc.append_element(Some(p), "span", Style::default(), Some("display:none"));

    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");
    apply_computed_to_style(&mut doc, &cr);

    // Only 1 *counted* inline-level child (the display:none sibling
    // doesn't count) — below the 2-child threshold.
    assert_eq!(doc.nodes[p].style.display, Display::Block);
}

#[test]
fn establish_minimal_line_boxes_hidden_sibling_does_not_disqualify_when_threshold_met() {
    use raikiri_style::{build_rule_tree, cascade};

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let p = doc.append_element(Some(body), "p", Style::default(), Some("display: block"));
    let _a = doc.append_text(p, "A");
    let _hidden = doc.append_element(Some(p), "span", Style::default(), Some("display:none"));
    let _c = doc.append_text(p, "C");

    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");
    apply_computed_to_style(&mut doc, &cr);

    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        doc.nodes[p].style.display,
        Display::Flex,
        "a display:none sibling must not block qualification once the \
             2 visible-inline-level threshold is otherwise met"
    );
}

#[test]
fn qualifies_for_minimal_line_box_skips_not_in_document_sibling() {
    // Regression check for the `if !doc.nodes[c].is_in_document() {
    // continue; }` guard in `qualifies_for_minimal_line_box`: a
    // Comment sibling is reachable in the raw arena tree but always
    // has `IS_IN_DOCUMENT` cleared by `mark_in_document_flags`
    // (`NodeData::Comment`'s doc) — it must be skipped entirely
    // (neither counted nor disqualifying) while the 2 real Text
    // children still meet the threshold.
    use raikiri_style::{build_rule_tree, cascade};

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let p = doc.append_element(Some(body), "p", Style::default(), Some("display: block"));
    let _a = doc.append_text(p, "A");
    let comment = doc.append_comment(Some(p), "not rendered");
    let _c = doc.append_text(p, "C");
    doc.mark_in_document_flags();
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert!(
        !doc.nodes[comment].is_in_document(),
        "fixture precondition: the comment sibling must actually be \
             not-in-document for this test to exercise the guard"
    );

    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");
    apply_computed_to_style(&mut doc, &cr);

    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        doc.nodes[p].style.display,
        Display::Flex,
        "a not-in-document sibling (e.g. a Comment) must be skipped \
             entirely by the is_in_document() guard — neither counted \
             nor disqualifying — while the 2 real Text children still \
             meet the threshold"
    );
}

#[test]
fn establish_minimal_line_boxes_excludes_plain_inline_container() {
    use raikiri_style::{build_rule_tree, cascade};

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    // 2 text children, but `b` itself must not qualify: a plain
    // `inline` node's content flows into its ancestor's line box, it
    // does not get its own (this module's doc, "does not qualify
    // here even with 2+ inline-level children").
    let b = doc.append_element(Some(body), "b", Style::default(), Some("display: inline"));
    let _x = doc.append_text(b, "X");
    let _y = doc.append_text(b, "Y");

    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");
    apply_computed_to_style(&mut doc, &cr);

    assert_eq!(doc.nodes[b].style.display, Display::Block);
    assert!(!doc.nodes[b].flags.contains(NodeFlags::IS_INLINE_ROOT));
}

#[test]
fn establish_minimal_line_boxes_bridges_nested_plain_inline_children() {
    use raikiri_style::{build_rule_tree, cascade};

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let outer = doc.append_element(
        Some(body),
        "span",
        Style::default(),
        Some("display: inline"),
    );
    let inner = doc.append_element(Some(outer), "b", Style::default(), Some("display: inline"));
    let _text = doc.append_text(inner, "永X");

    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");
    apply_computed_to_style(&mut doc, &cr);

    assert!(doc.nodes[outer].flags.contains(NodeFlags::IS_INLINE_ROOT));
    assert!(!doc.nodes[inner].flags.contains(NodeFlags::IS_INLINE_ROOT));
    assert_eq!(doc.nodes[outer].style.display, Display::Flex);
    assert_eq!(doc.nodes[inner].style.display, Display::Block);
}

#[test]
fn establish_minimal_line_boxes_scopes_autospace_to_the_inline_context() {
    use raikiri_style::{build_rule_tree, cascade};

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let trigger = doc.append_element(Some(body), "div", Style::default(), Some("display: block"));
    let _trigger_text = doc.append_text(trigger, "永X");

    let unrelated = doc.append_element(Some(body), "div", Style::default(), Some("display: block"));
    let outer = doc.append_element(
        Some(unrelated),
        "span",
        Style::default(),
        Some("display: inline; text-autospace: no-autospace"),
    );
    let inner = doc.append_element(Some(outer), "b", Style::default(), Some("display: inline"));
    let _text = doc.append_text(inner, "永X");

    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");
    apply_computed_to_style(&mut doc, &cr);

    assert!(!doc.nodes[outer].flags.contains(NodeFlags::IS_INLINE_ROOT));
    assert_eq!(doc.nodes[outer].style.display, Display::Block);
}

#[test]
fn establish_minimal_line_boxes_bridges_stylesheet_authored_inline_wrapper() {
    use raikiri_style::{Origin, build_rule_tree, cascade};

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let outer = doc.append_element(Some(body), "x-inline", Style::default(), None::<&str>);
    let inner = doc.append_element(
        Some(outer),
        "span",
        Style::default(),
        Some("display: inline"),
    );
    let _text = doc.append_text(inner, "永X");

    let mut rules = build_rule_tree(&doc);
    rules.add_stylesheet("x-inline { display: inline !important; }", Origin::Author);
    let cr = cascade(&doc, &rules).expect("cascade Ok");
    apply_computed_to_style(&mut doc, &cr);

    assert!(doc.nodes[outer].flags.contains(NodeFlags::IS_INLINE_ROOT));
    assert_eq!(doc.nodes[outer].style.display, Display::Flex);
}

#[test]
fn establish_minimal_line_boxes_qualifies_inline_block_container() {
    use raikiri_style::{build_rule_tree, cascade};

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let ib = doc.append_element(
        Some(body),
        "span",
        Style::default(),
        Some("display:inline-block"),
    );
    let _x = doc.append_text(ib, "X");
    let _y = doc.append_text(ib, "Y");

    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");
    apply_computed_to_style(&mut doc, &cr);

    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        doc.nodes[ib].style.display,
        Display::Flex,
        "inline-block generates a block container for its own content, \
             same as block — it must qualify just like a block container"
    );
    assert!(doc.nodes[ib].flags.contains(NodeFlags::IS_INLINE_ROOT));
}

#[test]
fn establish_minimal_line_boxes_lays_out_children_side_by_side_not_stacked() {
    // End-to-end check (via the full `layout_single_page` pipeline, not
    // just `apply_computed_to_style` in isolation) that qualifying
    // inline-level siblings actually end up beside each other, not
    // independently stacked — the concrete geometry bug this pass
    // fixes.
    use parley::FontContext;
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let p = doc.append_element(Some(body), "p", Style::default(), Some("display: block"));
    let a = doc.append_text(p, "AAAA");
    let c = doc.append_text(p, "CCCC");

    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");
    layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("layout Ok");

    let a_loc = doc.nodes[a].unrounded_layout;
    let c_loc = doc.nodes[c].unrounded_layout;
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert!(
        (a_loc.location.y - c_loc.location.y).abs() < 1e-3,
        "both text children must sit on the same line (same y), got \
             a.y={} c.y={}",
        a_loc.location.y,
        c_loc.location.y
    );
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert!(
        c_loc.location.x >= a_loc.location.x + a_loc.size.width - 1e-3,
        "second child must start at or after the first child's right \
             edge (side-by-side placement), got a.x={} a.w={} c.x={}",
        a_loc.location.x,
        a_loc.size.width,
        c_loc.location.x
    );
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert!(
        c_loc.location.x > 0.0,
        "second child must not sit at x=0 — that would mean it's \
             still being independently stacked below the first child \
             rather than placed beside it"
    );
}

#[test]
fn text_align_center_on_flex_line_box_uses_justify_content() {
    // qualify する container (2+ inline-level children) の中央寄せは
    // container 側の `justify_content: Center` で実現すること
    // (parley 側ではなく flex 側 — `realign_text_after_layout` doc 参照)。
    use raikiri_style::{build_rule_tree, cascade};

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let p = doc.append_element(
        Some(body),
        "p",
        Style::default(),
        Some("display: block; text-align: center"),
    );
    let _a = doc.append_text(p, "AAAA");
    let _c = doc.append_text(p, "CCCC");

    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");
    apply_computed_to_style(&mut doc, &cr);
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        doc.nodes[p].style.justify_content,
        Some(TaffyAlignContent::CENTER),
        "centered line-box container must center via flex justify_content"
    );

    // end-to-end: line box 全体が container 中央に寄ること。
    use parley::FontContext;
    use raikiri_traits::PageBox;
    layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("layout Ok");
    let p_w = doc.nodes[p].unrounded_layout.size.width;
    let a_loc = doc.nodes[_a].unrounded_layout;
    let c_loc = doc.nodes[_c].unrounded_layout;
    let total = (c_loc.location.x + c_loc.size.width) - a_loc.location.x;
    let expected_left = (p_w - total) * 0.5;
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert!(
        (a_loc.location.x - expected_left).abs() < 2.0,
        "centered line box must start near (p_w-total)/2, got a.x={} expected={} (p_w={} total={})",
        a_loc.location.x,
        expected_left,
        p_w,
        total
    );
}

#[test]
fn establish_minimal_line_boxes_resets_conflicting_align_self() {
    // Regression check for the `align_self: None` reset documented on
    // `establish_minimal_line_boxes`. `b` below carries an explicit
    // `align-self: flex-end` — inert while `p` was a plain block
    // container, a live cross-axis override once `p` qualifies here,
    // absent this reset (it would otherwise diverge from the
    // container's own `align_items: FlexStart`).
    use raikiri_style::{build_rule_tree, cascade};

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let p = doc.append_element(Some(body), "p", Style::default(), Some("display: block"));
    let _a = doc.append_text(p, "A");
    let b = doc.append_element(
        Some(p),
        "b",
        Style::default(),
        Some("display: inline; align-self: flex-end"),
    );
    let _c = doc.append_text(b, "B");

    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");
    apply_computed_to_style(&mut doc, &cr);

    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        doc.nodes[b].style.align_self, None,
        "`b`'s explicit align-self:flex-end must be reset to None \
             (= auto) once `p` qualifies for minimal-line-box treatment, \
             so `b` falls back to `p`'s own align_items:FlexStart \
             instead of diverging from it"
    );
}

#[test]
fn establish_minimal_line_boxes_br_switches_container_to_wrap_and_forces_full_basis() {
    // Unit-level check for the "`<br>` forced break" mechanism documented
    // on `establish_minimal_line_boxes`: a qualifying container with a
    // participating `<br>` switches from `flex_wrap: NoWrap` to `Wrap`,
    // and only the `<br>` child (not its siblings) gets its flex_basis
    // forced to 100%.
    use raikiri_style::{build_rule_tree, cascade};

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let p = doc.append_element(Some(body), "p", Style::default(), Some("display: block"));
    let a = doc.append_text(p, "A");
    let br = doc.append_element(Some(p), "br", Style::default(), None::<&str>);
    let c = doc.append_text(p, "C");

    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");
    apply_computed_to_style(&mut doc, &cr);

    assert_eq!(doc.nodes[p].style.display, Display::Flex);
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        doc.nodes[p].style.flex_wrap,
        TaffyFlexWrap::Wrap,
        "a qualifying container with a participating <br> must enable \
             flex_wrap so taffy's own line-packing algorithm can place the \
             <br> (and anything after it) onto a new line"
    );
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        doc.nodes[br].style.flex_basis,
        Dimension::percent(1.0),
        "<br> itself must get flex_basis:100% — the hypothetical main \
             size that never fits alongside a non-empty line, forcing it \
             (and everything after it) onto a new line"
    );
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        doc.nodes[a].style.flex_basis,
        Dimension::auto(),
        "a plain sibling text node must keep the ordinary content-based \
             flex_basis — only <br> itself gets the 100% override"
    );
    assert_eq!(doc.nodes[c].style.flex_basis, Dimension::auto());
}

#[test]
fn establish_minimal_line_boxes_display_none_br_does_not_switch_to_wrap() {
    // A `display:none` `<br>` generates no box at all (CSS Display 4
    // §2's "the element and its descendants generate no boxes"), so it
    // must not be treated as a forced break — otherwise a container
    // with no visually-effective `<br>` would still gain
    // `flex_wrap: Wrap`, silently enabling size-based wrapping
    // (`establish_minimal_line_boxes`'s Non-goals doc) for content that
    // never asked for it.
    use raikiri_style::{build_rule_tree, cascade};

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let p = doc.append_element(Some(body), "p", Style::default(), Some("display: block"));
    let _a = doc.append_text(p, "A");
    let _br = doc.append_element(Some(p), "br", Style::default(), Some("display: none"));
    let _c = doc.append_text(p, "C");

    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");
    apply_computed_to_style(&mut doc, &cr);

    assert_eq!(doc.nodes[p].style.display, Display::Flex);
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        doc.nodes[p].style.flex_wrap,
        TaffyFlexWrap::NoWrap,
        "a display:none <br> must not count as a forced break"
    );
}

#[test]
fn establish_minimal_line_boxes_br_line_stays_zero_height_under_explicit_tall_container_height() {
    // Regression check for a property-leak this pass must guard against:
    // `bridge_size` unconditionally copies an author `height` into
    // `style.size.height`, and `bridge_alignment` unconditionally
    // copies an author `align-content` into `style.align_content`
    // (`None` when unauthored). Once this container becomes a
    // multi-line flex container (this pass's "`<br>` forced break"),
    // an explicit `height` taller than the natural content height
    // leaves leftover cross space that taffy's own default
    // (`align_content: Stretch` when unset) would distribute across
    // ALL flex lines — including the zero-height line the `<br>`
    // alone occupies — growing it above 0 and inserting a visible gap
    // between the two real text lines. This container's own
    // `align_content` must be reset (mirroring the `align_items`
    // reset a few lines up) so that leftover space is not distributed
    // onto lines at all.
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
        Some("display: block; height: 200px"),
    );
    let a = doc.append_text(p, "AAAA");
    let br = doc.append_element(Some(p), "br", Style::default(), None::<&str>);
    let c = doc.append_text(p, "CCCC");

    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");
    layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("layout Ok");

    let p_loc = doc.nodes[p].unrounded_layout;
    let a_loc = doc.nodes[a].unrounded_layout;
    let br_loc = doc.nodes[br].unrounded_layout;
    let c_loc = doc.nodes[c].unrounded_layout;

    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert!(
        (p_loc.size.height - 200.0).abs() < 1e-3,
        "fixture precondition: explicit height:200px must actually \
             take effect (and exceed the two text lines' natural height), \
             got p.h={}",
        p_loc.size.height
    );
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        br_loc.size.height, 0.0,
        "the <br>'s own line must stay at 0 height even when the \
             container's explicit height leaves leftover cross space — \
             that space must not stretch onto any line, got br.h={}",
        br_loc.size.height
    );
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert!(
        (c_loc.location.y - (a_loc.location.y + a_loc.size.height)).abs() < 1e-3,
        "no visible gap between the two real lines: the second \
             line's top must sit exactly at the first line's bottom \
             edge, got a.y={} a.h={} c.y={}",
        a_loc.location.y,
        a_loc.size.height,
        c_loc.location.y
    );
}

#[test]
fn establish_minimal_line_boxes_br_forces_second_line_on_inline_block_container() {
    // Same forced-break geometry as
    // `establish_minimal_line_boxes_br_forces_second_line_end_to_end`,
    // but on a qualifying `inline-block` container rather than `block`
    // — pins that the mechanism does not depend on which of the two
    // qualifying display values establishes the line box (this crate's
    // block layout gives both a definite available main size from
    // their own containing block; see `bridge_display`'s doc, neither
    // display value gets shrink-to-fit sizing here).
    use parley::FontContext;
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let ib = doc.append_element(
        Some(body),
        "span",
        Style::default(),
        Some("display:inline-block"),
    );
    let a = doc.append_text(ib, "AAAA");
    let br = doc.append_element(Some(ib), "br", Style::default(), None::<&str>);
    let c = doc.append_text(ib, "CCCCCCCCCCCCCCCCCCCC");

    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");
    layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("layout Ok");

    let a_loc = doc.nodes[a].unrounded_layout;
    let br_loc = doc.nodes[br].unrounded_layout;
    let c_loc = doc.nodes[c].unrounded_layout;
    let ib_loc = doc.nodes[ib].unrounded_layout;

    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert!(
        c_loc.location.y >= a_loc.location.y + a_loc.size.height - 1e-3,
        "got a.y={} a.h={} c.y={}",
        a_loc.location.y,
        a_loc.size.height,
        c_loc.location.y
    );
    assert_eq!(br_loc.size.height, 0.0);
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert!(
        (ib_loc.size.height - (a_loc.size.height + c_loc.size.height)).abs() < 1e-3,
        "got ib.h={} a.h={} c.h={}",
        ib_loc.size.height,
        a_loc.size.height,
        c_loc.size.height
    );
}

#[test]
fn apply_computed_to_style_reserves_inside_list_marker_gutter() {
    use raikiri_style::{build_rule_tree, cascade};

    let mut doc = Document::new();
    let inside = doc.append_element(
        Some(0),
        "li",
        Style::default(),
        Some("display: list-item; list-style-position: inside"),
    );
    let outside = doc.append_element(
        Some(0),
        "li",
        Style::default(),
        Some("display: list-item; list-style-position: outside"),
    );
    let explicit_padding = doc.append_element(
        Some(0),
        "li",
        Style::default(),
        Some("display: list-item; list-style-position: inside; padding-left: 5px"),
    );
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");

    apply_computed_to_style(&mut doc, &cr);

    assert_eq!(
        doc.nodes[inside].style.padding.left.into_raw().value(),
        24.0
    );
    assert_eq!(
        doc.nodes[outside].style.padding.left.into_raw().value(),
        0.0
    );
    assert_eq!(
        doc.nodes[explicit_padding]
            .style
            .padding
            .left
            .into_raw()
            .value(),
        5.0
    );
}

#[test]
fn apply_computed_to_style_bridges_margin_to_taffy() {
    // bridge_margin が
    // Sides<ComputedLengthPercentageOrAuto> を taffy::Rect<LengthPercentageAuto>
    // に translate することを確認する regression check。bridge の 3 分岐
    // (Px / Percent / Auto) をそれぞれ 1 case で covering。
    //
    // Case 4 の `pt` は bridge の分岐ではなくなった
    // (cascade の phase 3 が px に絶対化する) が、end-to-end の期待値は
    // 変わらないので test は残す。
    //
    // Test 戦略: 各 case は独立 fixture で cascade → apply_computed_to_style
    // → body.style.margin を assert。inline style 経由なので raikiri-style
    // の parse_margin_shorthand + longhand path も同時に regression check。
    use raikiri_style::{build_rule_tree, cascade};
    use taffy::{LengthPercentageAuto, Rect};

    fn margin_for(inline: &str) -> Rect<LengthPercentageAuto> {
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
        let body = doc.append_element(Some(html), "body", Style::default(), Some(inline));
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");
        apply_computed_to_style(&mut doc, &cr);
        doc.nodes[body].style.margin
    }

    // Case 1: shorthand `margin: 10px 20px 30px 40px` (top/right/bottom/left)
    //   → Rect { top: 10, right: 20, bottom: 30, left: 40 } (all Px identity)。
    //   Sides.top,right,bottom,left → Rect.top,right,bottom,left の field-name
    //   mapping を check (positional silent transpose を防ぐ)。
    assert_eq!(
        margin_for("margin: 10px 20px 30px 40px"),
        Rect {
            top: LengthPercentageAuto::length(10.0),
            right: LengthPercentageAuto::length(20.0),
            bottom: LengthPercentageAuto::length(30.0),
            left: LengthPercentageAuto::length(40.0),
        }
    );

    // Case 2: shorthand `margin: auto` → 4 side 全て auto()。
    assert_eq!(
        margin_for("margin: auto"),
        Rect {
            top: LengthPercentageAuto::auto(),
            right: LengthPercentageAuto::auto(),
            bottom: LengthPercentageAuto::auto(),
            left: LengthPercentageAuto::auto(),
        }
    );

    // Case 3: longhand `margin-left: 50%` → left = percent(0.5)、他 3 side は
    //   initial (0.0 px)。CSS spec の authored 0-100 → taffy fraction 0.0-1.0
    //   の div-by-100 policy を pin。
    assert_eq!(
        margin_for("margin-left: 50%"),
        Rect {
            top: LengthPercentageAuto::length(0.0),
            right: LengthPercentageAuto::length(0.0),
            bottom: LengthPercentageAuto::length(0.0),
            left: LengthPercentageAuto::percent(0.5),
        }
    );

    // Case 4: longhand `margin-top: 10pt` → top = length(10 * 4/3) = length(13.333...)。
    //   CSS Values 4 §6.2 の `1pt = 4/3 px` (1pt=1/72in、1in=96px → 96/72=4/3)。
    //   **この変換は bridge ではなく cascade の phase 3
    //   (`raikiri_style::resolve_length_percentage_or_auto`) が行う**。
    //   bridge に届く時点で既に px。本 case は
    //   end-to-end の値を check する。
    //   f32 bit-identical assert のため右辺を expression のまま書く
    //   (`13.333` literal は round-trip で drift する。この式は
    //   `resolve::pt_to_px` 本体と同じ `v * 4.0 / 3.0` の評価順を使う —
    //   f32 は結合則を満たさないため簡約すると bit が変わる。詳細は
    //   `resolve::pt_to_px` の doc 参照)。
    assert_eq!(
        margin_for("margin-top: 10pt"),
        Rect {
            top: LengthPercentageAuto::length(10.0 * 4.0 / 3.0),
            right: LengthPercentageAuto::length(0.0),
            bottom: LengthPercentageAuto::length(0.0),
            left: LengthPercentageAuto::length(0.0),
        }
    );
}

#[test]
fn apply_computed_to_style_bridges_padding_to_taffy() {
    // bridge_padding が
    // Sides<ComputedLengthPercentage> を taffy::Rect<LengthPercentage> に
    // translate することを確認する regression check。padding は margin と違い
    // `auto` を持たない (<length-percentage `[0,∞]`>) ため bridge は **2 arm**
    // (Px / Percent) で網羅する。
    //
    // Case 3 の `pt` は **bridge の分岐ではなくなった**
    // (cascade の phase 3 が px に絶対化する) が、end-to-end の期待値は
    // 変わらないので test は残す。
    //
    // Test 戦略: 各 case は独立 fixture で cascade → apply_computed_to_style
    // → body.style.padding を assert。inline style 経由なので raikiri-style
    // の parse_padding_shorthand + longhand path も同時に regression check。
    use raikiri_style::{build_rule_tree, cascade};

    fn padding_for(inline: &str) -> Rect<LengthPercentage> {
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
        let body = doc.append_element(Some(html), "body", Style::default(), Some(inline));
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");
        apply_computed_to_style(&mut doc, &cr);
        doc.nodes[body].style.padding
    }

    // Case 1: shorthand `padding: 5px 10px 15px 20px` (top/right/bottom/left)
    //   → Rect { top: 5, right: 10, bottom: 15, left: 20 } (all Px identity)。
    //   Sides.top,right,bottom,left → Rect.top,right,bottom,left の field-name
    //   mapping を check (positional silent transpose を防ぐ — Sides の field 順は
    //   top,right,bottom,left、Rect の field 順は left,right,top,bottom で異なる)。
    assert_eq!(
        padding_for("padding: 5px 10px 15px 20px"),
        Rect {
            top: LengthPercentage::length(5.0),
            right: LengthPercentage::length(10.0),
            bottom: LengthPercentage::length(15.0),
            left: LengthPercentage::length(20.0),
        }
    );

    // Case 2: longhand `padding-left: 5%` → left = percent(0.05)、他 3 side は
    //   initial (0.0 px)。CSS spec の authored 0-100 → taffy fraction 0.0-1.0
    //   の div-by-100 policy を pin。
    assert_eq!(
        padding_for("padding-left: 5%"),
        Rect {
            top: LengthPercentage::length(0.0),
            right: LengthPercentage::length(0.0),
            bottom: LengthPercentage::length(0.0),
            left: LengthPercentage::percent(0.05),
        }
    );

    // Case 3: longhand `padding-top: 3pt` → top = length(3 * 4/3) = length(4.0)。
    //   CSS Values 4 §6.2 の `1pt = 4/3 px` (1pt=1/72in、1in=96px → 96/72=4/3)。
    //   **変換の所在は cascade の phase 3** (`resolve_length_percentage`) で
    //   bridge ではない。
    //   f32 bit-identical assert のため右辺を expression のまま書く
    //   (`4.0` literal は 3*4/3 と bit-identical だが policy 明示のため式のまま)。
    assert_eq!(
        padding_for("padding-top: 3pt"),
        Rect {
            top: LengthPercentage::length(3.0 * 4.0 / 3.0),
            right: LengthPercentage::length(0.0),
            bottom: LengthPercentage::length(0.0),
            left: LengthPercentage::length(0.0),
        }
    );
}

#[test]
fn apply_computed_to_style_bridges_width_to_taffy() {
    // bridge_size (width component)
    // が cv.width: ComputedLengthPercentageOrAuto を taffy::Style::size.width:
    // Dimension に translate することを check する。bridge の 3 分岐
    // (Px / Percent / Auto) をそれぞれ 1 case で covering
    // (`pt` は cascade の phase 3 で px 化される)。
    //
    // Test 戦略: fixture は **非 body element** (この場合 `<p>`) を使う —
    // `<body>` は後段 `apply_page_box_to_body` で clobber されるため本 bridge
    // の効果は observable でない (別 test `apply_page_box_clobbers_body_width_from_bridge`
    // で clobber 挙動を pin)。inline style 経由なので raikiri-style の
    // parse_width path + ComputedLengthPercentageOrAuto encoding も同時に
    // regression check。
    //
    // 本 test は width 軸に絞る — height 軸は sibling test
    // `apply_computed_to_style_bridges_height_to_taffy`
    // が同 fixture pattern で LengthOrAuto → Dimension bridge を check する。
    use raikiri_style::{build_rule_tree, cascade};

    fn width_for(inline: &str) -> Dimension {
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
        let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
        // 非 body element (p) に inline を載せる。apply_page_box_to_body は
        // body だけを触るため、p の style.size は bridge 実行後そのまま観測可能。
        let p = doc.append_element(Some(body), "p", Style::default(), Some(inline));
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");
        apply_computed_to_style(&mut doc, &cr);
        doc.nodes[p].style.size.width
    }

    // Case 1: `width: 100px` → Dimension::length(100.0) (Px identity)。
    assert_eq!(width_for("width: 100px"), Dimension::length(100.0));

    // Case 2: `width: auto` → Dimension::auto()
    //   (ComputedLengthPercentageOrAuto::Auto arm)。
    assert_eq!(width_for("width: auto"), Dimension::auto());

    // Case 3: `width: 50%` → Dimension::percent(0.5)。CSS spec の authored
    //   0-100 → taffy fraction 0.0-1.0 の div-by-100 policy を pin。
    assert_eq!(width_for("width: 50%"), Dimension::percent(0.5));

    // Case 4: `width: 20pt` → Dimension::length(20 * 4/3) = length(26.666...)。
    //   CSS Values 4 §6.2 の `1pt = 4/3 px` (1pt=1/72in、1in=96px → 96/72=4/3)。
    //   **変換の所在は cascade の phase 3** で bridge ではない。
    //   f32 bit-identical assert のため右辺を expression で書く。
    assert_eq!(
        width_for("width: 20pt"),
        Dimension::length(20.0 * 4.0 / 3.0)
    );
}

#[test]
fn apply_computed_to_style_bridges_height_to_taffy() {
    // bridge_size の height 側
    // 拡張。cv.height: ComputedLengthPercentageOrAuto を
    // taffy::Style::size.height: Dimension に translate することを check する。
    // sibling test `apply_computed_to_style_bridges_width_to_taffy` と
    // 対を成し、struct literal 化 (Size { width, height } の 1 発
    // assign) で height 側の 3 分岐 (Px / Percent / Auto) が意図通り
    // 書き込まれるか確認する。
    //
    // Test 戦略: fixture は **非 body element** (`<p>`) を使う — `<body>` は
    // 後段 `apply_page_box_to_body` で height も clobber されるため本 bridge
    // の効果は body 上で observable でない。inline style 経由で raikiri-style
    // の parse_height path + ComputedLengthPercentageOrAuto encoding も同時に
    // regression check。
    //
    // Pt case は sibling width test が同じ
    // computed_length_percentage_or_auto_to_taffy_dimension policy を
    // check しているため redundant (かつ pt → px 変換は
    // cascade の phase 3 の責務)。ここでは height
    // 特有の 3 arm (auto default 保持、`Px` 通路、`Percent` 通路) に絞る。
    use raikiri_style::{build_rule_tree, cascade};

    fn height_for(inline: &str) -> Dimension {
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
        let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
        // 非 body element (p) に inline を載せる。apply_page_box_to_body は
        // body だけを触るため、p の style.size は bridge 実行後そのまま観測可能。
        let p = doc.append_element(Some(body), "p", Style::default(), Some(inline));
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");
        apply_computed_to_style(&mut doc, &cr);
        doc.nodes[p].style.size.height
    }

    // Case 1: `height: 100px` → Dimension::length(100.0) (Px identity)。
    assert_eq!(height_for("height: 100px"), Dimension::length(100.0));

    // Case 2: `height: auto` → Dimension::auto()
    //   (ComputedLengthPercentageOrAuto::Auto arm)。
    //   CSS Sizing 3 §3.1.1 initial `height: auto` の identity round-trip pin。
    assert_eq!(height_for("height: auto"), Dimension::auto());

    // Case 3: `height: 50%` → Dimension::percent(0.5)。CSS spec の authored
    //   0-100 → taffy fraction 0.0-1.0 の div-by-100 policy を pin。
    assert_eq!(height_for("height: 50%"), Dimension::percent(0.5));
}

#[test]
fn apply_computed_to_style_bridges_min_size_to_taffy() {
    // bridge_min_max_size の min 側。cv.min_width / cv.min_height:
    // ComputedLengthPercentageOrAuto を taffy::Style::min_size:
    // Size<Dimension> に translate することを check する。sibling test
    // `apply_computed_to_style_bridges_height_to_taffy` と同 fixture
    // pattern (非 body element `<p>` — body は apply_page_box_to_body
    // が size のみ clobber し min/max には触らないが、size 系 test と
    // 同じ fixture に揃える)。
    //
    // CSS Sizing 3 §4 initial `auto` の identity round-trip (unspecified
    // → taffy default と一致) も同時に check — bridge が unspecified 時に
    // default を壊さないことの regression guard。
    use raikiri_style::{build_rule_tree, cascade};

    fn min_for(inline: Option<&str>) -> Size<LengthPercentageAuto> {
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
        let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
        let p = doc.append_element(Some(body), "p", Style::default(), inline);
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");
        apply_computed_to_style(&mut doc, &cr);
        doc.nodes[p].style.min_size
    }

    // Case 1: unspecified → taffy default (min initial `auto` round-trip)。
    assert_eq!(min_for(None), <taffy::Style as Default>::default().min_size);

    // Case 2: `min-width: 100px; min-height: 50%` → length + percent。
    assert_eq!(
        min_for(Some("min-width: 100px; min-height: 50%")),
        Size {
            width: LengthPercentageAuto::length(100.0),
            height: LengthPercentageAuto::percent(0.5),
        }
    );

    // Case 3: `min-width: auto` → LengthPercentageAuto::auto() (no minimum)。
    assert_eq!(
        min_for(Some("min-width: auto")).width,
        LengthPercentageAuto::auto()
    );

    // Case 4: 負値は grammar `[0,∞]` 違反で declaration drop → Auto のまま。
    assert_eq!(
        min_for(Some("min-width: -10px")).width,
        LengthPercentageAuto::auto()
    );
}

#[test]
fn apply_computed_to_style_bridges_max_size_to_taffy() {
    // bridge_min_max_size の max 側。cv.max_width / cv.max_height を
    // taffy::Style::max_size: Size<Dimension> に translate することを check
    // する。sibling min test と同 fixture pattern。
    //
    // CSS Sizing 3 §5 initial `none` → computed Auto placeholder →
    // `Dimension::auto()` (no max) の連鎖を check — unspecified が taffy
    // default と一致することも同時に確認する。
    use raikiri_style::{build_rule_tree, cascade};

    fn max_for(inline: Option<&str>) -> Size<LengthPercentageAuto> {
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
        let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
        let p = doc.append_element(Some(body), "p", Style::default(), inline);
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");
        apply_computed_to_style(&mut doc, &cr);
        doc.nodes[p].style.max_size
    }

    // Case 1: unspecified → taffy default (max initial `none` round-trip)。
    assert_eq!(max_for(None), <taffy::Style as Default>::default().max_size);

    // Case 2: `max-width: 100px; max-height: 50%` → length + percent。
    assert_eq!(
        max_for(Some("max-width: 100px; max-height: 50%")),
        Size {
            width: LengthPercentageAuto::length(100.0),
            height: LengthPercentageAuto::percent(0.5),
        }
    );

    // Case 3: calc() remains a live Taffy handle instead of becoming zero.
    assert!(
        max_for(Some("max-width: calc(100px + 1%)"))
            .width
            .into_raw()
            .is_calc()
    );

    // Case 4: `max-width: none` → LengthPercentageAuto::auto() (no max)。
    assert_eq!(
        max_for(Some("max-width: none")).width,
        LengthPercentageAuto::auto()
    );

    // Case 5: 負値は grammar `[0,∞]` 違反で declaration drop → Auto のまま。
    assert_eq!(
        max_for(Some("max-height: -10px")).height,
        LengthPercentageAuto::auto()
    );
}

#[test]
fn apply_computed_to_style_bridges_border_to_taffy() {
    // bridge_border が
    // Sides<ComputedBorder> を taffy::Rect<LengthPercentage> に translate
    // することを確認する regression check。
    //
    // spec correctness gate: border-style が `none` / `hidden` の場合、
    // specified border-width にかかわらず width は 0 でなければならない。
    // **gate の所在は本 bridge ではなく上流の
    // `raikiri_style::resolve_border` (computed 層)** — CSS Backgrounds 3
    // §3.3 "Line Thickness: the border-width properties"
    // <https://www.w3.org/TR/css-backgrounds-3/#border-width> の propdef が
    // "Computed value: absolute length, snapped as a border width; zero if
    // the border style is none or hidden" と規定するため (TR 版 — ED は
    // CSSWG Issue 11494 で resolved-value 効果へ移動済)
    // (used 層から computed 層へ移動済で、bridge 側の
    // `used_border_width` helper は削除済)。§3.2 "Line Patterns: the
    // border-style properties"
    // <https://www.w3.org/TR/css-backgrounds-3/#border-style> の `none` も
    // "No border. Color and width are ignored (i.e., the border has width
    // 0)." と整合する。
    //
    // 本 test は依然 gating の **end-to-end** check である (gate が上流に
    // 移っても `5px none red` の 5px が taffy に leak しないことを保証する
    // のが目的)。§ 番号と引用は spec の `data-level` / 本文実測に基づく —
    // 以前あった "§5.2 The used values of the corresponding border-*-width
    // become 0." は css-backgrounds-3 に存在しない文だったので差し替えた。
    //
    // Test 戦略: `border: <w> <s> <c>` 4-side shorthand と longhand の
    // 両方を使い、shorthand 展開 → per-side cascade → bridge_border の
    // pipeline を end-to-end で check する (単一 side shorthand
    // `border-top: ...` は現時点で parser 未対応、
    // computed.rs 228-229 参照)。
    use raikiri_style::{build_rule_tree, cascade};
    use taffy::{LengthPercentage, Rect};

    fn border_for(inline: &str) -> Rect<LengthPercentage> {
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
        let body = doc.append_element(Some(html), "body", Style::default(), Some(inline));
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");
        apply_computed_to_style(&mut doc, &cr);
        doc.nodes[body].style.border
    }

    // Case 1 (positive path): `border: 5px solid red` shorthand → 4 side
    //   全て width=5、style=solid で cascade。gating off (solid ≠ None/Hidden)
    //   なので 4 side 全て length(5.0) になる。Rect.top/right/bottom/left ↔
    //   Sides.top/right/bottom/left の field-name mapping pin。
    assert_eq!(
        border_for("border: 5px solid red"),
        Rect {
            top: LengthPercentage::length(5.0),
            right: LengthPercentage::length(5.0),
            bottom: LengthPercentage::length(5.0),
            left: LengthPercentage::length(5.0),
        }
    );

    // Case 2 (spec correctness): `border: 5px none red` shorthand
    //   → 4 side 全て width=5, style=None で cascade。§3.2 の `none` と
    //   §3.3 propdef (TR 版、逐語引用は冒頭 block) による style-gating で
    //   computed width = 0 → 4 side 全て length(0.0)。gating が壊れると 5.0
    //   が leak するので、この case が canary。
    assert_eq!(
        border_for("border: 5px none red"),
        Rect {
            top: LengthPercentage::length(0.0),
            right: LengthPercentage::length(0.0),
            bottom: LengthPercentage::length(0.0),
            left: LengthPercentage::length(0.0),
        }
    );

    // Case 3 (spec correctness): `border: 5px hidden red`
    //   shorthand → computed width = 0。直接の根拠は §3.3 propdef (TR 版) が
    //   `none` と並べて `hidden` を名指ししていること。§3.2 の `hidden` は
    //   "Same as none, but has different behavior in the border conflict
    //   resolution rules for border-collapsed tables `CSS2`." であり、`none`
    //   との差は border-collapsed table の conflict resolution だけ。
    assert_eq!(
        border_for("border: 5px hidden red"),
        Rect {
            top: LengthPercentage::length(0.0),
            right: LengthPercentage::length(0.0),
            bottom: LengthPercentage::length(0.0),
            left: LengthPercentage::length(0.0),
        }
    );

    // Case 4 (pt unit conversion): `border-top-width: 3pt` + solid → top
    //   only、他 3 side は initial (width=medium=3px, style=None) → gating
    //   で length(0.0)。top は 3pt × 4/3 = 4.0 px (CSS Values 4 §6.2、
    //   1pt = 96/72 px = 4/3 px)。f32 bit-identical のため右辺は式のまま。
    assert_eq!(
        border_for("border-top-width: 3pt; border-top-style: solid"),
        Rect {
            top: LengthPercentage::length(3.0 * 4.0 / 3.0),
            right: LengthPercentage::length(0.0),
            bottom: LengthPercentage::length(0.0),
            left: LengthPercentage::length(0.0),
        }
    );

    // Case 5 (medium keyword): `border-top-width: medium` + solid → top =
    //   3.0 px (property.rs `parse_border_width_side` 参照)。§3.3 は "The
    //   thin, medium, and thick keywords are equivalent to 1px, 3px, and
    //   5px, respectively." と**固定値を規定**する。CSS2.1 §8.5.1
    //   <https://www.w3.org/TR/CSS21/box.html#border-width-properties>
    //   の "The interpretation of the first three values depends on the
    //   user agent." から性格が変わっている点に注意。他 3 side は Case 4
    //   同様 gating で 0。medium keyword が Length::Px(3.0) にパースされる
    //   ことを end-to-end で pin。
    assert_eq!(
        border_for("border-top-width: medium; border-top-style: solid"),
        Rect {
            top: LengthPercentage::length(3.0),
            right: LengthPercentage::length(0.0),
            bottom: LengthPercentage::length(0.0),
            left: LengthPercentage::length(0.0),
        }
    );
}

/// `ComputedBorder::width` / `::style` を `pub`
/// field から `pub(crate)` + read-only accessor へ narrow した動機になった
/// invariant の end-to-end pin。
///
/// `border-top-width` だけを宣言し `border-top-style` を宣言しない
/// (= 未宣言側の computed style は initial `none`、CSS Backgrounds 3
/// §3.2) 素朴な入力で、declared width が bridge を通って taffy に **0** と
/// して届くことを確認する。narrowing 前はこの gate を consumer が
/// `ComputedValues::initial()` 等で得た `ComputedBorder` の
/// `.style = BorderStyle::None` 直接書き換えで迂回でき、`.width` が非 0 の
/// まま taffy に leak し得た (`used_border_width` bridge 削除後)。narrowing は
/// `width` / `style` に限り crate 外
/// からのその書き換え経路を塞ぐ — `color` は pub のまま、
/// `cv.border.top = cv.border.left` のような side 単位の丸ごと代入も
/// 引き続き可能で、いずれも本 invariant を破らない。
///
/// **本 test は「narrowing 前の値で確認できる 1 入力が正しく gate される」
/// ことの check であり、「crate 外からこの invariant を破る経路が存在しない」
/// ことを本 test 自身が総当たりで示すものではない。** ただし後者自体は
/// 現状すでに **型の visibility 境界で構造的に防がれている** — `width` /
/// `style` は `pub(crate)`、公開 API は値渡し read-only accessor (`width()` /
/// `style()`) のみで setter / builder / ctor が無いため、crate 外の
/// safe code がこの 2 field を書き換える経路はコンパイル時に存在しない。
///
/// **未解決なのは別の軸 — regression 検知**: 将来誰かが `width` /
/// `style` を `pub(crate)` から `pub` に戻す (= 上記の型保証そのものを
/// 撤回する) 変更をしても、それを検知して落ちる test が現状無い。
/// 他の 3 field (`Declaration::value` 等) には同じ形の regression を
/// 検知する compile-fail test setup がすでに追加され既に main に merge 済みだが、
/// `ComputedBorder::width` / `::style` への横展開はまだ行われていない。
#[test]
fn border_width_alone_without_declared_style_reaches_taffy_as_zero() {
    use raikiri_style::{build_rule_tree, cascade};
    use taffy::{LengthPercentage, Rect};

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(
        Some(html),
        "body",
        Style::default(),
        // `border-style` は一切宣言しない — 4 side とも computed style は
        // initial `none` (CSS Backgrounds 3 §3.2 "Inherited: no")。
        Some("border-top-width: 5px"),
    );
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");
    apply_computed_to_style(&mut doc, &cr);

    // border-style unset (initial `none`) must gate width to 0 all the way to taffy.
    assert_eq!(
        doc.nodes[body].style.border,
        Rect {
            top: LengthPercentage::length(0.0),
            right: LengthPercentage::length(0.0),
            bottom: LengthPercentage::length(0.0),
            left: LengthPercentage::length(0.0),
        }
    );
}

#[test]
fn font_relative_lengths_reach_taffy_as_real_pixels() {
    // 以前の bridge は specified 層の `Length` を受けており、
    // `Length::Em(_) | Length::Rem(_) => length(0.0)` で font-relative unit
    // を **黙って 0px に潰していた** (fail-quiet)。cascade が phase 2 /
    // phase 3 で絶対化するようになったので、実 px が taffy に届く。
    //
    // この test は「0.0 に潰れる」regression の canary である — 期待値は
    // すべて font-size から計算した非ゼロ値。
    use raikiri_style::{build_rule_tree, cascade};
    use taffy::{Dimension, LengthPercentage, LengthPercentageAuto, Rect};

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), Some("font-size: 20px"));
    let body = doc.append_element(
        Some(html),
        "body",
        Style::default(),
        // font-size は inherit で 20px。em は自 node の 20px、rem は root の
        // 20px 基準。
        Some(
            "padding: 2em; margin: 1.5rem; width: 3em; \
                 border-top-width: 0.5em; border-top-style: solid",
        ),
    );
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");
    apply_computed_to_style(&mut doc, &cr);
    let style = &doc.nodes[body].style;

    // 2em × 20px = 40px (従来は 0.0)。
    assert_eq!(
        style.padding,
        Rect {
            top: LengthPercentage::length(40.0),
            right: LengthPercentage::length(40.0),
            bottom: LengthPercentage::length(40.0),
            left: LengthPercentage::length(40.0),
        }
    );
    // 1.5rem × 20px (root font-size) = 30px (従来は 0.0)。
    assert_eq!(
        style.margin,
        Rect {
            top: LengthPercentageAuto::length(30.0),
            right: LengthPercentageAuto::length(30.0),
            bottom: LengthPercentageAuto::length(30.0),
            left: LengthPercentageAuto::length(30.0),
        }
    );
    // 3em × 20px = 60px (従来は 0.0)。
    assert_eq!(style.size.width, Dimension::length(60.0));
    // 0.5em × 20px = 10px、style: solid なので gating も通り抜ける
    // (従来は 0.0)。
    assert_eq!(style.border.top, LengthPercentage::length(10.0));
}

#[test]
fn apply_computed_to_style_bridges_box_sizing_to_taffy() {
    // bridge_box_sizing が
    // raikiri_style::BoxSizing → taffy::BoxSizing の enum 1:1 mapping を
    // 実施することを確認する regression check。
    //
    // 3 case:
    //   #1 border-box (specified)   → taffy::BoxSizing::BorderBox
    //   #2 content-box (specified)  → taffy::BoxSizing::ContentBox
    //   #3 unspecified (cascade default = raikiri-style initial = ContentBox)
    //      → taffy::BoxSizing::ContentBox
    //
    // 特筆: taffy 0.12 default は BorderBox (spec 違反)、raikiri-style initial
    // は ContentBox (CSS Sizing 3 §3.3 準拠)。#3 は cascade が initial 経由で
    // ContentBox を seed し、bridge がそれを taffy に伝播することで、taffy default
    // の spec 違反を副作用的に補正することを check する。
    use raikiri_style::{build_rule_tree, cascade};

    fn box_sizing_for(inline: Option<&str>) -> TaffyBoxSizing {
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
        let body = doc.append_element(Some(html), "body", Style::default(), inline);
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");
        apply_computed_to_style(&mut doc, &cr);
        doc.nodes[body].style.box_sizing
    }

    // Case 1: border-box → taffy::BoxSizing::BorderBox
    assert_eq!(
        box_sizing_for(Some("box-sizing: border-box")),
        TaffyBoxSizing::BorderBox
    );

    // Case 2: content-box (explicit) → taffy::BoxSizing::ContentBox
    assert_eq!(
        box_sizing_for(Some("box-sizing: content-box")),
        TaffyBoxSizing::ContentBox
    );

    // Case 3: unspecified → raikiri-style initial (ContentBox) → taffy ContentBox
    // (taffy default の BorderBox を上書き、spec 補正 pin)
    assert_eq!(box_sizing_for(None), TaffyBoxSizing::ContentBox);
}

#[test]
fn apply_page_box_clobbers_body_width_from_bridge() {
    // PageBox
    // 妥協の regression check — `<body style="width: 100px">` に対して
    //   Step 1 (`apply_computed_to_style`) → bridge_size が body.style.size.width
    //       を length(100.0) に write
    //   Step 4 (`apply_page_box_to_body`) → PageBox.width で clobber
    // の順で走ると、最終 body.style.size.width は PageBox.width (author 値
    // ではない) になる。将来 @page per-page PageBox に refactor するまで
    // この clobber 挙動を意図的に保つ (現行実装での妥協) — silent regression 検出用。
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), Some("width: 100px"));
    let rules = raikiri_style::build_rule_tree(&doc);
    let cr = raikiri_style::cascade(&doc, &rules).expect("cascade Ok");

    // Step 1: bridge 実行後、body.style.size.width は author 値 100px。
    apply_computed_to_style(&mut doc, &cr);
    assert_eq!(
        doc.nodes[body].style.size.width,
        Dimension::length(100.0),
        "bridge_size must first write author width (100px) to body.style.size.width"
    );

    // Step 4: PageBox clobber 後、author 値は消えて PageBox.width が入る。
    apply_page_box_to_body(&mut doc, body, PageBox::A4);
    assert_eq!(
        doc.nodes[body].style.size.width,
        Dimension::length(PageBox::A4.width),
        "apply_page_box_to_body must clobber author width with PageBox.width (現行実装での妥協)"
    );
    // author 値と PageBox 値は不一致 (clobber が実際に起きていることを pin)。
    assert_ne!(
        doc.nodes[body].style.size.width,
        Dimension::length(100.0),
        "post-clobber body.style.size.width must NOT equal author 100px"
    );
}

#[test]
fn layout_single_page_bridges_display_flow_root() {
    // Standalone `display: flow-root` must reach taffy as its dedicated
    // block-formatting-context display value rather than falling through
    // to a plain block approximation.
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let _head = doc.append_element(Some(html), "head", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let flow_root = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:flow-root;width:200px"),
    );
    let child = doc.append_element(
        Some(flow_root),
        "div",
        Style::default(),
        Some("height:20px;background:red"),
    );
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");
    layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("layout Ok");

    assert_eq!(
        doc.nodes[flow_root].style.display,
        Display::FlowRoot,
        // cov:ignore: panic-message literal only executes on assertion failure.
        "bridge_display must map DisplayValue::FlowRoot to taffy::Display::FlowRoot"
    );
    assert!(doc.nodes[child].unrounded_layout.size.height > 0.0);
}

#[test]
fn layout_single_page_bridges_display_flex_to_taffy_flexbox() {
    // display bridge が Display::Flex を実際に taffy::compute_flexbox_layout
    // へ届けることを **geometry** で確認する — style.display の値を
    // asserting するだけでは bridge が繋がったことしか示さず、taffy_impl.rs
    // の compute_child_layout dispatch が実際に flex を起動していることの
    // 証明にはならない。CSS Flexbox Level 1 の initial value
    // (`flex-direction: row`、`flex-wrap: nowrap`) 通りなら、明示 width の
    // 2 child は主軸 (x) 方向に並び、交差軸 (y) は揃う。
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let _head = doc.append_element(Some(html), "head", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let flex_container =
        doc.append_element(Some(body), "div", Style::default(), Some("display:flex"));
    let child_a = doc.append_element(
        Some(flex_container),
        "div",
        Style::default(),
        Some("width:100px;height:20px"),
    );
    let child_b = doc.append_element(
        Some(flex_container),
        "div",
        Style::default(),
        Some("width:100px;height:20px"),
    );
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");

    layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("layout Ok");

    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        doc.nodes[flex_container].style.display,
        Display::Flex,
        "bridge_display must map DisplayValue::Flex to taffy::Display::Flex"
    );
    let a_loc = doc.nodes[child_a].unrounded_layout.location;
    let b_loc = doc.nodes[child_b].unrounded_layout.location;
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert!(
        (a_loc.y - b_loc.y).abs() < 0.5,
        "row-direction flex items must share the same cross-axis (y) offset, got a.y={}, b.y={}",
        a_loc.y,
        b_loc.y
    );
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert!(
        (b_loc.x - a_loc.x - 100.0).abs() < 0.5,
        "second flex item should sit 100px (first item's width) further along the main axis (x), got a.x={}, b.x={}",
        a_loc.x,
        b_loc.x
    );
}

#[test]
fn bridge_flex_maps_every_flex_direction_and_flex_wrap_keyword() {
    // `bridge_flex`'s `flex_direction`/`flex_wrap` match arms — the
    // sibling `flex_direction_column_stacks_children_vertically` /
    // `gap_adds_space_between_flex_items` tests only exercise `Row`
    // (default) and `Column` end-to-end through real layout geometry;
    // this test pins the remaining keyword→taffy-constant mappings
    // directly, since geometry alone can't discriminate e.g.
    // `RowReverse` from `Row` without a multi-child fixture per
    // variant.
    for (direction, expected) in [
        (FlexDirectionValue::Row, TaffyFlexDirection::Row),
        (
            FlexDirectionValue::RowReverse,
            TaffyFlexDirection::RowReverse,
        ),
        (FlexDirectionValue::Column, TaffyFlexDirection::Column),
        (
            FlexDirectionValue::ColumnReverse,
            TaffyFlexDirection::ColumnReverse,
        ),
    ] {
        let mut cv = ComputedValues::initial();
        cv.flex_direction = direction;
        let mut style = Style::default();
        let mut diag = Vec::new();
        bridge_flex(&mut style, &cv, &mut diag);
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            style.flex_direction, expected,
            "flex-direction: {direction:?}"
        );
    }

    for (wrap, expected) in [
        (FlexWrapValue::NoWrap, TaffyFlexWrap::NoWrap),
        (FlexWrapValue::Wrap, TaffyFlexWrap::Wrap),
        (FlexWrapValue::WrapReverse, TaffyFlexWrap::WrapReverse),
    ] {
        let mut cv = ComputedValues::initial();
        cv.flex_wrap = wrap;
        let mut style = Style::default();
        let mut diag = Vec::new();
        bridge_flex(&mut style, &cv, &mut diag);
        assert_eq!(style.flex_wrap, expected, "flex-wrap: {wrap:?}");
    }
}

#[test]
fn bridge_flex_absolutizes_flex_basis_px_and_percent() {
    // `bridge_flex`'s `ComputedFlexBasis::Px`/`Percent` arms — no
    // existing test sets an explicit `<length-percentage>` flex-basis
    // (`flex_grow_absorbs_free_space` below leaves it at the `auto`
    // initial value), so these two reachable arms had no direct
    // coverage.
    let mut cv = ComputedValues::initial();
    cv.flex_basis = ComputedFlexBasis::Px(40.0);
    let mut style = Style::default();
    let mut diag = Vec::new();
    bridge_flex(&mut style, &cv, &mut diag);
    assert_eq!(style.flex_basis, Dimension::length(40.0));

    // `ComputedFlexBasis::Percent` holds the authored number (`50.0`
    // for `50%`, not a `0.0..=1.0` fraction) — the bridge divides by
    // 100 before handing it to taffy, same convention as
    // `computed_length_percentage_or_auto_to_taffy_dimension`'s other
    // callers (`width`/`height`/`margin`).
    let mut cv = ComputedValues::initial();
    cv.flex_basis = ComputedFlexBasis::Percent(50.0);
    let mut style = Style::default();
    let mut diag = Vec::new();
    bridge_flex(&mut style, &cv, &mut diag);
    assert_eq!(style.flex_basis, Dimension::percent(0.5));
}

#[test]
fn bridge_flex_maps_intrinsic_basis_keywords_to_taffy_dimensions() {
    // taffy 0.14 migration: `min-content` / `max-content` /
    // `fit-content` / `content` map to taffy's own intrinsic
    // `Dimension` variants (no `auto` collapse anymore).
    for (value, expected) in [
        (ComputedFlexBasis::MinContent, Dimension::min_content()),
        (ComputedFlexBasis::MaxContent, Dimension::max_content()),
        (ComputedFlexBasis::FitContent, Dimension::fit_content()),
        (ComputedFlexBasis::Content, Dimension::content()),
    ] {
        let mut cv = ComputedValues::initial();
        cv.flex_basis = value;
        let mut style = Style::default();
        let mut diag = Vec::new();
        bridge_flex(&mut style, &cv, &mut diag);
        assert_eq!(style.flex_basis, expected);
        assert!(diag.is_empty());
    }
}

#[test]
fn content_alignment_to_taffy_maps_every_keyword() {
    // `content_alignment_to_taffy` backs both `justify-content` and
    // `align-content` (`bridge_alignment`) — check every keyword→taffy
    // constant mapping directly, since only `Center`-ish geometry is
    // exercised end-to-end elsewhere.
    for (value, expected) in [
        (ContentAlignmentValue::Normal, None),
        (
            ContentAlignmentValue::Stretch,
            Some(TaffyAlignContent::STRETCH),
        ),
        (
            ContentAlignmentValue::SpaceBetween,
            Some(TaffyAlignContent::SPACE_BETWEEN),
        ),
        (
            ContentAlignmentValue::SpaceEvenly,
            Some(TaffyAlignContent::SPACE_EVENLY),
        ),
        (
            ContentAlignmentValue::SpaceAround,
            Some(TaffyAlignContent::SPACE_AROUND),
        ),
        (
            ContentAlignmentValue::Center,
            Some(TaffyAlignContent::CENTER),
        ),
        (ContentAlignmentValue::Start, Some(TaffyAlignContent::START)),
        (ContentAlignmentValue::End, Some(TaffyAlignContent::END)),
        (
            ContentAlignmentValue::FlexStart,
            Some(TaffyAlignContent::FLEX_START),
        ),
        (
            ContentAlignmentValue::FlexEnd,
            Some(TaffyAlignContent::FLEX_END),
        ),
    ] {
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            content_alignment_to_taffy(value),
            expected,
            "value: {value:?}"
        );
    }
}

#[test]
fn self_alignment_to_taffy_maps_every_keyword() {
    // `self_alignment_to_taffy` backs `align-items` and the
    // non-`auto` branch of `align-self` (`bridge_alignment`) — check
    // every keyword→taffy constant mapping directly.
    for (value, expected) in [
        (SelfAlignmentValue::Normal, None),
        (SelfAlignmentValue::Stretch, Some(TaffyAlignItems::STRETCH)),
        (SelfAlignmentValue::Center, Some(TaffyAlignItems::CENTER)),
        (SelfAlignmentValue::Start, Some(TaffyAlignItems::START)),
        (SelfAlignmentValue::End, Some(TaffyAlignItems::END)),
        (
            SelfAlignmentValue::FlexStart,
            Some(TaffyAlignItems::FLEX_START),
        ),
        (SelfAlignmentValue::FlexEnd, Some(TaffyAlignItems::FLEX_END)),
        (
            SelfAlignmentValue::Baseline,
            Some(TaffyAlignItems::BASELINE),
        ),
    ] {
        assert_eq!(self_alignment_to_taffy(value), expected, "value: {value:?}");
    }
}

#[test]
fn bridge_alignment_align_self_auto_maps_to_none() {
    // `bridge_alignment`'s `AlignSelfValue::Auto` arm — the `None`
    // mapping (CSS Box Alignment 3 §6.2: falls back to the parent's
    // `align-items`, delegated to taffy) has no other coverage.
    let mut cv = ComputedValues::initial();
    cv.align_self = AlignSelfValue::Auto;
    let mut style = Style::default();
    bridge_alignment(&mut style, &cv);
    assert_eq!(style.align_self, None);
}

#[test]
fn bridge_alignment_align_self_value_delegates_to_self_alignment_to_taffy() {
    // `bridge_alignment`'s `AlignSelfValue::Value(v)` arm — distinct
    // from the sibling test above, which only pins the `Auto` arm.
    // `self_alignment_to_taffy` itself is already pinned directly by
    // `self_alignment_to_taffy_maps_every_keyword`, but that doesn't
    // exercise this match arm's delegation to it.
    let mut cv = ComputedValues::initial();
    cv.align_self = AlignSelfValue::Value(SelfAlignmentValue::Center);
    let mut style = Style::default();
    bridge_alignment(&mut style, &cv);
    assert_eq!(style.align_self, Some(TaffyAlignItems::CENTER));
}

#[test]
fn bridge_alignment_align_self_normal_maps_to_stretch_not_none() {
    // Regression test (§8.3 review finding): `align-self: normal` must
    // NOT collapse to the same `None` mapping as `align-self: auto`.
    // `auto` computes to the parent's `align-items` value (CSS Box
    // Alignment 3 §8.3), which taffy's `align_self: None` already
    // implements by inheriting the container's `align_items`. But
    // `normal` independently behaves as `stretch` in flex layout
    // regardless of the parent's `align-items` — mapping it to `None`
    // would incorrectly make it inherit the parent's value like `auto`
    // does. See the end-to-end behavioral check below.
    let mut cv = ComputedValues::initial();
    cv.align_self = AlignSelfValue::Value(SelfAlignmentValue::Normal);
    let mut style = Style::default();
    bridge_alignment(&mut style, &cv);
    assert_eq!(style.align_self, Some(TaffyAlignItems::STRETCH));
}

#[test]
fn align_self_normal_stretches_even_when_parent_align_items_is_center() {
    // End-to-end check of the §8.3 review finding: with the container's
    // `align-items: center`, a child with `align-self: auto` would
    // center (inheriting the parent's value per spec), but a child
    // with `align-self: normal` must independently stretch to fill the
    // cross axis — the two must NOT produce the same layout, even
    // though `bridge_alignment` maps both through `Option<AlignItems>`.
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let _head = doc.append_element(Some(html), "head", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let flex_container = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:flex;height:100px;align-items:center"),
    );
    let normal_child = doc.append_element(
        Some(flex_container),
        "div",
        Style::default(),
        Some("width:50px;align-self:normal"),
    );
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");

    layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("layout Ok");

    let child_height = doc.nodes[normal_child].unrounded_layout.size.height;
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert!(
        (child_height - 100.0).abs() < 0.5,
        "align-self:normal must stretch to fill the 100px container cross-size regardless of the parent's align-items:center, got height={child_height}"
    );
}

#[test]
fn flex_direction_column_stacks_children_vertically() {
    // `bridge_flex`'s `flex_direction` field must actually reach
    // taffy's `compute_flexbox_layout` — asserting `style.flex_direction
    // == Column` alone would only prove the assignment, not the integration,
    // since reading the field back off the taffy `Style` cannot
    // distinguish "assigned" from "used by the layout algorithm". With
    // `flex-direction: column`, the main axis flips to the block (y)
    // axis: 2 children with an explicit height must land at the same
    // x offset with y offsets 20px (the first child's height) apart.
    //
    // Also carries `gap:10px 30px` (row-gap column-gap) to independently
    // check `row-gap → taffy::Style::gap.height`, which the sibling
    // `gap_adds_space_between_flex_items` test cannot exercise — that
    // test's default row-direction container only puts `column-gap` on
    // the main axis. Here, `flex-direction: column` makes `row-gap` the
    // *main*-axis gap (`Size::main()` for a column container resolves to
    // `height`, per `bridge_gap`'s `width: column_gap, height: row_gap`
    // mapping), so the two children's y-offset becomes 20px (first
    // child's height) + 10px (row-gap) = 30px — discriminating a dropped
    // row-gap (would stay at 20px) or a transposed bridge (would become
    // 20px + 30px = 50px) from the correct integration.
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let _head = doc.append_element(Some(html), "head", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let flex_container = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:flex;flex-direction:column;gap:10px 30px"),
    );
    let child_a = doc.append_element(
        Some(flex_container),
        "div",
        Style::default(),
        Some("width:100px;height:20px"),
    );
    let child_b = doc.append_element(
        Some(flex_container),
        "div",
        Style::default(),
        Some("width:100px;height:20px"),
    );
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");

    layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("layout Ok");

    let a_loc = doc.nodes[child_a].unrounded_layout.location;
    let b_loc = doc.nodes[child_b].unrounded_layout.location;
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert!(
        (a_loc.x - b_loc.x).abs() < 0.5,
        "column-direction flex items must share the same cross-axis (x) offset, got a.x={}, b.x={}",
        a_loc.x,
        b_loc.x
    );
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert!(
        (b_loc.y - a_loc.y - 30.0).abs() < 0.5,
        "second flex item should sit 30px (20px first item's height + 10px row-gap) further along the main axis (y), got a.y={}, b.y={}",
        a_loc.y,
        b_loc.y
    );
}

#[test]
fn flex_grow_absorbs_free_space() {
    // `bridge_flex`'s `flex_grow` field reaching taffy's flexible-length
    // resolution algorithm (§9.7 "Resolving Flexible Lengths") — a
    // growable child must widen past its own basis to consume the free
    // space in the container, while a non-growable sibling stays at its
    // basis.
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let _head = doc.append_element(Some(html), "head", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let flex_container = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:flex;width:500px"),
    );
    let grower = doc.append_element(
        Some(flex_container),
        "div",
        Style::default(),
        Some("width:100px;height:20px;flex-grow:1"),
    );
    let fixed = doc.append_element(
        Some(flex_container),
        "div",
        Style::default(),
        Some("width:100px;height:20px"),
    );
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");

    layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("layout Ok");

    let grower_width = doc.nodes[grower].unrounded_layout.size.width;
    let fixed_width = doc.nodes[fixed].unrounded_layout.size.width;
    // Container is 500px, fixed sibling stays 100px, so the grower must
    // absorb the remaining 400px free space (500 - 100 = 400).
    //
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert!(
        (grower_width - 400.0).abs() < 0.5,
        "flex-grow:1 item should absorb the container's free space, got width={grower_width}"
    );
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert!(
        (fixed_width - 100.0).abs() < 0.5,
        "non-growing sibling should stay at its flex-basis width, got width={fixed_width}"
    );
}

#[test]
fn bridge_float_maps_every_float_and_clear_keyword() {
    // `bridge_float`'s enum match arms — direct pin, same rationale as
    // `bridge_flex_maps_every_flex_direction_and_flex_wrap_keyword`:
    // geometry-based tests below only exercise `Float::Left` /
    // `Clear::Left`, so this pins the remaining keyword→taffy-constant
    // mappings that a geometry fixture can't discriminate from each
    // other without a dedicated fixture per variant.
    for (float, expected) in [
        (FloatValue::None, TaffyFloat::None),
        (FloatValue::Left, TaffyFloat::Left),
        (FloatValue::Right, TaffyFloat::Right),
    ] {
        let mut cv = ComputedValues::initial();
        cv.float = float;
        let mut style = Style::default();
        bridge_float(&mut style, &cv);
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(style.float, expected, "float: {float:?}");
    }

    for (clear, expected) in [
        (ClearValue::None, TaffyClear::None),
        (ClearValue::Left, TaffyClear::Left),
        (ClearValue::Right, TaffyClear::Right),
        (ClearValue::Both, TaffyClear::Both),
    ] {
        let mut cv = ComputedValues::initial();
        cv.clear = clear;
        let mut style = Style::default();
        bridge_float(&mut style, &cv);
        assert_eq!(style.clear, expected, "clear: {clear:?}");
    }
}

#[test]
fn floated_box_with_explicit_width_is_positioned_at_the_containing_block_edge() {
    // Real float layout (taffy's `float_layout` feature, wired via
    // `bridge_float` + `taffy_impl`'s `LayoutBlockContainer` impl) —
    // a `float:left` box with an explicit width must be sized to
    // that width (not stretch to the container's full width like a
    // normal block child would) and must be positioned flush against
    // the containing block's start edge (CSS2 §9.5.1
    // <https://www.w3.org/TR/CSS2/visuren.html#propdef-float>: "The
    // left outer edge of a left-floating box may not be to the left
    // of the left edge of its containing block").
    //
    // This does NOT exercise shrink-to-fit sizing (CSS2 §10.3.5
    // <https://www.w3.org/TR/CSS2/visudet.html#float-width>, which only
    // applies when 'width' computes to 'auto') — this fixture gives an
    // explicit width on purpose. Shrink-to-fit-width coverage is a
    // separate, currently-untested gap.
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let _head = doc.append_element(Some(html), "head", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let container = doc.append_element(Some(body), "div", Style::default(), Some("width:200px"));
    let float_child = doc.append_element(
        Some(container),
        "div",
        Style::default(),
        Some("float:left;width:60px;height:40px"),
    );

    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");

    layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("layout Ok");

    let loc = doc.nodes[float_child].unrounded_layout.location;
    let size = doc.nodes[float_child].unrounded_layout.size;
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert!(
        (size.width - 60.0).abs() < 0.5,
        "explicit width must still be honored, got width={}",
        size.width
    );
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert!(
        loc.x.abs() < 0.5,
        "left float must sit flush against the containing block's left edge, got x={}",
        loc.x
    );
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert!(
        loc.y.abs() < 0.5,
        "first child's float must sit flush against the containing block's top edge, got y={}",
        loc.y
    );
}

#[test]
fn bridge_gap_collapses_normal_to_zero() {
    // `computed_gap_component_to_taffy`'s `Normal` arm (CSS Box
    // Alignment 3 §8.1's "normal" keyword has no taffy equivalent, so
    // this bridge collapses it to `0`) — a direct pin, since it's
    // plausible for this branch to appear covered only incidentally
    // through unrelated tests' default (gap-less) fixtures rather
    // than being pinned on its own.
    let cv = ComputedValues::initial();
    assert_eq!(cv.row_gap, ComputedLengthPercentageOrNormal::Normal);
    assert_eq!(cv.column_gap, ComputedLengthPercentageOrNormal::Normal);
    let mut style = Style::default();
    let mut diag = Vec::new();
    bridge_gap(&mut style, &cv, &mut diag);
    assert_eq!(
        style.gap,
        Size {
            width: LengthPercentage::length(0.0),
            height: LengthPercentage::length(0.0),
        }
    );
}

#[test]
fn bridge_gap_absolutizes_percent_gap() {
    // `computed_gap_component_to_taffy`'s `Percent` arm — the `Px`
    // arm has incidental coverage through `gap_adds_space_between_flex_items`
    // below, but no existing test exercises `row-gap`/`column-gap` with
    // a `<percentage>` value. `ComputedLengthPercentageOrNormal::Percent`
    // holds the authored number (`25.0` for `25%`), and the bridge
    // divides by 100 before handing it to taffy, same convention as
    // `bridge_flex_absolutizes_flex_basis_px_and_percent` above.
    let mut cv = ComputedValues::initial();
    cv.row_gap = ComputedLengthPercentageOrNormal::Percent(25.0);
    cv.column_gap = ComputedLengthPercentageOrNormal::Percent(10.0);
    let mut style = Style::default();
    let mut diag = Vec::new();
    bridge_gap(&mut style, &cv, &mut diag);
    assert_eq!(
        style.gap,
        Size {
            width: LengthPercentage::percent(0.1),
            height: LengthPercentage::percent(0.25),
        }
    );
}

#[test]
fn gap_adds_space_between_flex_items() {
    // `bridge_gap` maps the `gap` shorthand's 2 components onto taffy's
    // `Size<LengthPercentage>` as `Size { width: column_gap, height:
    // row_gap }`. The 2 components are deliberately asymmetric (10px
    // vs 30px) so that a transposed mapping (swapping row/column) would
    // fail this assertion instead of passing it unnoticed — in a
    // row-direction container (the default `flex-direction`), the
    // main axis (x) gap is `column-gap`, so the second item's x offset
    // must be the first item's width **plus 30px**, not 10px.
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let _head = doc.append_element(Some(html), "head", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let flex_container = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:flex;gap:10px 30px"),
    );
    // Source formatting whitespace is not an anonymous flex item. Keep
    // it in the DOM to exercise the same filtering used by HTML parsing.
    let _whitespace_before = doc.append_text(flex_container, "\n  ");
    let child_a = doc.append_element(
        Some(flex_container),
        "div",
        Style::default(),
        Some("width:100px;height:20px"),
    );
    let _whitespace_between = doc.append_text(flex_container, "\n  ");
    let child_b = doc.append_element(
        Some(flex_container),
        "div",
        Style::default(),
        Some("width:100px;height:20px"),
    );
    let _whitespace_after = doc.append_text(flex_container, "\n");
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");

    layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("layout Ok");

    let a_loc = doc.nodes[child_a].unrounded_layout.location;
    let b_loc = doc.nodes[child_b].unrounded_layout.location;
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert!(
        (b_loc.x - a_loc.x - 130.0).abs() < 0.5,
        "second flex item should sit 130px (100px width + 30px column-gap) further along the main axis (x), got a.x={}, b.x={}",
        a_loc.x,
        b_loc.x
    );
}

#[test]
fn justify_content_flex_end_pushes_children_to_container_end() {
    // `bridge_alignment`'s `justify_content` field reaching taffy's
    // main-axis alignment (§9.5 "Main-Axis Alignment") — with
    // `justify-content: flex-end` in a 500px-wide row container, 2
    // 100px-wide children must land flush against the container's end
    // edge (x = 500 - 200 = 300 for the first child).
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let _head = doc.append_element(Some(html), "head", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let flex_container = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:flex;width:500px;justify-content:flex-end"),
    );
    let child_a = doc.append_element(
        Some(flex_container),
        "div",
        Style::default(),
        Some("width:100px;height:20px"),
    );
    let child_b = doc.append_element(
        Some(flex_container),
        "div",
        Style::default(),
        Some("width:100px;height:20px"),
    );
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");

    layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("layout Ok");

    let a_loc = doc.nodes[child_a].unrounded_layout.location;
    let b_loc = doc.nodes[child_b].unrounded_layout.location;
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert!(
        (a_loc.x - 300.0).abs() < 0.5,
        "first item should be pushed flush to the container's end edge (500 - 100 - 100 = 300), got a.x={}",
        a_loc.x
    );
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert!(
        (b_loc.x - 400.0).abs() < 0.5,
        "second item should sit immediately after the first (300 + 100 = 400), got b.x={}",
        b_loc.x
    );
}

#[test]
fn layout_single_page_bridges_display_grid_to_taffy_grid() {
    // display bridge が Display::Grid を taffy::compute_grid_layout へ
    // 届けることを確認する。raikiri-style は `grid-template-columns` 等の
    // grid-* property を未実装 (本 task の scope 外、"entry point を開く"
    // だけが scope) なので、implicit single-track grid の挙動は block と
    // 見分けがつきにくい — ここでは「bridge が Display::Grid を発火させ、
    // compute_grid_layout がクラッシュせず有限な box を返す」ことのみを
    // check する。track-level の挙動 check は grid-* property 実装時の
    // follow-up の責務。
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let _head = doc.append_element(Some(html), "head", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let grid_container =
        doc.append_element(Some(body), "div", Style::default(), Some("display:grid"));
    let child_a = doc.append_element(
        Some(grid_container),
        "div",
        Style::default(),
        Some("width:100px;height:20px"),
    );
    let child_b = doc.append_element(
        Some(grid_container),
        "div",
        Style::default(),
        Some("width:100px;height:20px"),
    );
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");

    layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("layout Ok");

    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        doc.nodes[grid_container].style.display,
        Display::Grid,
        "bridge_display must map DisplayValue::Grid to taffy::Display::Grid"
    );
    for id in [child_a, child_b] {
        let size = doc.nodes[id].unrounded_layout.size;
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert!(
            size.width.is_finite()
                && size.height.is_finite()
                && size.width >= 0.0
                && size.height >= 0.0,
            "grid item layout must be finite and non-negative, got {:?}",
            size
        );
    }
}

// ── bridge_grid (CSS Grid Layout Module Level 1) ────────────────────

use raikiri_style::property::{GridTemplateAreaEntry, GridTemplateAreas};
use raikiri_style::{ComputedGridTrackList, ComputedGridTrackRepeat};

#[test]
fn bridge_grid_maps_grid_auto_flow_keywords() {
    for (flow, expected) in [
        (GridAutoFlowValue::Row, TaffyGridAutoFlow::Row),
        (GridAutoFlowValue::Column, TaffyGridAutoFlow::Column),
        (GridAutoFlowValue::RowDense, TaffyGridAutoFlow::RowDense),
        (
            GridAutoFlowValue::ColumnDense,
            TaffyGridAutoFlow::ColumnDense,
        ),
    ] {
        let mut cv = ComputedValues::initial();
        cv.grid_auto_flow = flow;
        let mut style = Style::default();
        let mut diag = Vec::new();
        bridge_grid(&mut style, &cv, &mut diag);
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(style.grid_auto_flow, expected, "grid-auto-flow: {flow:?}");
    }
}

#[test]
fn bridge_grid_maps_every_grid_line_placement_variant() {
    for (value, expected) in [
        (GridLineValue::Auto, GridPlacement::Auto),
        (GridLineValue::Line(-2), taffy_style_helpers::line(-2i16)),
        (
            GridLineValue::Named("col".into()),
            GridPlacement::NamedLine("col".to_string(), 1),
        ),
        (
            GridLineValue::NamedLine("col".into(), 3),
            GridPlacement::NamedLine("col".to_string(), 3),
        ),
        (GridLineValue::Span(4), GridPlacement::Span(4)),
        (
            GridLineValue::SpanNamed("col".into(), 2),
            GridPlacement::NamedSpan("col".to_string(), 2),
        ),
    ] {
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            grid_line_value_to_taffy_placement(&value),
            expected,
            "grid-line: {value:?}"
        );
    }
}

#[test]
fn bridge_grid_maps_grid_row_and_grid_column_from_the_4_longhands() {
    let mut cv = ComputedValues::initial();
    cv.grid_row_start = GridLineValue::Line(2);
    cv.grid_row_end = GridLineValue::Span(3);
    cv.grid_column_start = GridLineValue::Named("main".into());
    cv.grid_column_end = GridLineValue::Auto;
    let mut style = Style::default();
    let mut diag = Vec::new();
    bridge_grid(&mut style, &cv, &mut diag);
    assert_eq!(
        style.grid_row,
        TaffyLine {
            start: taffy_style_helpers::line(2i16),
            end: GridPlacement::Span(3),
        }
    );
    assert_eq!(
        style.grid_column,
        TaffyLine {
            start: GridPlacement::NamedLine("main".to_string(), 1),
            end: GridPlacement::Auto,
        }
    );
}

#[test]
fn bridge_grid_maps_track_list_with_repeat_minmax_fr_and_named_lines() {
    // `grid-template-columns: [a] 100px repeat(2, [b] minmax(0, 1fr))`
    // — exercises a bare fixed track, a `repeat()` with `minmax()`/`fr`
    // inside, and named lines both outside and inside the `repeat()`.
    let mut cv = ComputedValues::initial();
    cv.grid_template_columns =
        ComputedGridTemplateTracks::List(std::sync::Arc::new(ComputedGridTrackList {
            line_names: vec![vec!["a".into()], vec![], vec![]],
            components: vec![
                ComputedGridTrackListComponent::Size(ComputedGridTrackSize::Breadth(
                    ComputedGridTrackBreadth::Px(100.0),
                )),
                ComputedGridTrackListComponent::Repeat(ComputedGridTrackRepeat {
                    count: GridRepeatCount::Count(2),
                    line_names: vec![vec!["b".into()], vec![]],
                    tracks: vec![ComputedGridTrackSize::MinMax(
                        ComputedGridTrackBreadth::Px(0.0),
                        ComputedGridTrackBreadth::Flex(1.0),
                    )],
                }),
            ],
        }));
    let mut style = Style::default();
    let mut diag = Vec::new();
    bridge_grid(&mut style, &cv, &mut diag);

    assert_eq!(style.grid_template_columns.len(), 2);
    assert_eq!(
        style.grid_template_columns[0],
        GridTemplateComponent::Single(TrackSizingFunction {
            min: MinTrackSizingFunction::length(100.0),
            max: MaxTrackSizingFunction::length(100.0),
        })
    );
    assert_eq!(
        style.grid_template_columns[1],
        GridTemplateComponent::Repeat(GridTemplateRepetition {
            count: TaffyRepetitionCount::Count(2),
            tracks: vec![TrackSizingFunction {
                min: MinTrackSizingFunction::length(0.0),
                max: MaxTrackSizingFunction::fr(1.0),
            }],
            line_names: vec![vec!["b".to_string()], vec![]],
        })
    );
    assert_eq!(
        style.grid_template_column_names,
        vec![vec!["a".to_string()], vec![], vec![]]
    );
}

#[test]
fn bridge_grid_maps_grid_template_areas() {
    let mut cv = ComputedValues::initial();
    cv.grid_template_areas =
        GridTemplateAreasValue::Areas(std::sync::Arc::new(GridTemplateAreas {
            row_strings: vec!["header".into()],
            areas: vec![GridTemplateAreaEntry {
                name: "header".into(),
                row_start: 1,
                row_end: 2,
                column_start: 1,
                column_end: 3,
            }],
            row_count: 1,
            column_count: 2,
        }));
    let mut style = Style::default();
    let mut diag = Vec::new();
    bridge_grid(&mut style, &cv, &mut diag);
    assert_eq!(
        style.grid_template_areas,
        Some(taffy::style::GridTemplateAreas {
            areas: vec![TaffyGridTemplateArea {
                name: "header".to_string(),
                row_start: 1,
                row_end: 2,
                column_start: 1,
                column_end: 3,
            }],
            row_count: 1,
            column_count: 2,
        })
    );
}

#[test]
fn bridge_grid_maps_grid_auto_columns_and_rows_track_sizes() {
    let mut cv = ComputedValues::initial();
    cv.grid_auto_columns = std::sync::Arc::new(vec![ComputedGridTrackSize::Breadth(
        ComputedGridTrackBreadth::Px(50.0),
    )]);
    cv.grid_auto_rows = std::sync::Arc::new(vec![ComputedGridTrackSize::Breadth(
        ComputedGridTrackBreadth::MinContent,
    )]);
    let mut style = Style::default();
    let mut diag = Vec::new();
    bridge_grid(&mut style, &cv, &mut diag);
    assert_eq!(
        style.grid_auto_columns,
        vec![TrackSizingFunction {
            min: MinTrackSizingFunction::length(50.0),
            max: MaxTrackSizingFunction::length(50.0),
        }]
    );
    assert_eq!(
        style.grid_auto_rows,
        vec![TrackSizingFunction {
            min: MinTrackSizingFunction::min_content(),
            max: MaxTrackSizingFunction::min_content(),
        }]
    );
}

#[test]
fn bridge_grid_maps_grid_auto_columns_percent_and_max_content_breadth() {
    // Sibling of `bridge_grid_maps_grid_auto_columns_and_rows_track_sizes`
    // above, which only exercises `Px`/`MinContent` — `Percent` and
    // `MaxContent` are the 2 `ComputedGridTrackBreadth` variants that
    // test left unreached in both `grid_track_breadth_to_taffy_min` and
    // `grid_track_breadth_to_taffy_max` (`Flex`/`Auto` are covered
    // elsewhere by the repeat/minmax and named-line tests).
    let mut cv = ComputedValues::initial();
    cv.grid_auto_columns = std::sync::Arc::new(vec![ComputedGridTrackSize::Breadth(
        ComputedGridTrackBreadth::Percent(25.0),
    )]);
    cv.grid_auto_rows = std::sync::Arc::new(vec![ComputedGridTrackSize::Breadth(
        ComputedGridTrackBreadth::MaxContent,
    )]);
    let mut style = Style::default();
    let mut diag = Vec::new();
    bridge_grid(&mut style, &cv, &mut diag);
    assert_eq!(
        style.grid_auto_columns,
        vec![TrackSizingFunction {
            min: MinTrackSizingFunction::percent(0.25),
            max: MaxTrackSizingFunction::percent(0.25),
        }]
    );
    assert_eq!(
        style.grid_auto_rows,
        vec![TrackSizingFunction {
            min: MinTrackSizingFunction::max_content(),
            max: MaxTrackSizingFunction::max_content(),
        }]
    );
}

#[test]
fn bridge_grid_maps_grid_auto_columns_fit_content() {
    // `fit-content(<length-percentage>)` (CSS Grid 1 §7.2.1) — no other
    // `bridge_grid` test constructs `ComputedGridTrackSize::FitContent`,
    // so `grid_track_size_to_taffy`'s `FitContent` arm (both the px and
    // percentage forms) was previously unreached by any test.
    let mut cv = ComputedValues::initial();
    cv.grid_auto_columns = std::sync::Arc::new(vec![
        ComputedGridTrackSize::FitContent(ComputedLengthPercentage::Px(80.0)),
        ComputedGridTrackSize::FitContent(ComputedLengthPercentage::Percent(40.0)),
    ]);
    let mut style = Style::default();
    let mut diag = Vec::new();
    bridge_grid(&mut style, &cv, &mut diag);
    assert_eq!(
        style.grid_auto_columns,
        vec![
            TrackSizingFunction {
                min: MinTrackSizingFunction::auto(),
                max: MaxTrackSizingFunction::fit_content_px(80.0),
            },
            TrackSizingFunction {
                min: MinTrackSizingFunction::auto(),
                max: MaxTrackSizingFunction::fit_content_percent(0.4),
            },
        ]
    );
}

#[test]
fn grid_repeat_count_to_taffy_maps_auto_fill_and_auto_fit() {
    // `repeat(auto-fill, ...)` / `repeat(auto-fit, ...)` (CSS Grid 1
    // §7.2.3.2) — the other `bridge_grid` track-list tests only
    // exercise `GridRepeatCount::Count(_)`, so the two auto-repeat
    // variants had no direct coverage.
    assert_eq!(
        grid_repeat_count_to_taffy(GridRepeatCount::AutoFill),
        TaffyRepetitionCount::AutoFill
    );
    assert_eq!(
        grid_repeat_count_to_taffy(GridRepeatCount::AutoFit),
        TaffyRepetitionCount::AutoFit
    );
}

#[test]
fn bridge_alignment_maps_justify_items_and_justify_self() {
    let mut cv = ComputedValues::initial();
    cv.justify_items = SelfAlignmentValue::Center;
    cv.justify_self = AlignSelfValue::Value(SelfAlignmentValue::End);
    let mut style = Style::default();
    bridge_alignment(&mut style, &cv);
    assert_eq!(style.justify_items, Some(TaffyAlignItems::CENTER));
    assert_eq!(style.justify_self, Some(TaffyAlignItems::END));

    // `justify-self: normal` behaves as `stretch` (CSS Box Alignment 3
    // §8.3), not as `None` (which would mean "inherit justify-items")
    // — same special-case as `align-self: normal`
    // (`self_alignment_or_auto_to_taffy` doc).
    let mut cv = ComputedValues::initial();
    cv.justify_self = AlignSelfValue::Value(SelfAlignmentValue::Normal);
    let mut style = Style::default();
    bridge_alignment(&mut style, &cv);
    assert_eq!(style.justify_self, Some(TaffyAlignItems::STRETCH));
}

#[test]
fn grid_template_columns_actually_sizes_columns_through_taffy_grid_algorithm() {
    // `bridge_grid`'s `grid_template_columns` field must actually reach
    // taffy's `compute_grid_layout` — asserting the `taffy::Style`
    // field alone would only prove the assignment, not the integration
    // (same rationale as `flex_direction_column_stacks_children_vertically`
    // above). A 2-column `100px 200px` grid with one child explicitly
    // placed in each column must position the second child 100px to
    // the right of the first.
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
        Some("display:grid;grid-template-columns:100px 200px"),
    );
    let cell_a = doc.append_element(
        Some(grid_container),
        "div",
        Style::default(),
        Some("grid-column:1;grid-row:1;height:20px"),
    );
    let cell_b = doc.append_element(
        Some(grid_container),
        "div",
        Style::default(),
        Some("grid-column:2;grid-row:1;height:20px"),
    );
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");

    layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("layout Ok");

    let a_loc = doc.nodes[cell_a].unrounded_layout.location;
    let b_loc = doc.nodes[cell_b].unrounded_layout.location;
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert!(
        (b_loc.x - a_loc.x - 100.0).abs() < 0.5,
        "second grid cell should sit 100px (first column's width) to \
             the right of the first, got a.x={}, b.x={}",
        a_loc.x,
        b_loc.x
    );
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert!(
        (a_loc.y - b_loc.y).abs() < 0.5,
        "both cells share row 1, so they must share the same y \
             offset, got a.y={}, b.y={}",
        a_loc.y,
        b_loc.y
    );
}

#[test]
fn grid_auto_flow_column_places_implicit_items_column_wise_through_taffy() {
    // `bridge_grid`'s `grid_auto_flow` field reaching taffy's
    // auto-placement algorithm — with `grid-auto-flow: column` and no
    // explicit placement, 2 children must be auto-placed into
    // successive *columns* of the same row (not successive rows, the
    // `row` default), which the 100px `grid-auto-columns` track makes
    // observable as a 100px x-offset between them.
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
        Some("display:grid;grid-auto-flow:column;grid-auto-columns:100px"),
    );
    let cell_a = doc.append_element(
        Some(grid_container),
        "div",
        Style::default(),
        Some("height:20px"),
    );
    let cell_b = doc.append_element(
        Some(grid_container),
        "div",
        Style::default(),
        Some("height:20px"),
    );
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");

    layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("layout Ok");

    let a_loc = doc.nodes[cell_a].unrounded_layout.location;
    let b_loc = doc.nodes[cell_b].unrounded_layout.location;
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert!(
        (b_loc.x - a_loc.x - 100.0).abs() < 0.5,
        "column-flow auto-placement should put the second item 100px \
             (grid-auto-columns) to the right of the first, got a.x={}, b.x={}",
        a_loc.x,
        b_loc.x
    );
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert!(
        (a_loc.y - b_loc.y).abs() < 0.5,
        "column-flow auto-placement should keep both items on row 1, \
             got a.y={}, b.y={}",
        a_loc.y,
        b_loc.y
    );
}
