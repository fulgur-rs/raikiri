use super::*;

fn srgb_interpolation() -> raikiri_style::property::GradientColorInterpolation {
    use raikiri_dom::Document;
    use raikiri_style::property::{BackgroundImage, Gradient};
    use raikiri_style::{build_rule_tree, cascade};

    let mut document = Document::new();
    let node = document.append_element(
        Some(0),
        "div",
        Some("background-image:conic-gradient(in srgb,red,blue)"),
    );
    let rules = build_rule_tree(&document);
    let cascade = cascade(&document, &rules).expect("cascade");
    let BackgroundImage::Gradient(gradient) = &cascade.computed[node].background_layers()[0].image
    else {
        panic!("gradient");
    };
    let Gradient::Conic(conic) = &**gradient else {
        panic!("conic");
    };
    conic.interpolation
}

#[test]
fn vector_limit_is_inclusive_and_overflow_is_rejected_without_a_charge() {
    let paint = ConicPaintBudget::default();
    let budget = paint.for_gradient();
    assert!(budget.charge::<u8>(MAX_VECTOR_ENTRIES).is_some());
    let before = paint.used.get();
    assert!(budget.charge::<u8>(MAX_VECTOR_ENTRIES + 1).is_none());
    assert!(budget.charge::<u64>(usize::MAX).is_none());
    assert_eq!(paint.used.get().entries, before.entries);
    assert_eq!(paint.used.get().bytes, before.bytes);
}

#[test]
fn gradient_and_shared_byte_limits_are_inclusive() {
    let paint = ConicPaintBudget::default();
    for _ in 0..4 {
        let budget = paint.for_gradient();
        assert!(budget.charge::<[u8; 1024]>(16 * 1024).is_some());
        assert!(budget.charge::<u8>(1).is_none());
    }
    assert_eq!(paint.used.get().bytes, MAX_PAINT_BYTES);
    assert!(paint.for_gradient().charge::<u8>(1).is_none());
    assert!(
        ConicPaintBudget::default()
            .for_gradient()
            .charge::<u8>(1)
            .is_some()
    );
}

#[test]
fn generated_entry_limit_is_shared_across_gradients() {
    let paint = ConicPaintBudget::default();
    for _ in 0..MAX_PAINT_ENTRIES / MAX_VECTOR_ENTRIES {
        assert!(
            paint
                .for_gradient()
                .charge::<u8>(MAX_VECTOR_ENTRIES)
                .is_some()
        );
    }
    assert!(paint.for_gradient().charge::<u8>(1).is_none());
}

#[test]
fn push_reserves_growth_before_extending_a_buffer() {
    let paint = ConicPaintBudget::default();
    let budget = paint.for_gradient();
    let mut values = budget.vector::<u64>(2).expect("initial reservation");
    for value in 0..3 {
        budget.push(&mut values, value).expect("growth reservation");
    }
    assert_eq!(values, [0, 1, 2]);
    assert!(paint.used.get().bytes >= values.capacity() * size_of::<u64>());
    assert!(paint.used.get().entries >= values.capacity());
}

#[test]
fn a_full_stop_vector_cannot_grow_past_its_limit() {
    let paint = ConicPaintBudget::default();
    let budget = paint.for_gradient();
    let mut values = budget
        .vector::<u8>(MAX_VECTOR_ENTRIES)
        .expect("reservation");
    values.resize(MAX_VECTOR_ENTRIES, 0);
    assert!(budget.push(&mut values, 1).is_none());
    assert_eq!(values.len(), MAX_VECTOR_ENTRIES);
}

#[test]
fn growth_can_use_the_exact_remaining_budget() {
    let paint = ConicPaintBudget::default();
    let budget = paint.for_gradient();
    let mut values = budget.vector::<u64>(2).expect("initial buffer");
    budget
        .charge::<[u8; 1024]>(16 * 1024 - 1)
        .expect("prior work");
    budget.charge::<u8>(984).expect("leave 24 bytes");
    // A doubled four-entry buffer would exceed the limit; an exact
    // three-entry allocation still fits, including its copied entries.
    for value in 0..3 {
        budget.push(&mut values, value).expect("bounded growth");
    }
    assert_eq!(values, [0, 1, 2]);
    assert_eq!(budget.bytes.get(), MAX_GRADIENT_BYTES);
}

#[test]
fn gamut_samples_and_the_average_use_the_same_finite_budget() {
    use super::super::{try_expand_stops, try_ramp_average_color};
    use peniko::color::{AlphaColor, ColorSpaceTag, DynamicColor, HueDirection, Srgb};
    let red = DynamicColor::from_alpha_color(AlphaColor::<Srgb>::new([2.0, 0.0, 0.0, 1.0]));
    let black = DynamicColor::from_alpha_color(AlphaColor::<Srgb>::new([0.0, 0.0, 0.0, 1.0]));
    let interpolation = srgb_interpolation();
    for average in [false, true] {
        let paint = ConicPaintBudget::default();
        let budget = paint.for_gradient();
        // Leave 1KiB: enough for the endpoints, but not the clipped ramp's
        // samples and owning copies. Neither helper may retry unbounded.
        budget
            .charge::<[u8; 1024]>(16 * 1024 - 1)
            .expect("prior work");
        if average {
            assert!(
                try_ramp_average_color(&[(0.0, red), (1.0, black)], &interpolation, Some(&budget))
                    .is_none()
            );
        } else {
            assert!(
                try_expand_stops(
                    &[(0.0, red), (1.0, black)],
                    ColorSpaceTag::Srgb,
                    HueDirection::Shorter,
                    false,
                    Some(&budget)
                )
                .is_none()
            );
        }
        assert!(paint.used.get().bytes <= MAX_PAINT_BYTES);
        assert!(budget.bytes.get() <= MAX_GRADIENT_BYTES);
    }
}

#[test]
fn finite_gamut_expansion_respects_quotas_at_each_sampling_stage() {
    use super::super::{try_expand_stops, try_repeat_over_turn};
    use peniko::color::{AlphaColor, ColorSpaceTag, DynamicColor, HueDirection, Srgb};

    let interpolation = srgb_interpolation();
    let paint = ConicPaintBudget::default();
    let budget = paint.for_gradient();
    assert!(
        try_expand_stops(
            &[],
            ColorSpaceTag::Srgb,
            HueDirection::Shorter,
            false,
            Some(&budget)
        )
        .expect("empty stop list")
        .is_empty()
    );
    let black = DynamicColor::from_alpha_color(AlphaColor::<Srgb>::new([0.0, 0.0, 0.0, 1.0]));
    for red_channel in [1.3, 1.7, 4.3] {
        let red =
            DynamicColor::from_alpha_color(AlphaColor::<Srgb>::new([red_channel, 0.0, 0.0, 1.0]));
        for repeating in [false, true] {
            let mut rejected = false;
            let mut completed = false;
            // Moving the quota across the work needed by a clipped ramp
            // exercises exhaustion during refinement and final sample copies.
            for allowance in [
                128, 256, 384, 512, 640, 768, 896, 1024, 1280, 1536, 1792, 2048, 2304, 2560, 3072,
                4096, 5120, 6144, 7168, 8192, 12288, 16384, 32768,
            ] {
                let paint = ConicPaintBudget::default();
                let budget = paint.for_gradient();
                let prior = MAX_GRADIENT_BYTES - allowance;
                budget
                    .charge::<[u8; 1024]>(prior / 1024)
                    .expect("prior work");
                budget.charge::<u8>(prior % 1024).expect("prior remainder");
                let success = if repeating {
                    try_repeat_over_turn(
                        vec![(0.0, red), (1.0 / 8192.0, black)],
                        &interpolation,
                        &budget,
                    )
                    .is_some()
                } else {
                    try_expand_stops(
                        &[(0.0, red), (1.0, black)],
                        ColorSpaceTag::Srgb,
                        HueDirection::Shorter,
                        false,
                        Some(&budget),
                    )
                    .is_some()
                };
                completed |= success;
                rejected |= !success;
                assert!(budget.bytes.get() <= MAX_GRADIENT_BYTES);
                assert!(paint.used.get().bytes <= MAX_PAINT_BYTES);
                assert!(paint.used.get().entries <= MAX_PAINT_ENTRIES);
            }
            assert!(rejected && completed);
        }
    }
}
