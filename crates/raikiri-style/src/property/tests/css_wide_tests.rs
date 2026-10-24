use super::*;

#[test]
fn css_wide_values_are_supported_for_color_background_and_font_size() {
    for name in ["color", "background-color", "font-size", "vertical-align"] {
        for keyword in ["inherit", "initial", "unset", "revert", "revert-layer"] {
            assert!(parse_entire(keyword, name).is_some(), "{name}: {keyword}");
            assert!(parse_entire(&format!("{keyword} red"), name).is_none());
        }
        for source in ["/**/ INITIAL /**/", r"\69 nitial"] {
            assert!(parse_entire(source, name).is_some(), "{name}: {source}");
        }
    }
}

#[test]
fn svg_export_properties_accept_css_wide_defaulting() {
    for name in [
        "display",
        "opacity",
        "visibility",
        "font-family",
        "font-weight",
        "font-style",
        "fill",
        "stroke",
        "fill-opacity",
        "stroke-opacity",
        "stroke-width",
        "fill-rule",
        "clip-rule",
        "stroke-linecap",
        "stroke-linejoin",
        "stroke-miterlimit",
        "stroke-dasharray",
        "stroke-dashoffset",
    ] {
        for keyword in [
            "inherit",
            "initial",
            "unset",
            "revert",
            "revert-layer",
            r"\69 nherit",
        ] {
            assert!(
                matches!(
                    parse_entire(keyword, name),
                    Some(PropertyValue::Deferred(_))
                ),
                "{name}: {keyword}"
            );
            assert!(parse_entire(&format!("{keyword} extra"), name).is_none());
        }
    }
}

#[test]
fn all_takes_the_css_wide_keywords_and_nothing_else() {
    for keyword in ["inherit", "initial", "unset", "revert", "revert-layer"] {
        let expected = CssWideKeyword::from_css_ident(keyword).expect("a CSS-wide keyword");
        for source in [
            keyword.to_owned(),
            keyword.to_uppercase(),
            format!("/**/ {keyword} /**/"),
        ] {
            assert_eq!(
                parse_entire(&source, "all"),
                Some(PropertyValue::All(expected)),
                "{source}"
            );
        }
        assert!(parse_entire(&format!("{keyword} {keyword}"), "all").is_none());
    }
    assert_eq!(
        PropertyValue::All(CssWideKeyword::Unset).key(),
        PropertyKey::All
    );
    // The component values `all` does not have, and the `var()` form, which
    // is not supported.
    for source in [
        "auto",
        "none",
        "1",
        "red",
        "default",
        "var(--x)",
        "initial red",
    ] {
        assert!(parse_entire(source, "all").is_none(), "{source}");
    }
}
