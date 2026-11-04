use super::*;

#[test]
fn actual_content_distinguishes_colliding_annotation_root_ids() {
    let mut owners = RubyContainers::default();
    owners.source(7, Some(6));
    owners.source(9, Some(8));
    owners.root(7, Some(6), false);
    owners.root(7, Some(8), false);
    assert_eq!(owners.resolve([7], Some(7)), Owner::Container(6));
    assert_eq!(owners.resolve([9], Some(7)), Owner::Container(8));
    assert_eq!(owners.resolve([], Some(7)), Owner::Unknown);
    assert_eq!(owners.resolve([7, 9], Some(7)), Owner::Ambiguous);
}

#[test]
fn an_explicit_empty_root_needs_agreement_from_all_producers() {
    let mut owners = RubyContainers::default();
    owners.root(4, Some(2), true);
    owners.root(4, Some(2), false);
    assert_eq!(owners.resolve([], Some(4)), Owner::Container(2));
    owners.root(4, None, false);
    assert_eq!(owners.resolve([], Some(4)), Owner::Ambiguous);
    owners.source(8, Some(2));
    assert_eq!(owners.resolve([8, 8], Some(4)), Owner::Container(2));
}

#[test]
fn anonymous_or_conflicting_content_does_not_use_a_root_hint() {
    let mut owners = RubyContainers::default();
    owners.root(4, Some(2), true);
    owners.source(5, None);
    assert_eq!(owners.resolve([5], Some(4)), Owner::Anonymous);
    owners.source(5, Some(2));
    assert_eq!(owners.resolve([5], Some(4)), Owner::Ambiguous);
    assert_eq!(owners.resolve([99], None), Owner::Unknown);
}
