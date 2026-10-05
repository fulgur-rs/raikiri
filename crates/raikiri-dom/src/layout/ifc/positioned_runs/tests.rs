use super::*;
use crate::layout::ifc::test_support::block_fixture;
use crate::layout::layout_single_page;
use crate::layout::test_support::{page_box_800x600, with_ahem};
use crate::node::MulticolTextFragment;

/// A 50px wide paragraph of Ahem text on two lines: "aaaa" and "bbbb".
fn two_lines() -> (Document, CascadeResult, usize) {
    let fixture = block_fixture("width:50px;line-height:10px", |doc, root| {
        doc.append_text(root, "aaaa bbbb");
    });
    let (mut doc, cascade, root) = (fixture.doc, fixture.cascade, fixture.root);
    layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600()).expect("layout");
    (doc, cascade, root)
}

#[test]
fn glyphs_sit_on_each_line_baseline_from_the_content_box() {
    let (doc, cascade, root) = two_lines();
    let lines = PositionedLines::new(&doc, &cascade, root, None).expect("lines");
    let lines: Vec<_> = lines.lines().collect();
    assert_eq!(lines.len(), 2);
    let first = &lines[0].runs[0];
    assert_eq!(
        first.glyphs.iter().map(|g| (g.x, g.y)).collect::<Vec<_>>()[..4],
        [(0.0, 8.0), (10.0, 8.0), (20.0, 8.0), (30.0, 8.0)]
    );
    assert_eq!(lines[1].runs[0].glyphs[0].y, 18.0);
    assert_eq!(first.offset, (0.0, 0.0));
}

#[test]
fn runs_without_computed_values_are_left_out() {
    let (doc, _, root) = two_lines();
    // The cascade of another, empty document has no values for the text.
    let empty = Document::new();
    let rules = raikiri_style::build_rule_tree(&empty);
    let other = raikiri_style::cascade(&empty, &rules).expect("cascade");
    let lines = PositionedLines::new(&doc, &other, root, None).expect("lines");
    assert!(lines.lines().all(|line| line.runs.is_empty()));
}

#[test]
fn column_fragments_move_their_lines_and_drop_the_others() {
    let (mut doc, cascade, root) = two_lines();
    let ifc = doc.nodes[root].ifc.as_mut().expect("ifc root");
    ifc.multicol_fragments = Some(vec![
        // A range past the last line is ignored.
        MulticolTextFragment {
            line_start: 9,
            line_end: 10,
            fragmentainer: 0,
            x: 0.0,
            y: 0.0,
        },
        MulticolTextFragment {
            line_start: 1,
            line_end: 2,
            fragmentainer: 1,
            x: 30.0,
            y: 0.0,
        },
    ]);
    // The second line is drawn at the top of its column; the first line has
    // no column and is not drawn.
    let lines = PositionedLines::new(&doc, &cascade, root, None).expect("lines");
    let offsets: Vec<_> = lines
        .lines()
        .map(|line| (line.index, line.offset))
        .collect();
    assert_eq!(offsets, [(1, (30.0, -10.0))]);
    // Asked for one column, its lines start at the column's inline start.
    let lines = PositionedLines::new(&doc, &cascade, root, Some(1)).expect("lines");
    let offsets: Vec<_> = lines
        .lines()
        .map(|line| (line.index, line.offset))
        .collect();
    assert_eq!(offsets, [(1, (0.0, -10.0))]);
    let lines = PositionedLines::new(&doc, &cascade, root, Some(0)).expect("lines");
    assert_eq!(lines.lines().count(), 0);
}
