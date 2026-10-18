use super::*;
use crate::layout::ifc::test_support::{Fixture, sheet_fixture, span};
use crate::layout::test_support::with_ahem;
use crate::node::MulticolTextFragment;
use column_projection::{LineColumns, charge_column_ranges, split_inline_elements};

/// Lines `start..end` in `column`, which starts `y` down the paragraph.
fn range(start: usize, end: usize, column: usize, y: f32) -> MulticolTextFragment {
    MulticolTextFragment {
        line_start: start,
        line_end: end,
        fragmentainer: column,
        x: 0.0,
        y,
    }
}

#[test]
fn a_paragraph_that_is_not_in_columns_has_all_its_lines_in_the_fallback_column() {
    let columns = LineColumns::new(None, 3, 7, true);
    assert_eq!(columns.column, [7, 7, 7]);
    assert_eq!(columns.origin, [0.0, 0.0, 0.0]);
}

#[test]
fn each_range_of_lines_is_in_its_column_which_starts_where_the_layout_recorded() {
    let ranges = [range(0, 2, 1, 0.0), range(2, 5, 2, 30.0)];
    let columns = LineColumns::new(Some(&ranges), 6, 9, true);
    // The line no range holds is in the fallback column.
    assert_eq!(columns.column, [1, 1, 2, 2, 2, 9]);
    assert_eq!(columns.origin, [0.0, 0.0, 30.0, 30.0, 30.0, 0.0]);
}

#[test]
fn columns_start_at_zero_unless_their_origins_were_recorded() {
    let ranges = [range(0, 1, 1, 12.0), range(1, 2, 2, 34.0)];
    let columns = LineColumns::new(Some(&ranges), 2, 0, false);
    assert_eq!(columns.column, [1, 2]);
    assert_eq!(columns.origin, [0.0, 0.0]);
}

#[test]
fn a_line_that_several_ranges_hold_is_in_the_first_of_them() {
    let ranges = [range(0, 2, 1, 5.0), range(1, 3, 2, 8.0)];
    let columns = LineColumns::new(Some(&ranges), 3, 0, true);
    assert_eq!(columns.column, [1, 1, 2]);
    assert_eq!(columns.origin, [5.0, 5.0, 8.0]);
}

#[test]
fn ranges_beyond_the_last_line_hold_only_the_lines_there_are() {
    let ranges = [range(1, 100, 3, 4.0), range(50, 60, 4, 6.0)];
    let columns = LineColumns::new(Some(&ranges), 3, 0, true);
    assert_eq!(columns.column, [0, 3, 3]);
    assert_eq!(columns.origin, [0.0, 4.0, 4.0]);
}

/// The work left of `limit` after charging `ranges` over `count` lines, or
/// the limit that was exceeded.
fn charged(
    ranges: Option<&[MulticolTextFragment]>,
    count: usize,
    limit: usize,
) -> Result<usize, usize> {
    let control = PageLayoutControl::default();
    let mut work = ProjectionWork::new(&control, limit);
    match charge_column_ranges(ranges, count, &mut work) {
        Ok(()) => Ok(work.remaining()),
        Err(LayoutError::FragmentLimitExceeded { limit }) => Err(limit),
        Err(error) => panic!("unexpected error: {error:?}"),
    }
}

#[test]
fn a_paragraph_that_is_not_in_columns_costs_nothing_to_prepare() {
    assert_eq!(charged(None, 1000, 0), Ok(0));
}

#[test]
fn ranges_cost_the_lines_of_the_paragraph_and_the_lines_of_each_range() {
    // 4 lines, then 2 and 3 lines of the ranges, one of them beyond the end.
    let ranges = [range(0, 2, 0, 0.0), range(1, 40, 1, 0.0)];
    assert_eq!(charged(Some(&ranges), 4, 100), Ok(100 - (4 + 2 + 3)));
    assert_eq!(charged(Some(&ranges), 4, 9), Ok(0));
    assert_eq!(charged(Some(&ranges), 4, 8), Err(8));
}

#[test]
fn ranges_that_overlap_are_charged_for_each_line_again() {
    let ranges = vec![range(0, 3, 0, 0.0); 10];
    assert_eq!(charged(Some(&ranges), 3, 33), Ok(0));
    assert_eq!(charged(Some(&ranges), 3, 32), Err(32));
}

/// The inline elements of the paragraph of `build`, under `sheet`, that are
/// listed by their line pieces; the elements built are returned beside them.
fn split_elements(
    sheet: &str,
    root_css: &str,
    build: impl FnOnce(&mut Document, usize) -> Vec<usize>,
) -> (Vec<usize>, Vec<usize>) {
    let mut built = Vec::new();
    let Fixture {
        mut doc,
        cascade,
        root,
    } = sheet_fixture(sheet, root_css, |doc, root| built = build(doc, root));
    layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600()).expect("layout");
    let node = &doc.nodes[root];
    let pieces = node.ifc_inline_boxes().expect("pieces");
    let mut split: Vec<_> = split_inline_elements(&doc, &cascade, root, node, &pieces)
        .into_iter()
        .collect();
    split.sort_unstable();
    (split, built)
}

/// A paragraph of two elements, each on its own line: `first` with the text
/// of the first line, `second` with that of the second. The space between them
/// is the break opportunity that puts them on separate lines.
fn two_lines(doc: &mut Document, root: usize) -> Vec<usize> {
    let first = span(doc, root, "display:inline");
    doc.append_text(first, "aaaa");
    doc.append_text(root, " ");
    let second = span(doc, root, "display:inline");
    doc.append_text(second, "bbbb");
    vec![first, second]
}

#[test]
fn without_a_first_line_no_element_is_listed_by_its_pieces() {
    let (split, _) = split_elements("", "width:50px", two_lines);
    assert!(split.is_empty());
}

#[test]
fn an_element_on_the_first_line_that_the_first_line_recolors_is_listed_by_its_pieces() {
    let (split, built) = split_elements("div::first-line{color:red}", "width:50px", two_lines);
    // Only the element on the first line has another color there.
    assert_eq!(split, [built[0]]);
}

#[test]
fn a_translucent_first_line_lists_the_elements_on_both_sides_of_the_break() {
    let (split, built) =
        split_elements("div::first-line{opacity:.5}", "width:50px", |doc, root| {
            let across = span(doc, root, "display:inline");
            doc.append_text(across, "aaaa bbbb");
            let later = span(doc, root, "display:inline");
            doc.append_text(later, " cccc");
            vec![across, later]
        });
    // `across` has pieces on the first line and on the next; `later` has none
    // on the first.
    assert_eq!(split, [built[0]]);
}

/// The paragraph built by `build` under `sheet` and `root_css`, laid out and
/// prepared for projection.
fn prepared(
    sheet: &str,
    root_css: &str,
    build: impl FnOnce(&mut Document, usize),
) -> (Document, usize, ParagraphProjection) {
    let Fixture {
        mut doc,
        cascade,
        root,
    } = sheet_fixture(sheet, root_css, build);
    layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600()).expect("layout");
    let control = PageLayoutControl::default();
    let mut work = ProjectionWork::new(&control, 1_000_000);
    let projection = ParagraphProjection::prepare(&doc, &cascade, root, 0, &mut work)
        .expect("a paragraph prepares");
    (doc, root, projection)
}

/// The owners and kinds of the generated pieces of `projection`.
fn generated_kinds(projection: &ParagraphProjection) -> Vec<(usize, Option<crate::GeneratedKind>)> {
    projection
        .generated(None)
        .iter()
        .map(|&(owner, kind, ..)| (owner, kind))
        .collect()
}

fn text_paragraph(doc: &mut Document, root: usize) {
    doc.append_text(root, "aa bb");
}

#[test]
fn a_block_level_before_is_one_piece_painted_as_its_box() {
    let (_, root, projection) = prepared(
        r#"div::before { content: "x"; display: block }"#,
        "width:200px",
        text_paragraph,
    );
    // The `::before` is listed once, by its box, and not again by the piece
    // of its text on its line.
    assert_eq!(
        generated_kinds(&projection),
        [(root, Some(crate::GeneratedKind::Before))]
    );
    let (.., edges) = projection.generated(None)[0];
    assert_eq!(edges, Some((true, true)));
}

#[test]
fn a_block_level_after_follows_the_text_it_comes_after() {
    let (_, root, projection) = prepared(
        r#"div::after { content: "y"; display: block }"#,
        "width:200px",
        text_paragraph,
    );
    let [(owner, Some(kind), piece, offset, shift, top, height, Some(_))] =
        projection.generated(None)[..]
    else {
        panic!("one block-level piece");
    };
    assert_eq!((owner, kind), (root, crate::GeneratedKind::After));
    // It sits on the second line, below the text, and is as tall as its line.
    assert_eq!((offset.0, shift, top, height), (0.0, 0.0, 10.0, 10.0));
    assert_eq!(piece.border_box.y, 10.0);
}

#[test]
fn a_block_level_generated_box_that_is_not_visible_is_not_painted() {
    for css in [
        "visibility:hidden",
        "height:0",
        "width:0",
        "visibility:collapse",
    ] {
        let sheet = format!(r#"div::before {{ content: "x"; display: block; {css} }}"#);
        let (doc, root, projection) = prepared(&sheet, "width:200px", text_paragraph);
        // The layout placed the box; it only is not painted.
        assert_eq!(
            doc.nodes[root].ifc_generated_block_pieces().len(),
            1,
            "{css}"
        );
        assert!(generated_kinds(&projection).is_empty(), "{css}");
    }
}

#[test]
fn an_inline_generated_box_is_painted_apart_from_its_text_by_the_piece_on_its_line() {
    let (_, root, projection) = prepared(
        r#"div::before { content: "x" }"#,
        "width:200px",
        text_paragraph,
    );
    let [(owner, Some(kind), _, offset, shift, _, height, None)] = projection.generated(None)[..]
    else {
        panic!("one inline piece");
    };
    assert_eq!((owner, kind), (root, crate::GeneratedKind::Before));
    assert_eq!((offset, shift, height), ((0.0, 0.0), 0.0, 10.0));
}

#[test]
fn an_inline_generated_box_that_is_not_visible_is_not_painted_apart() {
    let (_, _, projection) = prepared(
        r#"div::before { content: "x"; visibility: hidden }"#,
        "width:200px",
        text_paragraph,
    );
    assert!(generated_kinds(&projection).is_empty());
}

#[test]
fn a_plain_inline_element_is_painted_apart_only_when_the_first_line_changes_it() {
    let (_, _, plain) = prepared("", "width:200px", |doc, root| {
        two_lines(doc, root);
    });
    assert!(generated_kinds(&plain).is_empty());
    let mut built = Vec::new();
    let (_, _, restyled) = prepared("div::first-line{color:red}", "width:50px", |doc, root| {
        built = two_lines(doc, root);
    });
    assert_eq!(generated_kinds(&restyled), [(built[0], None)]);
}
