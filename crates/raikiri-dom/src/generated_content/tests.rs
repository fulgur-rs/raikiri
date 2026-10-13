use super::*;

use raikiri_style::{PseudoElem, StyleNodeId, build_rule_tree, cascade};
use raikiri_traits::Symbol;
use taffy::Style;

#[test]
fn generated_ids_round_trip_and_stay_above_document_ids() {
    for element in [0, 1, 42, usize::MAX >> 4] {
        for pseudo in [
            PseudoElem::Before,
            PseudoElem::After,
            PseudoElem::Marker,
            PseudoElem::FirstLetter,
        ] {
            let id = generated_node_id(element, pseudo);
            assert!(id > usize::MAX >> 3, "{element} {pseudo:?}");
            assert_eq!(generated_origin(id), Some((element, pseudo)));
        }
    }
    assert_eq!(generated_origin(usize::MAX >> 3), None);
}

#[test]
fn generated_image_ids_round_trip_and_stay_apart_from_other_ids() {
    let largest = (usize::MAX >> (4 + IMAGE_INDEX_BITS)) >> 1;
    for element in [0, 1, 42, largest] {
        for pseudo in [PseudoElem::Before, PseudoElem::After] {
            for index in [0, 1, MAX_GENERATED_IMAGES - 1] {
                let id = generated_image_node_id(element, pseudo, index).unwrap();
                assert_eq!(generated_image_origin(id), Some((element, pseudo, index)));
                assert_eq!(generated_origin(id), Some((element, pseudo)));
                assert_ne!(id, generated_node_id(element, pseudo));
            }
            assert_eq!(
                generated_image_origin(generated_node_id(element, pseudo)),
                None
            );
        }
    }
    assert_eq!(
        generated_image_node_id(1, PseudoElem::Before, MAX_GENERATED_IMAGES),
        None
    );
    assert_eq!(generated_image_node_id(1, PseudoElem::Marker, 0), None);
    assert_eq!(
        generated_image_node_id(usize::MAX >> 4, PseudoElem::Before, 0),
        None
    );
    assert_eq!(
        generated_image_origin(generated_node_id(1, PseudoElem::Marker)),
        None
    );
    assert_eq!(generated_image_origin(42), None);
}

#[test]
fn generated_pieces_keep_images_in_place_and_text_drops_them() {
    let mut document = Document::new();
    let style = document.append_element(
        Some(document.root_index()),
        "style",
        Style::default(),
        None::<&str>,
    );
    document.append_text(
        style,
        "div::before { content: 'A' url(a.png) url(b.png) 'B' } div::after { color: red }",
    );
    let element = document.append_element(
        Some(document.root_index()),
        "div",
        Style::default(),
        None::<&str>,
    );
    document.mark_in_document_flags();
    let rules = build_rule_tree(&document);
    let cascade = cascade(&document, &rules).expect("cascade succeeds");
    let (_, pieces) = generated_pieces(&document, &cascade, element, PseudoElem::Before, &[])
        .expect("before content");
    assert_eq!(
        pieces,
        [
            GeneratedPiece::Text("A".into()),
            GeneratedPiece::Image(0),
            GeneratedPiece::Image(1),
            GeneratedPiece::Text("B".into()),
        ]
    );
    // The text of the same content leaves the images out.
    let (_, text) =
        generated_text(&document, &cascade, element, PseudoElem::Before, &[]).expect("before text");
    assert_eq!(text, "AB");
    // `::marker` content is not split into pieces, and a pseudo-element
    // without content has none.
    assert!(generated_pieces(&document, &cascade, element, PseudoElem::Marker, &[]).is_none());
    assert!(
        cascade
            .pseudo
            .contains_key(&(StyleNodeId::new(element as u64), PseudoElem::After))
    );
    assert!(generated_pieces(&document, &cascade, element, PseudoElem::After, &[]).is_none());
}

#[test]
fn pseudo_counter_view_matches_directive_application_without_cloning_the_map() {
    let mut document = Document::new();
    let style = document.append_element(
        Some(document.root_index()),
        "style",
        Style::default(),
        None::<&str>,
    );
    document.append_text(
        style,
        "div::before { content: counters(existing, \".\"); counter-reset: existing 6 reset_only 4; counter-increment: existing 3 increment_only 2; counter-set: existing 11 set_only 9 }",
    );
    let element = document.append_element(
        Some(document.root_index()),
        "div",
        Style::default(),
        None::<&str>,
    );
    document.mark_in_document_flags();
    let rules = build_rule_tree(&document);
    let cascade = cascade(&document, &rules).expect("cascade succeeds");
    let pseudo = cascade
        .pseudo
        .get(&(StyleNodeId::new(element as u64), PseudoElem::Before))
        .expect("before pseudo is computed");

    let mut base = CounterSnapshot::default();
    base.insert(Symbol::new("existing"), vec![1, 2]);
    base.insert(Symbol::new("reset_only"), vec![7]);
    base.insert(Symbol::new("increment_only"), vec![8, 9]);
    let mut expected = base.clone();
    apply_counter_directives_to_snapshot(&mut expected, pseudo);

    let view = CounterSnapshotView::new(&base, Some(pseudo));
    for name in ["existing", "reset_only", "increment_only", "set_only"] {
        let actual: Vec<_> = view.values_for(name).iter().collect();
        assert_eq!(
            actual,
            expected
                .get(&Symbol::new(name))
                .cloned()
                .unwrap_or_default()
        );
    }
}

#[test]
fn first_letter_group_ids_round_trip_and_stay_apart_from_other_ids() {
    let largest = usize::MAX >> (3 + LETTER_GROUP_BITS);
    for element in [0, 1, 42, largest] {
        assert_eq!(
            first_letter_group_id(element, 0),
            Some(generated_node_id(element, PseudoElem::FirstLetter))
        );
        let mut seen = std::collections::HashSet::new();
        for index in [1, 2, MAX_LETTER_GROUPS - 1] {
            let id = first_letter_group_id(element, index).unwrap();
            assert!(seen.insert(id));
            assert_eq!(
                generated_origin(id),
                Some((element, PseudoElem::FirstLetter))
            );
            assert_eq!(generated_image_origin(id), None);
            for pseudo in [
                PseudoElem::Before,
                PseudoElem::After,
                PseudoElem::Marker,
                PseudoElem::FirstLetter,
            ] {
                assert_ne!(id, generated_node_id(element, pseudo));
            }
        }
    }
    assert_eq!(first_letter_group_id(1, MAX_LETTER_GROUPS), None);
    assert_eq!(first_letter_group_id(largest + 1, 1), None);
}
