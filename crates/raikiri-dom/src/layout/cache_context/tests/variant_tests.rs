use super::*;
use raikiri_layout_style::{LengthProperty, OwnedPropertyOverride};
use raikiri_style::ComputedLengthPercentageOrAuto as L;

const FIRST: u64 = 1;
const SECOND: u64 = 2;

fn performing(width: f32) -> LayoutInput {
    LayoutInput {
        run_mode: RunMode::PerformLayout,
        ..layout_input(width)
    }
}

/// A box with `entries` as its width, and a session that is ready to lay it
/// out.
fn width_entries(key: BoxKey) -> Vec<(BoxKey, OwnedPropertyOverride)> {
    vec![(
        key,
        OwnedPropertyOverride::length(LengthProperty::Width, L::Px(11.0), true).unwrap(),
    )]
}

/// Store `input` in the cache of the box `idx` as a layout of it does.
fn lay_out(tree: &mut LayoutSession<'_>, idx: usize, input: &LayoutInput) {
    let id = protocol::NodeId::from(idx);
    tree.with_cache_frame(idx, ViewContext::default(), |tree| {
        tree.cache_store(id, input, LayoutOutput::HIDDEN)
    });
}

fn holds(tree: &mut LayoutSession<'_>, idx: usize, input: &LayoutInput) -> bool {
    tree.cache_get(protocol::NodeId::from(idx), input).is_some()
}

macro_rules! session {
    ($tree:ident, $child:ident, $key:ident) => {
        let (mut doc, cascade, $child, _) = crate::layout::style_views::tests::fixture();
        crate::layout::apply_computed_to_style(&mut doc, &cascade).unwrap();
        let mut $tree = LayoutSession::with_cascade(&mut doc, &cascade);
        $tree.prepare_style_views();
        let $key = $tree.nodes.box_key($child).unwrap();
    };
}

#[test]
fn a_variant_keeps_what_it_computed_for_the_next_layout_of_the_box() {
    session!(tree, child, key);
    let input = performing(10.0);
    let mut computed = 0;
    for _ in 0..3 {
        tree.with_style_variant(child, FIRST, width_entries(key), |tree| {
            if !holds(tree, child, &input) {
                computed += 1;
                lay_out(tree, child, &input);
            }
        });
    }
    assert_eq!(computed, 1);
    assert_eq!(tree.override_checkpoint(), 0);
}

#[test]
fn the_variants_of_a_box_and_its_own_style_keep_separate_caches() {
    session!(tree, child, key);
    let (own, first, second) = (layout_input(10.0), layout_input(20.0), layout_input(30.0));
    lay_out(&mut tree, child, &own);
    tree.with_style_variant(child, FIRST, width_entries(key), |tree| {
        assert!(!holds(tree, child, &own), "the cache starts out empty");
        lay_out(tree, child, &first);
    });
    tree.with_style_variant(child, SECOND, width_entries(key), |tree| {
        assert!(!holds(tree, child, &first));
        lay_out(tree, child, &second);
    });
    tree.with_style_variant(child, FIRST, width_entries(key), |tree| {
        assert!(holds(tree, child, &first));
        assert!(!holds(tree, child, &second));
        assert!(!holds(tree, child, &own));
    });
    // Neither variant stored anything where the box's own style is in place.
    assert!(!holds(&mut tree, child, &first));
    assert!(!holds(&mut tree, child, &second));
}

#[test]
fn a_variant_leaves_the_sizes_the_box_measured_with_its_own_style() {
    session!(tree, child, key);
    let measured = layout_input(10.0);
    lay_out(&mut tree, child, &measured);
    tree.with_style_variant(child, FIRST, width_entries(key), |_| ());
    assert!(holds(&mut tree, child, &measured));
}

#[test]
fn the_final_layout_of_a_box_is_not_answered_once_another_variant_laid_it_out() {
    session!(tree, child, key);
    let (measured, laid_out) = (layout_input(10.0), performing(10.0));
    lay_out(&mut tree, child, &measured);
    lay_out(&mut tree, child, &laid_out);
    tree.note_base_variant(child);
    assert!(holds(&mut tree, child, &laid_out));
    tree.with_style_variant(child, FIRST, width_entries(key), |tree| {
        lay_out(tree, child, &laid_out);
    });
    // The lines and the layouts of the boxes inside are the variant's now.
    assert!(!holds(&mut tree, child, &laid_out));
    assert!(holds(&mut tree, child, &measured));
    // The box lays out with its own style again, and then the variant is the
    // one that cannot answer.
    lay_out(&mut tree, child, &laid_out);
    tree.note_base_variant(child);
    assert!(holds(&mut tree, child, &laid_out));
    tree.with_style_variant(child, FIRST, width_entries(key), |tree| {
        assert!(!holds(tree, child, &laid_out));
    });
}

#[test]
fn a_variant_that_laid_the_box_out_last_still_answers_with_its_final_layout() {
    session!(tree, child, key);
    let input = performing(10.0);
    for _ in 0..2 {
        tree.with_style_variant(child, FIRST, width_entries(key), |tree| {
            lay_out(tree, child, &input);
        });
    }
    tree.with_style_variant(child, FIRST, width_entries(key), |tree| {
        assert!(holds(tree, child, &input));
    });
    // Another variant lays the box out in between.
    tree.with_style_variant(child, SECOND, width_entries(key), |_| ());
    tree.with_style_variant(child, FIRST, width_entries(key), |tree| {
        assert!(!holds(tree, child, &input));
    });
}

#[test]
fn a_variant_does_not_outlive_the_cache_generation() {
    session!(tree, child, key);
    let input = layout_input(10.0);
    tree.with_style_variant(child, FIRST, width_entries(key), |tree| {
        lay_out(tree, child, &input);
    });
    tree.layout_cache_context.invalidate();
    tree.with_style_variant(child, FIRST, width_entries(key), |tree| {
        assert!(!holds(tree, child, &input));
    });
}

#[test]
fn clearing_the_caches_of_a_subtree_drops_its_variants() {
    session!(tree, child, key);
    let input = layout_input(10.0);
    tree.with_style_variant(child, FIRST, width_entries(key), |tree| {
        lay_out(tree, child, &input);
    });
    tree.cache_clear_subtree(protocol::NodeId::from(child));
    tree.with_style_variant(child, FIRST, width_entries(key), |tree| {
        assert!(!holds(tree, child, &input));
    });
}

#[test]
fn a_layout_that_panics_leaves_the_style_and_the_cache_of_the_box_as_they_were() {
    session!(tree, child, key);
    let measured = layout_input(10.0);
    lay_out(&mut tree, child, &measured);
    let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        tree.with_style_variant(child, FIRST, width_entries(key), |_| {
            panic!("a layout that fails");
        })
    }));
    assert!(panicked.is_err());
    assert_eq!(tree.override_checkpoint(), 0);
    assert_eq!(tree.style_override_depth(child), 0);
    assert!(holds(&mut tree, child, &measured));
}
