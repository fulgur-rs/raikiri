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
    document.append_element(
        Some(owner),
        "div",
        taffy::Style::default(),
        Some("display:none"),
    );
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
                taffy::Style::default(),
                Some("display:block;height:500px;column-count:1;column-gap:10px;column-rule:2px solid red"),
            );
            document.append_element(
                Some(inner),
                "div",
                taffy::Style::default(),
                Some("display:block;height:600px"),
            );
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
                taffy::Style::default(),
                Some("display:block;column-count:2;column-gap:10px;column-rule:2px solid red"),
            );
            document.append_element(
                Some(inner),
                "div",
                taffy::Style::default(),
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
