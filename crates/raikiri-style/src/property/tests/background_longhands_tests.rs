//! Tests for `background_longhand_texts` in `parse/background_longhands.rs`.

use super::*;

fn texts(value: &str) -> Option<Vec<String>> {
    background_longhand_texts(value).map(|longhands| {
        assert_eq!(
            longhands.iter().map(|(name, _)| *name).collect::<Vec<_>>(),
            BACKGROUND_LONGHANDS
        );
        longhands.into_iter().map(|(_, text)| text).collect()
    })
}

#[test]
fn background_longhand_texts_fill_omitted_components_with_initial_values() {
    assert_eq!(
        texts("none").unwrap(),
        [
            "none",
            "0% 0%",
            "auto",
            "repeat",
            "scroll",
            "padding-box",
            "border-box",
            "transparent"
        ]
    );
}

#[test]
fn background_longhand_texts_keep_source_text_per_layer() {
    assert_eq!(
        texts(
            "url(a.png) left 1px top / 3px 4px space round local padding-box content-box, \
             fixed currentcolor right"
        )
        .unwrap(),
        [
            "url(a.png), none",
            "left 1px top, right",
            "3px 4px, auto",
            "space round, repeat",
            "local, fixed",
            "padding-box, padding-box",
            "content-box, border-box",
            "currentcolor"
        ]
    );
    assert_eq!(texts("border-area").unwrap()[5..7], ["border-box", "border-area"]);
}

#[test]
fn background_longhand_texts_reject_invalid_values() {
    assert_eq!(texts("red, none"), None);
    assert_eq!(texts("none none"), None);
    assert_eq!(texts(""), None);
}
