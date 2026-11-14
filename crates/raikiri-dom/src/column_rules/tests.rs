use super::*;
use crate::layout::test_support::ahem_paragraph;
use crate::layout_single_page;
use raikiri_traits::PageBox;

#[test]
fn layout_retains_rules_and_style_changes_clear_them() {
    let (mut document, cascade, owner) = ahem_paragraph(
        "A\nB\nC\nD",
        "width:100px;column-count:2;column-gap:20px;column-rule:2px solid red;orphans:1;widows:1",
    );
    layout_single_page(&mut document, &cascade, PageBox::default()).unwrap();
    assert_eq!(
        document.column_rules[&owner],
        [PaintRect::new(49.0, 0.0, 2.0, 20.0)]
    );
    document.set_element_inline_style(
        owner,
        Some("width:100px;column-count:2;column-rule:none".into()),
    );
    let cascade =
        raikiri_style::cascade(&document, &raikiri_style::build_rule_tree(&document)).unwrap();
    layout_single_page(&mut document, &cascade, PageBox::default()).unwrap();
    assert!(document.column_rules.is_empty());
}

#[test]
fn rules_on_an_ordinary_box_do_not_spend_column_work() {
    let (mut document, cascade, _) = ahem_paragraph("A", "column-rule:2px solid red");
    layout_single_page(&mut document, &cascade, PageBox::default()).unwrap();
    document.fragment_tree.limit = 0;
    assert!(prepare(&document, &cascade).unwrap().is_empty());
}

#[test]
fn retained_rule_work_is_bounded_before_publication() {
    let (mut document, cascade, _) = ahem_paragraph(
        "A\nB\nC\nD",
        "width:100px;column-count:2;column-gap:20px;column-rule:2px solid red;orphans:1;widows:1",
    );
    layout_single_page(&mut document, &cascade, PageBox::default()).unwrap();
    document.fragment_tree.limit = 1;
    assert!(matches!(
        prepare(&document, &cascade),
        Err(LayoutError::FragmentLimitExceeded { limit: 1 })
    ));
}

#[test]
fn hidden_descendants_and_zero_height_containers_do_not_create_rules() {
    for style in [
        "height:0;width:100px;column-count:2;column-gap:20px;column-rule:2px solid red",
        "width:100px;column-count:2;column-rule:none",
    ] {
        let (mut document, cascade, owner) = ahem_paragraph("A", style);
        layout_single_page(&mut document, &cascade, PageBox::default()).unwrap();
        assert!(!document.column_rules.contains_key(&owner));
    }
    let (mut document, _, owner) = ahem_paragraph(
        "A\nB\nC\nD",
        "width:100px;column-count:2;column-gap:20px;column-rule:2px solid red;orphans:1;widows:1",
    );
    document.append_element(Some(owner), "div", Some("display:none"));
    let cascade =
        raikiri_style::cascade(&document, &raikiri_style::build_rule_tree(&document)).unwrap();
    layout_single_page(&mut document, &cascade, PageBox::default()).unwrap();
    assert_eq!(
        document.column_rules[&owner],
        [PaintRect::new(49.0, 0.0, 2.0, 20.0)]
    );
}

#[test]
fn each_row_box_of_a_continued_nested_container_paints_its_own_rules() {
    let (mut document, cascade, outer) = crate::layout::test_support::ahem_paragraph_with(
        "width:200px;height:300px;column-count:1;column-gap:10px",
        |document, root| {
            let inner = document.append_element(
                Some(root),
                "div",
                Some("display:block;height:500px;column-count:1;column-gap:10px;column-rule:2px solid red"),
            );
            document.append_element(Some(inner), "div", Some("display:block;height:600px"));
        },
    );
    let inner = document.nodes[outer].children[0];
    layout_single_page(&mut document, &cascade, PageBox::default()).unwrap();
    let boxes: Vec<usize> = (0..document.fragment_tree.fragments.len())
        .filter(|&index| {
            let fragment = &document.fragment_tree.fragments[index];
            fragment.node_id == inner && !document.fragment_tree.is_retired(index)
        })
        .collect();
    let rules = |fragmentainer: usize| -> Vec<PaintRect> {
        let index = boxes
            .iter()
            .copied()
            .find(|&index| document.fragment_tree.fragments[index].fragmentainer == fragmentainer)
            .unwrap();
        document
            .fragment_column_rules(&cascade, inner, Some(index))
            .into_iter()
            .map(|rule| rule.rect)
            .collect()
    };
    // The first row has one column; the second row has an overflow column
    // beside its column, and the rule between them is drawn in its own box.
    assert!(rules(0).is_empty());
    assert_eq!(rules(1), [PaintRect::new(204.0, 0.0, 2.0, 200.0)]);
}

#[test]
fn rules_that_only_the_rows_of_a_continued_nested_container_have_are_painted() {
    // The rows of the container split its box that avoids breaks across
    // both columns of each row, so each row has a rule.
    let (mut document, cascade, outer) = crate::layout::test_support::ahem_paragraph_with(
        "width:200px;height:50px;column-count:2;column-gap:10px;column-fill:auto",
        |document, root| {
            let inner = document.append_element(
                Some(root),
                "div",
                Some("display:block;column-count:2;column-gap:10px;column-rule:2px solid red"),
            );
            document.append_element(
                Some(inner),
                "div",
                Some("display:block;height:200px;break-inside:avoid-column"),
            );
        },
    );
    let inner = document.nodes[outer].children[0];
    layout_single_page(&mut document, &cascade, PageBox::default()).unwrap();
    // The rules recorded after layout come from the rows, which is what the
    // painter and page projection check before drawing any.
    assert_eq!(document.column_rules(&cascade, inner).count(), 2);
    let boxes: Vec<usize> = (0..document.fragment_tree.fragments.len())
        .filter(|&index| document.fragment_tree.fragments[index].node_id == inner)
        .collect();
    assert!(boxes.len() >= 2);
    for index in boxes {
        assert_eq!(
            document
                .fragment_column_rules(&cascade, inner, Some(index))
                .len(),
            1
        );
    }
}

#[test]
fn each_row_box_of_a_continued_nested_container_is_a_slice_of_the_whole_box() {
    let (mut document, cascade, outer) = crate::layout::test_support::ahem_paragraph_with(
        "width:200px;height:300px;column-count:1;column-gap:10px",
        |document, root| {
            let inner = document.append_element(
                Some(root),
                "div",
                Some("display:block;height:500px;column-count:1"),
            );
            document.append_element(Some(inner), "div", Some("display:block;height:600px"));
        },
    );
    let inner = document.nodes[outer].children[0];
    layout_single_page(&mut document, &cascade, PageBox::default()).unwrap();
    let second = (0..document.fragment_tree.fragments.len())
        .find(|&index| {
            let fragment = &document.fragment_tree.fragments[index];
            fragment.node_id == inner && fragment.fragmentainer == 1
        })
        .unwrap();
    // A box painted without a record of its own is the first row's.
    assert_eq!(
        document.fragment_block_slice(inner, None),
        Some((0.0, 500.0))
    );
    assert_eq!(
        document.fragment_block_slice(inner, Some(second)),
        Some((300.0, 500.0))
    );
    assert_eq!(document.fragment_block_slice(outer, None), None);
}

/// Lays out one paragraph of three one-line `A` lines, each 50px tall, with
/// `css` on it, in three 100px tall columns that fill in turn, and gives the
/// document and the paragraph's box fragments in column order.
fn paragraph_in_three_columns(css: &str) -> (Document, usize, Vec<usize>) {
    let (mut document, cascade, owner) = crate::layout::test_support::ahem_paragraph_with(
        "width:150px;height:100px;column-count:3;column-gap:0;column-fill:auto",
        |document, root| {
            let paragraph = document.append_element(
                Some(root),
                "div",
                Some(&format!(
                    "display:block;line-height:50px;orphans:1;widows:1;padding:10px 0 20px;{css}"
                )),
            );
            for index in 0..3 {
                if index > 0 {
                    document.append_element(Some(paragraph), "br", Some("display:inline"));
                }
                document.append_text(paragraph, "A");
            }
        },
    );
    let paragraph = document.nodes[owner].children[0];
    layout_single_page(&mut document, &cascade, PageBox::default()).unwrap();
    let mut pieces: Vec<usize> = (0..document.fragment_tree.fragments.len())
        .filter(|&index| {
            let fragment = &document.fragment_tree.fragments[index];
            fragment.node_id == paragraph && fragment.line_start.is_some()
        })
        .collect();
    pieces.sort_by_key(|&index| document.fragment_tree.fragments[index].fragment_index);
    (document, paragraph, pieces)
}

/// The block offset of each line range of `paragraph` from where the
/// paragraph would draw it unbroken, and whether each piece is cut above and
/// below its column.
fn lines_and_cuts(
    document: &Document,
    paragraph: usize,
    pieces: &[usize],
) -> (Vec<f32>, Vec<Option<(bool, bool)>>) {
    let ys = document.nodes[paragraph]
        .ifc_multicol_fragments()
        .unwrap()
        .iter()
        .map(|range| range.y)
        .collect();
    let cuts = pieces
        .iter()
        .map(|&piece| {
            let clip = document.fragment_tree.fragments[piece].fragmentainer_clip?;
            Some((clip.rect.y > -1.0, clip.rect.y + clip.rect.height < 101.0))
        })
        .collect();
    (ys, cuts)
}

#[test]
fn the_pieces_of_a_sliced_paragraph_stitch_into_one_box() {
    let (document, paragraph, pieces) =
        paragraph_in_three_columns("border-radius:8px;background:green");
    assert_eq!(pieces.len(), 3);
    // The first two pieces run to the ends of their columns, and the last
    // holds its line and the padding below it: 100 + 100 + 70. A piece after
    // the first begins 10px above its column, where its box's top padding
    // would be.
    let slices: Vec<_> = pieces
        .iter()
        .map(|&piece| document.fragment_block_slice(paragraph, Some(piece)))
        .collect();
    assert_eq!(
        slices,
        [
            Some((0.0, 270.0)),
            Some((90.0, 270.0)),
            Some((190.0, 270.0))
        ]
    );
    // The lines of a later piece begin at the top of its column, below the
    // padding cut away above it. A rounded box shows its corners where it
    // begins and ends only, so the first piece is not cut above its column,
    // nor the last below it.
    assert_eq!(
        lines_and_cuts(&document, paragraph, &pieces),
        (
            vec![0.0, -10.0, -10.0],
            vec![Some((false, true)), Some((true, true)), Some((true, false))]
        )
    );
    // Plain padding is cut at the column on both sides.
    let (document, paragraph, pieces) = paragraph_in_three_columns("background:green");
    assert_eq!(
        lines_and_cuts(&document, paragraph, &pieces).1,
        [Some((true, true)); 3]
    );
}

#[test]
fn a_cloned_paragraph_has_a_closed_box_in_each_column() {
    let (document, paragraph, pieces) =
        paragraph_in_three_columns("box-decoration-break:clone;background:green");
    let boxes: Vec<_> = pieces
        .iter()
        .map(|&piece| {
            let fragment = &document.fragment_tree.fragments[piece];
            (
                fragment.fragmentainer,
                fragment.line_start,
                fragment.rect.height,
            )
        })
        .collect();
    // Each piece holds one line between the padding above and below it, 80px
    // tall; one the paragraph goes on from runs to the end of its column.
    assert_eq!(
        boxes,
        [(0, Some(0), 100.0), (1, Some(1), 100.0), (2, Some(2), 80.0)]
    );
    // The lines of each piece sit below its own top padding, as in the
    // first, and no piece is cut.
    assert_eq!(
        lines_and_cuts(&document, paragraph, &pieces),
        (vec![0.0, 0.0, 0.0], vec![None, None, None])
    );
}
