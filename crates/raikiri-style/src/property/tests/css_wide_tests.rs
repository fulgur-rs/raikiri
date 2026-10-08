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
