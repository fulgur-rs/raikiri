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
