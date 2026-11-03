use super::*;
use crate::layout::protocol::{RunMode, SizingMode};
use raikiri_layout_style::values::{
    Contain, Dimension, Display, LengthPercentage, LengthPercentageAuto, Overflow, Position,
};

fn style() -> BoxStyle {
    BoxStyle {
        display: Display::Block,
        size: Size {
            width: Dimension::length(0.0),
            height: Dimension::length(50.0),
        },
        ..BoxStyle::default()
    }
}

fn inputs() -> LayoutInput {
    LayoutInput {
        run_mode: RunMode::PerformLayout,
        parent_size: Size {
            width: Some(50.0),
            height: None,
        },
        ..LayoutInput::HIDDEN
    }
}

fn collapses(style: &BoxStyle) -> bool {
    empty_leaf_collapses_on_transposed_axes(inputs(), style, crate::layout::dispatch::resolve_calc)
}

#[test]
fn inline_extents_and_edges_do_not_prevent_block_collapse() {
    let mut style = style();
    style.padding.top = LengthPercentage::length(5.0);
    style.padding.bottom = LengthPercentage::length(7.0);
    style.border.top = LengthPercentage::length(1.0);
    style.border.bottom = LengthPercentage::length(2.0);
    style.min_size.height = LengthPercentageAuto::length(80.0);
    let original = style.clone();
    assert!(collapses(&style));
    assert_eq!(style, original);
}

#[test]
fn block_edges_and_minimum_size_prevent_collapse() {
    for side in 0..5 {
        let mut style = style();
        match side {
            0 => style.padding.left = LengthPercentage::length(1.0),
            1 => style.padding.right = LengthPercentage::length(1.0),
            2 => style.border.left = LengthPercentage::length(1.0),
            3 => style.border.right = LengthPercentage::length(1.0),
            _ => style.min_size.width = LengthPercentageAuto::length(1.0),
        }
        assert!(!collapses(&style));
    }
}

#[test]
fn formatting_context_and_position_guards_are_preserved() {
    for kind in 0..6 {
        let mut style = style();
        match kind {
            0 => style.display = Display::Flex,
            1 => style.position = Position::Absolute,
            2 => style.contain = Contain::LAYOUT,
            3 => style.overflow.x = Overflow::Hidden,
            4 => style.overflow.y = Overflow::Scroll,
            _ => style.display = Display::FlowRoot,
        }
        assert!(!collapses(&style));
    }
    let mut style = style();
    style.overflow.x = Overflow::Clip;
    style.overflow.y = Overflow::Clip;
    assert!(collapses(&style));
}

#[test]
fn percentage_block_size_uses_the_native_block_basis() {
    let mut style = style();
    style.size.width = Dimension::percent(0.1);
    assert!(collapses(&style));
    let mut inputs = inputs();
    inputs.parent_size.height = Some(100.0);
    assert!(!empty_leaf_collapses_on_transposed_axes(
        inputs,
        &style,
        crate::layout::dispatch::resolve_calc
    ));
    style.size.width = Dimension::length(0.0);
    style.padding.left = LengthPercentage::percent(0.1);
    assert!(!collapses(&style));
}

#[test]
fn sizing_mode_and_maximum_clamp_keep_the_leaf_rules() {
    let mut style = style();
    style.size.width = Dimension::length(20.0);
    style.max_size.width = LengthPercentageAuto::length(0.0);
    // With no known block size, the leaf reads the positive preferred size.
    assert!(!collapses(&style));
    // Real parent layout can pass the clamped zero block size instead.
    let mut known = inputs();
    known.known_dimensions.height = Some(0.0);
    assert!(empty_leaf_collapses_on_transposed_axes(
        known,
        &style,
        crate::layout::dispatch::resolve_calc
    ));
    for run_mode in [RunMode::ComputeSize, RunMode::PerformLayout] {
        let inputs = LayoutInput {
            run_mode,
            sizing_mode: SizingMode::ContentSize,
            ..inputs()
        };
        assert!(empty_leaf_collapses_on_transposed_axes(
            inputs,
            &style,
            crate::layout::dispatch::resolve_calc
        ));
    }
}

#[test]
fn ratio_probes_do_not_create_nonfinite_values() {
    let mut style = style();
    style.size.width = Dimension::auto();
    style.aspect_ratio = Some(2.0);
    assert!(!collapses(&style));
    for ratio in [0.0, -2.0, f32::INFINITY, f32::NAN, 1e-40] {
        style.aspect_ratio = Some(ratio);
        assert!(!collapses(&style));
    }
}
