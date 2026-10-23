//! Resolving style lengths against the layout's geometry gives the answers the
//! library's own resolution gives for the same lengths and contexts.

use super::*;
use crate::layout::taffy_adapter::{IntoEdges, MaybeResolve, ResolveOrZero};
use taffy::{Dimension, LengthPercentage, LengthPercentageAuto};

/// A resolver of `calc()` handles that reads the handle, so that a result that
/// ignored the handle or the basis would differ.
fn calc(handle: *const (), basis: f32) -> f32 {
    basis * 0.5 + (handle as usize >> 3) as f32
}

fn handle() -> *const () {
    0x40 as *const ()
}

fn contexts() -> [Option<f32>; 6] {
    [
        None,
        Some(f32::NAN),
        Some(-0.0),
        Some(0.0),
        Some(250.0),
        Some(1e7),
    ]
}

fn dimensions() -> Vec<Dimension> {
    vec![
        Dimension::auto(),
        Dimension::length(12.5),
        Dimension::length(-0.0),
        Dimension::percent(0.25),
        Dimension::percent(f32::NAN),
        Dimension::min_content(),
        Dimension::max_content(),
        Dimension::fit_content(),
        Dimension::fit_content_px(40.0),
        Dimension::fit_content_percent(0.5),
        Dimension::stretch(),
        Dimension::content(),
        Dimension::calc(handle()),
    ]
}

fn length_percentages() -> Vec<LengthPercentage> {
    vec![
        LengthPercentage::length(7.0),
        LengthPercentage::length(-0.0),
        LengthPercentage::percent(0.5),
        LengthPercentage::percent(f32::NEG_INFINITY),
        LengthPercentage::calc(handle()),
    ]
}

fn length_percentage_autos() -> Vec<LengthPercentageAuto> {
    vec![
        LengthPercentageAuto::auto(),
        LengthPercentageAuto::length(7.0),
        LengthPercentageAuto::length(-0.0),
        LengthPercentageAuto::percent(0.5),
        LengthPercentageAuto::percent(f32::INFINITY),
        LengthPercentageAuto::calc(handle()),
    ]
}

fn option_bits(value: Option<f32>) -> Option<u32> {
    value.map(f32::to_bits)
}

#[test]
fn a_dimension_resolves_as_the_library_resolves_it() {
    for length in dimensions() {
        for context in contexts() {
            assert_eq!(
                option_bits(MaybeResolve::maybe_resolve(length, context, calc)),
                option_bits(taffy::MaybeResolve::maybe_resolve(length, context, calc)),
                "maybe_resolve of {length:?} against {context:?}"
            );
            assert_eq!(
                ResolveOrZero::resolve_or_zero(length, context, calc).to_bits(),
                taffy::ResolveOrZero::resolve_or_zero(length, context, calc).to_bits(),
                "resolve_or_zero of {length:?} against {context:?}"
            );
            if let Some(definite) = context {
                assert_eq!(
                    option_bits(MaybeResolve::maybe_resolve(length, definite, calc)),
                    option_bits(taffy::MaybeResolve::maybe_resolve(length, definite, calc)),
                    "maybe_resolve of {length:?} against the definite {definite:?}"
                );
            }
        }
    }
}

#[test]
fn a_length_percentage_resolves_as_the_library_resolves_it() {
    for length in length_percentages() {
        for context in contexts() {
            assert_eq!(
                option_bits(MaybeResolve::maybe_resolve(length, context, calc)),
                option_bits(taffy::MaybeResolve::maybe_resolve(length, context, calc)),
                "maybe_resolve of {length:?} against {context:?}"
            );
            assert_eq!(
                ResolveOrZero::resolve_or_zero(length, context, calc).to_bits(),
                taffy::ResolveOrZero::resolve_or_zero(length, context, calc).to_bits(),
                "resolve_or_zero of {length:?} against {context:?}"
            );
        }
    }
}

#[test]
fn a_length_percentage_or_auto_resolves_as_the_library_resolves_it() {
    for length in length_percentage_autos() {
        for context in contexts() {
            assert_eq!(
                option_bits(MaybeResolve::maybe_resolve(length, context, calc)),
                option_bits(taffy::MaybeResolve::maybe_resolve(length, context, calc)),
                "maybe_resolve of {length:?} against {context:?}"
            );
            assert_eq!(
                ResolveOrZero::resolve_or_zero(length, context, calc).to_bits(),
                taffy::ResolveOrZero::resolve_or_zero(length, context, calc).to_bits(),
                "resolve_or_zero of {length:?} against {context:?}"
            );
        }
    }
}

#[test]
fn a_size_of_dimensions_resolves_against_a_size() {
    let dimensions = dimensions();
    let contexts = contexts();
    for (index, &width) in dimensions.iter().enumerate() {
        let height = dimensions[(index * 5 + 3) % dimensions.len()];
        let style = taffy::Size { width, height };
        for (context_index, &context_width) in contexts.iter().enumerate() {
            let context = Size {
                width: context_width,
                height: contexts[(context_index + 2) % contexts.len()],
            };
            let own: Size<Option<f32>> = MaybeResolve::maybe_resolve(style, context, calc);
            let theirs: taffy::Size<Option<f32>> =
                taffy::MaybeResolve::maybe_resolve(style, context.into_taffy(), calc);
            assert_eq!(
                bits_of(&own),
                bits_of(&Size::<Option<f32>>::from_taffy(theirs))
            );

            let own: Size<f32> = ResolveOrZero::resolve_or_zero(
                taffy::Size {
                    width: LengthPercentageAuto::percent(0.5),
                    height: LengthPercentageAuto::length(3.0),
                },
                context,
                calc,
            );
            let theirs: taffy::Size<f32> = taffy::ResolveOrZero::resolve_or_zero(
                taffy::Size {
                    width: LengthPercentageAuto::percent(0.5),
                    height: LengthPercentageAuto::length(3.0),
                },
                context.into_taffy(),
                calc,
            );
            assert_eq!(bits_of(&own), bits_of(&Size::<f32>::from_taffy(theirs)));
        }
    }
}

#[test]
fn style_edges_resolve_against_a_size_or_one_basis() {
    let rect = taffy::Rect {
        left: LengthPercentage::percent(0.1),
        right: LengthPercentage::length(-0.0),
        top: LengthPercentage::calc(handle()),
        bottom: LengthPercentage::percent(0.75),
    };
    let contexts = contexts();
    for (index, &width) in contexts.iter().enumerate() {
        let size_context = Size {
            width,
            height: contexts[(index + 1) % contexts.len()],
        };
        let own: Edges<f32> = ResolveOrZero::resolve_or_zero(rect, size_context, calc);
        let theirs: taffy::Rect<f32> =
            taffy::ResolveOrZero::resolve_or_zero(rect, size_context.into_taffy(), calc);
        assert_eq!(bits_of(&own), bits_of(&Edges::<f32>::from_taffy(theirs)));

        let own: Edges<f32> = ResolveOrZero::resolve_or_zero(rect, width, calc);
        let theirs: taffy::Rect<f32> = taffy::ResolveOrZero::resolve_or_zero(
            rect,
            taffy::Size {
                width,
                height: width,
            },
            calc,
        );
        // One basis for all four sides is the same as the basis for both axes.
        assert_eq!(bits_of(&own), bits_of(&Edges::<f32>::from_taffy(theirs)));
    }
}

#[test]
fn style_edges_become_layout_edges_side_for_side() {
    let rect = taffy::Rect {
        left: 1.0,
        right: 2.0,
        top: 3.0,
        bottom: 4.0,
    };
    let edges = rect.into_edges();
    assert_eq!(
        (edges.left, edges.right, edges.top, edges.bottom),
        (1.0, 2.0, 3.0, 4.0)
    );
    let styled = taffy::Rect {
        left: LengthPercentage::length(1.0),
        right: LengthPercentage::percent(0.2),
        top: LengthPercentage::length(3.0),
        bottom: LengthPercentage::percent(0.4),
    };
    let edges = styled.into_edges();
    assert_eq!(edges.left, LengthPercentage::length(1.0));
    assert_eq!(edges.right, LengthPercentage::percent(0.2));
    assert_eq!(edges.top, LengthPercentage::length(3.0));
    assert_eq!(edges.bottom, LengthPercentage::percent(0.4));
}
