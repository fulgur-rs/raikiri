use super::*;
use crate::layout::style_views::NativeStyle;
use std::cell::Cell;

/// A reduction that only carries the box's size, counting its runs.
fn counted_size(reads: &Cell<usize>) -> impl Fn(NativeStyle<'_>) -> LayoutStyle + Copy {
    move |style| {
        reads.set(reads.get() + 1);
        LayoutStyle {
            size: style.size(),
            ..LayoutStyle::default()
        }
    }
}

fn width_override(px: f32) -> OwnedPropertyOverride {
    OwnedPropertyOverride::length(LengthProperty::Width, L::Px(px), true).unwrap()
}

#[test]
fn a_reduced_physical_style_is_read_once_until_the_next_preparation() {
    let (mut doc, cascade, child, _) = fixture();
    let mut tree = LayoutSession::with_cascade(&mut doc, &cascade);
    apply_computed_to_style_in_session(&mut tree, &cascade).unwrap();
    let reads = Cell::new(0);
    let reduce = counted_size(&reads);

    let first = tree.reduced_physical_style(child, reduce);
    let second = tree.reduced_physical_style(child, reduce);
    assert_eq!(reads.get(), 1);
    assert_eq!(first.size, second.size);
    assert_eq!(first.size.width, taffy::Dimension::length(31.0));

    tree.prepare_style_views();
    tree.reduced_physical_style(child, reduce);
    assert_eq!(reads.get(), 2);
}

#[test]
fn a_scoped_override_hides_the_kept_reduction_only_while_it_applies() {
    let (mut doc, cascade, child, _) = fixture();
    let mut tree = LayoutSession::with_cascade(&mut doc, &cascade);
    apply_computed_to_style_in_session(&mut tree, &cascade).unwrap();
    let key = tree.nodes.box_key(child).unwrap();
    let reads = Cell::new(0);
    let reduce = counted_size(&reads);
    tree.reduced_physical_style(child, reduce);

    tree.with_overrides(vec![(key, width_override(11.0))], |tree| {
        let inside = tree.reduced_physical_style(child, reduce);
        assert_eq!(inside.size.width, taffy::Dimension::length(11.0));
        assert_eq!(reads.get(), 2);
    });

    // Leaving the scope may prepare the views again, which drops the kept
    // reduction; either way the box reads as it did before the scope and is
    // kept again from the first read on.
    let after = tree.reduced_physical_style(child, reduce);
    let reads_after = reads.get();
    assert_eq!(after.size.width, taffy::Dimension::length(31.0));
    tree.reduced_physical_style(child, reduce);
    assert_eq!(reads.get(), reads_after);
}

#[test]
fn a_pass_override_ends_the_kept_reduction() {
    let (mut doc, cascade, child, _) = fixture();
    let mut tree = LayoutSession::with_cascade(&mut doc, &cascade);
    apply_computed_to_style_in_session(&mut tree, &cascade).unwrap();
    let reads = Cell::new(0);
    let reduce = counted_size(&reads);
    tree.reduced_physical_style(child, reduce);

    tree.record_pass_override(child, width_override(50.0));
    for _ in 0..2 {
        let overridden = tree.reduced_physical_style(child, reduce);
        assert_eq!(overridden.size.width, taffy::Dimension::length(50.0));
    }
    assert_eq!(reads.get(), 3);
}

#[test]
fn a_percentage_basis_hides_the_kept_reduction_while_edges_are_recorded() {
    let (mut doc, cascade, child, _) = fixture();
    let mut tree = LayoutSession::with_cascade(&mut doc, &cascade);
    apply_computed_to_style_in_session(&mut tree, &cascade).unwrap();
    let reads = Cell::new(0);
    let reduce = counted_size(&reads);
    tree.reduced_physical_style(child, reduce);

    let previous = tree.begin_edge_recording(child, 100.0);
    tree.reduced_physical_style(child, reduce);
    assert_eq!(reads.get(), 2);
    tree.end_edge_recording(child, previous);

    tree.reduced_physical_style(child, reduce);
    assert_eq!(reads.get(), 2);
}

#[test]
fn a_session_without_styles_reads_the_reduction_every_time() {
    let (mut doc, _, child, _) = fixture();
    let tree = LayoutSession::new(&mut doc);
    let reads = Cell::new(0);
    let reduce = counted_size(&reads);
    tree.reduced_physical_style(child, reduce);
    tree.reduced_physical_style(child, reduce);
    assert_eq!(reads.get(), 2);
}
