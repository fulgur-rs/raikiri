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

fn embedded_ic_font_dir() -> tempfile::TempDir {
    let tmp = tempfile::tempdir().expect("temporary embedded ic font directory");
    for (name, bytes) in [
        (
            "Ahem.ttf",
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/data/text-autospace/Ahem.ttf"
            )) as &[u8],
        ),
        (
            "CanvasTest-nospace.ttf",
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/data/text-autospace/CanvasTest-nospace.ttf"
            )) as &[u8],
        ),
    ] {
        std::fs::write(tmp.path().join(name), bytes).expect("write embedded font");
    }
    for (name, bytes) in [
        (
            "ZeroWidth",
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/data/text-autospace/IcTestZeroWidth.woff2"
            )) as &[u8],
        ),
        (
            "HalfWidth",
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/data/text-autospace/IcTestHalfWidth.woff2"
            )) as &[u8],
        ),
        (
            "FullWidth",
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/data/text-autospace/IcTestFullWidth.woff2"
            )) as &[u8],
        ),
    ] {
        let decoded = wuff::decompress_woff2(bytes).expect("decode embedded ic fixture");
        std::fs::write(tmp.path().join(format!("IcTest{name}.ttf")), decoded)
            .expect("write decoded embedded ic fixture");
    }
    tmp
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
fn boundary_shaping_joiners_cover_joining_scripts_without_touching_latin() {
    assert_eq!(
        add_boundary_shaping_joiners("ع".to_owned(), true, true),
        "\u{200d}ع\u{200d}"
    );
    assert_eq!(
        add_boundary_shaping_joiners("ߞ".to_owned(), true, false),
        "\u{200d}ߞ"
    );
    assert_eq!(
        add_boundary_shaping_joiners("ᠨ".to_owned(), false, true),
        "ᠨ\u{200d}"
    );
    assert_eq!(
        add_boundary_shaping_joiners("A".to_owned(), true, true),
        "A"
    );
    // Only the characters that actually sit on each edge decide whether
    // a joiner is needed; an authored ZWNJ or a space at an edge blocks
    // that edge without affecting the other one.
    assert_eq!(
        add_boundary_shaping_joiners("ع\u{200c}".to_owned(), true, true),
        "\u{200d}ع\u{200c}"
    );
    assert_eq!(
        add_boundary_shaping_joiners(" ع".to_owned(), true, true),
        " ع\u{200d}"
    );
    assert_eq!(
        add_boundary_shaping_joiners("عA".to_owned(), true, true),
        "\u{200d}عA"
    );
    assert_eq!(add_boundary_shaping_joiners(String::new(), true, true), "");
    // Arabic presentation forms join like their nominal letters.
    assert_eq!(
        add_boundary_shaping_joiners("\u{fb50}\u{fe8d}".to_owned(), true, true),
        "\u{200d}\u{fb50}\u{fe8d}\u{200d}"
    );
}

#[test]
fn boundary_shaping_adjacency_uses_the_characters_on_the_boundary() {
    use raikiri_style::{build_rule_tree, cascade};

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let block = |doc: &mut Document| {
        doc.append_element(Some(body), "div", Style::default(), Some("display:block"))
    };
    let span = |doc: &mut Document, parent: usize, style: &str| {
        doc.append_element(Some(parent), "span", Style::default(), Some(style))
    };

    // `<div>هذا <span>مثال</span></div>`: a space ends the previous word.
    let word_block = block(&mut doc);
    let word_left = doc.append_text(word_block, "هذا ");
    let word_span = span(&mut doc, word_block, "display:inline");
    let word_inner = doc.append_text(word_span, "مثال");

    // `<div><span>ع</span> <span>ع</span></div>`: whitespace-only sibling.
    let ws_block = block(&mut doc);
    let ws_first = span(&mut doc, ws_block, "display:inline");
    let ws_left = doc.append_text(ws_first, "ع");
    let _ws = doc.append_text(ws_block, " ");
    let ws_second = span(&mut doc, ws_block, "display:inline");
    let ws_right = doc.append_text(ws_second, "ع");

    // Atomic inline nested at the neighbor's edge.
    let atomic_block = block(&mut doc);
    let atomic_left = doc.append_text(atomic_block, "ع");
    let atomic_outer = span(&mut doc, atomic_block, "display:inline");
    let atomic = span(&mut doc, atomic_outer, "display:inline-block");
    let _atomic_text = doc.append_text(atomic, "ع");

    // Inline padding nested at the neighbor's edge.
    let padded_block = block(&mut doc);
    let padded_left = doc.append_text(padded_block, "ع");
    let padded_outer = span(&mut doc, padded_block, "display:inline");
    let padded = span(&mut doc, padded_outer, "display:inline;padding-left:1px");
    let _padded_text = doc.append_text(padded, "ع");

    // `display:none` content at the edge generates nothing; the next
    // rendered character decides.
    let hidden_block = block(&mut doc);
    let hidden_left = doc.append_text(hidden_block, "ع");
    let hidden_outer = span(&mut doc, hidden_block, "display:inline");
    let hidden = span(&mut doc, hidden_outer, "display:none");
    let _hidden_text = doc.append_text(hidden, "A");
    let _shown = doc.append_text(hidden_outer, "ع");
    let hidden_latin_block = block(&mut doc);
    let hidden_latin_left = doc.append_text(hidden_latin_block, "ع");
    let hidden_latin_outer = span(&mut doc, hidden_latin_block, "display:inline");
    let hidden_arabic = span(&mut doc, hidden_latin_outer, "display:none");
    let _hidden_arabic_text = doc.append_text(hidden_arabic, "ع");
    let _shown_latin = doc.append_text(hidden_latin_outer, "A");

    // An empty inline box without edges is transparent to joining.
    let empty_block = block(&mut doc);
    let empty_left = doc.append_text(empty_block, "ع");
    let _empty = span(&mut doc, empty_block, "display:inline");
    let empty_right = doc.append_text(empty_block, "ع");

    // `<br>` ends the line.
    let br_block = block(&mut doc);
    let br_left = doc.append_text(br_block, "ع");
    let _br = doc.append_element(Some(br_block), "br", Style::default(), None::<&str>);
    let _br_right = doc.append_text(br_block, "ع");

    // Comments and template contents render nothing inline.
    let skipped_block = block(&mut doc);
    let skipped_left = doc.append_text(skipped_block, "ع");
    let _comment = doc.append_comment(Some(skipped_block), "x");
    let template = doc.append_element(
        Some(skipped_block),
        "template",
        Style::default(),
        Some("display:inline"),
    );
    let _template_text = doc.append_text(template, "A");
    let _skipped_right = doc.append_text(skipped_block, "ع");

    // An authored ZWJ at the neighbor's edge still continues the context.
    let zwj_block = block(&mut doc);
    let zwj_left = doc.append_text(zwj_block, "ع");
    let zwj_span = span(&mut doc, zwj_block, "display:inline");
    let _zwj = doc.append_text(zwj_span, "\u{200d}A");

    doc.mark_in_document_flags();
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");
    let mut parent_of = vec![None; doc.nodes.len()];
    for idx in 0..doc.nodes.len() {
        for &child in &doc.nodes[idx].children.clone() {
            parent_of[child] = Some(idx);
        }
    }
    let adjacent = |idx, dir| boundary_shaping_adjacent(&doc, &cr, &parent_of, idx, dir);

    // The following word starts with a joining letter, but this run ends
    // in a space, so its own edge keeps the joiner out.
    assert_eq!(
        add_boundary_shaping_joiners("هذا ".to_owned(), false, adjacent(word_left, 1)),
        "هذا "
    );
    assert!(!adjacent(word_inner, -1));
    assert!(!adjacent(ws_left, 1));
    assert!(!adjacent(ws_right, -1));
    assert!(!adjacent(atomic_left, 1));
    assert!(!adjacent(padded_left, 1));
    assert!(adjacent(hidden_left, 1));
    assert!(!adjacent(hidden_latin_left, 1));
    assert!(adjacent(empty_left, 1));
    assert!(adjacent(empty_right, -1));
    assert!(!adjacent(br_left, 1));
    assert!(adjacent(zwj_left, 1));
    assert!(adjacent(skipped_left, 1));
}

#[test]
fn boundary_shaping_adjacency_respects_inline_box_model_boundaries() {
    use raikiri_style::{build_rule_tree, cascade};

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let block = doc.append_element(Some(body), "div", Style::default(), Some("display:block"));
    let left = doc.append_text(block, "ع");
    let spaced = doc.append_element(
        Some(block),
        "span",
        Style::default(),
        Some("display:inline;margin:1px"),
    );
    let inner = doc.append_text(spaced, "ع");
    let right = doc.append_text(block, "ع");
    let plain_block =
        doc.append_element(Some(body), "div", Style::default(), Some("display:block"));
    let plain_left = doc.append_text(plain_block, "ع");
    let plain_span = doc.append_element(
        Some(plain_block),
        "span",
        Style::default(),
        Some("display:inline"),
    );
    let plain_inner = doc.append_text(plain_span, "ع");
    let zwnj_block = doc.append_element(Some(body), "div", Style::default(), Some("display:block"));
    let zwnj_left = doc.append_text(zwnj_block, "ع");
    let zwnj_span = doc.append_element(
        Some(zwnj_block),
        "span",
        Style::default(),
        Some("display:inline"),
    );
    let _zwnj = doc.append_text(zwnj_span, "\u{200c}");
    let zwnj_right = doc.append_text(zwnj_block, "ع");
    let mixed_block =
        doc.append_element(Some(body), "div", Style::default(), Some("display:block"));
    let mixed_arabic = doc.append_text(mixed_block, "ع");
    let mixed_latin_span = doc.append_element(
        Some(mixed_block),
        "span",
        Style::default(),
        Some("display:inline"),
    );
    let _mixed_latin = doc.append_text(mixed_latin_span, "A");
    let bdi_block = doc.append_element(Some(body), "div", Style::default(), Some("display:block"));
    let bdi_left = doc.append_text(bdi_block, "ع");
    let bdi = doc.append_element(Some(bdi_block), "bdi", Style::default(), None::<&str>);
    let bdi_inner = doc.append_text(bdi, "ع");
    let bdi_right = doc.append_text(bdi_block, "ع");
    let auto_block = doc.append_element(Some(body), "div", Style::default(), Some("display:block"));
    let auto_left = doc.append_text(auto_block, "ع");
    let auto_span = doc.append_element(
        Some(auto_block),
        "span",
        Style::default(),
        Some("display:inline"),
    );
    doc.set_element_attributes(auto_span, vec![("dir".into(), "auto".into())]);
    let auto_inner = doc.append_text(auto_span, "ع");
    let auto_right = doc.append_text(auto_block, "ع");
    doc.mark_in_document_flags();
    let rules = build_rule_tree(&doc);
    let mut cr = cascade(&doc, &rules).expect("cascade Ok");
    cr.computed[spaced].margin.top = ComputedLengthPercentageOrAuto::Px(0.0);
    cr.computed[spaced].margin.bottom = ComputedLengthPercentageOrAuto::Px(0.0);
    cr.computed[spaced].margin.left = ComputedLengthPercentageOrAuto::Calc(CalcLengthPercentage {
        percent: 25.0,
        px: 1.0,
    });
    cr.computed[spaced].margin.right = ComputedLengthPercentageOrAuto::Auto;
    let mut parent_of = vec![None; doc.nodes.len()];
    for idx in 0..doc.nodes.len() {
        for &child in &doc.nodes[idx].children.clone() {
            parent_of[child] = Some(idx);
        }
    }

    assert!(!boundary_shaping_adjacent(&doc, &cr, &parent_of, left, 1));
    assert!(!boundary_shaping_adjacent(&doc, &cr, &parent_of, inner, -1));
    assert!(!boundary_shaping_adjacent(&doc, &cr, &parent_of, inner, 1));
    assert!(!boundary_shaping_adjacent(&doc, &cr, &parent_of, right, -1));
    assert!(boundary_shaping_adjacent(
        &doc, &cr, &parent_of, plain_left, 1
    ));
    assert!(boundary_shaping_adjacent(
        &doc,
        &cr,
        &parent_of,
        plain_inner,
        -1
    ));
    assert!(!boundary_shaping_adjacent(
        &doc, &cr, &parent_of, zwnj_left, 1
    ));
    assert!(!boundary_shaping_adjacent(
        &doc, &cr, &parent_of, zwnj_right, -1
    ));
    assert!(!boundary_shaping_adjacent(&doc, &cr, &parent_of, 0, 1));
    assert!(!boundary_shaping_adjacent(
        &doc,
        &cr,
        &parent_of,
        mixed_arabic,
        1
    ));
    assert!(is_shaping_isolation_boundary(&doc, bdi));
    assert!(is_shaping_isolation_boundary(&doc, auto_span));
    assert!(!is_shaping_isolation_boundary(&doc, plain_span));
    assert!(!boundary_shaping_adjacent(
        &doc, &cr, &parent_of, bdi_left, 1
    ));
    assert!(!boundary_shaping_adjacent(
        &doc, &cr, &parent_of, bdi_inner, -1
    ));
    assert!(!boundary_shaping_adjacent(
        &doc, &cr, &parent_of, bdi_right, -1
    ));
    assert!(!boundary_shaping_adjacent(
        &doc, &cr, &parent_of, auto_left, 1
    ));
    assert!(!boundary_shaping_adjacent(
        &doc, &cr, &parent_of, auto_inner, -1
    ));
    assert!(!boundary_shaping_adjacent(
        &doc, &cr, &parent_of, auto_right, -1
    ));
    let mut broken_parent_of = parent_of.clone();
    broken_parent_of[plain_left] = Some(body);
    assert!(!boundary_shaping_adjacent(
        &doc,
        &cr,
        &broken_parent_of,
        plain_left,
        1
    ));
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
fn collapse_segment_break_runs_skip_whitespace_and_ignorables() {
    use raikiri_style::{build_rule_tree, cascade};

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let wide = doc.append_element(Some(body), "p", Style::default(), None::<&str>);
    let _ = doc.append_text(wide, "\u{4e00}");
    let _ = doc.append_text(wide, "\u{4e9b}");
    let first_space = doc.append_text(wide, " ");
    let first_break = doc.append_text(wide, "\n");
    let second_break = doc.append_text(wide, "\n");
    let last_space = doc.append_text(wide, " ");
    let _ = doc.append_text(wide, "\u{4e2d}");
    let _ = doc.append_text(wide, "\u{6587}");

    let ignorable = doc.append_element(Some(body), "p", Style::default(), None::<&str>);
    let _ = doc.append_text(ignorable, "葛");
    let _ = doc.append_text(ignorable, "\u{00ad}");
    let _ = doc.append_text(ignorable, "\n");
    let ignorable_tail = doc.append_text(ignorable, "  葛");

    doc.mark_in_document_flags();
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");
    let mut parent_of = vec![None; doc.nodes.len()];
    for idx in 0..doc.nodes.len() {
        for &child in &doc.nodes[idx].children.clone() {
            if child < parent_of.len() {
                parent_of[child] = Some(idx);
            }
        }
    }
    let collapse = |idx: usize| {
        let text = text_of(&doc, idx).expect("text node");
        collapse_text_for_shaping(
            &doc,
            &cr,
            &parent_of,
            idx,
            text,
            cr.computed[idx].white_space,
        )
    };

    for idx in [first_space, first_break, second_break, last_space] {
        let out = collapse(idx);
        assert_eq!(out.text, "", "node {idx} unexpectedly shaped");
        assert_eq!(out.migrate_count, 0, "node {idx} migrated a space");
    }
    assert_eq!(collapse(ignorable_tail).text, "葛");
}

#[test]
fn collapse_interior_lone_space_migrates_forward() {
    use raikiri_style::{build_rule_tree, cascade};
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let div = doc.append_element(Some(body), "div", Style::default(), Some("display: block"));
    let s1 = doc.append_element(Some(div), "span", Style::default(), Some("display: inline"));
    let _t1 = doc.append_text(s1, "a");
    let ws = doc.append_text(div, " ");
    let s2 = doc.append_element(Some(div), "span", Style::default(), Some("display: inline"));
    let _t2 = doc.append_text(s2, "b");
    doc.mark_in_document_flags();
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");
    let mut parent_of: Vec<Option<usize>> = vec![None; doc.nodes.len()];
    for idx in 0..doc.nodes.len() {
        for &c in &doc.nodes[idx].children.clone() {
            if c < parent_of.len() {
                parent_of[c] = Some(idx);
            }
        }
    }
    let out =
        collapse_text_for_shaping(&doc, &cr, &parent_of, ws, " ", cr.computed[ws].white_space);
    eprintln!(
        "collapsed={:?} migrate={} ws={:?}",
        out.text, out.migrate_count, cr.computed[ws].white_space
    );
    assert_eq!(out.text, "");
    assert_eq!(out.migrate_count, 1);
}

#[test]
fn collapse_text_for_shaping_normalizes_cr_crlf_tab_and_form_feed_in_normal_mode() {
    use raikiri_style::{build_rule_tree, cascade};

    // Phase I of CSS Text 3 §4.1.1 white-space processing: CR and CRLF
    // become a single LF, and tab/form-feed become a space, before the
    // ordinary collapse rules run. `normal` then collapses that
    // resulting single-LF run to one space, the same as any other
    // interior whitespace run.
    let collapse_in_normal_p = |raw: &str| -> CollapsedText {
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
        let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
        let p = doc.append_element(Some(body), "p", Style::default(), None::<&str>);
        let text = doc.append_text(p, raw);
        doc.mark_in_document_flags();
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");
        let mut parent_of: Vec<Option<usize>> = vec![None; doc.nodes.len()];
        for idx in 0..doc.nodes.len() {
            for &c in &doc.nodes[idx].children.clone() {
                if c < parent_of.len() {
                    parent_of[c] = Some(idx);
                }
            }
        }
        collapse_text_for_shaping(
            &doc,
            &cr,
            &parent_of,
            text,
            raw,
            cr.computed[text].white_space,
        )
    };

    for raw in ["a\r\nb", "a\rb", "a\tb", "a\x0Cb"] {
        let out = collapse_in_normal_p(raw);
        assert_eq!(out.text, "a b", "input {raw:?} should collapse to \"a b\"");
        assert_eq!(out.migrate_count, 0, "input {raw:?}");
    }
}

#[test]
fn collapse_text_for_shaping_extended_dedupe_skips_removed_and_hidden_siblings() {
    use raikiri_style::{build_rule_tree, cascade};

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let p = doc.append_element(Some(body), "p", Style::default(), None::<&str>);
    // Nearest-to-farthest from the trailing space under test: a
    // `display:none` element, a comment (cleared from
    // `IS_IN_DOCUMENT` by `mark_in_document_flags`), then the text run
    // that actually decides the outcome.
    let _prefix = doc.append_text(p, "hello ");
    let _hidden = doc.append_element(Some(p), "span", Style::default(), Some("display:none"));
    let _comment = doc.append_comment(Some(p), "note");
    let trailing = doc.append_text(p, " ");

    doc.mark_in_document_flags();
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");
    let mut parent_of: Vec<Option<usize>> = vec![None; doc.nodes.len()];
    for idx in 0..doc.nodes.len() {
        for &c in &doc.nodes[idx].children.clone() {
            if c < parent_of.len() {
                parent_of[c] = Some(idx);
            }
        }
    }

    let out = collapse_text_for_shaping(
        &doc,
        &cr,
        &parent_of,
        trailing,
        " ",
        cr.computed[trailing].white_space,
    );
    // The `display:none` span and the out-of-document comment are both
    // skipped while walking backward; the whitespace-ending "hello "
    // text node beyond them is what the run actually dedupes against,
    // so the trailing space node contributes nothing further.
    assert_eq!(out.text, "");
    assert_eq!(out.migrate_count, 0);
}

#[test]
fn collapse_text_for_shaping_drops_a_lone_break_at_both_block_edges() {
    use raikiri_style::{build_rule_tree, cascade};

    // CSS Text 3 §4.1.2: a collapsible segment break with no inline
    // content on either side (both `before`/`after` face a block edge,
    // not an inline sibling) is removed outright rather than becoming
    // a space, unlike the between-inlines case already covered by
    // `collapse_single_node_break_becomes_space_and_lone_wide_break_drops`.
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let p = doc.append_element(Some(body), "p", Style::default(), None::<&str>);
    let wb = doc.append_text(p, "\n");

    doc.mark_in_document_flags();
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");
    let mut parent_of: Vec<Option<usize>> = vec![None; doc.nodes.len()];
    for idx in 0..doc.nodes.len() {
        for &c in &doc.nodes[idx].children.clone() {
            if c < parent_of.len() {
                parent_of[c] = Some(idx);
            }
        }
    }

    let out =
        collapse_text_for_shaping(&doc, &cr, &parent_of, wb, "\n", cr.computed[wb].white_space);
    assert_eq!(out.text, "");
    assert_eq!(out.migrate_count, 0);
}

#[test]
fn collapse_text_for_shaping_drops_a_break_adjacent_to_a_zero_width_space() {
    use raikiri_style::{build_rule_tree, cascade};

    // CSS Text 3 §4.1.2: a segment break next to a zero-width space is
    // removed, mirroring the wide-character-pair removal rule but
    // keyed on U+200B specifically rather than East Asian Width.
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let p = doc.append_element(Some(body), "p", Style::default(), None::<&str>);
    let _before_text = doc.append_text(p, "a\u{200B}");
    let wb = doc.append_text(p, "\n");
    let _after_text = doc.append_text(p, "b");

    doc.mark_in_document_flags();
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");
    let mut parent_of: Vec<Option<usize>> = vec![None; doc.nodes.len()];
    for idx in 0..doc.nodes.len() {
        for &c in &doc.nodes[idx].children.clone() {
            if c < parent_of.len() {
                parent_of[c] = Some(idx);
            }
        }
    }

    let out =
        collapse_text_for_shaping(&doc, &cr, &parent_of, wb, "\n", cr.computed[wb].white_space);
    assert_eq!(out.text, "");
    assert_eq!(out.migrate_count, 0);
}

#[test]
fn collapse_text_for_shaping_drops_a_lone_space_at_both_block_edges() {
    use raikiri_style::{build_rule_tree, cascade};

    // The final "fully collapsed" arm: a lone collapsible space is
    // kept (and migrated forward) only when *both* sides face inline
    // content (`before && after`, already covered by
    // `collapse_interior_lone_space_migrates_forward` above); at a
    // block edge on either side it is dropped instead.
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let p = doc.append_element(Some(body), "p", Style::default(), None::<&str>);
    let sole = doc.append_text(p, " ");

    doc.mark_in_document_flags();
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");
    let mut parent_of: Vec<Option<usize>> = vec![None; doc.nodes.len()];
    for idx in 0..doc.nodes.len() {
        for &c in &doc.nodes[idx].children.clone() {
            if c < parent_of.len() {
                parent_of[c] = Some(idx);
            }
        }
    }

    let out = collapse_text_for_shaping(
        &doc,
        &cr,
        &parent_of,
        sole,
        " ",
        cr.computed[sole].white_space,
    );
    assert_eq!(out.text, "");
    assert_eq!(out.migrate_count, 0);
}

#[test]
fn collapse_text_for_shaping_extended_dedupe_stops_at_a_non_whitespace_ending_sibling() {
    use raikiri_style::{build_rule_tree, cascade};

    // The dedupe walk-back stops (rather than deduping through) the
    // first in-document, non-`display:none` sibling it reaches once
    // that sibling's own raw text does *not* end in whitespace:
    // nothing to dedupe against, so the space under test falls
    // through to the ordinary before/after handling instead.
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let p = doc.append_element(Some(body), "p", Style::default(), None::<&str>);
    let _prefix = doc.append_text(p, "hello");
    let trailing = doc.append_text(p, " ");

    doc.mark_in_document_flags();
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");
    let mut parent_of: Vec<Option<usize>> = vec![None; doc.nodes.len()];
    for idx in 0..doc.nodes.len() {
        for &c in &doc.nodes[idx].children.clone() {
            if c < parent_of.len() {
                parent_of[c] = Some(idx);
            }
        }
    }

    let out = collapse_text_for_shaping(
        &doc,
        &cr,
        &parent_of,
        trailing,
        " ",
        cr.computed[trailing].white_space,
    );
    // "hello" (no trailing whitespace) does not extend the dedupe run,
    // so this falls through to the lone-space arm: kept only when
    // both sides face inline content. There is no sibling *after* the
    // trailing space, so it is dropped rather than migrated.
    assert_eq!(out.text, "");
    assert_eq!(out.migrate_count, 0);
}

#[test]
fn collapse_text_for_shaping_pre_line_dedupe_stops_at_a_preserved_break() {
    use raikiri_style::{build_rule_tree, cascade};

    // `pre-line`-only nuance: the extended dedupe walk-back does not
    // collapse a later whitespace-only node against an earlier
    // sibling whose own raw text carries a preserved break -- that
    // break is meaningful content under `pre-line`, not a run this
    // node should silently absorb.
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let p = doc.append_element(
        Some(body),
        "p",
        Style::default(),
        Some("white-space:pre-line"),
    );
    let _prev = doc.append_text(p, "\n ");
    let target = doc.append_text(p, " ");

    doc.mark_in_document_flags();
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");
    let mut parent_of: Vec<Option<usize>> = vec![None; doc.nodes.len()];
    for idx in 0..doc.nodes.len() {
        for &c in &doc.nodes[idx].children.clone() {
            if c < parent_of.len() {
                parent_of[c] = Some(idx);
            }
        }
    }

    let out = collapse_text_for_shaping(
        &doc,
        &cr,
        &parent_of,
        target,
        " ",
        cr.computed[target].white_space,
    );
    // The walk-back stops at the preserved-break sibling instead of
    // deduping through it; with no significant sibling on either side
    // (the preceding node is itself whitespace-only), the lone space
    // is then dropped by the ordinary before/after rule.
    assert_eq!(out.text, "");
    assert_eq!(out.migrate_count, 0);
}

#[test]
fn collapse_text_for_shaping_converts_a_kept_leading_space_to_nbsp() {
    use raikiri_style::{build_rule_tree, cascade};

    // A leading collapsible space that survives trimming (because
    // inline content precedes it) is rewritten to NBSP in place:
    // parley trims an ordinary leading space from an independently
    // shaped run, but keeps a leading NBSP, which is how this node's
    // own shaped text preserves the gap after the previous node's
    // content.
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let p = doc.append_element(Some(body), "p", Style::default(), None::<&str>);
    let _prev = doc.append_text(p, "X");
    let target = doc.append_text(p, " Y");

    doc.mark_in_document_flags();
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");
    let mut parent_of: Vec<Option<usize>> = vec![None; doc.nodes.len()];
    for idx in 0..doc.nodes.len() {
        for &c in &doc.nodes[idx].children.clone() {
            if c < parent_of.len() {
                parent_of[c] = Some(idx);
            }
        }
    }

    let out = collapse_text_for_shaping(
        &doc,
        &cr,
        &parent_of,
        target,
        " Y",
        cr.computed[target].white_space,
    );
    assert_eq!(out.text, "\u{00A0}Y");
    assert_eq!(out.migrate_count, 0);
}

#[test]
fn preshape_text_populates_text_layout_for_text_nodes() {
    use parley::{FontContext, LayoutContext};
    use raikiri_style::{build_rule_tree, cascade};

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let p = doc.append_element(Some(body), "p", Style::default(), None::<&str>);
    let text = doc.append_text(p, "Hi");

    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");

    let mut fonts = FontContext::new();
    let mut layout_cx = LayoutContext::<()>::new();
    preshape_text(
        &mut doc,
        &cr,
        &mut fonts,
        &mut layout_cx,
        PageBox::A4.width,
        PageBox::A4.width,
    );

    assert!(
        doc.nodes[text].text_layout().is_some(),
        "text node's text_layout must be populated"
    );
    let layout = doc.nodes[text].text_layout().unwrap();
    assert!(layout.width() > 0.0, "text 'Hi' must have non-zero width");
    assert!(
        layout.height() > 0.0,
        "text 'Hi' must have non-zero line height"
    );

    // Element / Document は None のまま
    assert!(
        doc.nodes[html].text_layout().is_none(),
        "html element is not text"
    );
    assert!(
        doc.nodes[body].text_layout().is_none(),
        "body element is not text"
    );
    assert!(
        doc.nodes[p].text_layout().is_none(),
        "p element is not text"
    );
    assert!(
        doc.nodes[0].text_layout().is_none(),
        "document root is not text"
    );
}

#[test]
fn preshape_text_measures_preserved_inline_whitespace_item() {
    use parley::{FontContext, LayoutContext};
    use raikiri_style::{build_rule_tree, cascade};

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let block = doc.append_element(Some(body), "div", Style::default(), Some("display: block"));
    let left = doc.append_element(
        Some(block),
        "span",
        Style::default(),
        Some("display: inline"),
    );
    doc.append_text(left, "left");
    let whitespace = doc.append_text(block, "\n  ");
    let right = doc.append_element(
        Some(block),
        "span",
        Style::default(),
        Some("display: inline"),
    );
    doc.append_text(right, "right");

    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");
    let mut fonts = FontContext::new();
    let mut layout_cx = LayoutContext::<()>::new();
    preshape_text(
        &mut doc,
        &cr,
        &mut fonts,
        &mut layout_cx,
        PageBox::A4.width,
        PageBox::A4.width,
    );

    assert!(
        doc.nodes[whitespace]
            .style
            .size
            .width
            .into_option()
            .is_some_and(|width| width > 0.0)
    );
}

#[test]
fn has_authored_non_whitespace_text_walks_nested_inline_content() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let whitespace_only = doc.append_element(
        Some(body),
        "span",
        Style::default(),
        Some("display: inline"),
    );
    let whitespace = doc.append_text(whitespace_only, "\n  ");
    assert!(!has_authored_non_whitespace_text(&doc, whitespace_only));
    assert!(!has_authored_non_whitespace_text(&doc, whitespace));

    let wrapper = doc.append_element(
        Some(body),
        "span",
        Style::default(),
        Some("display: inline"),
    );
    let nested = doc.append_element(
        Some(wrapper),
        "em",
        Style::default(),
        Some("display: inline"),
    );
    let text = doc.append_text(nested, "content");
    assert!(has_authored_non_whitespace_text(&doc, wrapper));
    assert!(has_authored_non_whitespace_text(&doc, nested));
    assert!(has_authored_non_whitespace_text(&doc, text));
}

#[test]
fn preshape_text_respects_computed_font_size() {
    use parley::{FontContext, LayoutContext};
    use raikiri_style::{build_rule_tree, cascade};

    fn shape_text_height_at_font_size(px: &str) -> f32 {
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
        let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
        let inline = format!("font-size:{}", px);
        let p = doc.append_element(Some(body), "p", Style::default(), Some(inline.as_str()));
        let text = doc.append_text(p, "Hi");
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).unwrap();
        let mut fonts = FontContext::new();
        let mut layout_cx = LayoutContext::<()>::new();
        preshape_text(
            &mut doc,
            &cr,
            &mut fonts,
            &mut layout_cx,
            PageBox::A4.width,
            PageBox::A4.width,
        );
        doc.nodes[text].text_layout().unwrap().height()
    }

    let small = shape_text_height_at_font_size("8px");
    let large = shape_text_height_at_font_size("32px");
    // cov:ignore: assertion text is only evaluated when this test fails
    assert!(
        large > small,
        "font-size:32px must produce taller text than 8px (cascade→shape inheritance regression check, got small={small}, large={large})"
    );
}

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
fn multicol_min_constrained_child_requires_all_constraints() {
    let mut doc = Document::new();
    let parent = doc.append_element(Some(0), "div", Style::default(), None::<&str>);
    let child = doc.append_element(Some(parent), "div", Style::default(), None::<&str>);
    doc.nodes[child].has_logical_min_block_size = true;
    doc.nodes[child].style.min_size.height = LengthPercentageAuto::length(40.0);

    assert!(multicol_has_min_constrained_child(&doc, parent));

    doc.nodes[child].style.min_size.height = LengthPercentageAuto::auto();
    assert!(!multicol_has_min_constrained_child(&doc, parent));
}

#[test]
fn compute_multicol_layout_spaces_break_avoid_min_height_children_across_a_definite_height() {
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let container = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:block;column-count:2;column-gap:20px;width:200px;height:60px"),
    );
    // `min-block-size` + `break-inside:avoid` (no direct `<br>`) keeps
    // this multicol container on the foundational block projection
    // instead of the custom nested-fragmentation path (see the doc
    // comment above `compute_multicol_layout`'s `custom_scope`).
    let a = doc.append_element(
        Some(container),
        "div",
        Style::default(),
        Some("display:block;min-block-size:40px;break-inside:avoid"),
    );
    let b = doc.append_element(
        Some(container),
        "div",
        Style::default(),
        Some("display:block;min-block-size:40px;break-inside:avoid"),
    );

    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");
    let mut page = PageBox::new();
    page.width = 300.0;
    page.height = 200.0;
    layout_single_page(&mut doc, &cascade, page, parley::FontContext::new()).expect("layout Ok");

    // Taffy's foundational block algorithm cannot express a
    // fragmentainer break here, so each break-avoid min-height child is
    // pushed into its own vertical slot: `child_height + 2 *
    // |fragmentainer_height - child_height|` apart, consuming the
    // unfragmented overflow on both sides of the mismatch instead of
    // packing contiguously.
    let child_height = doc.nodes[a].unrounded_layout.size.height;
    assert!(child_height > 0.0, "child_height={child_height}");
    let step = child_height + 2.0 * (60.0_f32 - child_height).abs();
    assert!((doc.nodes[a].unrounded_layout.location.y - 0.0).abs() < 0.01);
    assert!(
        (doc.nodes[b].unrounded_layout.location.y - step).abs() < 0.01,
        "b.y={}, expected step={step}",
        doc.nodes[b].unrounded_layout.location.y
    );
}

#[test]
fn compute_multicol_layout_clamps_auto_height_to_the_largest_min_constrained_child() {
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
        Some("display:block;min-block-size:40px;break-inside:avoid"),
    );
    let _b = doc.append_element(
        Some(container),
        "div",
        Style::default(),
        Some("display:block;min-block-size:40px;break-inside:avoid"),
    );

    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");
    let mut page = PageBox::new();
    page.width = 300.0;
    page.height = 400.0;
    layout_single_page(&mut doc, &cascade, page, parley::FontContext::new()).expect("layout Ok");

    // An ordinary auto-height block container would size to the *sum*
    // of its two 40px min-height children (80px). A multicol container
    // with min-constrained children instead clamps its own auto used
    // height to the largest child's used height: the children overflow
    // the box rather than growing it (see the comment above this
    // clamp in `compute_multicol_layout`).
    let child_height = doc.nodes[a].unrounded_layout.size.height;
    assert!(child_height > 0.0, "child_height={child_height}");
    assert!(
        (doc.nodes[container].unrounded_layout.size.height - child_height).abs() < 0.01,
        "container height={}, expected clamp to child height={child_height}",
        doc.nodes[container].unrounded_layout.size.height,
    );
}

#[test]
fn full_size_kana_char_maps_small_hiragana_and_katakana_to_full_size() {
    // CSS Text Module Level 3 `text-transform: full-size-kana` converts
    // small kana used for youon/sokuon/etc. to their full-size form.
    assert_eq!(full_size_kana_char('ぁ'), 'あ');
    assert_eq!(full_size_kana_char('ゕ'), 'か');
    assert_eq!(full_size_kana_char('ゎ'), 'わ');
    assert_eq!(full_size_kana_char('ァ'), 'ア');
    assert_eq!(full_size_kana_char('ッ'), 'ツ');
    assert_eq!(full_size_kana_char('ョ'), 'ヨ');
}

#[test]
fn full_size_kana_char_maps_small_katakana_phonetic_extensions() {
    // The Katakana Phonetic Extensions block (small kana used for the
    // Ainu orthography) maps to its base katakana the same way.
    assert_eq!(full_size_kana_char('ㇰ'), 'ク');
    assert_eq!(full_size_kana_char('ㇻ'), 'ラ');
    assert_eq!(full_size_kana_char('ㇿ'), 'ロ');
}

#[test]
fn full_size_kana_char_maps_halfwidth_small_kana_to_halfwidth_full_size() {
    // Half-width small kana (U+FF67-FF6F) map to their half-width
    // full-size counterparts: this function only removes "smallness",
    // it does not also change the half/full width.
    assert_eq!(full_size_kana_char('ｧ'), 'ｱ');
    assert_eq!(full_size_kana_char('ｯ'), 'ﾂ');
    assert_eq!(full_size_kana_char('ｮ'), 'ﾖ');
}

#[test]
fn full_size_kana_char_maps_kana_supplement_and_extended_a_small_forms() {
    // Small "ko" (Kana Supplement) and the small wi/we/wo/n forms (Kana
    // Extended-A) also have full-size counterparts outside the BMP.
    assert_eq!(full_size_kana_char('\u{1B132}'), '\u{3053}');
    assert_eq!(full_size_kana_char('\u{1B150}'), '\u{3090}');
    assert_eq!(full_size_kana_char('\u{1B167}'), '\u{30F3}');
}

#[test]
fn full_size_kana_char_leaves_unmapped_and_already_full_size_chars_unchanged() {
    // Codepoints immediately outside the mapped ranges, and characters
    // that are already full-size, fall through the identity arm.
    assert_eq!(full_size_kana_char('あ'), 'あ');
    assert_eq!(full_size_kana_char('ー'), 'ー');
    assert_eq!(full_size_kana_char('A'), 'A');
    assert_eq!(full_size_kana_char('\u{1B131}'), '\u{1B131}');
    assert_eq!(full_size_kana_char('\u{1B133}'), '\u{1B133}');
    assert_eq!(full_size_kana_char('\u{1B153}'), '\u{1B153}');
    assert_eq!(full_size_kana_char('\u{1B154}'), '\u{1B154}');
    assert_eq!(full_size_kana_char('\u{1B163}'), '\u{1B163}');
    assert_eq!(full_size_kana_char('\u{1B168}'), '\u{1B168}');
}

#[test]
fn multicol_definite_dimension_resolves_absolute_length_regardless_of_basis() {
    let doc = Document::new();
    assert_eq!(
        multicol_definite_dimension(&doc, Dimension::length(120.5), None),
        Some(120.5)
    );
    assert_eq!(
        multicol_definite_dimension(&doc, Dimension::length(120.5), Some(9999.0)),
        Some(120.5)
    );
}

#[test]
fn multicol_definite_dimension_resolves_percent_against_the_given_basis() {
    let doc = Document::new();
    assert_eq!(
        multicol_definite_dimension(&doc, Dimension::percent(0.25), Some(200.0)),
        Some(50.0)
    );
}

#[test]
fn multicol_definite_dimension_resolves_percent_against_a_zero_default_basis() {
    // `basis` defaults to 0.0 when the caller has no known containing
    // block size yet, so a percentage resolves to zero rather than
    // `None`.
    let doc = Document::new();
    assert_eq!(
        multicol_definite_dimension(&doc, Dimension::percent(0.5), None),
        Some(0.0)
    );
}

#[test]
fn multicol_definite_dimension_returns_none_for_intrinsic_sizing_keywords() {
    // Every non-definite keyword collapses to `None`: the caller falls
    // back to Taffy's ordinary intrinsic-sizing pass instead of a used
    // length.
    let doc = Document::new();
    for dimension in [
        Dimension::auto(),
        Dimension::min_content(),
        Dimension::max_content(),
        Dimension::fit_content(),
        Dimension::fit_content_px(40.0),
        Dimension::fit_content_percent(0.5),
        Dimension::stretch(),
        Dimension::content(),
    ] {
        assert_eq!(
            multicol_definite_dimension(&doc, dimension, Some(200.0)),
            None
        );
    }
}

#[test]
fn multicol_definite_dimension_clamps_a_negative_length_to_zero() {
    let doc = Document::new();
    assert_eq!(
        multicol_definite_dimension(&doc, Dimension::length(-25.0), None),
        Some(0.0)
    );
}

#[test]
fn multicol_definite_dimension_returns_none_for_a_non_finite_result() {
    // An infinite basis makes the resolved percentage non-finite; the
    // `is_finite()` guard turns that into `None` rather than an
    // infinite used size.
    let doc = Document::new();
    assert_eq!(
        multicol_definite_dimension(&doc, Dimension::percent(0.5), Some(f32::INFINITY)),
        None
    );
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

/// Build a `<p>` with five pre-line-separated single-character lines at
/// an explicit 10px line-height, so each line's block extent is an exact
/// multiple of 10px regardless of the font actually resolved.
fn nested_text_line_ranges_fixture() -> (Document, usize) {
    use raikiri_style::{build_rule_tree, cascade};

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let p = doc.append_element(
        Some(body),
        "p",
        Style::default(),
        Some("font-size:10px;line-height:10px;white-space:pre-line"),
    );
    let text = doc.append_text(p, "a\nb\nc\nd\ne");

    doc.mark_in_document_flags();
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");
    let mut fonts = FontContext::new();
    let mut layout_cx = LayoutContext::<()>::new();
    preshape_text(&mut doc, &cr, &mut fonts, &mut layout_cx, 1000.0, 1000.0);
    (doc, text)
}

#[test]
fn nested_text_line_ranges_returns_empty_when_past_the_last_column() {
    let (doc, text) = nested_text_line_ranges_fixture();
    let layout = doc.nodes[text].text_layout().expect("text shaped");
    let context = FragmentationContext {
        available_width: 100.0,
        available_height: None,
        column_width: 100.0,
        column_count: 2,
        column_gap: 0.0,
        column_index: 2,
        origin_x: 0.0,
        origin_y: 0.0,
        orphans: 1,
        widows: 1,
    };
    assert!(nested_text_line_ranges(layout, context).is_empty());
}

#[test]
fn nested_text_line_ranges_splits_by_available_height_across_three_columns() {
    let (doc, text) = nested_text_line_ranges_fixture();
    let layout = doc.nodes[text].text_layout().expect("text shaped");
    assert_eq!(
        layout.len(),
        5,
        "sanity: five pre-line segments produce five lines"
    );
    // Each line's bottom edge advances by exactly one 10px line-height
    // step, which is what the height-fit check below relies on. (The
    // top edge is not asserted here: it can sit slightly outside the
    // nominal line box when a line's ascent/descent exceeds the
    // explicit `line-height`, which does not affect the block-max-based
    // fit check.)
    let first_max = layout.lines().next().unwrap().metrics().block_max_coord;
    for (i, line) in layout.lines().enumerate() {
        let metrics = line.metrics();
        assert!((metrics.block_max_coord - (first_max + i as f32 * 10.0)).abs() < 0.01);
    }
    let context = FragmentationContext {
        available_width: 300.0,
        available_height: Some(25.0),
        column_width: 100.0,
        column_count: 3,
        column_gap: 0.0,
        column_index: 0,
        origin_x: 0.0,
        origin_y: 0.0,
        orphans: 1,
        widows: 1,
    };
    let ranges = nested_text_line_ranges(layout, context);
    assert_eq!(ranges, vec![(0, 2, 0), (2, 4, 1), (4, 5, 2)]);
}

#[test]
fn nested_text_line_ranges_splits_evenly_when_height_is_unconstrained() {
    let (doc, text) = nested_text_line_ranges_fixture();
    let layout = doc.nodes[text].text_layout().expect("text shaped");
    let context = FragmentationContext {
        available_width: 200.0,
        available_height: None,
        column_width: 100.0,
        column_count: 2,
        column_gap: 0.0,
        column_index: 0,
        origin_x: 0.0,
        origin_y: 0.0,
        orphans: 1,
        widows: 1,
    };
    let ranges = nested_text_line_ranges(layout, context);
    assert_eq!(ranges, vec![(0, 3, 0), (3, 5, 1)]);
}

#[test]
fn nested_text_line_ranges_labels_fragments_with_the_starting_column_offset() {
    let (doc, text) = nested_text_line_ranges_fixture();
    let layout = doc.nodes[text].text_layout().expect("text shaped");
    let context = FragmentationContext {
        available_width: 300.0,
        available_height: None,
        column_width: 100.0,
        column_count: 3,
        column_gap: 0.0,
        column_index: 1,
        origin_x: 0.0,
        origin_y: 0.0,
        orphans: 1,
        widows: 1,
    };
    let ranges = nested_text_line_ranges(layout, context);
    assert_eq!(ranges, vec![(0, 3, 1), (3, 5, 2)]);
}

#[test]
fn nested_text_line_ranges_moves_lines_across_the_boundary_to_satisfy_widows() {
    // CSS Fragmentation Module Level 3 §3.3: a widows:3 minimum on the
    // final fragment borrows lines from the preceding fragment, bounded
    // by the preceding fragment's own orphans minimum.
    let (doc, text) = nested_text_line_ranges_fixture();
    let layout = doc.nodes[text].text_layout().expect("text shaped");
    let context = FragmentationContext {
        available_width: 200.0,
        available_height: None,
        column_width: 100.0,
        column_count: 2,
        column_gap: 0.0,
        column_index: 0,
        origin_x: 0.0,
        origin_y: 0.0,
        orphans: 1,
        widows: 3,
    };
    let ranges = nested_text_line_ranges(layout, context);
    assert_eq!(ranges, vec![(0, 2, 0), (2, 5, 1)]);
}

#[test]
fn nested_text_line_ranges_keeps_the_split_when_orphans_forbids_the_widows_move() {
    let (doc, text) = nested_text_line_ranges_fixture();
    let layout = doc.nodes[text].text_layout().expect("text shaped");
    let context = FragmentationContext {
        available_width: 200.0,
        available_height: None,
        column_width: 100.0,
        column_count: 2,
        column_gap: 0.0,
        column_index: 0,
        origin_x: 0.0,
        origin_y: 0.0,
        orphans: 3,
        widows: 3,
    };
    let ranges = nested_text_line_ranges(layout, context);
    assert_eq!(ranges, vec![(0, 3, 0), (3, 5, 1)]);
}

#[test]
fn nested_text_line_ranges_treats_a_zero_height_budget_like_unconstrained() {
    let (doc, text) = nested_text_line_ranges_fixture();
    let layout = doc.nodes[text].text_layout().expect("text shaped");
    let context = FragmentationContext {
        available_width: 200.0,
        available_height: Some(0.0),
        column_width: 100.0,
        column_count: 2,
        column_gap: 0.0,
        column_index: 0,
        origin_x: 0.0,
        origin_y: 0.0,
        orphans: 1,
        widows: 1,
    };
    let ranges = nested_text_line_ranges(layout, context);
    assert_eq!(ranges, vec![(0, 3, 0), (3, 5, 1)]);
}

#[test]
fn refresh_nested_text_fragments_records_line_ranges_and_column_offsets_on_a_text_node() {
    let (mut doc, text) = nested_text_line_ranges_fixture();
    let context = FragmentationContext {
        available_width: 300.0,
        available_height: None,
        column_width: 100.0,
        column_count: 3,
        column_gap: 10.0,
        column_index: 1,
        origin_x: 0.0,
        origin_y: 0.0,
        orphans: 1,
        widows: 1,
    };
    refresh_nested_text_fragments(&mut doc, text, context);

    let fragments = doc.nodes[text]
        .multicol_fragments()
        .expect("a shaped 5-line text node should get line-range fragments")
        .to_vec();
    assert_eq!(fragments.len(), 2);
    assert_eq!((fragments[0].line_start, fragments[0].line_end), (0, 3));
    assert_eq!((fragments[1].line_start, fragments[1].line_end), (3, 5));
    // The first fragment starts in the context's own column (index 1),
    // so its offset from that column's origin is zero; the second
    // fragment sits in the next column (index 2), one
    // column-width-plus-gap further.
    assert!((fragments[0].x - 0.0).abs() < 0.01);
    assert!((fragments[1].x - 110.0).abs() < 0.01);
    // Each line uses an explicit 10px line-height (see the fixture's
    // own doc comment), so the second fragment's line-3 origin sits
    // exactly three lines (30px) below the first fragment's line-0
    // origin.
    assert!(
        (fragments[1].y - fragments[0].y - 30.0).abs() < 0.01,
        "fragments={fragments:?}"
    );
}

#[test]
fn refresh_nested_text_fragments_recurses_into_element_children() {
    use raikiri_style::{build_rule_tree, cascade};

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let p = doc.append_element(
        Some(body),
        "p",
        Style::default(),
        Some("font-size:10px;line-height:10px;white-space:pre-line"),
    );
    let text = doc.append_text(p, "a\nb\nc\nd\ne");

    doc.mark_in_document_flags();
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");
    let mut fonts = FontContext::new();
    let mut layout_cx = LayoutContext::<()>::new();
    preshape_text(&mut doc, &cr, &mut fonts, &mut layout_cx, 1000.0, 1000.0);

    let context = FragmentationContext {
        available_width: 200.0,
        available_height: None,
        column_width: 100.0,
        column_count: 2,
        column_gap: 0.0,
        column_index: 0,
        origin_x: 0.0,
        origin_y: 0.0,
        orphans: 1,
        widows: 1,
    };
    // Called on the *element*, not the text node directly: the
    // recursive descent (the non-text arm) must reach the shaped text
    // descendant and populate its fragments exactly as a direct call
    // on the text node would.
    refresh_nested_text_fragments(&mut doc, p, context);

    let fragments = doc.nodes[text]
        .multicol_fragments()
        .expect("recursing through the <p> element should still reach its text child");
    assert_eq!(fragments.len(), 2);
    assert_eq!((fragments[0].line_start, fragments[0].line_end), (0, 3));
    assert_eq!((fragments[1].line_start, fragments[1].line_end), (3, 5));
}

#[test]
fn refresh_nested_text_fragments_leaves_an_unshaped_text_node_untouched() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let p = doc.append_element(Some(body), "p", Style::default(), None::<&str>);
    let text = doc.append_text(p, "no preshape ran on this node");

    let context = FragmentationContext {
        available_width: 200.0,
        available_height: None,
        column_width: 100.0,
        column_count: 2,
        column_gap: 0.0,
        column_index: 0,
        origin_x: 0.0,
        origin_y: 0.0,
        orphans: 1,
        widows: 1,
    };
    // No `preshape_text` call precedes this, so `text_layout` is still
    // `None`: the function must return without touching
    // `multicol_fragments`.
    refresh_nested_text_fragments(&mut doc, text, context);
    assert!(doc.nodes[text].multicol_fragments().is_none());
}

#[test]
fn refresh_nested_text_fragments_clears_stale_fragments_for_a_zero_line_layout() {
    use parley::{FontContext, LayoutContext};

    // A hand-built, never-`break_all_lines`-run layout reports zero
    // lines, giving a direct unit fixture for the function's own
    // `line_count == 0` guard without depending on any particular
    // content producing it through the real `preshape_text` pipeline
    // (an empty string there still shapes to one empty line).
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let p = doc.append_element(Some(body), "p", Style::default(), None::<&str>);
    let text = doc.append_text(p, "");

    let mut fonts = FontContext::new();
    let mut layout_cx = LayoutContext::<()>::new();
    let mut builder = layout_cx.ranged_builder(&mut fonts, "", 1.0, false);
    builder.push_default(parley::StyleProperty::FontSize(16.0));
    let empty_layout: parley::Layout<()> = builder.build("");
    assert_eq!(
        empty_layout.len(),
        0,
        "sanity: an unbroken layout has zero lines"
    );

    let NodeData::Text(text_data) = &mut doc.nodes[text].data else {
        panic!("expected a text node");
    };
    text_data.text_layout = Some(empty_layout);
    // Stale fragments left over from a previous (non-empty) layout pass.
    text_data.multicol_fragments = Some(vec![MulticolTextFragment {
        line_start: 0,
        line_end: 1,
        x: 5.0,
        y: 5.0,
    }]);

    let context = FragmentationContext {
        available_width: 200.0,
        available_height: None,
        column_width: 100.0,
        column_count: 2,
        column_gap: 0.0,
        column_index: 0,
        origin_x: 0.0,
        origin_y: 0.0,
        orphans: 1,
        widows: 1,
    };
    refresh_nested_text_fragments(&mut doc, text, context);
    assert!(
        doc.nodes[text].multicol_fragments().is_none(),
        "a zero-line layout must clear any stale fragments rather than keep them"
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
fn relayout_nested_multicol_children_uses_the_definite_height_budget_per_child() {
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    // The two tests above give the multicol container an auto block
    // size, which routes every child through the auto-measured
    // balancing pass. A *definite* container height instead skips that
    // pass (`auto_measurements` stays `None`) and lays each child out
    // against its own remaining per-column height budget directly. A
    // direct `<br>` child is what forces `compute_multicol_layout`'s
    // custom nested path even though the height is definite (see the
    // doc comment above its `custom_scope` computation).
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let container = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:block;column-count:2;column-gap:20px;width:200px;height:40px"),
    );
    let a = doc.append_element(
        Some(container),
        "div",
        Style::default(),
        Some("display:block;height:30px"),
    );
    let _br = doc.append_element(Some(a), "br", Style::default(), None::<&str>);
    let b = doc.append_element(
        Some(container),
        "div",
        Style::default(),
        Some("display:block;height:30px"),
    );
    // A direct text child (not wrapped in its own block box) exercises
    // the text-specific reset (`style.size.height = auto` + cache
    // clear) and the per-child multicol text-fragment recording, both
    // only reachable on this non-auto-measured child path.
    let text = doc.append_text(container, "hello column text");

    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");
    let mut page = PageBox::new();
    page.width = 300.0;
    page.height = 200.0;
    layout_single_page(&mut doc, &cascade, page, parley::FontContext::new()).expect("layout Ok");

    // `a` (30px) fits the 40px column budget; `b` (30px) does not
    // (30 + 30 > 40), so it starts a fresh column at y=0.
    assert!((doc.nodes[a].unrounded_layout.location.y - 0.0).abs() < 0.01);
    assert!((doc.nodes[b].unrounded_layout.location.y - 0.0).abs() < 0.01);
    // width 200 / 2 columns with a 20px gap: (200 - 20) / 2 = 90;
    // column 1 starts at 90 + 20 = 110.
    assert!((doc.nodes[a].unrounded_layout.location.x - 0.0).abs() < 0.01);
    assert!((doc.nodes[b].unrounded_layout.location.x - 110.0).abs() < 0.01);

    assert!(
        doc.nodes[text].multicol_fragments().is_some(),
        "direct multicol text should get explicit line-range fragments"
    );
}

#[test]
fn relayout_nested_multicol_children_column_places_a_block_sibling_of_an_empty_direct_text_child() {
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    // An *auto*-height multicol container (unlike the definite-height
    // test above) routes every child through the auto-measurement pass
    // first (`auto_measurements` is `Some` here), which has its own
    // copy of the "reset a direct text child's style before measuring"
    // branch. A direct text sibling with real content would make
    // `prepare_multicol_layout`'s pre-pass pin the container's own
    // height to a definite value (its line-count projection), which
    // would route this container through the definite-height path
    // instead and defeat the point of this test; an empty text node
    // never gets a shaped layout, so that pre-pass leaves the
    // container's auto height alone while the node still participates
    // in `relayout_nested_multicol_children`'s child list.
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
    let _text = doc.append_text(container, "");

    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");
    let mut page = PageBox::new();
    page.width = 300.0;
    page.height = 200.0;
    layout_single_page(&mut doc, &cascade, page, parley::FontContext::new()).expect("layout Ok");

    // `a`'s width narrowing from the container's full 200px to one
    // 90px column ((200 - 20) / 2) is only possible through the custom
    // nested-fragmentation path -- the ordinary Taffy block algorithm
    // would stretch it to the full content width instead. That path
    // only runs by measuring every child in `children`, the empty
    // text node included, so this indirectly exercises its branch.
    assert!((doc.nodes[a].unrounded_layout.size.width - 90.0).abs() < 0.01);
    assert!((doc.nodes[container].unrounded_layout.size.height - 30.0).abs() < 0.01);
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
fn vertical_align_linebox_extent_splits_ascent_and_descent() {
    assert_eq!(
        vertical_align_linebox_extent(VerticalAlign::Length(Length::Px(12.0)), 16.0),
        (12.0, 0.0)
    );
    assert_eq!(
        vertical_align_linebox_extent(VerticalAlign::Length(Length::Px(-4.0)), 16.0),
        (0.0, 4.0)
    );
    assert_eq!(
        vertical_align_linebox_extent(VerticalAlign::Super, 16.0),
        (16.0 / 3.0, 0.0)
    );
    assert_eq!(
        vertical_align_linebox_extent(VerticalAlign::Sub, 16.0),
        (0.0, 16.0 / 5.0)
    );
    assert_eq!(
        vertical_align_linebox_extent(VerticalAlign::Baseline, 16.0),
        (0.0, 0.0)
    );
    assert_eq!(
        vertical_align_linebox_extent(VerticalAlign::Length(Length::Px(f32::NAN)), 16.0),
        (0.0, 0.0)
    );
}

#[test]
fn padding_with_linebox_extent_preserves_percent_components() {
    let mut doc = Document::new();
    let without_extra = padding_with_linebox_extent(
        &mut doc,
        ComputedLengthPercentage::Px(4.0),
        0.0,
        "test-linebox-padding",
    );
    assert_eq!(without_extra, LengthPercentage::length(4.0));
    assert!(doc.calc_values.is_empty());

    let _with_extra = padding_with_linebox_extent(
        &mut doc,
        ComputedLengthPercentage::Percent(25.0),
        8.0,
        "test-linebox-padding",
    );
    assert_eq!(doc.calc_values.len(), 1);
    assert_eq!(doc.calc_values[0].percent, 25.0);
    assert_eq!(doc.calc_values[0].px, 8.0);
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
fn text_indent_does_not_shift_floated_or_out_of_flow_inline_blocks() {
    use raikiri_style::{build_rule_tree, cascade};

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let absolute_parent = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:block;text-indent:20px"),
    );
    let absolute_child = doc.append_element(
        Some(absolute_parent),
        "span",
        Style::default(),
        Some("display:inline-block;width:10px;height:10px;position:absolute"),
    );
    let float_parent = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:block;text-indent:20px"),
    );
    let float_child = doc.append_element(
        Some(float_parent),
        "span",
        Style::default(),
        Some("display:inline-block;width:10px;height:10px;float:left"),
    );
    doc.mark_in_document_flags();

    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");
    assert_eq!(
        cascade.computed[absolute_child].position,
        PositionValue::Absolute
    );
    assert_eq!(cascade.computed[float_child].float, FloatValue::Left);

    doc.nodes[absolute_parent].unrounded_layout.size.width = 100.0;
    doc.nodes[absolute_child].unrounded_layout.location.x = 3.0;
    doc.nodes[float_parent].unrounded_layout.size.width = 100.0;
    doc.nodes[float_child].unrounded_layout.location.x = 7.0;
    realign_single_empty_inline_block_indent(&mut doc, &cascade);

    assert_eq!(doc.nodes[absolute_child].unrounded_layout.location.x, 3.0);
    assert_eq!(doc.nodes[float_child].unrounded_layout.location.x, 7.0);
}

#[test]
fn text_indent_skips_child_cleared_from_document_flags() {
    use raikiri_style::{build_rule_tree, cascade};

    let mut doc = Document::new();
    let parent = doc.append_element(
        Some(0),
        "div",
        Style::default(),
        Some("display:block;text-indent:20px"),
    );
    let child = doc.append_element(
        Some(parent),
        "span",
        Style::default(),
        Some("display:inline-block;width:10px;height:10px"),
    );
    doc.mark_in_document_flags();
    assert!(doc.nodes[parent].is_in_document());
    assert!(doc.nodes[child].is_in_document());

    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");
    assert_eq!(cascade.computed[parent].display, DisplayValue::Block);
    assert_eq!(cascade.computed[child].display, DisplayValue::InlineBlock);
    doc.nodes[child].set_in_document(false);
    assert!(!doc.nodes[child].is_in_document());

    doc.nodes[parent].unrounded_layout.size.width = 100.0;
    doc.nodes[child].unrounded_layout.location.x = 4.0;
    realign_single_empty_inline_block_indent(&mut doc, &cascade);

    assert_eq!(doc.nodes[child].unrounded_layout.location.x, 4.0);
}

#[test]
fn text_indent_skips_negative_content_width() {
    use raikiri_style::{build_rule_tree, cascade};

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let parent = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:block;text-indent:20px"),
    );
    let child = doc.append_element(
        Some(parent),
        "span",
        Style::default(),
        Some("display:inline-block;width:10px;height:10px"),
    );
    doc.mark_in_document_flags();

    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");
    doc.nodes[parent].unrounded_layout.size.width = -10.0;
    doc.nodes[child].unrounded_layout.location.x = 4.0;
    assert!(doc.nodes[parent].unrounded_layout.content_box_width() < 0.0);
    realign_single_empty_inline_block_indent(&mut doc, &cascade);

    assert_eq!(doc.nodes[child].unrounded_layout.location.x, 4.0);
}

#[test]
fn text_indent_ch_uses_ahem_and_lato_zero_advances() {
    use parley::PositionedLayoutItem;
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;
    use std::path::PathBuf;

    let fonts_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("target")
        .join("wpt")
        .join("fonts");
    if !fonts_dir.join("Ahem.ttf").exists() || !fonts_dir.join("Lato-Medium.ttf").exists() {
        eprintln!(
            "skipping text-indent ch font test: Ahem.ttf and Lato-Medium.ttf are required under {}",
            fonts_dir.display()
        );
        return;
    }

    fn first_glyph_x(fonts_dir: &std::path::Path, family: &str, indent: &str) -> f32 {
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
        let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
        let style =
            format!("display:block;font-family:{family};font-size:16px;text-indent:{indent}ch");
        let p = doc.append_element(Some(body), "p", Style::default(), Some(&style));
        let text = doc.append_text(p, "AB");
        let rules = build_rule_tree(&doc);
        let cascade = cascade(&doc, &rules).expect("cascade Ok");
        let fonts =
            crate::fonts::build_wpt_font_ctx(fonts_dir).expect("bundled WPT fonts should register");
        layout_single_page(&mut doc, &cascade, PageBox::A4, fonts).expect("layout Ok");
        doc.nodes[text]
            .text_layout()
            .expect("text shaped")
            .lines()
            .next()
            .expect("one line")
            .items()
            .filter_map(|item| match item {
                PositionedLayoutItem::GlyphRun(run) => run.positioned_glyphs().next().map(|g| g.x),
                _ => None,
            })
            .next()
            .expect("first glyph")
    }

    fn inherited_source_glyph_x(fonts_dir: &std::path::Path) -> f32 {
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
        let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
        let parent = doc.append_element(
            Some(body),
            "p",
            Style::default(),
            Some("display:block;font-family:Ahem;font-size:16px;text-indent:1ch"),
        );
        let child = doc.append_element(
            Some(parent),
            "div",
            Style::default(),
            Some("display:block;font-family:Lato;font-size:32px"),
        );
        let text = doc.append_text(child, "AB");
        let rules = build_rule_tree(&doc);
        let cascade = cascade(&doc, &rules).expect("cascade Ok");
        let fonts =
            crate::fonts::build_wpt_font_ctx(fonts_dir).expect("bundled WPT fonts should register");
        layout_single_page(&mut doc, &cascade, PageBox::A4, fonts).expect("layout Ok");
        doc.nodes[text]
            .text_layout()
            .expect("text shaped")
            .lines()
            .next()
            .expect("one line")
            .items()
            .filter_map(|item| match item {
                PositionedLayoutItem::GlyphRun(run) => run.positioned_glyphs().next().map(|g| g.x),
                _ => None,
            })
            .next()
            .expect("first glyph")
    }

    let mut probe_fonts =
        crate::fonts::build_wpt_font_ctx(&fonts_dir).expect("bundled WPT fonts should register");
    let mut probe_layout = LayoutContext::<()>::new();
    let ahem = probe_text_advance(
        &mut probe_fonts,
        &mut probe_layout,
        "0",
        "Ahem",
        16.0,
        400.0,
        StyleFontStyle::Normal,
    );
    let lato = probe_text_advance(
        &mut probe_fonts,
        &mut probe_layout,
        "0",
        "Lato",
        16.0,
        400.0,
        StyleFontStyle::Normal,
    );
    let missing_generic = probe_ch_text_advance(
        &mut probe_fonts,
        &mut probe_layout,
        "Missing, serif",
        16.0,
        400.0,
        StyleFontStyle::Normal,
    );
    let quoted_generic = probe_ch_text_advance(
        &mut probe_fonts,
        &mut probe_layout,
        "\"serif\"",
        16.0,
        400.0,
        StyleFontStyle::Normal,
    );
    assert_eq!(missing_generic, 8.0);
    assert_eq!(quoted_generic, 16.0);
    let mut no_generic = FontContext {
        source_cache: parley::fontique::SourceCache::new_shared(),
        collection: parley::fontique::Collection::new(parley::fontique::CollectionOptions {
            shared: false,
            system_fonts: false,
        }),
    };
    assert!(!generic_family_has_ch_glyphs(
        &mut no_generic,
        parley::fontique::GenericFamily::Serif,
        400.0,
        StyleFontStyle::Normal,
    ));
    let no_space_path = fonts_dir.join("CanvasTest-nospace.ttf");
    assert!(no_space_path.exists(), "CanvasTest-nospace.ttf is required");
    let no_space_bytes = std::fs::read(&no_space_path).expect("read CanvasTest-nospace");
    let mut no_space_fonts = FontContext::new();
    let registered = no_space_fonts.collection.register_fonts(
        parley::fontique::Blob::new(std::sync::Arc::new(no_space_bytes) as _),
        None,
    );
    assert!(!registered.is_empty());
    let ids = registered.iter().map(|(id, _)| *id).collect::<Vec<_>>();
    no_space_fonts
        .collection
        .append_generic_families(parley::fontique::GenericFamily::Serif, ids.into_iter());
    let _ = generic_family_has_ch_glyphs(
        &mut no_space_fonts,
        parley::fontique::GenericFamily::Serif,
        400.0,
        StyleFontStyle::Normal,
    );

    let ahem_x = first_glyph_x(&fonts_dir, "Ahem", "1");
    let ahem_negative_x = first_glyph_x(&fonts_dir, "Ahem", "-1");
    let lato_x = first_glyph_x(&fonts_dir, "Lato", "1");
    let inherited_x = inherited_source_glyph_x(&fonts_dir);
    assert!(
        (ahem_x - ahem).abs() < 2.0,
        "Ahem indent x={ahem_x}, ch={ahem}"
    );
    assert!(
        (lato_x - lato).abs() < 2.0,
        "Lato indent x={lato_x}, ch={lato}"
    );
    assert!(
        (ahem_negative_x + ahem).abs() < 2.0,
        "negative Ahem indent x={ahem_negative_x}, ch={ahem}"
    );
    let lato_32 = probe_text_advance(
        &mut probe_fonts,
        &mut probe_layout,
        "0",
        "Lato",
        32.0,
        400.0,
        StyleFontStyle::Normal,
    );
    assert!(
        (inherited_x - ahem).abs() < 2.0,
        "inherited source x={inherited_x}, source ch={ahem}"
    );
    assert!(
        (inherited_x - lato_32).abs() > 0.5,
        "inherited indent must not use child font: x={inherited_x}, child ch={lato_32}"
    );
    assert!(
        (ahem_x - lato_x).abs() > 0.5,
        "proportional and Ahem text-indent must differ: Ahem={ahem_x}, Lato={lato_x}"
    );
}

#[test]
fn text_indent_ch_affects_taffy_height_before_layout() {
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
    if !fonts_dir.join("Ahem.ttf").exists() || !fonts_dir.join("Lato-Medium.ttf").exists() {
        eprintln!(
            "skipping pre-Taffy text-indent ch test: Ahem.ttf and Lato-Medium.ttf are required under {}",
            fonts_dir.display()
        );
        return;
    }

    assert!(fonts_dir.join("CanvasTest.ttf").exists());
    {
        let mut probe_fonts = crate::fonts::build_wpt_font_ctx(&fonts_dir)
            .expect("bundled WPT fonts should register");
        let mut probe_layout = LayoutContext::<()>::new();
        let canvas_test = probe_ch_text_advance(
            &mut probe_fonts,
            &mut probe_layout,
            "CanvasTest, Ahem",
            16.0,
            400.0,
            StyleFontStyle::Normal,
        );
        let ahem = probe_ch_text_advance(
            &mut probe_fonts,
            &mut probe_layout,
            "Ahem",
            16.0,
            400.0,
            StyleFontStyle::Normal,
        );
        assert_eq!(canvas_test, ahem);
    }

    fn measure_case(fonts_dir: &std::path::Path, source: &str, indent: &str) -> (usize, f32, f32) {
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
        let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
        let parent = doc.append_element(
            Some(body),
            "p",
            Style::default(),
            Some(&format!(
                "display:block;font-family:{source};font-size:16px;text-indent:{indent}ch"
            )),
        );
        let child = doc.append_element(
                Some(parent),
                "div",
                Style::default(),
                Some("display:block;width:76px;font-family:Lato;font-size:16px;line-height:20px;word-break:break-all"),
            );
        let text = doc.append_text(child, "0000000");
        let rules = build_rule_tree(&doc);
        let cascade = cascade(&doc, &rules).expect("cascade Ok");
        let fonts =
            crate::fonts::build_wpt_font_ctx(fonts_dir).expect("bundled WPT fonts should register");
        layout_single_page(&mut doc, &cascade, PageBox::A4, fonts).expect("layout Ok");
        let layout = doc.nodes[text].text_layout().expect("text shaped");
        (
            layout.len(),
            layout.height(),
            doc.nodes[child].unrounded_layout.size.height,
        )
    }

    fn measure_direct_case(
        fonts_dir: &std::path::Path,
        source: &str,
        indent: &str,
    ) -> (usize, f32, f32) {
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
        let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
        let parent = doc.append_element(
                Some(body),
                "p",
                Style::default(),
                Some(&format!(
                    "display:block;width:80px;font-family:{source};font-size:16px;line-height:20px;text-indent:{indent}ch"
                )),
            );
        let text = doc.append_text(parent, "00 00");
        let rules = build_rule_tree(&doc);
        let cascade = cascade(&doc, &rules).expect("cascade Ok");
        let fonts =
            crate::fonts::build_wpt_font_ctx(fonts_dir).expect("bundled WPT fonts should register");
        layout_single_page(&mut doc, &cascade, PageBox::A4, fonts).expect("layout Ok");
        let layout = doc.nodes[text].text_layout().expect("text shaped");
        (
            layout.len(),
            layout.height(),
            doc.nodes[parent].unrounded_layout.size.height,
        )
    }

    fn measure_inline_case(fonts_dir: &std::path::Path) -> (usize, f32, f32) {
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
        let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
        let parent = doc.append_element(
                Some(body),
                "p",
                Style::default(),
                Some("display:block;width:80px;font-family:Ahem;font-size:16px;line-height:20px;text-indent:1ch"),
            );
        let first = doc.append_text(parent, "00 ");
        let span = doc.append_element(
            Some(parent),
            "span",
            Style::default(),
            Some("display:inline;font-family:Ahem;font-size:16px"),
        );
        let second = doc.append_text(span, "00");
        let rules = build_rule_tree(&doc);
        let cascade = cascade(&doc, &rules).expect("cascade Ok");
        let fonts =
            crate::fonts::build_wpt_font_ctx(fonts_dir).expect("bundled WPT fonts should register");
        layout_single_page(&mut doc, &cascade, PageBox::A4, fonts).expect("layout Ok");
        let layout = doc.nodes[first].text_layout().expect("first text shaped");
        let second_layout = doc.nodes[second].text_layout().expect("second text shaped");
        (
            layout.len() + second_layout.len(),
            layout.height() + second_layout.height(),
            doc.nodes[parent].unrounded_layout.size.height,
        )
    }

    let inline_ahem = measure_inline_case(&fonts_dir);
    assert_eq!(inline_ahem.0, 2);
    assert!((inline_ahem.1 - inline_ahem.2).abs() < 0.01);

    let direct_ahem = measure_direct_case(&fonts_dir, "Ahem", "1");
    assert_eq!(direct_ahem.0, 2);
    assert!((direct_ahem.1 - direct_ahem.2).abs() < 0.01);

    let ahem_zero = measure_case(&fonts_dir, "Ahem", "0");
    let ahem_one = measure_case(&fonts_dir, "Ahem", "1");
    let lato_zero = measure_case(&fonts_dir, "Lato", "0");
    let lato_one = measure_case(&fonts_dir, "Lato", "1");
    let fallback_one = measure_case(&fonts_dir, "Missing, Ahem", "1");

    assert_eq!(ahem_zero.0, 1);
    assert_eq!(ahem_one.0, 2);
    assert_eq!(lato_zero.0, 1);
    assert_eq!(lato_one.0, 1);
    assert_eq!(fallback_one.0, ahem_one.0);
    for (_lines, text_height, block_height) in
        [ahem_zero, ahem_one, lato_zero, lato_one, fallback_one]
    {
        assert!((text_height - block_height).abs() < 0.01);
    }
    assert!(ahem_one.2 > lato_one.2 + 10.0);
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
fn prepare_text_indent_keeps_combined_width_and_wide_body_runs_on_legacy_paths() {
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut combined = Document::new();
    let html = combined.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = combined.append_element(Some(html), "body", Style::default(), None::<&str>);
    let block = combined.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:block;width:1ch;text-indent:1ch"),
    );
    combined.append_text(block, "0");
    let rules = build_rule_tree(&combined);
    let combined_cascade = cascade(&combined, &rules).expect("cascade Ok");
    layout_single_page(
        &mut combined,
        &combined_cascade,
        PageBox::A4,
        FontContext::new(),
    )
    .expect("combined layout Ok");

    let mut wide = Document::new();
    let html = wide.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = wide.append_element(
        Some(html),
        "body",
        Style::default(),
        Some("text-indent:1ch;white-space:nowrap"),
    );
    let text = wide.append_text(body, "000000000000000000000000000000000000000000");
    let rules = build_rule_tree(&wide);
    let cascade = cascade(&wide, &rules).expect("cascade Ok");
    layout_single_page(&mut wide, &cascade, PageBox::A4, FontContext::new())
        .expect("wide layout Ok");
    let mut fonts = FontContext::new();
    let mut layout_cx = LayoutContext::<()>::new();
    prepare_text_indent_before_taffy(&mut wide, &cascade, &mut fonts, &mut layout_cx, 1.0);
    assert!(matches!(
        &wide.nodes[text].data,
        crate::node::NodeData::Text(text) if text.text_indent_px.is_none()
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

/// Fixture helper: builds a DOM inside an `<article>` block and returns
/// the target text node's [`LineStart`] classification.
/// The `build` closure creates the target node from `(doc, article)`.
fn first_in_block_of(build: impl FnOnce(&mut Document, usize) -> usize) -> LineStart {
    use raikiri_style::{build_rule_tree, cascade};

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let article = doc.append_element(
        Some(body),
        "article",
        Style::default(),
        Some("display: block"),
    );
    let target = build(&mut doc, article);
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");
    let mut parent_of: Vec<Option<usize>> = vec![None; doc.nodes.len()];
    for idx in 0..doc.nodes.len() {
        for &c in doc.nodes[idx].children.clone().iter() {
            if c < parent_of.len() {
                parent_of[c] = Some(idx);
            }
        }
    }
    line_start_pos(&doc, &cr, &parent_of, target)
}

#[test]
fn line_start_single_text_is_block_start() {
    assert_eq!(
        first_in_block_of(|doc, article| { doc.append_text(article, "Hello") }),
        LineStart::BlockStart
    );
}

#[test]
fn line_start_second_text_is_mid_line() {
    assert_eq!(
        first_in_block_of(|doc, article| {
            doc.append_text(article, "Hello");
            doc.append_text(article, "World")
        }),
        LineStart::MidLine
    );
}

#[test]
fn line_start_after_br_is_after_break() {
    assert_eq!(
        first_in_block_of(|doc, article| {
            doc.append_text(article, "Hello");
            doc.append_element(Some(article), "br", Style::default(), None::<&str>);
            doc.append_text(article, "World")
        }),
        LineStart::AfterBreak
    );
}

#[test]
fn line_start_nested_div_first_text_is_block_start() {
    assert_eq!(
        first_in_block_of(|doc, article| {
            let div = doc.append_element(
                Some(article),
                "div",
                Style::default(),
                Some("display: block"),
            );
            doc.append_text(div, "Hello")
        }),
        LineStart::BlockStart
    );
}

#[test]
fn line_start_bare_text_after_div_is_after_break() {
    assert_eq!(
        first_in_block_of(|doc, article| {
            let div = doc.append_element(
                Some(article),
                "div",
                Style::default(),
                Some("display: block"),
            );
            doc.append_text(div, "Hello");
            doc.append_text(article, "World")
        }),
        LineStart::AfterBreak
    );
}

#[test]
fn collapse_ws_runs_to_single_mid_line() {
    let out = collapse_ws("a  b", WhiteSpace::Normal, LineStart::MidLine, false);
    assert_eq!(out.as_ref(), "a b");
}

#[test]
fn collapse_ws_trims_leading_at_block_start() {
    let out = collapse_ws("  a", WhiteSpace::Normal, LineStart::BlockStart, false);
    assert_eq!(out.as_ref(), "a");
}

#[test]
fn collapse_ws_keeps_single_leading_mid_line() {
    let out = collapse_ws("  a", WhiteSpace::Normal, LineStart::MidLine, false);
    assert_eq!(out.as_ref(), " a");
}

#[test]
fn collapse_ws_newline_to_space_under_normal() {
    let out = collapse_ws("a\nb", WhiteSpace::Normal, LineStart::MidLine, false);
    assert_eq!(out.as_ref(), "a b");
}

#[test]
fn collapse_ws_preline_keeps_newline_and_trims_after() {
    let out = collapse_ws("a\n  b", WhiteSpace::PreLine, LineStart::MidLine, false);
    assert_eq!(out.as_ref(), "a\nb");
}

#[test]
fn collapse_ws_trims_trailing_at_block_end() {
    let out = collapse_ws("a  ", WhiteSpace::Normal, LineStart::MidLine, true);
    assert_eq!(out.as_ref(), "a");
}

#[test]
fn collapse_ws_keeps_trailing_mid_block() {
    let out = collapse_ws("a  ", WhiteSpace::Normal, LineStart::MidLine, false);
    assert_eq!(out.as_ref(), "a ");
}

#[test]
fn collapse_ws_pre_untouched() {
    let out = collapse_ws("  a\n  b  ", WhiteSpace::Pre, LineStart::BlockStart, true);
    assert_eq!(out.as_ref(), "  a\n  b  ");
}

#[test]
fn collapse_ws_tab_to_space() {
    let out = collapse_ws("a\tb", WhiteSpace::Normal, LineStart::MidLine, false);
    assert_eq!(out.as_ref(), "a b");
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
fn indent_options_matrix() {
    use LineStart::{AfterBreak, BlockStart, MidLine};
    // MidLine never indents.
    assert!(indent_options_for_node(false, false, MidLine).is_none());
    assert!(indent_options_for_node(true, true, MidLine).is_none());
    // Basic: block start only.
    assert_eq!(
        indent_options_for_node(false, false, BlockStart),
        Some(IndentOptions::default())
    );
    assert!(indent_options_for_node(false, false, AfterBreak).is_none());
    // Each-line: block start and after break.
    for start in [BlockStart, AfterBreak] {
        let o = indent_options_for_node(false, true, start).expect("each-line applies");
        assert!(o.each_line && !o.hanging);
    }
    // Hanging block start passes through.
    let o = indent_options_for_node(true, false, BlockStart).expect("hanging applies");
    assert!(!o.each_line && o.hanging);
    // Hanging after break degrades to each-line shape (documents the
    // soft-wrap approximation in `indent_options_for_node` doc).
    let o = indent_options_for_node(true, false, AfterBreak).expect("hanging applies");
    assert!(o.each_line && !o.hanging);
    // Combined passes through in both positions.
    for start in [BlockStart, AfterBreak] {
        let o = indent_options_for_node(true, true, start).expect("combined applies");
        assert!(o.each_line && o.hanging);
    }
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

fn preshape_tab_test_entries(entries: &[(&str, &str)]) -> (Document, Vec<usize>) {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let mut text_nodes = Vec::with_capacity(entries.len());
    for &(style, text) in entries {
        let block = doc.append_element(Some(body), "p", Style::default(), Some(style));
        text_nodes.push(doc.append_text(block, text));
    }
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade Ok");
    let mut fonts = FontContext::new();
    let mut layout_cx = LayoutContext::<()>::new();
    preshape_text(
        &mut doc,
        &cascade,
        &mut fonts,
        &mut layout_cx,
        PageBox::A4.width,
        PageBox::A4.width,
    );
    (doc, text_nodes)
}

const PRE_TAB_TEST_STYLE: &str = "display:block; white-space:pre; tab-size:4; font-family:monospace; font-size:16px; word-spacing:0.25ch";

fn assert_tab_layout_reaches_next_stop(doc: &Document, text_nodes: &[usize]) {
    let full_width = |index: usize| {
        doc.nodes[text_nodes[index]]
            .text_layout()
            .unwrap()
            .full_width()
    };
    let interval = full_width(1) * 4.0;
    let prefix_width = full_width(2);
    let suffix_width = full_width(3);
    let expected = ((prefix_width / interval).floor() + 1.0) * interval + suffix_width;
    assert!((full_width(0) - expected).abs() < 0.1);
}

#[test]
fn local_tab_fallback_keeps_per_text_run_behavior() {
    assert_eq!(
        expand_tabs_locally("a\tb", ComputedTabSize::Number(4.0), 10.0).0,
        "a   b"
    );
    assert_eq!(
        expand_tabs_locally("ab\n\tc", ComputedTabSize::Number(4.0), 10.0).0,
        "ab\n    c"
    );
}

#[test]
fn local_tab_fallback_handles_length_stops_and_zero_intervals() {
    assert_eq!(
        expand_tabs_locally("a\tb", ComputedTabSize::Length(ComputedLength(20.0)), 10.0,).0,
        "a b"
    );
    assert_eq!(
        expand_tabs_locally("a\tb", ComputedTabSize::Length(ComputedLength(20.0)), 0.0,).0,
        "ab"
    );
    assert_eq!(
        expand_tabs_locally("a\tb", ComputedTabSize::Number(0.0), 10.0).0,
        "ab"
    );
}

#[test]
fn tab_stop_rejects_non_finite_values() {
    assert_eq!(
        tab_stop_advance(ComputedTabSize::Number(f32::NAN), 8.0),
        0.0
    );
    assert_eq!(
        tab_stop_advance(ComputedTabSize::Length(ComputedLength(f32::NAN)), 8.0,),
        0.0
    );
}

#[test]
fn preshape_simple_pre_block_uses_measured_tab_stops() {
    let entries = [
        (PRE_TAB_TEST_STYLE, "A\tB"),
        (PRE_TAB_TEST_STYLE, " "),
        (PRE_TAB_TEST_STYLE, "A"),
        (PRE_TAB_TEST_STYLE, "B"),
    ];
    let (doc, text_nodes) = preshape_tab_test_entries(&entries);
    assert_tab_layout_reaches_next_stop(&doc, &text_nodes);
    assert!(doc.nodes[text_nodes[0]].snap_glyph_x_to_1_64());
    assert_eq!(
        doc.nodes[text_nodes[0]]
            .text_layout()
            .unwrap()
            .lines()
            .count(),
        1
    );
}

#[test]
fn preshape_parallel_simple_pre_tab_runs_apply_spacing_ranges() {
    let mut entries = vec![
        (PRE_TAB_TEST_STYLE, "A\tB"),
        (PRE_TAB_TEST_STYLE, " "),
        (PRE_TAB_TEST_STYLE, "A"),
        (PRE_TAB_TEST_STYLE, "B"),
    ];
    entries.extend(std::iter::repeat_n((PRE_TAB_TEST_STYLE, "x"), 28));
    assert_eq!(entries.len(), 32);
    let (doc, text_nodes) = preshape_tab_test_entries(&entries);
    assert_tab_layout_reaches_next_stop(&doc, &text_nodes);
    assert!(doc.nodes[text_nodes[0]].snap_glyph_x_to_1_64());
}

#[test]
fn preshape_probes_block_ch_metrics_for_a_different_inline_font() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let block = doc.append_element(
            Some(body),
            "p",
            Style::default(),
            Some("display:block; white-space:pre; tab-size:4; font-family:monospace; font-size:16px; word-spacing:0.25ch"),
        );
    let inline = doc.append_element(
        Some(block),
        "span",
        Style::default(),
        Some("font-family:serif; word-spacing:0.25ch"),
    );
    let text = doc.append_text(inline, "A\tB");
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade Ok");
    let mut fonts = FontContext::new();
    let mut layout_cx = LayoutContext::<()>::new();
    preshape_text(
        &mut doc,
        &cascade,
        &mut fonts,
        &mut layout_cx,
        PageBox::A4.width,
        PageBox::A4.width,
    );
    assert!(!doc.nodes[text].snap_glyph_x_to_1_64());
    assert!(doc.nodes[text].text_layout().is_some());
}

#[test]
fn preshape_inline_pre_tabs_keep_the_local_fallback() {
    let entries = [(
        "display:inline; white-space:pre; tab-size:4; font-family:monospace; font-size:16px",
        "A\tB",
    )];
    let (doc, text_nodes) = preshape_tab_test_entries(&entries);
    assert!(!doc.nodes[text_nodes[0]].snap_glyph_x_to_1_64());
    assert!(doc.nodes[text_nodes[0]].text_layout().unwrap().full_width() > 0.0);
}

#[test]
fn tab_stop_number_uses_block_space_advance() {
    assert_eq!(tab_stop_advance(ComputedTabSize::Number(3.0), 8.0), 24.0);
}

#[test]
fn tab_stop_length_is_absolute() {
    assert_eq!(
        tab_stop_advance(ComputedTabSize::Length(ComputedLength(18.0)), 8.0),
        18.0
    );
}

#[test]
fn establish_minimal_line_boxes_does_not_compress_children_narrower_than_shaped_text() {
    // Regression check for the flex-shrink hazard documented on
    // `establish_minimal_line_boxes`: flex items default to
    // `flex-shrink: 1`, which under `flex_wrap: NoWrap` would (absent
    // this pass's override) compress each child's box narrower than
    // its own already-shaped glyph run once the combined content
    // exceeds the line's available width — the box would shrink but
    // the glyph run would not, so the rendered glyphs would overflow
    // the box and can overlap a neighboring child's glyphs.
    //
    // Two children, each a single unbroken run with no whitespace (so
    // `preshape_text`'s own per-node soft-wrap against the full page
    // width has no break opportunity and leaves each on one line,
    // regardless of length — see the `layout.len() == 1` precondition
    // below), together wide enough to exceed the page — real shrink
    // pressure, absent the override, would apply.
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
        Some("display: block; font-size: 72px"),
    );
    let a = doc.append_text(p, "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAA");
    let c = doc.append_text(p, "CCCCCCCCCCCCCCCCCCCCCCCCCCCCCC");

    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");
    layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("layout Ok");

    let mut shaped_widths = Vec::new();
    for &id in &[a, c] {
        let layout = doc.nodes[id]
            .text_layout()
            .expect("text node must have a preshaped layout");
        // Fixture precondition: each string is one unbroken run with
        // no whitespace, so `preshape_text`'s own per-node soft-wrap
        // (against the full page width) has no break opportunity and
        // leaves it on a single line regardless of length — check that
        // so a `shaped_width` below isn't silently the width of one
        // wrapped sub-line instead of the whole run.
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            layout.len(),
            1,
            "fixture precondition: each child's text must shape to \
                 exactly 1 line (no internal wrap) so its `width()` below \
                 reflects the whole run, not one wrapped sub-line"
        );
        shaped_widths.push(layout.width());
    }
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert!(
        shaped_widths[0] + shaped_widths[1] > PageBox::A4.width,
        "fixture precondition: the two children combined ({} + {}) \
             must exceed the page width ({}) so real shrink pressure \
             would apply absent the flex_shrink:0 override",
        shaped_widths[0],
        shaped_widths[1],
        PageBox::A4.width
    );

    for (&id, &shaped_width) in [a, c].iter().zip(shaped_widths.iter()) {
        let box_width = doc.nodes[id].unrounded_layout.size.width;
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert!(
            box_width + 1e-3 >= shaped_width,
            "child box (width={box_width}) must not be compressed \
                 narrower than its own shaped glyph run (width={shaped_width})"
        );
    }
}

#[test]
fn establish_minimal_line_boxes_resets_conflicting_flex_basis() {
    // Regression check for the `flex_basis: auto` reset documented on
    // `establish_minimal_line_boxes`. `b` below carries an explicit
    // `flex-basis: 10px` far smaller than its own shaped content — a
    // declaration that was inert while `p` was a plain block
    // container but would become a live flex-item property (and a
    // box/glyph-desync hazard) once `p` qualifies here, absent this
    // reset.
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
        Some("display: block; font-size: 72px"),
    );
    let _a = doc.append_text(p, "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAA");
    let b = doc.append_element(
        Some(p),
        "b",
        Style::default(),
        Some("display: inline; flex-basis: 10px"),
    );
    let c = doc.append_text(b, "CCCCCCCCCCCCCCCCCCCCCCCCCCCCCC");

    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");
    layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("layout Ok");

    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        doc.nodes[b].style.flex_basis,
        Dimension::auto(),
        "`b`'s explicit flex-basis:10px must be reset to auto once `p` \
             qualifies for minimal-line-box treatment — a non-auto \
             author flex-basis directly sets the box's main size \
             regardless of flex_shrink:0, so leaving it in place would \
             reopen the box/glyph desync hazard flex_shrink:0 exists to \
             prevent"
    );

    let shaped_width = doc.nodes[c]
        .text_layout()
        .expect("text node must have a preshaped layout")
        .width();
    let b_box_width = doc.nodes[b].unrounded_layout.size.width;
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert!(
        b_box_width + 1e-3 >= shaped_width,
        "`b`'s box (width={b_box_width}) must not be compressed \
             below its shaped content (width={shaped_width}) now that \
             the conflicting flex-basis:10px is reset to auto"
    );
}

#[test]
fn establish_minimal_line_boxes_resets_conflicting_justify_content_and_gap() {
    // Regression check for the `justify_content: None` / `gap: 0`
    // resets documented on `establish_minimal_line_boxes`. `p` below
    // carries explicit `justify-content: space-between` and
    // `column-gap: 500px` — both inert while `p` was a plain block
    // container, both live main-axis-affecting flex-container
    // properties once `p` qualifies here, absent these resets. If
    // either leaked through, the two participating children would
    // end up far apart (`justify-content: space-between` alone would
    // push the second child to the far right edge, and a 500px gap
    // would separate them further still) instead of adjacent.
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
        Some("display: block; justify-content: space-between; column-gap: 500px"),
    );
    let a = doc.append_text(p, "AAAA");
    let c = doc.append_text(p, "CCCC");

    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");
    layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("layout Ok");

    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        doc.nodes[p].style.justify_content, None,
        "`p`'s explicit justify-content:space-between must be reset \
             to taffy's default (None) once it qualifies for \
             minimal-line-box treatment — line boxes have no main-axis \
             item-redistribution semantics to preserve"
    );
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        doc.nodes[p].style.gap,
        Size {
            width: LengthPercentage::length(0.0),
            height: LengthPercentage::length(0.0),
        },
        "`p`'s explicit column-gap:500px must be reset to 0 once it \
             qualifies for minimal-line-box treatment — line boxes have \
             no author-controllable gap semantics to preserve"
    );

    let a_loc = doc.nodes[a].unrounded_layout;
    let c_loc = doc.nodes[c].unrounded_layout;
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert!(
        c_loc.location.x < a_loc.location.x + a_loc.size.width + 1.0,
        "the two children must still sit adjacent (within 1px) — \
             got a.x={} a.w={} c.x={}, i.e. neither the space-between \
             nor the 500px gap leaked through",
        a_loc.location.x,
        a_loc.size.width,
        c_loc.location.x
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
fn establish_minimal_line_boxes_br_full_basis_resolves_against_narrow_container_width() {
    // `<br>`'s flex_basis:100% (`establish_minimal_line_boxes`'s doc)
    // must resolve against the *qualifying container's own* used main
    // size, not its containing block's — otherwise a narrower
    // container (explicit `width`, rather than filling its parent)
    // would give <br> a too-wide box. Pins that by giving the
    // container an explicit width much narrower than the page and
    // checking <br>'s own box stays within it.
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
        Some("display: block; width: 100px"),
    );
    let a = doc.append_text(p, "AAAA");
    let br = doc.append_element(Some(p), "br", Style::default(), None::<&str>);
    let c = doc.append_text(p, "CCCC");

    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");
    layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("layout Ok");

    let p_loc = doc.nodes[p].unrounded_layout;
    let br_loc = doc.nodes[br].unrounded_layout;
    let a_loc = doc.nodes[a].unrounded_layout;
    let c_loc = doc.nodes[c].unrounded_layout;

    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert!(
        (p_loc.size.width - 100.0).abs() < 1e-3,
        "fixture precondition: explicit width:100px must actually take \
             effect, got p.w={}",
        p_loc.size.width
    );
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert!(
        br_loc.size.width <= p_loc.size.width + 1e-3,
        "<br>'s flex_basis:100% must resolve against the qualifying \
             container's own (narrow) used width, not some wider \
             containing block — got br.w={} p.w={}",
        br_loc.size.width,
        p_loc.size.width
    );
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert!(
        c_loc.location.y >= a_loc.location.y + a_loc.size.height - 1e-3,
        "the forced break must still occur on a narrow container, got \
             a.y={} a.h={} c.y={}",
        a_loc.location.y,
        a_loc.size.height,
        c_loc.location.y
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
fn establish_minimal_line_boxes_leading_br_does_not_add_leading_blank_line() {
    // Documents (rather than merely asserting) the accepted Non-goal on
    // `establish_minimal_line_boxes`: a `<br>` with no preceding
    // participating content on its line is the *first* item taffy
    // considers for that line, so the flex line-packing exception
    // ("an already-empty line accepts its first item even if it
    // overflows") places it there rather than moving it — and since
    // `<br>` measures 0x0 (no children, no text layout), that first
    // line has no visible height. So a leading `<br>` here does not
    // reproduce the leading blank line real browsers render for it;
    // this pins the current (accepted) behavior instead.
    use parley::FontContext;
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let p = doc.append_element(Some(body), "p", Style::default(), Some("display: block"));
    let br = doc.append_element(Some(p), "br", Style::default(), None::<&str>);
    let a = doc.append_text(p, "AAAA");

    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");
    layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("layout Ok");

    let br_loc = doc.nodes[br].unrounded_layout;
    let a_loc = doc.nodes[a].unrounded_layout;
    let p_loc = doc.nodes[p].unrounded_layout;

    assert_eq!(br_loc.location.y, 0.0);
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert!(
        (p_loc.size.height - a_loc.size.height).abs() < 1e-3,
        "no leading blank line: container height must equal just the \
             text line's height, got p.h={} a.h={}",
        p_loc.size.height,
        a_loc.size.height
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

/// sites 7-8 — `preshape_text` の `cv.line_height` → parley
/// `StyleProperty::LineHeight`。site 5 (`font-size`) と同じ「guard を
/// 外すと fail ではなく hang する」site だが、機構は別
/// (`sanitize_line_height` の doc参照 — `next_x <= max_advance` ではなく
/// `running_line_height > line_max_height` が恒真になる)。
///
/// 通常の `cascade()` だけで非有限値を作れる — `line-height: 1e40`
/// (unitless number)、`line-height: 1e40px` (absolute length) はいずれも
/// cssparser の f64→f32 変換で `+Inf` に saturate する (site 5 の
/// `font-size: 1e40px` と同じ機構)。font-size と違い 2 element も
/// bypass も要らない。
#[test]
fn nonfinite_line_height_is_clamped_before_parley() {
    fn shaped_height(inline_style: &str) -> f32 {
        use parley::{FontContext, LayoutContext};
        use raikiri_style::{build_rule_tree, cascade};

        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
        let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
        let p = doc.append_element(Some(body), "p", Style::default(), Some(inline_style));
        let text = doc.append_text(p, "Hi");
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");
        let mut fonts = FontContext::new();
        let mut layout_cx = LayoutContext::<()>::new();
        preshape_text(
            &mut doc,
            &cr,
            &mut fonts,
            &mut layout_cx,
            PageBox::A4.width,
            PageBox::A4.width,
        );
        doc.nodes[text].text_layout().unwrap().height()
    }

    /// guard 消失時の hang を有界時間の失敗に変える wrapper — site 5 の
    /// `shaped_height_bounded` と同じ構造 (doc参照)。
    fn shaped_height_bounded(inline_style: &str) -> f32 {
        use std::sync::mpsc::RecvTimeoutError;

        let owned = inline_style.to_owned();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(shaped_height(&owned));
        });
        // cov:ignore: every call site of this helper (both assertions
        // below) completes normally within its 30s bound — the Err arms
        // are diagnostics for failure modes (guard regression hang,
        // worker panic) this test's passing runs never hit, same as
        // `shape_raw_bounded`'s sibling match further down this file.
        match rx.recv_timeout(std::time::Duration::from_secs(30)) {
            Ok(h) => h,
            Err(RecvTimeoutError::Timeout) => panic!(
                "parley shaping が 30 秒で終わらなかった — line-height の非有限 \
                     guard (sanitize_line_height) が外れると BreakerState::add_line_height \
                     の max_height_exceeded 分岐が spin する"
            ),
            Err(RecvTimeoutError::Disconnected) => {
                panic!("worker thread が panic した (hang ではない、上の stderr を参照)")
            }
        }
    }

    let number_inf = shaped_height_bounded("line-height: 1e40");
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert!(
        number_inf.is_finite(),
        "line-height +Inf (unitless number 由来) が parley に届いた: {number_inf}"
    );

    let length_inf = shaped_height_bounded("line-height: 1e40px");
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert!(
        length_inf.is_finite(),
        "line-height +Inf (absolute length 由来) が parley に届いた: {length_inf}"
    );
}

// ── site 6: sanitize_font_weight ─────────────
//
// sanitize_finite / sanitize_taffy と同型の unit test。`ComputedValues`
// が全 field `pub` であることに由来する非有限 font_weight (f32 格上げで
// 型による排除ができなくなった) が
// `parley::FontWeight::new` の直前で有限 + `[1,1000]` に収まることを
// 直接検証する。

#[test]
fn sanitize_font_weight_maps_nan_to_normal_fallback() {
    // `f32::clamp` は NaN を NaN のまま返すので、この分岐が無いと NaN が
    // 素通りする。fallback は `0.0` ではなく `FALLBACK_FONT_WEIGHT`
    // (400.0、CSS Fonts 4 §2.2 "Font weight: the font-weight property"
    // <https://www.w3.org/TR/css-fonts-4/#valdef-font-weight-normal> の
    // `normal` の computed value) — `sanitize_finite` の length 系 site
    // とは異なる fallback を選ぶ理由は `sanitize_font_weight` の doc 参照。
    let mut diag = Vec::new();
    assert_eq!(
        sanitize_font_weight(f32::NAN, &mut diag),
        FALLBACK_FONT_WEIGHT
    );
    assert_eq!(diag.len(), 1, "clamp が発火したので 1 event 積まれること");
    match diag[0] {
        LayoutWarn::NonFiniteClamped { site, raw, clamped } => {
            assert_eq!(site, "font-weight");
            assert!(raw.is_nan());
            assert_eq!(clamped, FALLBACK_FONT_WEIGHT);
        }
        other => panic!("unexpected LayoutWarn variant: {other:?}"),
    }
}

#[test]
fn sanitize_font_weight_clamps_infinities_to_bounds() {
    let mut diag = Vec::new();
    assert_eq!(
        sanitize_font_weight(f32::INFINITY, &mut diag),
        MAX_FONT_WEIGHT
    );
    assert_eq!(
        sanitize_font_weight(f32::NEG_INFINITY, &mut diag),
        MIN_FONT_WEIGHT
    );
    assert_eq!(diag.len(), 2, "+Inf / -Inf とも clamp が発火する");
}

#[test]
fn sanitize_font_weight_clamps_out_of_range_finite_values() {
    // 有限でも範囲外なら寄せる (「有限化するだけ」ではない) —
    // `sanitize_taffy_clamps_out_of_range_finite_values` の font-weight 版。
    let mut diag = Vec::new();
    assert_eq!(sanitize_font_weight(1e30, &mut diag), MAX_FONT_WEIGHT);
    assert_eq!(sanitize_font_weight(-1e30, &mut diag), MIN_FONT_WEIGHT);
    // `0.0` は length 系 site では有効な値だが font-weight の妥当域
    // `[1, 1000]` の外 — MIN_FONT_WEIGHT に寄る。
    assert_eq!(sanitize_font_weight(0.0, &mut diag), MIN_FONT_WEIGHT);
    assert_eq!(diag.len(), 3);
}

#[test]
fn sanitize_font_weight_passes_through_in_range_values() {
    // 通常値 (fractional weight 含む) は
    // bit-identical に素通しする。
    let mut diag = Vec::new();
    for v in [
        MIN_FONT_WEIGHT,
        1.0,
        100.0,
        349.5,
        400.0,
        700.0,
        MAX_FONT_WEIGHT,
    ] {
        assert_eq!(
            sanitize_font_weight(v, &mut diag),
            v,
            "in-range value must pass through: {v}"
        );
    }
    assert!(
        diag.is_empty(),
        "in-range value must not push a LayoutWarn: {diag:?}"
    );
}

#[test]
fn sanitize_line_height_passes_through_in_range_values() {
    let mut diag = Vec::new();
    assert_eq!(
        sanitize_line_height(ComputedLineHeight::Normal, &mut diag),
        ComputedLineHeight::Normal
    );
    assert_eq!(
        sanitize_line_height(ComputedLineHeight::Number(0.0), &mut diag),
        ComputedLineHeight::Number(0.0)
    );
    assert_eq!(
        sanitize_line_height(ComputedLineHeight::Number(1.5), &mut diag),
        ComputedLineHeight::Number(1.5)
    );
    assert_eq!(
        sanitize_line_height(ComputedLineHeight::Length(ComputedLength(0.0)), &mut diag),
        ComputedLineHeight::Length(ComputedLength(0.0))
    );
    assert_eq!(
        sanitize_line_height(ComputedLineHeight::Length(ComputedLength(32.0)), &mut diag),
        ComputedLineHeight::Length(ComputedLength(32.0))
    );
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert!(
        diag.is_empty(),
        "in-range value must not push a LayoutWarn: {diag:?}"
    );
}

#[test]
fn font_style_to_parley_maps_normal_and_italic() {
    assert_eq!(
        font_style_to_parley(StyleFontStyle::Normal),
        FontStyle::Normal
    );
    assert_eq!(
        font_style_to_parley(StyleFontStyle::Italic),
        FontStyle::Italic
    );
}

#[test]
fn font_style_to_parley_maps_oblique_not_normal() {
    // Regression: `Oblique` previously fell through the wildcard arm
    // and was silently converted to `FontStyle::Normal`, discarding a
    // real cascade winner (e.g. a child overriding an inherited
    // `font-style: italic` with `font-style: oblique` would render
    // upright instead of slanted).
    assert_eq!(
        font_style_to_parley(StyleFontStyle::Oblique),
        FontStyle::Oblique(None)
    );
}

#[test]
fn line_height_to_parley_maps_all_three_variants() {
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        line_height_to_parley(ComputedLineHeight::Normal),
        LineHeight::MetricsRelative(1.0),
        "Normal must map to parley's own default (MetricsRelative(1.0))"
    );
    assert_eq!(
        line_height_to_parley(ComputedLineHeight::Number(1.5)),
        LineHeight::FontSizeRelative(1.5)
    );
    assert_eq!(
        line_height_to_parley(ComputedLineHeight::Length(ComputedLength(32.0))),
        LineHeight::Absolute(32.0)
    );
}

#[test]
fn preshape_text_pushes_computed_font_style_into_parley_run_attrs() {
    // `preshape_text` が `cv.font_style` を実際に RangedBuilder へ push して
    // いることを、shape 済 `Run` の font-matching 属性から確認する。
    //
    // `GlyphRun::style()` (`parley::layout::Style<B>`) は brush /
    // underline / strikethrough / 非公開 line_height 等のみで
    // `font_style` field を持たないため使えない。代わりに
    // `Run::font_attrs()` (`&fontique::Attributes`, `pub style: FontStyle`
    // field を持つ) を使う — これは実際に選ばれた font file の属性では
    // なく、font matching に**渡された** CSS-requested attribute
    // そのもの (parley `shape` module が `RangedBuilder` へ push した
    // `StyleProperty::FontStyle` から直接組み立てる) なので、実行環境に
    // italic face を持つフォントがあるかどうかに関わらず決定的に検証できる。
    use parley::{FontContext, LayoutContext, PositionedLayoutItem};
    use raikiri_style::{build_rule_tree, cascade};

    fn shape_run_font_style(inline_style: &str) -> FontStyle {
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
        let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
        let p = doc.append_element(Some(body), "p", Style::default(), Some(inline_style));
        let text = doc.append_text(p, "Hi");
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).unwrap();
        let mut fonts = FontContext::new();
        let mut layout_cx = LayoutContext::<()>::new();
        preshape_text(
            &mut doc,
            &cr,
            &mut fonts,
            &mut layout_cx,
            PageBox::A4.width,
            PageBox::A4.width,
        );
        let layout = doc.nodes[text].text_layout().unwrap();
        let line = layout.lines().next().expect("shaped text has one line");
        let item = line
            .items()
            .next()
            .expect("shaped line has at least one item");
        let PositionedLayoutItem::GlyphRun(glyph_run) = item else {
            // cov:ignore: only reached if shaping produces an InlineBox
            // instead of a GlyphRun — preshape_text (this file) never
            // pushes an inline box, matching the same premise
            // `crates/raikiri-paint/src/text.rs`'s glyph-draw walk
            // relies on ("InlineBox は現状生成されない" there), so
            // this is unreachable for plain text today.
            panic!("expected shaped text to produce a GlyphRun, got an InlineBox");
        };
        glyph_run.run().font_attrs().style
    }

    assert_eq!(shape_run_font_style("font-style:normal"), FontStyle::Normal);
    // cov:ignore: this multi-line assert_eq! is exempted as a whole
    // block by patch_coverage.py's bracket-depth scoping rule (the
    // marker's block starts at the open paren below and doesn't close
    // until the matching `);`, so every line in between — including
    // the always-executed call/comparison lines, not just the
    // panic-message continuation lines below — reports exempted). The
    // assertion itself is fully exercised on every run and would fail
    // on an integration regression; only the panic-message string (only
    // "entered" on assertion failure, which doesn't happen while this
    // test passes) is the actual reason for the coverage-attribution
    // gap this marker works around.
    assert_eq!(
        shape_run_font_style("font-style:italic"),
        FontStyle::Italic,
        "font-style:italic must reach the shaped Run's font-matching attributes \
             (integration regression: StyleProperty::FontStyle push missing or dropped)"
    );
}

#[test]
fn preshape_text_pushes_computed_line_height_into_parley_run_metrics() {
    // `preshape_text` が `cv.line_height` を実際に RangedBuilder へ push
    // していることを、shape 済 `Run` の `RunMetrics::line_height` から
    // 確認する。`font_style` の兄弟 test と違い `Run::font_attrs()` では
    // 検証できない (`fontique::Attributes` に line-height 相当の field は
    // 無い) — 代わりに `Run::metrics()` (`&RunMetrics`, `pub line_height:
    // f32` field を持つ) を使う。`Number` / `Length` はいずれも font
    // metrics (ascent / descent / leading) に依存しない計算式
    // (`parley-0.10.0/src/layout/data.rs` の `push_run` 内 `match
    // style.line_height`) なので、実行環境のフォントに関わらず厳密な値で
    // 決定的に検証できる。
    use parley::{FontContext, LayoutContext, PositionedLayoutItem};
    use raikiri_style::{build_rule_tree, cascade};

    fn shaped_run_line_height(inline_style: &str) -> f32 {
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
        let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
        let p = doc.append_element(Some(body), "p", Style::default(), Some(inline_style));
        let text = doc.append_text(p, "Hi");
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).unwrap();
        let mut fonts = FontContext::new();
        let mut layout_cx = LayoutContext::<()>::new();
        preshape_text(
            &mut doc,
            &cr,
            &mut fonts,
            &mut layout_cx,
            PageBox::A4.width,
            PageBox::A4.width,
        );
        let layout = doc.nodes[text].text_layout().unwrap();
        let line = layout.lines().next().expect("shaped text has one line");
        let item = line
            .items()
            .next()
            .expect("shaped line has at least one item");
        let PositionedLayoutItem::GlyphRun(glyph_run) = item else {
            // cov:ignore: preshape_text never produces an InlineBox for
            // plain text — see the same premise in the font-style
            // sibling test above.
            panic!("expected shaped text to produce a GlyphRun, got an InlineBox");
        };
        glyph_run.run().metrics().line_height
    }

    // `Number` (unitless multiplier) — `data.rs`'s `FontSizeRelative(value)
    // => value * font_size` is exact arithmetic on the pushed
    // `StyleProperty::FontSize` value (`font-size: 16px` here), so the
    // expected result is bit-computable, not just "taller than".
    //
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        shaped_run_line_height("font-size: 16px; line-height: 3"),
        48.0,
        "line-height: 3 at font-size: 16px must reach the shaped Run's \
             line_height as exactly 3.0 * 16.0 (integration regression: \
             StyleProperty::LineHeight push missing, dropped, or mismapped)"
    );

    // `Length` (absolute px) — `data.rs`'s `Absolute(value) => value` is a
    // pure passthrough, so the expected result is the declared px value
    // verbatim, independent of font-size.
    //
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        shaped_run_line_height("font-size: 16px; line-height: 50px"),
        50.0,
        "line-height: 50px must reach the shaped Run's line_height as \
             exactly 50.0 regardless of font-size (integration regression: \
             StyleProperty::LineHeight push missing, dropped, or mismapped)"
    );

    // `Normal` (the property's initial value, and the case this module
    // used unconditionally before this integration) must still resolve to a
    // positive, finite metrics-derived value — pins that leaving
    // line-height unset doesn't regress to 0 or a non-finite value.
    let normal = shaped_run_line_height("font-size: 16px");
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert!(
        normal.is_finite() && normal > 0.0,
        "line-height: normal (default) must still produce a finite, \
             positive line_height: {normal}"
    );
}

#[test]
fn preshape_text_sanitizes_non_finite_font_weight_bypassing_cascade() {
    // `ComputedValues` は全 field が `pub` なので、cascade を経由しない
    // 直接構築 (ここでは cascade() 後に該当 node の font_weight だけを
    // 上書きする形で再現) から非有限値が来る経路がある。この経路が
    // `preshape_text` を panic させないこと — sink 直前で
    // `sanitize_font_weight` が有限化すること — を確認する。
    //
    // `CascadeResult` / `ComputedValues` はどちらも `#[non_exhaustive]`
    // なので、raikiri-dom (外部 crate) からは struct literal で直接
    // construct できない。正当な `cascade()` 呼び出しで得た
    // `CascadeResult` の `pub computed: Vec<ComputedValues>` を後から
    // 上書きすることで、「cascade を経由しない値」を再現する — これは
    // `ComputedValues::font_weight` の doc が挙げる
    // `crate::page::cascade_page` の継承元 root 引数と同じ攻撃面
    // (呼び出し元が任意の `ComputedValues` を用意して渡せる) の縮図。
    //
    // `text_layout().is_some()` だけでは「panic しなかった」ことしか
    // 検証できない — 将来誰かが `preshape_text` から
    // `sanitize_font_weight` の呼び出しを誤って外しても (parley が
    // 非有限値を panic せず黒箱処理する場合)、それは検知できない。
    // そこで `doc.layout_warnings` (`sanitize_font_weight` が実際に
    // clamp した時だけ push する `LayoutWarn::NonFiniteClamped`
    // の蓄積先) を直接検査し、sink 直前に渡った raw 値と、そこから
    // 実際に有限化された値の両方を assert する — 配線が外れれば
    // site `"font-weight"` の event が一切積まれなくなるので、
    // その断線をここで検知できる。
    use parley::{FontContext, LayoutContext};
    use raikiri_style::{build_rule_tree, cascade};

    for (label, weight) in [
        ("NaN", f32::NAN),
        ("+Inf", f32::INFINITY),
        ("-Inf", f32::NEG_INFINITY),
        ("out-of-range finite (1e30)", 1e30_f32),
    ] {
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
        let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
        let p = doc.append_element(Some(body), "p", Style::default(), None::<&str>);
        let text = doc.append_text(p, "Hi");

        let rules = build_rule_tree(&doc);
        let mut cr = cascade(&doc, &rules).expect("cascade Ok");
        cr.computed[p].font_weight = weight;
        cr.computed[text].font_weight = weight;

        let mut fonts = FontContext::new();
        let mut layout_cx = LayoutContext::<()>::new();
        preshape_text(
            &mut doc,
            &cr,
            &mut fonts,
            &mut layout_cx,
            PageBox::A4.width,
            PageBox::A4.width,
        );

        assert!(
            doc.nodes[text].text_layout().is_some(),
            "{label}: preshape_text must not panic and must still populate \
                 text_layout despite a non-finite font_weight bypassing cascade"
        );

        // 配線検証: font-size はこの test では触っていないので clamp は
        // 発火せず、"font-weight" site の event だけが (毎 case とも
        // clamp が実際に効くので) ちょうど 1 件積まれるはず。
        let font_weight_events: Vec<LayoutWarn> = doc
            .layout_warnings
            .iter()
            .copied()
            .filter(|w| {
                matches!(
                    w,
                    LayoutWarn::NonFiniteClamped {
                        site: "font-weight",
                        ..
                    }
                )
            })
            .collect();
        assert_eq!(
            font_weight_events.len(),
            1,
            "{label}: expected exactly one \"font-weight\" NonFiniteClamped \
                 event (only pushed when sanitize_font_weight actually ran and \
                 clamped) — 0 events means the sanitize_font_weight call was \
                 removed from preshape_text without this test noticing; \
                 all events: {:?}",
            doc.layout_warnings
        );
        let LayoutWarn::NonFiniteClamped { raw, clamped, .. } = font_weight_events[0] else {
            unreachable!("filtered for this variant above");
        };
        assert!(
            raw.is_nan() == weight.is_nan() && (raw.is_nan() || raw == weight),
            "{label}: the raw value sanitize_font_weight saw ({raw}) must be \
                 the exact font_weight this test set ({weight}), proving \
                 sanitize_font_weight is wired to cv.font_weight and not some \
                 unrelated value"
        );
        assert!(
            clamped.is_finite(),
            "{label}: the value handed to parley::FontWeight::new must be \
                 finite, got {clamped}"
        );
        assert!(
            (MIN_FONT_WEIGHT..=MAX_FONT_WEIGHT).contains(&clamped),
            "{label}: the value handed to parley::FontWeight::new must be \
                 within [{MIN_FONT_WEIGHT}, {MAX_FONT_WEIGHT}], got {clamped}"
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

#[test]
fn preshape_text_removes_soft_hyphens_when_hyphenation_is_disabled() {
    use parley::{FontContext, LayoutContext};
    use raikiri_style::{build_rule_tree, cascade};

    fn shaped_text_end(inline_style: &str) -> usize {
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
        let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
        let p = doc.append_element(Some(body), "p", Style::default(), Some(inline_style));
        let text = doc.append_text(p, "a\u{00AD}b");
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");
        let mut fonts = FontContext::new();
        let mut layout_cx = LayoutContext::<()>::new();
        preshape_text(
            &mut doc,
            &cr,
            &mut fonts,
            &mut layout_cx,
            PageBox::A4.width,
            PageBox::A4.width,
        );
        doc.nodes[text]
            .text_layout()
            .expect("text should be shaped")
            .lines()
            .next()
            .expect("shaped text should have a line")
            .text_range()
            .end
    }

    assert_eq!(shaped_text_end("hyphens: none"), 2);
    assert_eq!(shaped_text_end("hyphens: manual"), 4);
}

#[test]
fn probe_ic_text_advance_uses_mapped_wpt_metrics() {
    let tmp = embedded_ic_font_dir();

    let mut fonts = crate::fonts::build_wpt_font_ctx(tmp.path()).expect("register WPT fonts");
    let mut layout_cx = LayoutContext::<()>::new();
    let probe = |family: &str, fonts: &mut FontContext, layout_cx: &mut LayoutContext<()>| -> f32 {
        probe_ic_text_advance(
            fonts,
            layout_cx,
            family,
            16.0,
            400.0,
            StyleFontStyle::Normal,
        )
    };

    assert!((probe("IcTestZeroWidth", &mut fonts, &mut layout_cx) - 0.0).abs() < 0.001);
    assert!((probe("IcTestHalfWidth", &mut fonts, &mut layout_cx) - 8.0).abs() < 0.001);
    assert!((probe("IcTestFullWidth", &mut fonts, &mut layout_cx) - 16.0).abs() < 0.001);
    assert!((probe("CanvasTestNoSpace", &mut fonts, &mut layout_cx) - 16.0).abs() < 0.001);
    assert!(
        (probe(
            "CanvasTestNoSpace, IcTestHalfWidth",
            &mut fonts,
            &mut layout_cx,
        ) - 8.0)
            .abs()
            < 0.001
    );
    assert!((probe("\"IcTestHalfWidth\"", &mut fonts, &mut layout_cx) - 8.0).abs() < 0.001);
    assert!((probe("MissingFamily, serif", &mut fonts, &mut layout_cx) - 16.0).abs() < 0.001);
    assert!(
        (probe_ic_zero_advance(
            &mut fonts,
            &mut layout_cx,
            "CanvasTestNoSpace",
            16.0,
            400.0,
            StyleFontStyle::Normal,
        ) - 16.0)
            .abs()
            < 0.001
    );
}

#[test]
fn family_candidate_has_ch_glyphs_checks_space_and_zero_glyph_coverage() {
    // CSS Values 4 §6.1.1 `ch`: the first-available face must supply a
    // usable space glyph *and* U+0030, whose advance becomes the `ch`
    // metric (see the function's own doc comment).
    let tmp = embedded_ic_font_dir();
    let mut fonts = crate::fonts::build_wpt_font_ctx(tmp.path()).expect("register WPT fonts");

    // Ahem maps every ASCII printable codepoint it supports (including
    // space and '0') to its square glyph, so both required glyphs are
    // present and the query stops at the first candidate.
    assert!(family_candidate_has_ch_glyphs(
        &mut fonts,
        "Ahem",
        400.0,
        StyleFontStyle::Normal,
    ));

    // CanvasTest-nospace.ttf deliberately omits a usable space glyph
    // (see the sibling `generic_family_has_ch_glyphs` coverage above),
    // so the query callback keeps returning `QueryStatus::Continue` and
    // the loop exhausts without ever finding a usable face.
    assert!(!family_candidate_has_ch_glyphs(
        &mut fonts,
        "CanvasTestNoSpace",
        400.0,
        StyleFontStyle::Normal,
    ));

    // An unregistered family name matches no face at all: the query
    // callback never runs and `has_ch_glyphs` keeps its initial `false`.
    assert!(!family_candidate_has_ch_glyphs(
        &mut fonts,
        "NotARegisteredFamily",
        400.0,
        StyleFontStyle::Normal,
    ));
}

#[test]
fn probe_text_advance_distinguishes_ahem_and_proportional_wpt_fonts() {
    use std::path::PathBuf;

    let fonts_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("target")
        .join("wpt")
        .join("fonts");
    if !fonts_dir.join("Ahem.ttf").exists() || !fonts_dir.join("Lato-Medium.ttf").exists() {
        eprintln!(
            "skipping WPT font probe: Ahem.ttf and Lato-Medium.ttf are required under {}",
            fonts_dir.display()
        );
        return;
    }

    let mut fonts =
        crate::fonts::build_wpt_font_ctx(&fonts_dir).expect("bundled WPT fonts should register");
    let mut layout_cx = LayoutContext::<()>::new();
    let ahem = probe_text_advance(
        &mut fonts,
        &mut layout_cx,
        "0",
        "Ahem",
        16.0,
        400.0,
        StyleFontStyle::Normal,
    );
    let lato = probe_text_advance(
        &mut fonts,
        &mut layout_cx,
        "0",
        "Lato",
        16.0,
        400.0,
        StyleFontStyle::Normal,
    );
    assert!((ahem - 16.0).abs() < 0.1, "Ahem zero advance={ahem}");
    assert!(lato.is_finite() && lato > 0.0, "Lato zero advance={lato}");
    assert!(
        (ahem - lato).abs() > 0.5,
        "Ahem and proportional Lato should have different zero advances: Ahem={ahem}, Lato={lato}"
    );

    fn shaped_width(fonts_dir: &std::path::Path, font_family: &str, letter_spacing: &str) -> f32 {
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
        let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
        let style =
            format!("font-family:{font_family};font-size:16px;letter-spacing:{letter_spacing}");
        let p = doc.append_element(Some(body), "p", Style::default(), Some(&style));
        let text = doc.append_text(p, "AB");
        let rules = raikiri_style::build_rule_tree(&doc);
        let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade Ok");
        let mut fonts =
            crate::fonts::build_wpt_font_ctx(fonts_dir).expect("bundled WPT fonts should register");
        let mut layout_cx = LayoutContext::<()>::new();
        preshape_text(
            &mut doc,
            &cascade,
            &mut fonts,
            &mut layout_cx,
            PageBox::A4.width,
            PageBox::A4.width,
        );
        doc.nodes[text]
            .text_layout()
            .expect("text should be shaped")
            .full_width()
    }

    let ahem_normal = shaped_width(&fonts_dir, "Ahem", "0px");
    let ahem_spaced = shaped_width(&fonts_dir, "Ahem", "1ch");
    assert!(
        (ahem_spaced - ahem_normal - 2.0 * ahem).abs() < 0.1,
        "letter-spacing:1ch must use Ahem's measured zero advance: normal={ahem_normal}, spaced={ahem_spaced}, ch={ahem} (two glyphs)"
    );

    let lato_normal = shaped_width(&fonts_dir, "Lato", "0px");
    let lato_spaced = shaped_width(&fonts_dir, "Lato", "1ch");
    assert!(
        (lato_spaced - lato_normal - 2.0 * lato).abs() < 0.1,
        "letter-spacing:1ch must use Lato's measured zero advance: normal={lato_normal}, spaced={lato_spaced}, ch={lato} (two glyphs)"
    );
}

#[test]
fn preshape_text_applies_computed_letter_spacing_to_advance() {
    use parley::{FontContext, LayoutContext};
    use raikiri_style::{build_rule_tree, cascade};

    fn shaped_width(inline_style: &str) -> f32 {
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
        let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
        let p = doc.append_element(Some(body), "p", Style::default(), Some(inline_style));
        let text = doc.append_text(p, "Hello");
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");
        let mut fonts = FontContext::new();
        let mut layout_cx = LayoutContext::<()>::new();
        preshape_text(
            &mut doc,
            &cr,
            &mut fonts,
            &mut layout_cx,
            PageBox::A4.width,
            PageBox::A4.width,
        );
        doc.nodes[text]
            .text_layout()
            .expect("text should be shaped")
            .full_width()
    }

    let normal = shaped_width("letter-spacing: 0px");
    let spaced = shaped_width("letter-spacing: 4px");
    assert!(
        spaced > normal + 12.0,
        "letter spacing should increase the shaped advance: normal={normal}, spaced={spaced}"
    );
}

#[test]
fn preshape_text_applies_computed_letter_spacing_ch_to_advance() {
    use parley::{FontContext, LayoutContext};
    use raikiri_style::{build_rule_tree, cascade};

    fn shaped_width(inline_style: &str) -> f32 {
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
        let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
        let p = doc.append_element(Some(body), "p", Style::default(), Some(inline_style));
        let text = doc.append_text(p, "AB");
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");
        let mut fonts = FontContext::new();
        let mut layout_cx = LayoutContext::<()>::new();
        preshape_text(
            &mut doc,
            &cr,
            &mut fonts,
            &mut layout_cx,
            PageBox::A4.width,
            PageBox::A4.width,
        );
        doc.nodes[text]
            .text_layout()
            .expect("text should be shaped")
            .full_width()
    }

    let normal = shaped_width("letter-spacing: 0px");
    let spaced = shaped_width("letter-spacing: 1ch");
    assert!(
        spaced > normal + 1.0,
        "ch letter spacing should increase the shaped advance: normal={normal}, spaced={spaced}"
    );
}

#[test]
fn preshape_text_applies_computed_word_spacing_ch_to_advance() {
    use parley::{FontContext, LayoutContext};
    use raikiri_style::{build_rule_tree, cascade};

    fn shaped_width(inline_style: &str) -> f32 {
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
        let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
        let p = doc.append_element(Some(body), "p", Style::default(), Some(inline_style));
        let text = doc.append_text(p, "A B");
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");
        let mut fonts = FontContext::new();
        let mut layout_cx = LayoutContext::<()>::new();
        preshape_text(
            &mut doc,
            &cr,
            &mut fonts,
            &mut layout_cx,
            PageBox::A4.width,
            PageBox::A4.width,
        );
        doc.nodes[text]
            .text_layout()
            .expect("text should be shaped")
            .full_width()
    }

    let normal = shaped_width("word-spacing: 0px");
    let spaced = shaped_width("word-spacing: 1ch");
    assert!(
        spaced > normal + 1.0,
        "ch word spacing should increase the shaped advance: normal={normal}, spaced={spaced}"
    );
}

#[test]
fn text_transform_maps_case_width_kana_and_language_tailoring() {
    assert_eq!(
        apply_text_transform("hello world", TextTransform::Capitalize, ""),
        "Hello World"
    );
    assert_eq!(
        apply_text_transform("a b", TextTransform::UppercaseFullWidth, ""),
        "Ａ　Ｂ"
    );
    assert_eq!(
        apply_text_transform("ぁァㇰｧ", TextTransform::FullSizeKana, ""),
        "あアクｱ"
    );
    assert_eq!(
        apply_text_transform(
            "\u{1b132}\u{1b150}\u{1b151}\u{1b152}\u{1b155}\u{1b164}\u{1b165}\u{1b166}\u{1b167}",
            TextTransform::FullSizeKana,
            ""
        ),
        "こゐゑをコヰヱヲン"
    );
    assert_eq!(
        apply_text_transform("iIİı", TextTransform::Uppercase, "tr"),
        "İIİI"
    );
    assert_eq!(
        apply_text_transform("İI", TextTransform::Lowercase, "tr"),
        "iı"
    );
    assert_eq!(
        apply_text_transform("ijsland", TextTransform::Capitalize, "nl"),
        "IJsland"
    );
    assert_eq!(
        apply_text_transform("καλημέρα αύριο", TextTransform::Uppercase, "el"),
        "ΚΑΛΗΜΕΡΑ ΑΥΡΙΟ"
    );
    assert_eq!(
        apply_text_transform("ευφυΐα Νεράιδα", TextTransform::Uppercase, "el"),
        "ΕΥΦΥΪΑ ΝΕΡΑΪΔΑ"
    );
    // Enclosed alphanumerics are not titlecased by CSS capitalize.
    assert_eq!(
        apply_text_transform("ⓐ ⓑ", TextTransform::Capitalize, ""),
        "ⓐ ⓑ"
    );
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
    let mut first_page = PageFragment::default();
    first_page.page_index = 0;
    first_page.items.push(PageFragmentItem::new(
        NodeId::new(9),
        PageFragmentRect::new(0.0, 0.0, 10.0, 4.0),
        PageFragmentKind::Box,
        0,
        2,
        false,
    ));
    let mut second_page = PageFragment::default();
    second_page.page_index = 1;
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
fn text_autospace_boxes_insert_expected_boundary_advances() {
    use raikiri_style::property::{TextAutospace, TextAutospaceMode};

    let boxes = text_autospace_boxes("国A国1A", TextAutospace::Normal, "", 40.0);
    assert_eq!(
        boxes
            .iter()
            .map(|inline_box| inline_box.index)
            .collect::<Vec<_>>(),
        vec![3, 4, 7]
    );
    assert!(boxes.iter().all(|inline_box| {
        inline_box.kind == InlineBoxKind::InFlow
            && inline_box.width == 5.0
            && inline_box.height == 0.0
    }));

    // The caller supplies the measured `ic` advance; the placement helper
    // must preserve its one-eighth width rather than recomputing from a
    // font-size approximation.
    let measured_ic = 37.25;
    let measured_gap = measured_ic * 0.125;
    let measured =
        text_autospace_boxes_with_width("国A", TextAutospace::Normal, "", measured_gap, None, None);
    assert_eq!(measured.len(), 1);
    assert_eq!(measured[0].width, measured_gap);

    let zh_punctuation = text_autospace_boxes("国!国", TextAutospace::Normal, "zh", 40.0);
    assert_eq!(
        zh_punctuation
            .iter()
            .map(|inline_box| inline_box.index)
            .collect::<Vec<_>>(),
        vec![3, 4]
    );
    assert!(text_autospace_boxes("国!国", TextAutospace::Normal, "en", 40.0).is_empty());

    let custom = text_autospace_boxes(
        "国A",
        TextAutospace::Custom {
            ideograph_alpha: true,
            ideograph_numeric: false,
            punctuation: false,
            mode: TextAutospaceMode::None,
        },
        "en",
        40.0,
    );
    assert_eq!(
        custom
            .iter()
            .map(|inline_box| inline_box.index)
            .collect::<Vec<_>>(),
        vec![3]
    );

    let cross_before =
        text_autospace_boxes_with_edges("A", TextAutospace::Normal, "", 40.0, Some('国'), None);
    assert_eq!(cross_before[0].index, 0);
    let cross_after =
        text_autospace_boxes_with_edges("国", TextAutospace::Normal, "", 40.0, None, Some('A'));
    assert_eq!(cross_after[0].index, 3);
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
