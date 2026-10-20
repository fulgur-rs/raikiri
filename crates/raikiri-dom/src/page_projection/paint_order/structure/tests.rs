use super::*;

#[test]
fn foreign_group_seeds_do_not_create_an_unrelated_subtree() {
    let structure = ParagraphStructure {
        parents: [(10, None), (11, Some(10)), (12, Some(10))].into(),
        group_rank: [(10, 0), (11, 0), (12, 1)].into(),
        ..Default::default()
    };
    let nested = nested_groups(&structure, [999, 12, 11, 12].into_iter());
    assert_eq!(nested.len(), 2);
    assert_eq!(nested[&None], [10]);
    assert_eq!(nested[&Some(10)], [11, 12]);
}

#[test]
fn a_missing_paragraph_has_no_slots_or_box_membership() {
    let doc = Document::new();
    let cascade = raikiri_style::cascade(&doc, &raikiri_style::build_rule_tree(&doc)).unwrap();
    let missing = doc.node_count();
    let structure = doc.paragraph_structure(&cascade, missing);
    assert!(structure.contents.is_empty());
    assert!(structure.boxes.is_empty());
    assert!(
        doc.ifc_boxes_in_group(&cascade, missing, None, LinePart::All)
            .is_empty()
    );
}
