use super::*;

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
