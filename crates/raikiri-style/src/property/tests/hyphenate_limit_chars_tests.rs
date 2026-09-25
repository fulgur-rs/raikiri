use super::*;
use HyphenateLimitCharsValue::{Auto, Integer};

fn value(source: &str) -> Option<HyphenateLimitChars> {
    match parse_entire(source, "hyphenate-limit-chars")? {
        PropertyValue::HyphenateLimitChars(value) => Some(value),
        _ => None,
    }
}

#[test]
fn hyphenate_limit_chars_expands_one_to_three_components() {
    assert_eq!(value("auto"), Some(HyphenateLimitChars::INITIAL));
    assert_eq!(
        value("5"),
        Some(HyphenateLimitChars {
            total: Integer(5),
            before: Auto,
            after: Auto,
        })
    );
    assert_eq!(
        value("5 2"),
        Some(HyphenateLimitChars {
            total: Integer(5),
            before: Integer(2),
            after: Integer(2),
        })
    );
    assert_eq!(
        value("5 2 1"),
        Some(HyphenateLimitChars {
            total: Integer(5),
            before: Integer(2),
            after: Integer(1),
        })
    );
    assert_eq!(
        value("auto 2 auto"),
        Some(HyphenateLimitChars {
            total: Auto,
            before: Integer(2),
            after: Auto,
        })
    );
    assert_eq!(value("auto auto"), Some(HyphenateLimitChars::INITIAL));
    assert_eq!(value("auto auto auto"), Some(HyphenateLimitChars::INITIAL));
}

#[test]
fn hyphenate_limit_chars_computes_calc_as_an_integer() {
    assert_eq!(
        value("calc(3.1)"),
        Some(HyphenateLimitChars {
            total: Integer(3),
            before: Auto,
            after: Auto,
        })
    );
    // Integer math rounds to nearest; ties go toward positive infinity.
    assert_eq!(
        value("calc(1.5) calc(2.5) calc(3.5)"),
        Some(HyphenateLimitChars {
            total: Integer(2),
            before: Integer(3),
            after: Integer(4),
        })
    );
    // The non-negative range applies after integer conversion: -0.5 rounds
    // to zero, while -1.5 rounds to an invalid negative integer.
    assert_eq!(
        value("calc(-0.5)"),
        Some(HyphenateLimitChars {
            total: Integer(0),
            before: Auto,
            after: Auto,
        })
    );
    assert_eq!(value("calc(-1.5)"), None);
}

#[test]
fn hyphenate_limit_chars_rejects_invalid_values() {
    for source in [
        "",
        "2 1 0 auto",
        "-1",
        "3.1",
        "3.0",
        "3.1 2",
        "1px",
        "10%",
        "none",
        "inherit",
        "1, 2",
        "calc(3.1px)",
    ] {
        assert_eq!(value(source), None, "source = {source}");
    }
}

#[test]
fn hyphenate_limit_chars_is_registered_and_has_one_cascade_key() {
    let value = PropertyValue::HyphenateLimitChars(HyphenateLimitChars::INITIAL);
    assert!(is_supported_property_name("hyphenate-limit-chars"));
    assert_eq!(value.key(), PropertyKey::HyphenateLimitChars);
}
