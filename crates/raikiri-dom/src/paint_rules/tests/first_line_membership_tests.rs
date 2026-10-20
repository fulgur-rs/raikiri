use super::*;
use crate::layout::ifc::test_support::{Fixture, sheet_fixture, span};
use crate::layout::test_support::with_ahem;
use raikiri_traits::PageBox;

fn positioned_paragraph(
    count: usize,
    depth: usize,
) -> (Document, CascadeResult, usize, Vec<usize>) {
    let mut ids = Vec::new();
    let Fixture {
        mut doc,
        cascade,
        root,
    } = sheet_fixture(
        "div::first-line { opacity:.5 }",
        "width:80px;line-height:10px",
        |doc, root| {
            for index in 0..count {
                let level = if index % 2 == 0 { -1 } else { 1 };
                let id = span(
                    doc,
                    root,
                    &format!("display:inline;position:relative;z-index:{level}"),
                );
                ids.push(id);
                let mut parent = id;
                for _ in 1..depth {
                    parent = span(doc, parent, "display:inline");
                }
                doc.append_text(parent, "aa");
                doc.append_text(root, " ");
            }
        },
    );
    with_ahem(&mut doc);
    let pages = crate::layout::layout_pages(&mut doc, &cascade, PageBox::A4).unwrap();
    doc.project_pages(&cascade, PageBox::A4, &pages, &[])
        .unwrap();
    assert!(first_line_opacity_layer(&doc, root).is_some());
    assert!(doc.get_node(root).unwrap().ifc_lines().unwrap().len() > 1);
    (doc, cascade, root, ids)
}

#[test]
fn first_line_membership_is_shared_for_positioned_inline_queries() {
    let mut observations = Vec::new();
    for (count, depth) in [(16, 1), (64, 1), (256, 1), (64, 4), (256, 4)] {
        let (doc, cascade, root, ids) = positioned_paragraph(count, depth);
        FIRST_LINE_WORK.with(|work| work.set([0; 4]));
        let expected = ids
            .iter()
            .map(|&id| !uncached_within(&doc, root, id))
            .collect::<Vec<_>>();
        let start = std::time::Instant::now();
        let classifications = ids
            .iter()
            .enumerate()
            .map(|(index, &id)| {
                if index % 2 == 0 {
                    negative_inline_group(&doc, &cascade, root, id)
                } else {
                    inline_stacking_group(&doc, &cascade, root, id)
                }
            })
            .collect::<Vec<_>>();
        let elapsed = start.elapsed().as_micros();
        assert_eq!(classifications, expected);
        assert!(classifications.iter().any(|&grouped| grouped));
        assert!(classifications.iter().any(|&grouped| !grouped));
        let runs = doc.page_text_runs(&cascade, 0);
        let order = format!(
            "{:?}",
            doc.page_paint_order_for_text_runs(&cascade, 0, &runs)
        );
        assert_eq!(
            order,
            format!(
                "{:?}",
                doc.page_paint_order_for_text_runs(&cascade, 0, &runs)
            )
        );
        for run in &runs {
            doc.text_run_opacity_group(&cascade, run);
        }
        let work = FIRST_LINE_WORK.with(|work| work.get());
        let node = doc.get_node(root).unwrap();
        let fragments = node
            .ifc_lines()
            .unwrap()
            .iter()
            .skip(1)
            .flat_map(|line| line.fragments())
            .count();
        let pieces = node.ifc_generated_block_pieces().len();
        assert_eq!(work[1], fragments);
        assert_eq!(work[3], pieces);
        // Each source can revisit one ancestor after the unique ancestors are inserted.
        assert!(work[2] <= doc.nodes.len() + fragments + pieces);
        eprintln!(
            "N={count} depth={depth}: builds={} fragments={} ancestors={} pieces={} query-us={elapsed}",
            work[0], work[1], work[2], work[3]
        );
        observations.push((count, depth, work));
    }
    assert!(
        observations.iter().all(|&(_, _, work)| work[0] == 1),
        "{observations:?}"
    );
}

fn uncached_within(document: &Document, root: usize, id: usize) -> bool {
    if first_line_opacity_layer(document, root).is_none() {
        return false;
    }
    let node = document.get_node(root).unwrap();
    let inside = |source| {
        source == id
            || document
                .inclusive_ancestors(
                    crate::generated_content::generated_origin(source)
                        .map_or(source, |(origin, _)| origin),
                )
                .any(|ancestor| ancestor == id)
    };
    !node
        .ifc_lines()
        .into_iter()
        .flatten()
        .skip(1)
        .flat_map(|line| line.fragments())
        .filter_map(fragment_node)
        .any(inside)
        && !node
            .ifc_generated_block_pieces()
            .iter()
            .any(|piece| piece.line > 0 && inside(piece.node))
}

fn laid_out(
    sheet: &str,
    css: &str,
    build: impl FnOnce(&mut Document, usize),
) -> (Document, CascadeResult, usize) {
    let Fixture {
        mut doc,
        cascade,
        root,
    } = sheet_fixture(sheet, css, build);
    crate::layout::layout_single_page(with_ahem(&mut doc), &cascade, PageBox::A4).unwrap();
    (doc, cascade, root)
}

#[test]
fn membership_changes_after_reparenting_a_later_line_descendant() {
    let mut old = 0;
    let mut new = 0;
    let (mut doc, _, root) = laid_out(
        "div::first-line { opacity:.5 }",
        "width:50px;line-height:10px",
        |doc, root| {
            doc.append_text(root, "aaaa ");
            old = span(doc, root, "display:contents");
            let child = span(doc, old, "display:inline");
            doc.append_text(child, "bbbb");
            new = span(doc, root, "display:contents");
        },
    );
    assert!(!within_translucent_first_line(&doc, root, old));
    assert!(within_translucent_first_line(&doc, root, new));
    doc.reparent_children(old, new);
    assert!(
        doc.page_projection
            .later_line_membership
            .borrow()
            .is_empty()
    );
    assert!(within_translucent_first_line(&doc, root, old));
    assert!(!within_translucent_first_line(&doc, root, new));
    for id in [old, new] {
        assert_eq!(
            within_translucent_first_line(&doc, root, id),
            uncached_within(&doc, root, id)
        );
    }
}

#[test]
fn membership_changes_after_narrow_and_wide_rebreaks() {
    let mut id = 0;
    let (mut doc, cascade, root) = laid_out(
        "div::first-line { opacity:.5 }",
        "line-height:10px",
        |doc, root| {
            id = span(doc, root, "display:inline");
            doc.append_text(id, "aa bb cc dd");
        },
    );
    assert!(within_translucent_first_line(&doc, root, id));
    crate::layout::relayout_text_for_width(&mut doc, &cascade, 30.0);
    assert!(!within_translucent_first_line(&doc, root, id));
    crate::layout::relayout_text_for_width(&mut doc, &cascade, 200.0);
    assert!(within_translucent_first_line(&doc, root, id));
}

#[test]
fn empty_generated_pieces_keep_before_and_after_membership_distinct() {
    let sheet = r#"div::first-line { opacity:.5 }
        div::before { content:""; display:block; height:10px }
        div::after { content:""; display:block; height:10px }"#;
    let (mut doc, _, root) = laid_out(sheet, "width:30px;line-height:10px", |doc, root| {
        doc.append_text(root, "aa bb cc dd");
    });
    let before = crate::generated_content::generated_node_id(root, PseudoElem::Before);
    let after = crate::generated_content::generated_node_id(root, PseudoElem::After);
    assert!(
        doc.get_node(root)
            .unwrap()
            .ifc_generated_block_pieces()
            .iter()
            .any(|piece| piece.node == after && piece.line > 0)
    );
    assert!(within_translucent_first_line(&doc, root, before));
    assert!(!within_translucent_first_line(&doc, root, after));
    let style = (0..doc.nodes.len())
        .find(|&id| doc.nodes[id].tag_name() == Some("style"))
        .unwrap();
    doc.set_element_text_content(style, format!("{sheet} div::after {{ content:none }}"))
        .unwrap();
    doc.mark_in_document_flags();
    let cascade = cascaded(&doc);
    crate::layout::layout_single_page(&mut doc, &cascade, PageBox::A4).unwrap();
    assert!(
        doc.get_node(root)
            .unwrap()
            .ifc_generated_block_pieces()
            .iter()
            .all(|piece| piece.node != after)
    );
    assert!(within_translucent_first_line(&doc, root, after));
}

#[test]
fn opaque_first_line_builds_no_membership_index() {
    let (doc, _, root) = laid_out("", "width:30px", |doc, root| {
        doc.append_text(root, "aa bb cc");
    });
    FIRST_LINE_WORK.with(|work| work.set([0; 4]));
    assert!(!within_translucent_first_line(&doc, root, root));
    assert_eq!(FIRST_LINE_WORK.with(|work| work.get()), [0; 4]);
}

#[test]
fn projection_rebuild_drops_later_line_membership() {
    let Fixture {
        mut doc,
        cascade,
        root,
    } = sheet_fixture(
        "div::first-line { opacity:.5 }",
        "width:30px;line-height:10px",
        |doc, root| {
            doc.append_text(root, "aa bb cc");
        },
    );
    with_ahem(&mut doc);
    let pages = crate::layout::layout_pages(&mut doc, &cascade, PageBox::A4).unwrap();
    doc.project_pages(&cascade, PageBox::A4, &pages, &[])
        .unwrap();
    FIRST_LINE_WORK.with(|work| work.set([0; 4]));
    assert!(!within_translucent_first_line(&doc, root, root));
    assert_eq!(FIRST_LINE_WORK.with(|work| work.get()[0]), 1);
    doc.project_pages(&cascade, PageBox::A4, &pages, &[])
        .unwrap();
    assert!(
        doc.page_projection
            .later_line_membership
            .borrow()
            .is_empty()
    );
    assert!(!within_translucent_first_line(&doc, root, root));
    assert_eq!(FIRST_LINE_WORK.with(|work| work.get()[0]), 2);
}
