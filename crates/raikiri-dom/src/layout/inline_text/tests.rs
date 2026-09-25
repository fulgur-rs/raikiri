use super::*;
use taffy::Style;

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

    let image_block =
        doc.append_element(Some(body), "div", Style::default(), Some("display: block"));
    doc.append_element(
        Some(image_block),
        "img",
        Style::default(),
        Some("display: inline; width: 20px; height: 20px"),
    );
    let image_whitespace = doc.append_text(image_block, "\u{00a0}\u{00a0}\u{00a0}\u{00a0}");
    doc.append_element(
        Some(image_block),
        "img",
        Style::default(),
        Some("display: inline; width: 20px; height: 20px"),
    );

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
    assert!(
        doc.nodes[image_whitespace]
            .style
            .size
            .width
            .into_option()
            .is_some_and(|width| width > 0.0),
        "NBSPs between inline images keep a measured advance"
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
fn collect_inline_linebox_extents_composes_shifts_and_stops_at_boundaries() {
    use raikiri_style::{build_rule_tree, cascade};

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let p = doc.append_element(
        Some(body),
        "p",
        Style::default(),
        Some("display:block;font-size:16px"),
    );
    let outer = doc.append_element(
        Some(p),
        "span",
        Style::default(),
        Some("display:inline;vertical-align:super"),
    );
    let inner = doc.append_element(
        Some(outer),
        "span",
        Style::default(),
        Some("display:inline;vertical-align:super"),
    );
    doc.append_text(inner, "nested");
    let block_child = doc.append_element(
        Some(outer),
        "div",
        Style::default(),
        Some("display:block;vertical-align:super"),
    );
    let block_inner = doc.append_element(
        Some(block_child),
        "span",
        Style::default(),
        Some("display:inline;vertical-align:super"),
    );
    doc.append_text(block_inner, "block");
    let atomic = doc.append_element(
        Some(p),
        "span",
        Style::default(),
        Some("display:inline-block;vertical-align:super"),
    );
    let atomic_child = doc.append_element(
        Some(atomic),
        "span",
        Style::default(),
        Some("display:inline;vertical-align:super"),
    );
    doc.append_text(atomic_child, "atomic");

    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");
    let mut synthetic_line_roots = vec![false; doc.nodes.len()];
    let mut extents = SyntheticLineboxExtents::default();

    collect_inline_linebox_extents(
        &doc,
        &cr,
        &synthetic_line_roots,
        outer,
        16.0,
        0.0,
        &mut extents,
    );
    assert_eq!(extents.leading, 16.0 / 3.0 + 16.0 / 3.0);
    assert_eq!(extents.trailing, 0.0);

    // A nested synthetic root owns the extent of its descendants; the
    // containing root sees only that wrapper's outer vertical-align shift.
    synthetic_line_roots[outer] = true;
    extents = SyntheticLineboxExtents::default();
    collect_inline_linebox_extents(
        &doc,
        &cr,
        &synthetic_line_roots,
        outer,
        16.0,
        0.0,
        &mut extents,
    );
    assert_eq!(extents.leading, 16.0 / 3.0);
    assert_eq!(extents.trailing, 0.0);

    // An inline-block contributes its own outer shift, not its internal
    // line-box shifts, to the containing line root.
    extents = SyntheticLineboxExtents::default();
    collect_inline_linebox_extents(
        &doc,
        &cr,
        &synthetic_line_roots,
        atomic,
        16.0,
        0.0,
        &mut extents,
    );
    assert_eq!(extents.leading, 16.0 / 3.0);
    assert_eq!(extents.trailing, 0.0);
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
