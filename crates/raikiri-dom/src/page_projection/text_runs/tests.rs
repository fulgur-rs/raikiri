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
        width: 0.0,
        lines_along_x: false,
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

/// A flex container of Ahem items, each given as its style and text, with
/// 7px of padding above them.
fn flex_items(items: &[&str]) -> (Document, CascadeResult, usize) {
    container_items(
        "display:flex;align-items:baseline;padding-top:7px;font:10px Ahem",
        items,
    )
}

/// A grid container of Ahem items with 20px lines, each given as its style,
/// with 7px of padding above them. An item styled with `empty` has no text.
fn grid_items(container: &str, items: &[&str]) -> (Document, CascadeResult, usize) {
    container_items(
        &format!("display:grid;padding-top:7px;font:10px/20px Ahem;{container}"),
        items,
    )
}

/// A container of the given style holding one `p` per item style, each
/// with an X unless its style mentions `empty`.
fn container_items(container: &str, items: &[&str]) -> (Document, CascadeResult, usize) {
    let mut document = Document::new();
    let body = document.append_element(
        Some(0),
        "body",
        taffy::Style::default(),
        Some("display:block;margin:0"),
    );
    let container_node =
        document.append_element(Some(body), "div", taffy::Style::default(), Some(container));
    for style in items {
        let item = document.append_element(
            Some(container_node),
            "p",
            taffy::Style::default(),
            Some(&format!(
                "display:block;margin:0;{}",
                style.replace("empty", "")
            )),
        );
        if !style.contains("empty") {
            document.append_text(item, "X".to_string());
        }
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
    (document, cascade, container_node)
}

/// A flex container's first line is the one its layout took the first
/// baseline from; the middle of that line decides its page, not the baseline.
#[test]
fn a_flex_first_line_keeps_the_center_of_its_item_line() {
    let (document, cascade, flex) = flex_items(&["line-height:50px"]);
    let line = first_line(&document, &cascade, flex).unwrap();
    // Ahem's ascent is 0.8em, and the 40px of leading is split evenly.
    assert_eq!(line.baseline, 7.0 + 20.0 + 8.0);
    assert_eq!(line.center, 7.0 + 25.0);
}

/// Items that share the baseline are searched in layout order, so the line
/// is the one of the item `order` places first.
#[test]
fn a_flex_first_line_follows_the_order_of_the_items() {
    let (document, cascade, flex) = flex_items(&[
        "line-height:50px",
        "order:-1;font-size:20px;line-height:20px",
    ]);
    let line = first_line(&document, &cascade, flex).unwrap();
    // The 20px item's baseline, 16px down, aligns with the other's at 28px.
    assert_eq!(line.baseline, 7.0 + 28.0);
    assert_eq!(line.center, 7.0 + 12.0 + 10.0);
}

/// A relative offset moves the item's box, not the line the flex layout
/// aligned, so the item still supplies the line's middle.
#[test]
fn a_flex_first_line_ignores_the_relative_offset_of_its_item() {
    let (document, cascade, flex) = flex_items(&["line-height:50px;position:relative;top:9px"]);
    let line = first_line(&document, &cascade, flex).unwrap();
    assert_eq!(line.baseline, 7.0 + 20.0 + 8.0);
    assert_eq!(line.center, 7.0 + 25.0);
}

/// A percentage offset resolves against the container's definite height,
/// as layout resolves it, so taking it back out leaves the line in place.
#[test]
fn a_first_line_ignores_a_percentage_relative_offset() {
    let (document, cascade, flex) = container_items(
        "display:flex;align-items:baseline;height:100px;padding-top:7px;font:10px Ahem",
        &["line-height:50px;position:relative;top:calc(10% - 1px)"],
    );
    let line = first_line(&document, &cascade, flex).unwrap();
    assert_eq!(line.baseline, 7.0 + 20.0 + 8.0);
    assert_eq!(line.center, 7.0 + 25.0);
    // Hand-computed: the 20px line of the X starts 7px down, with its
    // baseline 5px of half-leading plus 8px of ascent below that, whatever
    // the 50px offset that 50% of the 100px container gives the block.
    let (document, cascade, block) = container_items(
        "display:block;height:100px;padding-top:7px;font:10px/20px Ahem",
        &["position:relative;top:50%"],
    );
    let line = first_line(&document, &cascade, block).unwrap();
    assert_eq!(line.baseline, 7.0 + 5.0 + 8.0);
    assert_eq!(line.center, 7.0 + 10.0);
}

/// Manual column placement leaves the block offset of a relative box to the
/// painter, which resolves a percentage against the multicol container.
#[test]
fn a_column_box_shift_resolves_a_percentage_relative_offset() {
    let (document, cascade, multicol) = container_items(
        "display:block;column-count:2;width:200px;height:60px;font:10px/20px Ahem",
        &["min-block-size:40px;break-inside:avoid;position:relative;left:10%;top:calc(10% + 2px)"],
    );
    let item = document.nodes[multicol].children[0];
    assert!(document.nodes[item].needs_relative_block_paint_offset());
    let shift = box_paint_shift(
        &document,
        &cascade,
        None,
        item,
        &cascade.computed[item],
        false,
    )
    .unwrap();
    // Hand-computed: 10% of 60px plus 2px; the horizontal offset is
    // already part of the layout location.
    assert_eq!(shift.offset, (0.0, 8.0));
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

/// `float` does not apply to a flex item, which still gives the line.
#[test]
fn a_flex_first_line_includes_a_floated_item() {
    let (document, cascade, flex) = flex_items(&["line-height:50px;float:left"]);
    let line = first_line(&document, &cascade, flex).unwrap();
    assert_eq!(line.baseline, 7.0 + 20.0 + 8.0);
    assert_eq!(line.center, 7.0 + 25.0);
}

/// A grid container takes its first line from the first item of its first
/// row, measured from that item's own line rather than its box.
#[test]
fn a_grid_first_line_is_the_line_of_its_first_row_item() {
    let (document, cascade, grid) = grid_items("", &["padding-top:20px", ""]);
    let line = first_line(&document, &cascade, grid).unwrap();
    // Ahem's ascent is 0.8em, and the 10px of leading is split evenly.
    assert_eq!(line.baseline, 7.0 + 20.0 + 5.0 + 8.0);
    assert_eq!(line.center, 7.0 + 20.0 + 10.0);
}

/// A first-row item that takes part in baseline alignment gives the line
/// before an earlier item that does not.
#[test]
fn a_grid_first_line_prefers_a_baseline_aligned_item() {
    let (document, cascade, grid) = grid_items(
        "grid-template-columns:50px 50px",
        &["padding-top:20px", "align-self:baseline"],
    );
    let line = first_line(&document, &cascade, grid).unwrap();
    assert_eq!(line.baseline, 7.0 + 5.0 + 8.0);
    assert_eq!(line.center, 7.0 + 10.0);
}

/// A percentage height in an intrinsically or flexibly sized row depends on
/// the row, so that item takes no part in baseline alignment and the next
/// baseline-aligned item gives the line, as in layout.
#[test]
fn a_grid_first_line_skips_an_item_with_a_cyclic_percentage_height() {
    for rows in [
        "",
        "grid-template-rows:auto",
        "grid-template-rows:1fr",
        "grid-template-rows:minmax(10px,max-content)",
        "grid-auto-rows:min-content",
        "grid-template-rows:repeat(2,fit-content(40px))",
    ] {
        let (document, cascade, grid) = grid_items(
            &format!("grid-template-columns:50px 50px;{rows}"),
            &[
                "align-self:baseline;height:50%;padding-top:20px",
                "align-self:baseline",
            ],
        );
        let line = first_line(&document, &cascade, grid).unwrap();
        assert_eq!(line.baseline, 7.0 + 5.0 + 8.0, "{rows}");
        assert_eq!(line.center, 7.0 + 10.0, "{rows}");
    }
}

/// In a row of fixed size the same item keeps its part in baseline
/// alignment; spanning into an intrinsic row takes it away again.
#[test]
fn a_grid_first_line_keeps_a_percentage_height_item_in_a_fixed_row() {
    for rows in [
        "grid-template-rows:100px",
        "grid-auto-rows:100px",
        "grid-template-rows:repeat(auto-fill,100px)",
        "grid-template-rows:50%",
    ] {
        let (document, cascade, grid) = grid_items(
            &format!("grid-template-columns:50px 50px;height:200px;{rows}"),
            &[
                "align-self:baseline;height:50%;padding-top:20px",
                "align-self:baseline",
            ],
        );
        let line = first_line(&document, &cascade, grid).unwrap();
        assert_eq!(line.baseline, 7.0 + 20.0 + 5.0 + 8.0, "{rows}");
    }
    let (document, cascade, grid) = grid_items(
        "grid-template-columns:50px 50px;grid-template-rows:100px",
        &[
            "align-self:baseline;height:50%;padding-top:20px;grid-row:1/span 2",
            "align-self:baseline",
        ],
    );
    let line = first_line(&document, &cascade, grid).unwrap();
    assert_eq!(line.baseline, 7.0 + 5.0 + 8.0);
}

/// Items are searched in order-modified grid order.
#[test]
fn a_grid_first_line_follows_the_order_of_the_items() {
    let (document, cascade, grid) = grid_items(
        "grid-template-columns:50px 50px",
        &["padding-top:20px", "order:-1;line-height:40px"],
    );
    let line = first_line(&document, &cascade, grid).unwrap();
    assert_eq!(line.baseline, 7.0 + 15.0 + 8.0);
    assert_eq!(line.center, 7.0 + 20.0);
}

/// An item with no line gives the bottom of its border box, and a relative
/// offset does not move it.
#[test]
fn a_grid_first_line_synthesizes_the_baseline_of_an_empty_item() {
    let (document, cascade, grid) =
        grid_items("", &["empty;height:30px;position:relative;top:9px", ""]);
    let line = first_line(&document, &cascade, grid).unwrap();
    assert_eq!(line.baseline, 7.0 + 30.0);
    assert_eq!(line.center, 7.0 + 30.0);
}

/// Items that only start in a later row do not give the line, even when
/// they come first in the source.
#[test]
fn a_grid_first_line_skips_items_placed_in_later_rows() {
    let (document, cascade, grid) = grid_items(
        "grid-template-columns:50px",
        &["grid-row:2;padding-top:20px", "grid-row:1;line-height:40px"],
    );
    let line = first_line(&document, &cascade, grid).unwrap();
    assert_eq!(line.baseline, 7.0 + 15.0 + 8.0);
    assert_eq!(line.center, 7.0 + 20.0);
}

/// Grid order runs along the first row by column, so an item placed in the
/// first column gives the line before one earlier in the source.
#[test]
fn a_grid_first_line_follows_the_columns_of_the_first_row() {
    let (document, cascade, grid) = grid_items(
        "grid-template-columns:50px 50px",
        &[
            "grid-row:1;grid-column:2;padding-top:20px",
            "grid-row:1;grid-column:1;line-height:40px",
        ],
    );
    let line = first_line(&document, &cascade, grid).unwrap();
    assert_eq!(line.baseline, 7.0 + 15.0 + 8.0);
    assert_eq!(line.center, 7.0 + 20.0);
}

/// The line of a scrolling item that lies below its box is clamped to the
/// bottom of the box.
#[test]
fn a_grid_first_line_clamps_the_baseline_of_a_scrolling_item() {
    let (document, cascade, grid) = grid_items("", &["overflow:auto;height:10px;padding-top:20px"]);
    let line = first_line(&document, &cascade, grid).unwrap();
    assert_eq!(line.baseline, 7.0 + 30.0);
    assert_eq!(line.center, 7.0 + 30.0);
}

/// Text directly in a grid is an anonymous item that gives its own line.
#[test]
fn a_grid_first_line_includes_text_directly_in_the_grid() {
    let mut document = Document::new();
    let body = document.append_element(
        Some(0),
        "body",
        taffy::Style::default(),
        Some("display:block;margin:0"),
    );
    let grid = document.append_element(
        Some(body),
        "div",
        taffy::Style::default(),
        Some("display:grid;padding-top:7px;font:10px/20px Ahem"),
    );
    document.append_text(grid, "X".to_string());
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
    let line = first_line(&document, &cascade, grid).unwrap();
    assert_eq!(line.baseline, 7.0 + 5.0 + 8.0);
    assert_eq!(line.center, 7.0 + 10.0);
}

/// A layout-contained item exports no baseline, so the bottom of its border
/// box stands in for its line.
#[test]
fn a_grid_first_line_synthesizes_the_baseline_of_a_layout_contained_item() {
    let (document, cascade, grid) = grid_items("", &["contain:layout;height:30px"]);
    let line = first_line(&document, &cascade, grid).unwrap();
    assert_eq!(line.baseline, 7.0 + 30.0);
    assert_eq!(line.center, 7.0 + 30.0);
}

/// Items inside a `display: contents` child of the grid are grid items of
/// their own.
#[test]
fn a_grid_first_line_finds_items_inside_display_contents() {
    let (document, cascade, grid) = grid_items("", &["display:contents", ""]);
    let line = first_line(&document, &cascade, grid).unwrap();
    // The X inside the contents box is an anonymous item on the first row.
    assert_eq!(line.baseline, 7.0 + 5.0 + 8.0);
    assert_eq!(line.center, 7.0 + 10.0);
}

/// The painter moves an element by its `transform` exactly when layout
/// treats it as transformable (CSS Transforms 1 §1): a non-replaced inline
/// box is not, but a form control, a MathML root and an inline that layout
/// blockifies are.
#[test]
fn box_paint_shift_translates_the_boxes_layout_finds_transformable() {
    let mut document = Document::new();
    let body = document.append_element(
        Some(0),
        "body",
        taffy::Style::default(),
        Some("display:block;margin:0"),
    );
    let paragraph = document.append_element(
        Some(body),
        "p",
        taffy::Style::default(),
        Some("display:block;margin:0"),
    );
    let translated = "display:inline;transform:translateX(10px)";
    let span = document.append_element(
        Some(paragraph),
        "span",
        taffy::Style::default(),
        Some(translated),
    );
    document.append_text(span, "X".to_string());
    let button = document.append_element(
        Some(paragraph),
        "button",
        taffy::Style::default(),
        Some(translated),
    );
    let math = document.append_element(
        Some(paragraph),
        "math",
        taffy::Style::default(),
        Some(translated),
    );
    let floated = document.append_element(
        Some(paragraph),
        "span",
        taffy::Style::default(),
        Some(&format!("{translated};float:left")),
    );
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
    let offset = |id: usize| {
        box_paint_shift(&document, &cascade, None, id, &cascade.computed[id], false)
            .unwrap()
            .offset
    };
    assert_eq!(offset(span), (0.0, 0.0));
    assert_eq!(offset(button), (10.0, 0.0));
    assert_eq!(offset(math), (10.0, 0.0));
    assert_eq!(offset(floated), (10.0, 0.0));
}
