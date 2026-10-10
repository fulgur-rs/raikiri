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

/// Two Ahem paragraphs, one X per 10px line: 25 lines, then 3 lines below.
fn tall_paragraphs() -> (Document, CascadeResult, [usize; 2]) {
    let mut document = Document::new();
    let body = document.append_element(
        Some(0),
        "body",
        taffy::Style::default(),
        Some("display:block;margin:0"),
    );
    let mut roots = [0; 2];
    for (root, count) in roots.iter_mut().zip([25, 3]) {
        *root = document.append_element(
            Some(body),
            "div",
            taffy::Style::default(),
            Some("display:block;width:10px;font:10px/10px Ahem"),
        );
        document.append_text(*root, vec!["X"; count].join(" "));
    }
    document.mark_in_document_flags();
    let fonts = crate::build_wpt_font_collection(std::path::Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/data/text-autospace"
    )))
    .unwrap();
    document.set_font_collection(fonts);
    let cascade =
        raikiri_style::cascade(&document, &raikiri_style::build_rule_tree(&document)).unwrap();
    crate::layout_single_page(&mut document, &cascade, raikiri_traits::PageBox::A4).unwrap();
    (document, cascade, roots)
}

fn project_at(document: &mut Document, cascade: &CascadeResult, origins: &[f32]) {
    let slices: Vec<_> = origins
        .iter()
        .enumerate()
        .map(|(index, &content_origin_y)| crate::PageSlice {
            page_index: index as u32,
            content_origin_y,
            page_name: None,
        })
        .collect();
    document
        .project_pages(cascade, raikiri_traits::PageBox::A4, &slices, &[])
        .unwrap();
}

/// The distinct lines of a page's runs, as (root, line index) in run order.
fn page_lines(document: &Document, cascade: &CascadeResult, page: u32) -> Vec<(usize, usize)> {
    let mut lines = Vec::new();
    for run in document.page_text_runs(cascade, page) {
        let line = (run.line.root.0 as usize, run.line.index);
        if lines.last() != Some(&line) {
            lines.push(line);
        }
    }
    lines
}

#[test]
fn each_page_draws_only_the_lines_in_its_flow_slice() {
    let (mut document, cascade, [first, second]) = tall_paragraphs();
    project_at(&mut document, &cascade, &[0.0, 100.0, 200.0, 300.0]);
    let lines_of = |root, range: std::ops::Range<usize>| range.map(move |index| (root, index));
    assert_eq!(
        page_lines(&document, &cascade, 0),
        lines_of(first, 0..10).collect::<Vec<_>>()
    );
    assert_eq!(
        page_lines(&document, &cascade, 1),
        lines_of(first, 10..20).collect::<Vec<_>>()
    );
    assert_eq!(
        page_lines(&document, &cascade, 2),
        lines_of(first, 20..25)
            .chain(lines_of(second, 0..3))
            .collect::<Vec<_>>()
    );
    assert!(page_lines(&document, &cascade, 3).is_empty());
    // Each paragraph is listed only for the pages its lines reach.
    let by_page = &document.page_projection.text_roots_by_page;
    let roots = &document.page_projection.text_roots;
    let listed = |page: usize| -> Vec<usize> {
        by_page[page]
            .iter()
            .map(|&index| roots[index].node)
            .collect()
    };
    assert_eq!(listed(0), [first]);
    assert_eq!(listed(1), [first]);
    assert_eq!(listed(2), [first, second]);
    assert!(listed(3).is_empty());
}

#[test]
fn pages_without_ordered_flow_slices_are_each_checked() {
    let (mut document, cascade, [first, second]) = tall_paragraphs();
    // A repeated origin gives the first page a slice that runs past the
    // second's, so the pages cannot be searched in order.
    project_at(&mut document, &cascade, &[0.0, 0.0, 250.0]);
    let first_lines = (0..25).map(|index| (first, index));
    let second_lines = (0..3).map(|index| (second, index));
    assert_eq!(
        page_lines(&document, &cascade, 0),
        first_lines
            .clone()
            .chain(second_lines.clone())
            .collect::<Vec<_>>()
    );
    assert_eq!(
        page_lines(&document, &cascade, 1),
        first_lines.collect::<Vec<_>>()
    );
    assert_eq!(
        page_lines(&document, &cascade, 2),
        second_lines.collect::<Vec<_>>()
    );
}
