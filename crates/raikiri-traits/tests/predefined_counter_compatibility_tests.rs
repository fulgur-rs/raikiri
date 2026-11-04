//! Compatibility decisions for predefined counter representations (RAIKIRI-1287).

use raikiri_style::property::CounterStyle;
use raikiri_traits::page::{format_predefined_counter, predefined_counter_suffix};
use smol_str::SmolStr;

fn representation(value: i32, name: &str) -> String {
    format_predefined_counter(value, &CounterStyle::Named(SmolStr::new(name)))
}

#[test]
fn square_uses_the_specified_small_square_for_every_value() {
    for value in [i32::MIN, -1, 0, 1, i32::MAX] {
        assert_eq!(representation(value, "square"), "\u{25aa}");
    }
}

#[test]
fn hebrew_zero_and_values_outside_the_defined_range_use_decimal() {
    for value in [i32::MIN, -1, 0, 11000, i32::MAX] {
        assert_eq!(representation(value, "hebrew"), value.to_string());
    }
    assert_eq!(representation(1, "hebrew"), "א");
    assert_eq!(representation(10999, "hebrew"), "י׳תתקצט");
    assert_eq!(predefined_counter_suffix("hebrew"), ". ");
}

#[test]
fn armenian_aliases_keep_the_defined_range_instead_of_extending_digits() {
    for (name, first, last) in [
        ("armenian", "Ա", "ՔՋՂԹ"),
        ("upper-armenian", "Ա", "ՔՋՂԹ"),
        ("lower-armenian", "ա", "քջղթ"),
    ] {
        assert_eq!(representation(1, name), first, "{name}");
        assert_eq!(representation(9999, name), last, "{name}");
        assert_eq!(predefined_counter_suffix(name), ". ", "{name}");
        for value in [i32::MIN, -1, 0, 10000, i32::MAX] {
            assert_eq!(representation(value, name), value.to_string(), "{name}");
        }
    }
}

#[test]
fn korean_negative_prefix_contains_one_decoded_space_and_no_marker_suffix() {
    // CSS Syntax consumes one space after the final hexadecimal escape.
    // The two source spaces in the descriptor therefore decode to one.
    for (name, three, eleven) in [
        ("korean-hangul-formal", "삼", "일십일"),
        ("korean-hanja-informal", "三", "十一"),
        ("korean-hanja-formal", "參", "壹拾壹"),
    ] {
        assert_eq!(
            representation(-3, name),
            format!("마이너스 {three}"),
            "{name}"
        );
        assert_eq!(
            representation(-11, name),
            format!("마이너스 {eleven}"),
            "{name}"
        );
        assert_eq!(predefined_counter_suffix(name), ", ", "{name}");
    }
}

#[test]
fn limited_east_asian_ranges_use_cjk_decimal_then_decimal_fallback() {
    // These are the limited-range definitions, without the optional extension.
    for (name, last) in [
        ("korean-hangul-formal", "구천구백구십구"),
        ("korean-hanja-informal", "九千九百九十九"),
        ("korean-hanja-formal", "九仟九百九拾九"),
        ("simp-chinese-informal", "九千九百九十九"),
        ("simp-chinese-formal", "玖仟玖佰玖拾玖"),
        ("trad-chinese-informal", "九千九百九十九"),
        ("trad-chinese-formal", "玖仟玖佰玖拾玖"),
        ("cjk-ideographic", "九千九百九十九"),
    ] {
        assert_eq!(representation(9999, name), last, "{name}");
        assert_eq!(representation(10000, name), "一〇〇〇〇", "{name}");
        assert_eq!(representation(-10000, name), "-10000", "{name}");
        assert_eq!(representation(i32::MIN, name), "-2147483648", "{name}");
        assert_eq!(
            representation(i32::MAX, name),
            "二一四七四八三六四七",
            "{name}"
        );
    }
}

#[test]
fn east_asian_native_range_preserves_zero_negative_signs_and_marker_suffixes() {
    for (name, one, zero, last, negative, suffix) in [
        (
            "korean-hangul-formal",
            "일",
            "영",
            "구천구백구십구",
            "마이너스 ",
            ", ",
        ),
        (
            "korean-hanja-informal",
            "一",
            "零",
            "九千九百九十九",
            "마이너스 ",
            ", ",
        ),
        (
            "korean-hanja-formal",
            "壹",
            "零",
            "九仟九百九拾九",
            "마이너스 ",
            ", ",
        ),
        (
            "simp-chinese-informal",
            "一",
            "零",
            "九千九百九十九",
            "负",
            "、",
        ),
        (
            "simp-chinese-formal",
            "壹",
            "零",
            "玖仟玖佰玖拾玖",
            "负",
            "、",
        ),
        (
            "trad-chinese-informal",
            "一",
            "零",
            "九千九百九十九",
            "負",
            "、",
        ),
        (
            "trad-chinese-formal",
            "壹",
            "零",
            "玖仟玖佰玖拾玖",
            "負",
            "、",
        ),
        ("cjk-ideographic", "一", "零", "九千九百九十九", "負", "、"),
    ] {
        assert_eq!(representation(0, name), zero, "{name}");
        assert_eq!(representation(1, name), one, "{name}");
        assert_eq!(
            representation(-1, name),
            format!("{negative}{one}"),
            "{name}"
        );
        assert_eq!(
            representation(-9999, name),
            format!("{negative}{last}"),
            "{name}"
        );
        assert_eq!(predefined_counter_suffix(name), suffix, "{name}");
    }
}
