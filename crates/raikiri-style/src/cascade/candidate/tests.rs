use super::*;
use crate::cascade::collect::collect_cascaded;
use crate::cascade::test_support::{BLUE, RED};
use crate::ruletree::build_rule_tree;
use crate::style_dom::{StyleDom, StyleNodeId};
use crate::test_dom::TestDoc;

fn author(source_order: u32) -> Precedence {
    Precedence::new(
        Origin::Author,
        false,
        0,
        source_order,
        LayerPosition::default(),
    )
}

fn owned(values: &[PropertyValue]) -> OwnedCandidates {
    OwnedCandidates::from_declarations(
        values
            .iter()
            .map(|value| (Declaration::new(value.clone(), false), author(0))),
    )
}

fn custom(name: &str, value: &str) -> PropertyValue {
    PropertyValue::CustomProperty(CustomProperty {
        name: name.into(),
        value: value.into(),
    })
}

#[test]
fn same_candidates_compares_the_values_behind_local_handles() {
    // All three lists refer to their own first declaration.
    let red = owned(&[PropertyValue::Color(RED)]);
    let blue = owned(&[PropertyValue::Color(BLUE)]);
    let red_again = owned(&[PropertyValue::Color(RED)]);
    assert!(!same_candidates(
        Some(red.candidates()),
        Some(blue.candidates())
    ));
    assert!(same_candidates(
        Some(red.candidates()),
        Some(red_again.candidates())
    ));
    assert!(same_candidates(None, None));
    assert!(!same_candidates(Some(red.candidates()), None));
}

#[test]
fn same_candidates_compares_precedence_and_order() {
    let declaration = || Declaration::new(PropertyValue::Color(RED), false);
    let early = OwnedCandidates::from_declarations([(declaration(), author(0))]);
    let late = OwnedCandidates::from_declarations([(declaration(), author(1))]);
    assert!(!same_candidates(
        Some(early.candidates()),
        Some(late.candidates())
    ));
    let both = owned(&[PropertyValue::Color(RED), PropertyValue::Opacity(0.5)]);
    let swapped = owned(&[PropertyValue::Opacity(0.5), PropertyValue::Color(RED)]);
    assert!(!same_candidates(
        Some(both.candidates()),
        Some(swapped.candidates())
    ));
    let fewer = owned(&[PropertyValue::Color(RED)]);
    assert!(!same_candidates(
        Some(both.candidates()),
        Some(fewer.candidates())
    ));
}

#[test]
fn same_custom_candidates_compares_names_values_and_precedence() {
    let one = owned(&[custom("--x", "1")]);
    let two = owned(&[custom("--x", "2")]);
    let renamed = owned(&[custom("--y", "1")]);
    let one_again = owned(&[custom("--x", "1")]);
    let later = OwnedCandidates::from_declarations([(
        Declaration::new(custom("--x", "1"), false),
        author(1),
    )]);
    let same = |a: &OwnedCandidates, b: &OwnedCandidates| {
        same_custom_candidates(Some(a.custom_candidates()), Some(b.custom_candidates()))
    };
    assert!(same(&one, &one_again));
    assert!(!same(&one, &two));
    assert!(!same(&one, &renamed));
    assert!(!same(&one, &later));
    assert!(same_custom_candidates(None, None));
    assert!(!same_custom_candidates(Some(one.custom_candidates()), None));
}

#[test]
fn custom_properties_and_all_revert_layer_are_told_apart_by_their_slot() {
    let owned = owned(&[
        PropertyValue::AllRevertLayer,
        custom("--x", "1"),
        PropertyValue::Color(RED),
    ]);
    let decls = owned.candidates();
    assert_eq!(decls.decls().len(), 2);
    assert!(decls.decls()[0].is_all_revert_layer());
    assert_eq!(decls.decls()[0].rollback(), Rollback::Layer);
    assert!(!decls.decls()[1].is_all_revert_layer());
    assert_eq!(*decls.value(1), PropertyValue::Color(RED));
    let names: Vec<_> = owned
        .custom_candidates()
        .iter()
        .map(|(property, _)| property.name.clone())
        .collect();
    assert_eq!(names, ["--x"]);
}

#[test]
fn filtered_views_keep_the_order_and_the_declarations() {
    let owned = owned(&[
        PropertyValue::Color(RED),
        PropertyValue::Opacity(0.5),
        PropertyValue::Color(BLUE),
    ]);
    let mut scratch = Vec::new();
    let colors = owned.candidates().filtered(&mut scratch, |candidate| {
        candidate.key() == PropertyKey::Color
    });
    assert_eq!(colors.decls().len(), 2);
    assert_eq!(*colors.value(0), PropertyValue::Color(RED));
    assert_eq!(*colors.value(1), PropertyValue::Color(BLUE));
}

#[test]
fn owned_copies_keep_the_candidates_and_make_every_handle_local() {
    let mut doc = TestDoc::new();
    let style = doc.push_element(0, "style", None);
    doc.push_text(style, "p { color: red; --x: 1 }");
    let p = doc.push_element(0, "p", Some("opacity: 0.5 !important; --y: 2"));
    let tree = build_rule_tree(&doc);
    let arena = collect_cascaded(&doc, doc.root_id(), &tree).expect("the cascade collects");
    let id = StyleNodeId(p as u64);
    let decls = arena.candidates(id).expect("p has candidates");
    let custom = arena.element(id).1.expect("p has custom properties");
    assert!(matches!(decls.decls()[0].value(), ValueRef::Rule { .. }));

    let owned = OwnedCandidates::copy(decls, custom).expect("the copy numbers its declarations");
    let copy = owned.candidates();
    assert_eq!(copy.decls().len(), decls.decls().len());
    for (idx, (original, copied)) in decls.decls().iter().zip(copy.decls()).enumerate() {
        assert_eq!(copy.value(idx), decls.value(idx));
        assert_eq!(copied.key(), original.key());
        assert_eq!(copied.rollback(), original.rollback());
        assert_eq!(copied.precedence(), original.precedence());
        assert_eq!(copied.value(), ValueRef::Local(idx as u32));
    }
    let properties = |custom: CustomCandidates<'_>| -> Vec<_> {
        custom
            .iter()
            .map(|(property, precedence)| (property.clone(), precedence))
            .collect()
    };
    assert_eq!(properties(owned.custom_candidates()), properties(custom));
    assert_eq!(properties(custom).len(), 2);
}

#[test]
fn precedence_keeps_its_parts_and_derives_the_rank() {
    let layer = LayerPosition {
        attached: false,
        rank: 3,
    };
    let precedence = Precedence::new(Origin::Author, true, 17, 42, layer);
    assert_eq!(precedence.origin(), Origin::Author);
    assert!(precedence.important());
    assert_eq!(precedence.layer(), layer);
    assert_eq!(precedence.specificity(), 17);
    assert_eq!(precedence.source_order(), 42);

    let ranked = precedence.ranked(5);
    assert_eq!(ranked.rank, cascade_rank(Origin::Author, true));
    assert_eq!(ranked.layer_priority, layer.priority(true));
    assert_eq!(
        (ranked.specificity, ranked.source_order, ranked.idx),
        (17, 42, 5)
    );
}

#[test]
fn layered_inspection_orders_like_the_ranking() {
    let layer = LayerPosition {
        attached: true,
        rank: 9,
    };
    let precedence = Precedence::new(Origin::User, false, 1, 2, layer);
    assert_eq!(
        precedence.layered(7, Rollback::Layer),
        (
            (cascade_rank(Origin::User, false), (true, 9), 1, 2, 7),
            Origin::User,
            layer,
            false,
            Rollback::Layer
        )
    );
}
