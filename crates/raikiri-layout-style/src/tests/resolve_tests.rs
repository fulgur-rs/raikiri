use crate::AlignedCalc;
use crate::resolve::{CalcHandle, IntoEdges, LengthExt, MaybeResolve, ResolveOrZero};
use raikiri_layout_geometry::{Edges, Size};
use raikiri_style::property::CalcLengthPercentage;
use taffy::{Dimension, LengthPercentage as Lp, LengthPercentageAuto as Lpa, Rect};

fn payload(percent: f32, px: f32, non_negative: bool) -> AlignedCalc {
    AlignedCalc::new(CalcLengthPercentage { percent, px }, non_negative)
}

#[test]
fn calc_handle_resolves_against_the_basis_and_reports_percentage() {
    let mixed = payload(50.0, 10.0, true);
    assert_eq!(mixed.pointer().resolve(200.0), 110.0);
    assert!(mixed.pointer().has_percentage());
    let fixed = payload(0.0, 4.0, false);
    assert_eq!(fixed.pointer().resolve(200.0), 4.0);
    assert!(!fixed.pointer().has_percentage());
    // The range check of the payload applies.
    let negative = payload(0.0, -4.0, true);
    assert_eq!(negative.pointer().resolve(200.0), 0.0);
}

/// Exercises one length type through its constructors, which differ per type.
fn exercise_length_ext<L: LengthExt>(length: L, percent: L, calc: L, keywords: &[L]) {
    assert_eq!(length.resolve(Some(200.0)), Some(12.0));
    assert_eq!(length.resolve(None), Some(12.0));
    assert_eq!(percent.resolve(Some(200.0)), Some(50.0));
    assert_eq!(percent.resolve(None), None);
    assert_eq!(calc.resolve(Some(200.0)), Some(110.0));
    assert_eq!(calc.resolve(None), None);
    assert!(calc.is_calc());
    assert!(!length.is_calc());
    assert!(!percent.is_calc());
    assert!(!length.uses_percentage());
    assert!(percent.uses_percentage());
    assert!(calc.uses_percentage());
    for keyword in keywords {
        assert_eq!(keyword.resolve(Some(200.0)), None);
        assert!(!keyword.is_calc());
        assert!(!keyword.uses_percentage());
    }
    assert_eq!(length.identity(), length.identity());
    assert_ne!(length.identity(), percent.identity());
    assert_ne!(length.identity(), calc.identity());
}

#[test]
fn length_ext_covers_every_style_length_kind() {
    let owner = payload(50.0, 10.0, true);
    let handle = owner.pointer();
    exercise_length_ext(
        Dimension::length(12.0),
        Dimension::percent(0.25),
        Dimension::calc(handle),
        &[
            Dimension::auto(),
            Dimension::min_content(),
            Dimension::max_content(),
        ],
    );
    exercise_length_ext(Lp::length(12.0), Lp::percent(0.25), Lp::calc(handle), &[]);
    exercise_length_ext(
        Lpa::length(12.0),
        Lpa::percent(0.25),
        Lpa::calc(handle),
        &[Lpa::auto()],
    );
}

#[test]
fn a_fit_content_limit_depends_on_the_container_but_does_not_resolve_to_a_length() {
    let limit = Dimension::fit_content_percent(0.5);
    assert!(limit.uses_percentage());
    assert!(!limit.is_calc());
    assert_eq!(limit.resolve(Some(200.0)), None);
}

#[test]
fn a_fit_content_calc_limit_is_not_a_plain_calc() {
    let owner = payload(50.0, 10.0, true);
    let limit = Dimension::fit_content_calc(owner.pointer());
    assert!(!limit.is_calc());
    assert_eq!(limit.resolve(Some(200.0)), None);
}

#[test]
fn length_identity_ignores_the_sign_of_zero_and_the_payload_of_nan() {
    assert_eq!(
        Dimension::length(0.0).identity(),
        Dimension::length(-0.0).identity()
    );
    assert_eq!(
        Dimension::length(f32::NAN).identity(),
        Dimension::length(f32::from_bits(0x7fc0_0001)).identity()
    );
    assert_ne!(
        Dimension::length(1.0).identity(),
        Dimension::length(2.0).identity()
    );
    let (first, second) = (payload(50.0, 10.0, true), payload(50.0, 10.0, true));
    assert_eq!(
        Dimension::calc(first.pointer()).identity(),
        Dimension::calc(first.pointer()).identity()
    );
    assert_ne!(
        Dimension::calc(first.pointer()).identity(),
        Dimension::calc(second.pointer()).identity()
    );
}

#[test]
fn into_edges_keeps_every_side() {
    let edges = Rect {
        left: 1.0,
        right: 2.0,
        top: 3.0,
        bottom: 4.0,
    }
    .into_edges();
    assert_eq!(
        edges,
        Edges {
            left: 1.0,
            right: 2.0,
            top: 3.0,
            bottom: 4.0,
        }
    );
}

#[test]
fn maybe_resolve_resolves_a_known_context_and_declines_an_unknown_one() {
    let owner = payload(50.0, 10.0, true);
    let calc = Dimension::calc(owner.pointer());
    assert_eq!(calc.maybe_resolve(Some(200.0)), Some(110.0));
    assert_eq!(calc.maybe_resolve(200.0), Some(110.0));
    assert_eq!(calc.maybe_resolve(None::<f32>), None);
    assert_eq!(
        Dimension::percent(0.25).maybe_resolve(Some(200.0)),
        Some(50.0)
    );
    assert_eq!(Dimension::auto().maybe_resolve(Some(200.0)), None);
    assert_eq!(Lp::percent(0.5).maybe_resolve(10.0), Some(5.0));
    assert_eq!(Lpa::length(3.0).maybe_resolve(None::<f32>), Some(3.0));

    let size = taffy::Size {
        width: Dimension::length(8.0),
        height: Dimension::percent(0.5),
    };
    assert_eq!(
        size.maybe_resolve(Size {
            width: None::<f32>,
            height: Some(40.0),
        }),
        Size {
            width: Some(8.0),
            height: Some(20.0),
        }
    );
}

#[test]
fn resolve_or_zero_gives_zero_where_a_length_cannot_be_resolved() {
    let owner = payload(50.0, 10.0, true);
    assert_eq!(
        Lp::calc(owner.pointer()).resolve_or_zero(Some(200.0)),
        110.0
    );
    assert_eq!(Lp::calc(owner.pointer()).resolve_or_zero(None), 0.0);
    assert_eq!(Lpa::auto().resolve_or_zero(Some(200.0)), 0.0);
    assert_eq!(Dimension::length(6.0).resolve_or_zero(None), 6.0);

    let size = taffy::Size {
        width: Lp::percent(0.5),
        height: Lp::length(7.0),
    };
    assert_eq!(
        size.resolve_or_zero(Size {
            width: Some(40.0),
            height: None,
        }),
        Size {
            width: 20.0,
            height: 7.0,
        }
    );

    let rect = Rect {
        left: Lp::percent(0.5),
        right: Lp::length(1.0),
        top: Lp::percent(0.5),
        bottom: Lp::length(2.0),
    };
    // Horizontal sides resolve against the width, vertical ones against the height.
    assert_eq!(
        rect.resolve_or_zero(Size {
            width: Some(40.0),
            height: Some(10.0),
        }),
        Edges {
            left: 20.0,
            right: 1.0,
            top: 5.0,
            bottom: 2.0,
        }
    );
    // One context for all four sides.
    assert_eq!(
        rect.resolve_or_zero(Some(10.0)),
        Edges {
            left: 5.0,
            right: 1.0,
            top: 5.0,
            bottom: 2.0,
        }
    );
    assert_eq!(
        rect.resolve_or_zero(None::<f32>),
        Edges {
            left: 0.0,
            right: 1.0,
            top: 0.0,
            bottom: 2.0,
        }
    );
}
