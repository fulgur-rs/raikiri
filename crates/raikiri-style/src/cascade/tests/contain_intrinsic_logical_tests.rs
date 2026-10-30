//! `contain-intrinsic-inline-size` and `contain-intrinsic-block-size` (CSS Box
//! Sizing 4 §5): each shares its computed value with the physical
//! `contain-intrinsic-width` or `-height` on its axis under the element's own
//! writing mode.

use super::*;
use crate::resolve::{
    ComputedContainIntrinsicSize, ComputedLength, ComputedLengthPercentageOrAuto,
};

/// A parent whose intrinsic sizes are away from the initial `none`.
const PARENT: &str = "contain-intrinsic-width: 30px; contain-intrinsic-height: 40px";

fn length(px: f32) -> ComputedContainIntrinsicSize {
    ComputedContainIntrinsicSize {
        auto: false,
        length: Some(ComputedLength(px)),
    }
}

const NONE: ComputedContainIntrinsicSize = ComputedContainIntrinsicSize::NONE;

/// The computed values of the child `p` (`id="target"`) of a parent styled
/// `parent`, under `ua` as a user agent sheet followed by `css` as an author
/// sheet.
fn child_in(parent: &str, ua: &str, css: &str) -> ComputedValues {
    let mut doc = TestDoc::new();
    let parent = doc.push_element(0, "div", Some(parent));
    let child = doc.push_element_with_attrs(parent, "p", None, &[("id", "target")]);
    let mut tree = RuleTree::empty();
    tree.add_stylesheet(ua, Origin::UserAgent);
    tree.add_stylesheet(css, Origin::Author);
    cascade(&doc, &tree).unwrap().computed[child].clone()
}

/// The `(width, height)` intrinsic sizes of a computed style.
fn sizes_of(
    values: &ComputedValues,
) -> (ComputedContainIntrinsicSize, ComputedContainIntrinsicSize) {
    (
        values.contain_intrinsic_width,
        values.contain_intrinsic_height,
    )
}

/// The child `p` (`id="target"`) of [`PARENT`] under `css` as an author sheet,
/// as its computed `(width, height)` intrinsic sizes.
fn sizes(css: &str) -> (ComputedContainIntrinsicSize, ComputedContainIntrinsicSize) {
    sizes_of(&child_in(PARENT, "", css))
}

#[test]
fn a_horizontal_box_maps_the_inline_size_to_the_width() {
    assert_eq!(
        sizes(
            "#target { contain-intrinsic-inline-size: 100px; contain-intrinsic-block-size: 50px }"
        ),
        (length(100.0), length(50.0))
    );
}

#[test]
fn a_vertical_box_maps_the_inline_size_to_the_height() {
    for mode in ["vertical-lr", "vertical-rl", "sideways-lr", "sideways-rl"] {
        let css = format!(
            "#target {{ writing-mode: {mode}; contain-intrinsic-inline-size: 50px; \
             contain-intrinsic-block-size: 100px }}"
        );
        assert_eq!(sizes(&css), (length(100.0), length(50.0)), "{mode}");
    }
}

#[test]
fn the_direction_does_not_change_the_axis() {
    for direction in ["ltr", "rtl"] {
        let css = format!(
            "#target {{ direction: {direction}; contain-intrinsic-inline-size: 7px; \
             contain-intrinsic-block-size: 9px }}"
        );
        assert_eq!(sizes(&css), (length(7.0), length(9.0)), "{direction}");
    }
}

#[test]
fn the_later_of_a_logical_and_a_physical_declaration_on_one_axis_wins() {
    assert_eq!(
        sizes("#target { contain-intrinsic-width: 10px; contain-intrinsic-inline-size: 20px }").0,
        length(20.0)
    );
    assert_eq!(
        sizes("#target { contain-intrinsic-inline-size: 20px; contain-intrinsic-width: 10px }").0,
        length(10.0)
    );
    // The axis follows the writing mode: here the inline size is the height.
    let vertical = "writing-mode: vertical-rl;";
    assert_eq!(
        sizes(&format!(
            "#target {{ {vertical} contain-intrinsic-height: 10px; \
             contain-intrinsic-inline-size: 20px }}"
        )),
        (NONE, length(20.0))
    );
    assert_eq!(
        sizes(&format!(
            "#target {{ {vertical} contain-intrinsic-inline-size: 20px; \
             contain-intrinsic-height: 10px }}"
        )),
        (NONE, length(10.0))
    );
}

#[test]
fn a_more_specific_declaration_beats_a_later_one_on_the_same_axis() {
    assert_eq!(
        sizes(
            "#target { contain-intrinsic-block-size: 20px } \
             p { contain-intrinsic-height: 10px }"
        )
        .1,
        length(20.0)
    );
}

#[test]
fn the_shorthand_and_the_logical_longhands_compete_by_order() {
    assert_eq!(
        sizes("#target { contain-intrinsic-size: 7px 9px; contain-intrinsic-block-size: 3px }"),
        (length(7.0), length(3.0))
    );
    assert_eq!(
        sizes("#target { contain-intrinsic-block-size: 3px; contain-intrinsic-size: 7px 9px }"),
        (length(7.0), length(9.0))
    );
}

#[test]
fn auto_and_none_are_taken_by_the_mapped_axis() {
    assert_eq!(
        sizes("#target { contain-intrinsic-inline-size: auto 30px }").0,
        ComputedContainIntrinsicSize {
            auto: true,
            length: Some(ComputedLength(30.0)),
        }
    );
    assert_eq!(
        sizes("#target { contain-intrinsic-height: 8px; contain-intrinsic-block-size: auto none }")
            .1,
        ComputedContainIntrinsicSize {
            auto: true,
            length: None,
        }
    );
}

#[test]
fn an_invalid_logical_value_is_dropped() {
    assert_eq!(
        sizes(
            "#target { contain-intrinsic-width: 10px; contain-intrinsic-inline-size: 5%; \
             contain-intrinsic-block-size: -3px }"
        ),
        (length(10.0), NONE)
    );
}

#[test]
fn var_substitution_keeps_the_mapped_axis() {
    assert_eq!(
        sizes(
            "#target { --a: 7px; --b: auto 9px; writing-mode: vertical-lr; \
             contain-intrinsic-inline-size: var(--a); contain-intrinsic-block-size: var(--b) }"
        ),
        (
            ComputedContainIntrinsicSize {
                auto: true,
                length: Some(ComputedLength(9.0)),
            },
            length(7.0)
        )
    );
}

#[test]
fn inherit_takes_the_parents_value_on_the_mapped_axis() {
    assert_eq!(
        sizes(
            "#target { contain-intrinsic-inline-size: inherit; contain-intrinsic-block-size: inherit }"
        ),
        (length(30.0), length(40.0))
    );
    // The inline size of a vertical box is its height, and `inherit` takes the parent's value of
    // the physical longhand that axis maps to, as it does for the flow-relative margins.
    assert_eq!(
        sizes(
            "#target { writing-mode: vertical-rl; contain-intrinsic-inline-size: inherit; \
             contain-intrinsic-block-size: inherit }"
        ),
        (length(30.0), length(40.0))
    );
    assert_eq!(
        sizes("#target { writing-mode: vertical-rl; contain-intrinsic-inline-size: inherit }"),
        (NONE, length(40.0))
    );
}

#[test]
fn initial_and_unset_give_none_on_the_mapped_axis() {
    for keyword in ["initial", "unset"] {
        let css = format!(
            "#target {{ contain-intrinsic-size: 7px 9px; writing-mode: vertical-lr; \
             contain-intrinsic-inline-size: {keyword} }}"
        );
        assert_eq!(sizes(&css), (length(7.0), NONE), "{keyword}");
    }
}

#[test]
fn all_resets_an_earlier_logical_declaration_only() {
    assert_eq!(
        sizes("#target { contain-intrinsic-inline-size: 20px; all: initial }"),
        (NONE, NONE)
    );
    assert_eq!(
        sizes("#target { all: initial; contain-intrinsic-inline-size: 20px }"),
        (length(20.0), NONE)
    );
}

#[test]
fn revert_rolls_back_to_the_declaration_on_the_mapped_axis() {
    // The user agent sets the height; the logical `revert` of a vertical box rolls the height
    // back to it, whatever the author wrote for it before.
    let ua = "#target { contain-intrinsic-height: 30px; contain-intrinsic-width: 20px }";
    let values = child_in(
        "",
        ua,
        "#target { writing-mode: vertical-rl; contain-intrinsic-height: 11px; \
         contain-intrinsic-inline-size: revert }",
    );
    assert_eq!(sizes_of(&values), (length(20.0), length(30.0)));
    // With nothing below to roll back to, the axis is `none`.
    let values = child_in(
        "",
        "",
        "#target { writing-mode: vertical-rl; contain-intrinsic-height: 11px; \
         contain-intrinsic-inline-size: revert }",
    );
    assert_eq!(sizes_of(&values), (NONE, NONE));
}

#[test]
fn revert_layer_rolls_back_to_the_layer_below_on_the_mapped_axis() {
    let values = child_in(
        "",
        "",
        "@layer a { #target { contain-intrinsic-height: 30px; contain-intrinsic-width: 20px } } \
         @layer b { #target { writing-mode: vertical-lr; contain-intrinsic-inline-size: \
         revert-layer; contain-intrinsic-block-size: revert-layer } }",
    );
    // The inline size is the height and the block size the width of a vertical box.
    assert_eq!(sizes_of(&values), (length(20.0), length(30.0)));
}

#[test]
fn a_vertical_child_of_a_table_or_multicol_box_keeps_its_own_axes() {
    // Block layout places these children on horizontal axes, which the flow-relative margins
    // follow, but an intrinsic size is physical and follows the writing mode the box has.
    for parent in ["display: table", "column-count: 2"] {
        let values = child_in(
            parent,
            "",
            "#target { writing-mode: vertical-rl; contain-intrinsic-inline-size: 50px; \
             contain-intrinsic-block-size: 100px; margin-inline-start: 7px }",
        );
        assert_eq!(sizes_of(&values), (length(100.0), length(50.0)), "{parent}");
        let px = ComputedLengthPercentageOrAuto::Px;
        assert_eq!(values.margin.left, px(7.0), "{parent}");
        assert_eq!(values.margin.top, px(0.0), "{parent}");
    }
}
