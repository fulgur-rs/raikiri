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
fn initial_page_context_errors_have_descriptive_messages() {
    let layout_error = InitialPageContextError::Layout(LayoutError::Internal {
        message: "missing body".to_owned(),
    });
    assert_eq!(
        layout_error.to_string(),
        "initial page placement failed: Layout internal error: missing body"
    );
    let geometry_error = InitialPageContextError::PageGeometryDidNotConverge { iterations: 3 };
    assert_eq!(
        geometry_error.to_string(),
        "initial page context did not converge after 3 placement passes"
    );
}

#[test]
fn find_body_returns_index_when_present() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let _head = doc.append_element(Some(html), "head", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    assert_eq!(find_body(&doc), Some(body));
}

#[test]
fn propagated_start_page_name_handles_dom_edge_cases() {
    use raikiri_style::{build_rule_tree, cascade};

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let named = doc.append_element(Some(body), "section", Style::default(), Some("page: named"));
    let inline = doc.append_element(
        Some(named),
        "span",
        Style::default(),
        Some("display: inline"),
    );
    let inline_text = doc.append_text(inline, "inline content");
    let whitespace = doc.append_text(named, "   ");
    let hidden = doc.append_element(Some(named), "div", Style::default(), Some("display: none"));
    let absolute = doc.append_element(
        Some(named),
        "div",
        Style::default(),
        Some("position: absolute"),
    );
    let normal = doc.append_element(Some(named), "div", Style::default(), None::<&str>);
    let _normal_text = doc.append_text(normal, "normal content");
    let empty = doc.append_element(Some(body), "div", Style::default(), None::<&str>);
    let detached = doc.append_element(None, "div", Style::default(), None::<&str>);

    doc.mark_in_document_flags();
    // `is_display_none` reads the bridged Taffy style rather than the
    // cascade result, so make the defensive predicate explicit here.
    doc.nodes[hidden].style.display = Display::None;
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");

    assert_eq!(
        propagated_start_page_name(&doc, &cr, usize::MAX, None),
        (false, None)
    );
    assert_eq!(propagated_start_page_name(&doc, &cr, 0, None), (true, None));
    assert_eq!(
        propagated_start_page_name(&doc, &cr, detached, None),
        (false, None)
    );
    assert_eq!(
        propagated_start_page_name(&doc, &cr, hidden, None),
        (false, None)
    );
    assert_eq!(
        propagated_start_page_name(&doc, &cr, whitespace, Some("inherited")),
        (false, None)
    );
    assert_eq!(
        propagated_start_page_name(&doc, &cr, inline, Some("inherited")),
        (true, Some("inherited".to_owned()))
    );
    assert!(matches!(
        cr.computed[absolute].position,
        PositionValue::Absolute
    ));
    assert_eq!(
        propagated_start_page_name(&doc, &cr, absolute, Some("inherited")),
        (true, Some("inherited".to_owned()))
    );
    assert_eq!(
        propagated_start_page_name(&doc, &cr, inline_text, Some("inherited")),
        (true, Some("inherited".to_owned()))
    );

    // An arena child can be stale while a cascade is still in use. The
    // bounds check keeps this helper defensive, and the leaf fallback
    // still reports the inherited context.
    doc.nodes[empty].children.push(cr.computed.len());
    assert_eq!(
        propagated_start_page_name(&doc, &cr, empty, Some("inherited")),
        (true, Some("inherited".to_owned()))
    );

    // The normal descendant is reached after whitespace, hidden, and
    // out-of-flow children have been skipped.
    assert_eq!(
        propagated_start_page_name(&doc, &cr, named, Some("outer")),
        (true, Some("outer".to_owned()))
    );
}

#[test]
fn page_border_inset_requires_page_content_box() {
    let mut declarations = std::collections::HashMap::new();
    declarations.insert(
        PropertyKey::BorderTopWidth,
        PropertyValue::BorderTopWidth(Length::Px(3.0)),
    );

    // A border-only page keeps the legacy overlay behavior.
    assert_eq!(
        page_box_side(
            &declarations,
            PropertyKey::PaddingTop,
            PropertyKey::BorderTopWidth,
            100.0
        ),
        0.0
    );

    // An explicit page margin makes the page content box distinct from
    // the border edge, so the border consumes flow space.
    declarations.insert(
        PropertyKey::MarginTop,
        PropertyValue::MarginTop(LengthOrAuto::Length(Length::Px(30.0))),
    );
    assert_eq!(
        page_box_side(
            &declarations,
            PropertyKey::PaddingTop,
            PropertyKey::BorderTopWidth,
            100.0
        ),
        3.0
    );
}

#[test]
fn page_auto_margins_preserve_negative_remainder() {
    use raikiri_style::{Origin, RuleTree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let _body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let mut rules = RuleTree::empty();
    rules.add_stylesheet(
        "@page { size: 300px; width: 340px; height: 340px; margin: auto; }",
        Origin::Author,
    );
    let cascade = cascade(&doc, &rules).expect("cascade Ok");
    let mut page = PageBox::new();
    page.width = 300.0;
    page.height = 300.0;
    let margins = page_margins(&cascade, page);
    assert_eq!(margins.left, -20.0);
    assert_eq!(margins.right, -20.0);
    assert_eq!(margins.top, -20.0);
    assert_eq!(margins.bottom, -20.0);
}

#[test]
fn find_body_returns_none_when_absent() {
    // Fragment 相当: <p> を Document root 直下に append、<body> なし
    let mut doc = Document::new();
    let _p = doc.append_element(Some(0), "p", Style::default(), None::<&str>);
    assert_eq!(find_body(&doc), None);
}

#[test]
fn find_body_iterative_no_stack_overflow_on_deep_dom() {
    // 5000 深さで stack overflow を起こさず None を返す。
    // cascade §deep_nesting_5000_cascade_no_overflow と同水準の regression check。
    let mut doc = Document::new();
    let mut parent = 0usize;
    for _ in 0..5000 {
        parent = doc.append_element(Some(parent), "div", Style::default(), None::<&str>);
    }
    assert_eq!(find_body(&doc), None);
}

#[test]
fn find_body_returns_first_body_in_document_order() {
    // 2 個の <body> がある病理的なケースでは最初の document order の <body> を返す
    // (html5ever は 1 個しか作らない想定だが、defensive contract を pin)
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body1 = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let _body2 = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    assert_eq!(find_body(&doc), Some(body1));
}

#[test]
fn apply_page_box_to_body_sets_body_style_size_to_page_dimensions() {
    use raikiri_traits::PageBox;
    use taffy::{Dimension, Size};

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);

    apply_page_box_to_body(&mut doc, body, PageBox::A4);

    let size: Size<Dimension> = doc.nodes[body].style.size;
    assert_eq!(size.width, Dimension::length(793.7008));
    assert_eq!(size.height, Dimension::length(1122.5197));
}

#[test]
fn page_length_to_px_converts_absolute_units_independently_of_basis() {
    // CSS Values 4 §6.2 Absolute Lengths conversion table: every
    // absolute unit resolves to a fixed `px` multiple and must ignore
    // the percentage basis entirely (unlike `Length::Percent`).
    for basis in [0.0_f32, 100.0, 99999.0] {
        assert!((page_length_to_px(Length::Px(10.0), basis) - 10.0).abs() < 1e-4);
        assert!((page_length_to_px(Length::Pt(12.0), basis) - 16.0).abs() < 1e-3);
        assert!((page_length_to_px(Length::In(1.0), basis) - 96.0).abs() < 1e-3);
        assert!((page_length_to_px(Length::Pc(1.0), basis) - 16.0).abs() < 1e-3);
        assert!((page_length_to_px(Length::Cm(1.0), basis) - 96.0 / 2.54).abs() < 1e-3);
        assert!((page_length_to_px(Length::Mm(10.0), basis) - 96.0 / 2.54).abs() < 1e-3);
        assert!((page_length_to_px(Length::Q(1.0), basis) - 96.0 / 101.6).abs() < 1e-3);
    }
}

#[test]
fn page_length_to_px_falls_back_font_relative_units_to_the_initial_font_size() {
    // Page-context absolutization normally resolves font-relative units
    // before this consumer sees them (see the function's own doc
    // comment); every font-relative variant therefore falls back to a
    // fixed 16px (1em) initial-font-size multiple here.
    for length in [
        Length::Em(1.0),
        Length::Rem(1.0),
        Length::Ex(1.0),
        Length::Rex(1.0),
        Length::Ch(1.0),
        Length::Rch(1.0),
        Length::Ic(1.0),
        Length::Ric(1.0),
        Length::Lh(1.0),
        Length::Rlh(1.0),
    ] {
        assert!((page_length_to_px(length, 0.0) - 16.0).abs() < 1e-4);
    }
    assert!((page_length_to_px(Length::Em(0.5), 0.0) - 8.0).abs() < 1e-4);
}

#[test]
fn page_length_to_px_resolves_percent_against_the_given_basis() {
    // CSS Values 4 §5.5 Percentages: unlike the absolute/font-relative
    // arms above, `Percent` is the one variant that actually consults
    // `basis`.
    assert_eq!(page_length_to_px(Length::Percent(50.0), 200.0), 100.0);
    assert_eq!(page_length_to_px(Length::Percent(50.0), 0.0), 0.0);
}

#[test]
fn layout_single_page_hello_world_produces_body_at_page_width() {
    use raikiri_traits::PageBox;
    let (mut doc, cr) = hello_world_doc();
    layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("layout Ok");
    // body の layout size.width が A4 幅 (793.7008) と一致
    let body_id = find_body(&doc).expect("body exists");
    let body_size = doc.nodes[body_id].unrounded_layout.size;
    assert!(
        (body_size.width - 793.7008).abs() < 0.5,
        "body width should be A4.width (793.7008), got {}",
        body_size.width
    );
    assert!(
        body_size.height > 0.0,
        "body height should be non-zero from block layout of <p>Hi</p>, got {}",
        body_size.height
    );
}

#[test]
fn layout_single_page_can_be_called_multiple_times() {
    use raikiri_traits::PageBox;
    let (mut doc, cr) = hello_world_doc();
    layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("first call Ok");
    let body_id = find_body(&doc).expect("body exists");
    let first_size = doc.nodes[body_id].unrounded_layout.size;

    // 2 回目呼び出し — text_layout の re-entrance clear と layout の再走が
    // 同じ結果を返すことを check (将来 incremental optimization が silent
    // regression を起こしても検出できる)
    layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("second call Ok");
    let second_size = doc.nodes[body_id].unrounded_layout.size;

    assert!((first_size.width - second_size.width).abs() < 0.001);
    assert!((first_size.height - second_size.height).abs() < 0.001);
}

/// Ordering check: `layout_single_page_with_resolver` must refresh flat-tree
/// membership *before* running the `<img>` pre-pass, not rely on the
/// refresh that `layout_single_page` does afterwards. The pre-pass skips
/// inert `<img>` elements by `is_in_document()`, so stale flags would
/// make it fetch a URL for an element that is never laid out or painted.
///
/// This document is deliberately left with `flags_dirty == true` (nothing
/// calls `mark_in_document_flags` between the appends and the layout
/// call), which is the state a caller that skips the parser sink is in.
#[test]
fn used_style_length_helpers_cover_used_value_forms() {
    use raikiri_style::property::CalcLengthPercentage;

    assert_eq!(
        used_style_length_percentage(LengthPercentage::length(12.0), 100.0),
        Some(12.0)
    );
    assert_eq!(
        used_style_length_percentage(LengthPercentage::percent(0.25), 100.0),
        Some(25.0)
    );
    let calc_token = 0usize;
    let calc_ptr = &calc_token as *const usize as *const ();
    assert_eq!(
        used_style_length_percentage(LengthPercentage::calc(calc_ptr), 100.0),
        None
    );
    assert_eq!(
        used_style_length_percentage(LengthPercentage::length(f32::NAN), 100.0),
        None
    );
    assert_eq!(style_dimension_length(Dimension::length(12.0)), Some(12.0));
    assert_eq!(style_dimension_length(Dimension::percent(0.5)), None);
    assert_eq!(style_dimension_length(Dimension::auto()), None);
    assert_eq!(
        used_style_length_percentage_auto(LengthPercentageAuto::length(12.0), 100.0),
        Some(12.0)
    );
    assert_eq!(
        used_style_length_percentage_auto(LengthPercentageAuto::percent(0.25), 100.0),
        Some(25.0)
    );
    assert_eq!(
        used_style_length_percentage_auto(LengthPercentageAuto::auto(), 100.0),
        None
    );
    assert_eq!(
        used_style_length_percentage_auto(LengthPercentageAuto::calc(calc_ptr), 100.0),
        None
    );
    assert_eq!(
        used_computed_length_percentage_or_auto(ComputedLengthPercentageOrAuto::Px(12.0), 100.0),
        Some(12.0)
    );
    assert_eq!(
        used_computed_length_percentage_or_auto(
            ComputedLengthPercentageOrAuto::Percent(25.0),
            100.0
        ),
        Some(25.0)
    );
    assert_eq!(
        used_computed_length_percentage_or_auto(
            ComputedLengthPercentageOrAuto::Calc(CalcLengthPercentage {
                percent: 25.0,
                px: 10.0,
            }),
            100.0
        ),
        Some(35.0)
    );
    assert_eq!(
        used_computed_length_percentage_or_auto(ComputedLengthPercentageOrAuto::Auto, 100.0),
        None
    );
    assert_eq!(
        used_computed_length_percentage_or_auto(
            ComputedLengthPercentageOrAuto::Px(f32::NAN),
            100.0
        ),
        None
    );
}

#[test]
fn first_page_name_follows_ordered_auto_grid_items() {
    use raikiri_style::{Origin, build_rule_tree, cascade};

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
        Some("order:1;page:wide;height:10px"),
    );
    doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("order:0;page:narrow;height:10px"),
    );
    doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:none;order:-1;page:wide"),
    );
    let mut rules = build_rule_tree(&doc);
    rules.add_stylesheet(
        "@page wide { size:200px 300px; margin:5px; } \
             @page narrow { size:120px 180px; margin:12px; }",
        Origin::Author,
    );
    let cascade = cascade(&doc, &rules).expect("cascade Ok");

    assert_eq!(first_page_name(&doc, &cascade).as_deref(), Some("narrow"));
}

#[test]
fn first_page_name_uses_order_in_nested_flex_before_layout() {
    use raikiri_style::{build_rule_tree, cascade};

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let flex = doc.append_element(
        Some(body),
        "section",
        Style::default(),
        Some("display:flex;flex-direction:column"),
    );
    doc.append_element(
        Some(flex),
        "div",
        Style::default(),
        Some("display:block;order:1;page:wide"),
    );
    doc.append_element(
        Some(flex),
        "div",
        Style::default(),
        Some("display:block;order:0;page:narrow"),
    );
    doc.mark_in_document_flags();
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("nested flex cascade Ok");

    assert_eq!(first_page_name(&doc, &cascade).as_deref(), Some("narrow"));
}

#[test]
fn resolved_image_intrinsic_size_changes_flex_grid_column_count_and_page_order() {
    struct FixedImageResolver(f32);
    impl raikiri_traits::ReplacedResolver for FixedImageResolver {
        fn resolve(
            &self,
            _request: raikiri_traits::ResolverRequest<'_>,
        ) -> Result<raikiri_traits::ResolvedIntrinsic, raikiri_traits::ResolverError> {
            Ok(raikiri_traits::ResolvedIntrinsic {
                intrinsic: raikiri_traits::IntrinsicBox::new(self.0, 40.0),
                disposition: raikiri_traits::ResolveDisposition::Ok,
            })
        }
    }

    let layout_for_image_width = |image_width| {
        use raikiri_style::{build_rule_tree, cascade};

        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
        let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
        let flex = doc.append_element(
            Some(body),
            "div",
            Style::default(),
            Some("display:flex;width:300px"),
        );
        let grid = doc.append_element(
                Some(flex),
                "section",
                Style::default(),
                Some("display:grid;order:0;flex:1 1 0;min-width:0;grid-template-columns:repeat(auto-fit,minmax(100px,1fr));grid-template-rows:auto auto"),
            );
        doc.append_element(
            Some(grid),
            "div",
            Style::default(),
            Some("display:block;grid-row:2;order:0;page:wide;height:10px"),
        );
        doc.append_element(
            Some(grid),
            "div",
            Style::default(),
            Some("display:block;grid-row:1;order:1;page:narrow;height:10px"),
        );
        let image = doc.append_element(
            Some(flex),
            "img",
            Style::default(),
            Some("order:1;flex:0 0 auto"),
        );
        doc.set_element_attributes(
            image,
            vec![("src".into(), "https://example.test/image.png".into())],
        );
        doc.mark_in_document_flags();
        let rules = build_rule_tree(&doc);
        let cascade = cascade(&doc, &rules).expect("cascade Ok");
        layout_single_page_with_resolver_and_base_url(
            &mut doc,
            &cascade,
            PageBox::A4,
            FontContext::new(),
            &FixedImageResolver(image_width),
            None,
        )
        .expect("layout Ok");
        (
            doc.nodes[grid].grid_column_count,
            first_page_name(&doc, &cascade),
        )
    };

    let (small_columns, small_page) = layout_for_image_width(50.0);
    let (large_columns, large_page) = layout_for_image_width(250.0);
    assert_eq!(small_columns, 2);
    assert_eq!(small_page.as_deref(), Some("wide"));
    assert_eq!(large_columns, 1);
    assert_eq!(large_page.as_deref(), Some("narrow"));
}

#[test]
fn first_page_name_ignores_resolved_order_from_an_older_cascade() {
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let flex = doc.append_element(
        Some(body),
        "section",
        Style::default(),
        Some("display:flex;flex-direction:column"),
    );
    let wide = doc.append_element(
        Some(flex),
        "div",
        Style::default(),
        Some("display:block;order:1;page:wide"),
    );
    let narrow = doc.append_element(
        Some(flex),
        "div",
        Style::default(),
        Some("display:block;order:0;page:narrow"),
    );
    doc.mark_in_document_flags();
    let old_rules = build_rule_tree(&doc);
    let old_cascade = cascade(&doc, &old_rules).expect("initial cascade Ok");
    layout_single_page(&mut doc, &old_cascade, PageBox::A4, FontContext::new())
        .expect("initial layout Ok");
    assert!(!doc.layout_dirty);
    assert!(document_has_resolved_pagination_order(&doc, &old_cascade));
    assert_eq!(doc.nodes[flex].layout_children(), &[narrow, wide]);

    doc.nodes[wide].data.as_element_mut().unwrap().inline_style =
        Some("display:block;order:-1;page:wide".into());
    let new_rules = build_rule_tree(&doc);
    let new_cascade = cascade(&doc, &new_rules).expect("updated cascade Ok");
    assert_eq!(old_cascade.computed[wide].order, 1);
    assert_eq!(new_cascade.computed[wide].order, -1);
    assert!(!document_has_resolved_pagination_order(&doc, &new_cascade));

    assert_eq!(first_page_name(&doc, &new_cascade).as_deref(), Some("wide"));
}

#[test]
fn first_page_name_ignores_resolved_grid_rows_from_an_older_cascade() {
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let grid = doc.append_element(
        Some(body),
        "section",
        Style::default(),
        Some("display:grid;grid-template-columns:100%;grid-template-rows:auto auto"),
    );
    let wide = doc.append_element(
        Some(grid),
        "div",
        Style::default(),
        Some("display:block;grid-row:2;order:0;page:wide"),
    );
    let narrow = doc.append_element(
        Some(grid),
        "div",
        Style::default(),
        Some("display:block;grid-row:1;order:0;page:narrow"),
    );
    doc.mark_in_document_flags();
    let old_rules = build_rule_tree(&doc);
    let old_cascade = cascade(&doc, &old_rules).expect("initial cascade Ok");
    layout_single_page(&mut doc, &old_cascade, PageBox::A4, FontContext::new())
        .expect("initial layout Ok");
    assert!(document_has_resolved_pagination_order(&doc, &old_cascade));
    assert_eq!(
        first_page_name(&doc, &old_cascade).as_deref(),
        Some("narrow")
    );

    doc.nodes[wide].data.as_element_mut().unwrap().inline_style =
        Some("display:block;grid-row:1;order:0;page:wide".into());
    doc.nodes[narrow]
        .data
        .as_element_mut()
        .unwrap()
        .inline_style = Some("display:block;grid-row:2;order:0;page:narrow".into());
    let new_rules = build_rule_tree(&doc);
    let new_cascade = cascade(&doc, &new_rules).expect("updated cascade Ok");
    assert_ne!(old_cascade.generation(), new_cascade.generation());
    assert_eq!(
        old_cascade.computed[wide].grid_row_start,
        raikiri_style::property::GridLineValue::Line(2)
    );
    assert_eq!(
        old_cascade.computed[narrow].grid_row_start,
        raikiri_style::property::GridLineValue::Line(1)
    );
    assert_eq!(
        new_cascade.computed[wide].grid_row_start,
        raikiri_style::property::GridLineValue::Line(1)
    );
    assert_eq!(
        new_cascade.computed[narrow].grid_row_start,
        raikiri_style::property::GridLineValue::Line(2)
    );
    assert_eq!(
        old_cascade.computed[wide].order,
        new_cascade.computed[wide].order
    );
    assert_eq!(
        old_cascade.computed[narrow].order,
        new_cascade.computed[narrow].order
    );
    assert!(!document_has_resolved_pagination_order(&doc, &new_cascade));

    assert_eq!(first_page_name(&doc, &new_cascade).as_deref(), Some("wide"));

    layout_single_page(&mut doc, &new_cascade, PageBox::A4, FontContext::new())
        .expect("updated layout Ok");
    assert!(document_has_resolved_pagination_order(&doc, &new_cascade));
    assert_eq!(first_page_name(&doc, &new_cascade).as_deref(), Some("wide"));
}

#[test]
fn first_page_name_follows_order_and_column_reverse_for_flex_children() {
    use raikiri_style::{build_rule_tree, cascade};

    let mut normal_doc = Document::new();
    let html = normal_doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = normal_doc.append_element(
        Some(html),
        "body",
        Style::default(),
        Some("display:flex;flex-direction:column"),
    );
    normal_doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("order:1;page:wide"),
    );
    normal_doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("order:0;page:narrow"),
    );
    let rules = build_rule_tree(&normal_doc);
    let normal_cascade = cascade(&normal_doc, &rules).expect("normal flex cascade Ok");
    assert_eq!(
        first_page_name(&normal_doc, &normal_cascade).as_deref(),
        Some("narrow")
    );

    let mut reverse_doc = Document::new();
    let html = reverse_doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = reverse_doc.append_element(
        Some(html),
        "body",
        Style::default(),
        Some("display:flex;flex-direction:column-reverse"),
    );
    reverse_doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("order:0;page:wide"),
    );
    reverse_doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("order:1;page:narrow"),
    );
    let rules = build_rule_tree(&reverse_doc);
    let reverse_cascade = cascade(&reverse_doc, &rules).expect("reverse flex cascade Ok");
    assert_eq!(
        first_page_name(&reverse_doc, &reverse_cascade).as_deref(),
        Some("narrow")
    );
}

#[test]
fn layout_pages_orders_implicit_and_repeated_single_column_grids() {
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    for grid_style in [
        "display:grid",
        "display:grid;grid-template-columns:repeat(1,40px)",
    ] {
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
        let body = doc.append_element(Some(html), "body", Style::default(), Some(grid_style));
        doc.append_element(
            Some(body),
            "div",
            Style::default(),
            Some("display:block;grid-column:1;order:1;page:wide;height:10px"),
        );
        doc.append_element(
            Some(body),
            "div",
            Style::default(),
            Some("display:block;grid-column:1;order:0;page:narrow;height:10px"),
        );
        let rules = build_rule_tree(&doc);
        let cascade = cascade(&doc, &rules).expect("cascade Ok");
        assert_eq!(first_page_name(&doc, &cascade).as_deref(), Some("narrow"));

        let slices = layout_pages(&mut doc, &cascade, PageBox::A4, FontContext::new())
            .expect("pagination Ok");
        assert_eq!(doc.nodes[body].grid_column_count, 1);
        assert_eq!(slices.len(), 2);
        assert_eq!(slices[0].page_name.as_deref(), Some("narrow"));
        assert_eq!(slices[1].page_name.as_deref(), Some("wide"));
    }
}

#[test]
fn initial_page_child_order_keeps_explicit_grid_columns_in_source_order() {
    use raikiri_style::{build_rule_tree, cascade};

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(
        Some(html),
        "body",
        Style::default(),
        Some("display:grid;grid-template-columns:100px 100px"),
    );
    let second_column = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("order:0;grid-column:2;page:wide;height:10px"),
    );
    let first_column = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("order:1;grid-column:1;page:narrow;height:10px"),
    );
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");

    assert_eq!(
        initial_page_child_order(&doc, &cascade, body),
        vec![second_column, first_column]
    );
}

#[test]
fn grid_pagination_item_filter_matches_in_flow_boxes_and_text() {
    use raikiri_style::{build_rule_tree, cascade};

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let static_box = doc.append_element(Some(body), "div", Style::default(), None::<&str>);
    let relative_box = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("position:relative"),
    );
    let hidden_box = doc.append_element(Some(body), "div", Style::default(), Some("display:none"));
    let absolute_box = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("position:absolute"),
    );
    let text = doc.append_text(body, "content");
    let whitespace = doc.append_text(body, "   ");
    let comment = doc.append_comment(Some(body), "not a grid item");
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");

    assert!(is_in_flow_grid_item_for_pagination(
        &doc, &cascade, static_box
    ));
    assert!(is_in_flow_grid_item_for_pagination(
        &doc,
        &cascade,
        relative_box
    ));
    assert!(!is_in_flow_grid_item_for_pagination(
        &doc, &cascade, hidden_box
    ));
    assert!(!is_in_flow_grid_item_for_pagination(
        &doc,
        &cascade,
        absolute_box
    ));
    assert!(is_in_flow_grid_item_for_pagination(&doc, &cascade, text));
    assert!(!is_in_flow_grid_item_for_pagination(
        &doc, &cascade, whitespace
    ));
    assert!(!is_in_flow_grid_item_for_pagination(
        &doc, &cascade, comment
    ));
}

#[test]
fn pagination_child_order_falls_back_when_grid_row_details_are_incomplete() {
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let grid = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:grid;grid-template-columns:100px"),
    );
    let later = doc.append_element(
        Some(grid),
        "div",
        Style::default(),
        Some("order:1;height:10px"),
    );
    let earlier = doc.append_element(
        Some(grid),
        "div",
        Style::default(),
        Some("order:0;height:10px"),
    );
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");
    layout_single_page(&mut doc, &cascade, PageBox::A4, FontContext::new()).expect("layout Ok");
    assert_eq!(doc.nodes[grid].grid_column_count, 1);
    doc.nodes[grid].grid_item_row_starts = Vec::new().into_boxed_slice();

    assert_eq!(
        pagination_child_order(&doc, &cascade, grid),
        vec![earlier, later]
    );
}
