use super::*;
use crate::Document;
use crate::layout::ifc::test_support::{ahem_fonts, block_fixture};
use crate::layout::test_support::with_ahem;

#[test]
fn debug_reports_fragment_box_placement_count() {
    let lines = IfcLines {
        width: 100.0,
        lines: std::sync::Arc::new(Vec::new()),
        height: 0.0,
        beside_floats: false,
        escaping_margin: taffy::CollapsibleMarginSet::ZERO,
        leading_block_margin: None,
        block_line_starts: Vec::new(),
        shifts: Default::default(),
        fragment_box_placements: vec![IfcBoxFragment {
            node_id: 1,
            fragmentainer: 0,
            rect: crate::fragment::FragmentRect {
                x: 0.0,
                y: 0.0,
                width: 10.0,
                height: 20.0,
            },
        }],
        fragmentainer_line_ranges: None,
        unfragmented_tail_column: None,
        placed_blocks: Vec::new(),
    };

    assert!(format!("{lines:?}").contains("fragment_box_placements: 1"));
}

#[test]
fn a_document_opts_in_explicitly() {
    let mut doc = Document::new();
    assert!(!doc.has_font_collection());
    doc.set_font_collection_with_limits(ahem_fonts(), Limits::default());
    assert!(doc.has_font_collection());
}

#[test]
fn a_cloned_document_keeps_its_fonts() {
    let mut doc = Document::new();
    doc.set_font_collection_with_limits(ahem_fonts(), Limits::default());
    assert!(doc.clone().has_font_collection());
}

#[test]
fn an_ifc_root_keeps_its_writing_mode_without_lines() {
    for (mode, expected) in [
        ("vertical-rl", shodo::geometry::WritingMode::VerticalRl),
        ("vertical-lr", shodo::geometry::WritingMode::VerticalLr),
    ] {
        let fixture = block_fixture(&format!("writing-mode:{mode}"), |doc, root| {
            doc.append_text(root, "x");
        });
        let projected = crate::layout::ifc::projection::project_ifc(
            &fixture.doc,
            &fixture.cascade,
            fixture.root,
            &mut LayoutContext::new(),
            &ahem_fonts(),
            &Limits::default(),
        )
        .expect("project");
        let root = IfcRoot::new(projected);
        assert_eq!(root.writing_mode, expected, "{mode}");
        assert_eq!(root.without_lines().writing_mode, expected, "{mode}");
    }
}

#[test]
fn with_state_puts_the_state_back() {
    let mut doc = Document::new();
    doc.set_font_collection_with_limits(ahem_fonts(), Limits::default());
    let seen = with_state(&mut doc, |state| state.limits.clone());
    assert!(seen.is_some());
    assert!(doc.has_font_collection(), "the state must be restored");
}

#[test]
fn with_state_does_nothing_without_fonts() {
    let mut doc = Document::new();
    assert!(with_state(&mut doc, |_| ()).is_none());
    assert!(!doc.has_font_collection());
}

#[test]
fn a_cloned_document_shares_the_lines_of_its_ifc_roots() {
    use crate::layout::test_support::{ahem_paragraph, ifc_ahem_fonts, page_box_800x600};
    let (mut doc, cascade, root) = ahem_paragraph("aaaa bbbb cccc", "width:40px");
    doc.set_font_collection_with_limits(ifc_ahem_fonts(), Limits::default());
    crate::layout::layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600())
        .expect("layout");
    let copy = doc.clone();
    let a = doc.nodes[root]
        .ifc
        .as_ref()
        .and_then(|r| r.lines.as_ref())
        .expect("lines");
    let b = copy.nodes[root]
        .ifc
        .as_ref()
        .and_then(|r| r.lines.as_ref())
        .expect("lines");
    assert!(std::sync::Arc::ptr_eq(&a.lines, &b.lines));
}

#[test]
fn a_rebreak_that_skips_a_root_keeps_the_same_lines() {
    use crate::layout::test_support::{
        ahem_paragraph_with_atomic, ifc_ahem_fonts, page_box_800x600,
    };
    // A root with a box of its own keeps the lines its box was placed with.
    let (mut doc, cascade, _atomic, root) =
        ahem_paragraph_with_atomic("aa ", "width:10px;height:10px", " bb", "");
    doc.set_font_collection_with_limits(ifc_ahem_fonts(), Limits::default());
    crate::layout::layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600())
        .expect("layout");
    assert!(doc.nodes[root].is_ifc_root());
    let before = doc.nodes[root]
        .ifc
        .as_ref()
        .and_then(|r| r.lines.as_ref())
        .map(|l| std::sync::Arc::clone(&l.lines))
        .expect("lines");
    crate::layout::relayout_text_for_width(with_ahem(&mut doc), &cascade, 30.0);
    let after = doc.nodes[root]
        .ifc
        .as_ref()
        .and_then(|r| r.lines.as_ref())
        .expect("lines");
    assert!(std::sync::Arc::ptr_eq(&before, &after.lines));
}

#[test]
fn a_rebreak_keeps_the_lines_of_a_min_content_root() {
    use crate::layout::test_support::{ahem_paragraph, ifc_ahem_fonts, page_box_800x600};
    let (mut doc, cascade, root) = ahem_paragraph("aaaa bbbb cccc", "width:min-content");
    doc.set_font_collection_with_limits(ifc_ahem_fonts(), Limits::default());
    crate::layout::layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600())
        .expect("layout");
    assert_eq!(doc.nodes[root].ifc_lines().expect("lines").len(), 3);
    crate::layout::relayout_text_for_width(with_ahem(&mut doc), &cascade, 800.0);
    // The min-content box stays 40px wide, so its text stays on 3 lines.
    assert_eq!(doc.nodes[root].ifc_lines().expect("lines").len(), 3);
}

#[test]
fn line_shifts_match_a_direct_sum_in_and_out_of_line_order() {
    fn direct(entries: &[(usize, f32)], index: usize) -> f32 {
        entries
            .iter()
            .filter(|(first, _)| *first <= index)
            .map(|(_, delta)| delta)
            .sum()
    }
    for entries in [
        vec![],
        vec![(2, 0.1), (2, 0.2), (5, 0.3), (9, 1.0e7)],
        vec![(5, 0.3), (2, 0.1), (9, 1.0e7), (2, 0.2)],
    ] {
        let mut shifts = LineShifts::default();
        for &(first, delta) in &entries {
            shifts.push(first, delta);
        }
        for index in 0..12 {
            assert_eq!(
                shifts.of(index).to_bits(),
                direct(&entries, index).to_bits(),
                "line {index} of {entries:?}"
            );
        }
        let all: f32 = entries.iter().map(|(_, delta)| delta).sum();
        assert_eq!(shifts.total().to_bits(), all.to_bits());
    }
}

/// An empty inline-block `::before` keeps its clamped size when the lines
/// are broken again, and its piece names the inline element it belongs to.
#[test]
fn an_empty_atomic_before_keeps_its_size_through_a_rebreak() {
    use crate::layout::ifc::test_support::{sheet_fixture, span};
    use crate::layout::test_support::page_box_800x600;
    let mut owner = 0;
    let fixture = sheet_fixture(
        "span::before{content:'';display:inline-block;width:30px;height:10px;max-width:20px;min-width:25px}",
        "line-height:10px",
        |doc, root| {
            doc.append_text(root, "aa ");
            owner = span(doc, root, "display:inline");
            doc.append_text(owner, "b");
        },
    );
    let (mut doc, cascade, root) = (fixture.doc, fixture.cascade, fixture.root);
    doc.set_font_collection_with_limits(ahem_fonts(), Limits::default());
    crate::layout::layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600())
        .expect("layout");
    crate::layout::relayout_text_for_width(with_ahem(&mut doc), &cascade, 800.0);
    let before =
        crate::generated_content::generated_node_id(owner, raikiri_style::PseudoElem::Before);
    let atomic = doc.nodes[root]
        .ifc_lines()
        .expect("lines")
        .iter()
        .flat_map(|line| line.fragments())
        .find_map(|fragment| match fragment {
            shodo::Fragment::Atomic(atomic) if atomic.node.0 as usize == before => Some(atomic),
            _ => None,
        })
        .expect("the pseudo-element is on a line");
    assert_eq!(atomic.border_rect.inline_size, 25.0);
    let piece = doc.nodes[root]
        .ifc_inline_boxes()
        .expect("pieces")
        .into_iter()
        .find(|piece| piece.node == before)
        .expect("the pseudo-element has a piece");
    assert_eq!(piece.parent, Some(owner));
}

/// A rebreak of a vertical paragraph keeps an empty inline-block `::before`
/// on its lines, its physical height along the inline axis and its
/// percentage width resolved against the paragraph's physical width.
#[test]
fn an_empty_atomic_before_keeps_its_size_through_a_vertical_rebreak() {
    use crate::layout::ifc::test_support::sheet_fixture;
    use crate::layout::test_support::page_box_800x600;
    let fixture = sheet_fixture(
        "div::before{content:'';display:inline-block;width:50%;height:15px}",
        "line-height:10px;writing-mode:vertical-rl;width:200px;height:200px",
        |doc, root| {
            doc.append_text(root, "aa");
        },
    );
    let (mut doc, cascade, root) = (fixture.doc, fixture.cascade, fixture.root);
    doc.set_font_collection_with_limits(ahem_fonts(), Limits::default());
    crate::layout::layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600())
        .expect("layout");
    crate::layout::relayout_text_for_width(with_ahem(&mut doc), &cascade, 800.0);
    let before =
        crate::generated_content::generated_node_id(root, raikiri_style::PseudoElem::Before);
    let atomic = doc.nodes[root]
        .ifc_lines()
        .expect("lines")
        .iter()
        .flat_map(|line| line.fragments())
        .find_map(|fragment| match fragment {
            shodo::Fragment::Atomic(atomic) if atomic.node.0 as usize == before => Some(atomic),
            _ => None,
        })
        .expect("the pseudo-element is on a line");
    assert_eq!(atomic.border_rect.inline_size, 15.0);
    // The percentage width refers to the paragraph's physical width.
    assert_eq!(atomic.border_rect.block_size, 100.0);
}

/// The percentage width of an empty inline-block `::before` in a vertical
/// paragraph sized by its lines is the same after its lines are broken again.
#[test]
fn an_empty_atomic_before_in_an_auto_width_vertical_root_keeps_its_width_through_a_rebreak() {
    use crate::layout::ifc::test_support::sheet_fixture;
    use crate::layout::test_support::page_box_800x600;
    let fixture = sheet_fixture(
        "div::before{content:'';display:inline-block;width:50%;height:15px}",
        "line-height:10px;writing-mode:vertical-rl;height:200px",
        |doc, root| {
            doc.append_text(root, "aa");
        },
    );
    let (mut doc, cascade, root) = (fixture.doc, fixture.cascade, fixture.root);
    doc.set_font_collection_with_limits(ahem_fonts(), Limits::default());
    crate::layout::layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600())
        .expect("layout");
    let before =
        crate::generated_content::generated_node_id(root, raikiri_style::PseudoElem::Before);
    let block_size = |doc: &Document| {
        doc.nodes[root]
            .ifc_lines()
            .expect("lines")
            .iter()
            .flat_map(|line| line.fragments())
            .find_map(|fragment| match fragment {
                shodo::Fragment::Atomic(atomic) if atomic.node.0 as usize == before => {
                    Some(atomic.border_rect.block_size)
                }
                _ => None,
            })
            .expect("the pseudo-element is on a line")
    };
    let laid_out = block_size(&doc);
    crate::layout::relayout_text_for_width(with_ahem(&mut doc), &cascade, 800.0);
    assert_eq!(block_size(&doc), laid_out);
}
