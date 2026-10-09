use super::*;

#[test]
fn cloned_projection_keeps_marker_text_after_the_original_is_dropped() {
    let mut document = Document::new();
    let body = document.append_element(
        Some(0),
        "body",
        taffy::Style::default(),
        Some("display:block;margin:0"),
    );
    document.append_element(
        Some(body),
        "li",
        taffy::Style::default(),
        Some("display:list-item;list-style-type:decimal;font:10px Ahem"),
    );
    let fonts = crate::build_wpt_font_collection(std::path::Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/data/text-autospace"
    )))
    .unwrap();
    document.set_font_collection(fonts);
    let cascade =
        raikiri_style::cascade(&document, &raikiri_style::build_rule_tree(&document)).unwrap();
    let page = raikiri_traits::PageBox::A4;
    crate::layout_single_page(&mut document, &cascade, page).unwrap();
    document
        .project_pages(
            &cascade,
            page,
            &[crate::PageSlice {
                page_index: 0,
                content_origin_y: 0.0,
                page_name: None,
            }],
            &[],
        )
        .unwrap();
    let cloned = document.clone();
    drop(document);
    let runs = cloned.page_text_runs(&cascade, 0);
    assert!(
        runs.iter()
            .any(|run| run.is_standalone_marker() && run.text.trim_end() == "1.")
    );
    let diagnostic = format!("{cloned:?}");
    assert!(
        diagnostic.contains("MarkerText") && diagnostic.contains("lines: 1"),
        "marker cache must remain inspectable"
    );
}

#[test]
fn marker_projection_propagates_the_fixed_counter_snapshot_budget() {
    let mut document = Document::new();
    let names = (0..1024)
        .map(|index| format!("part{index} 0"))
        .collect::<Vec<_>>()
        .join(" ");
    let parent = document.append_element(
        Some(0),
        "ol",
        taffy::Style::default(),
        Some(&format!("display:block;counter-reset:{names}")),
    );
    let mut owner = 0;
    for _ in 0..1100 {
        owner = document.append_element(
            Some(parent),
            "li",
            taffy::Style::default(),
            Some("display:list-item"),
        );
    }
    let cascade =
        raikiri_style::cascade(&document, &raikiri_style::build_rule_tree(&document)).unwrap();
    let roots = [ProjectedTextRoot {
        node: crate::generated_content::generated_node_id(owner, PseudoElem::Marker),
        x: 0.0,
        y: 0.0,
        is_repeat: false,
        fragmentainer: None,
        fragment_clip: None,
        overflow_chain: None,
    }];
    let error = prepare_markers(&document, &cascade, &roots, &[]).unwrap_err();
    let raikiri_traits::LayoutError::CounterSnapshotLimitExceeded { limit, actual } = error else {
        panic!("expected a snapshot budget error: {error:?}")
    };
    assert_eq!(limit, crate::MAX_COUNTER_SNAPSHOT_ESTIMATED_BYTES);
    assert!(actual > limit);
}

#[test]
fn each_glyph_covers_its_cluster_up_to_the_next_one() {
    // Clusters at bytes 10, 11 and 13 of a run spanning 10..15; the glyph
    // of cluster 11 is a ligature of two characters.
    assert_eq!(
        glyph_text_ranges(&[10, 11, 13], 10, 15),
        vec![0..1, 1..3, 3..5]
    );
}

#[test]
fn glyphs_of_one_cluster_share_its_range() {
    // A base and its mark share cluster 4.
    assert_eq!(glyph_text_ranges(&[4, 4, 6], 4, 8), vec![0..2, 0..2, 2..4]);
}

#[test]
fn a_cluster_outside_the_run_is_clamped_to_the_run_text() {
    // A cluster shared with the previous run starts before this run.
    assert_eq!(glyph_text_ranges(&[2, 5], 3, 7), vec![0..2, 2..4]);
    assert_eq!(glyph_text_ranges(&[9], 3, 7), vec![4..4]);
}

#[test]
fn variations_keep_their_axis_and_value() {
    let variation = shodo::style::FontVariation {
        tag: *b"wght",
        value: 650.0,
    };
    assert_eq!(
        font_variation(&variation),
        FontVariation {
            tag: Tag(*b"wght"),
            value: 650.0,
        }
    );
}

#[test]
fn ellipsis_text_is_one_ellipsis_or_one_period_per_glyph() {
    assert_eq!(
        ellipsis_text(1, false),
        ("\u{2026}", std::iter::once(0..3).collect())
    );
    assert_eq!(ellipsis_text(3, false), ("...", vec![0..1, 1..2, 2..3]));
    // A period of a fallback split across fonts is a run of its own.
    assert_eq!(
        ellipsis_text(1, true),
        (".", std::iter::once(0..1).collect())
    );
    assert_eq!(ellipsis_text(2, true), ("..", vec![0..1, 1..2]));
    // Extra glyphs share the last period.
    assert_eq!(
        ellipsis_text(4, false),
        ("...", vec![0..1, 1..2, 2..3, 2..3])
    );
}

#[test]
fn a_typographic_box_is_not_reported_as_generated_before_text() {
    let mut document = Document::new();
    let owner = document.append_element(Some(0), "div", taffy::Style::default(), None::<&str>);
    let letter = crate::generated_content::generated_node_id(owner, PseudoElem::FirstLetter);
    assert_eq!(run_source(&document, letter), None);
    for (pseudo, kind) in [
        (PseudoElem::Before, GeneratedKind::Before),
        (PseudoElem::After, GeneratedKind::After),
        (PseudoElem::Marker, GeneratedKind::Marker),
    ] {
        let id = crate::generated_content::generated_node_id(owner, pseudo);
        assert_eq!(
            run_source(&document, id),
            Some(RunSource::Generated(NodeId::new(owner as u64), kind))
        );
    }
}
