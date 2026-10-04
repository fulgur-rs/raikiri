use super::{AnimationProperty, KeyframePair};

#[test]
fn samples_linear_midpoint_and_endpoints() {
    let font_size = KeyframePair::new(AnimationProperty::FontSize, 0.0, 40.0, 40.0)
        .expect("valid font-size keyframes");
    assert_eq!(font_size.sample(0.0), Some(0.0));
    assert_eq!(font_size.sample(20.0), Some(20.0));
    assert_eq!(font_size.sample(-0.01), None);
    assert_eq!(font_size.sample(40.0), None);
    assert_eq!(font_size.sample(f32::NAN), None);
    assert_eq!(font_size.sample(f32::INFINITY), None);

    let word_spacing = KeyframePair::new(AnimationProperty::WordSpacing, 0.0, 40.0, 40.0)
        .expect("valid word-spacing keyframes");
    assert_eq!(word_spacing.sample(20.0), Some(20.0));
}

#[test]
fn rejects_unsupported_property_values_and_invalid_durations() {
    assert!(AnimationProperty::parse_cssom_name("opacity").is_err());

    for duration in [-1.0, f32::NAN, f32::INFINITY] {
        assert!(
            KeyframePair::new(AnimationProperty::FontSize, 0.0, 40.0, duration).is_err(),
            "duration {duration:?} must be rejected"
        );
    }

    for value in [f32::NAN, f32::INFINITY] {
        assert!(
            KeyframePair::new(AnimationProperty::FontSize, value, 40.0, 40.0).is_err(),
            "keyframe {value:?} must be rejected"
        );
    }
}

#[test]
fn zero_duration_has_no_active_sample() {
    let effect = KeyframePair::new(AnimationProperty::WordSpacing, 0.0, 40.0, 0.0)
        .expect("zero duration is valid");
    assert_eq!(effect.sample(0.0), None);
}
