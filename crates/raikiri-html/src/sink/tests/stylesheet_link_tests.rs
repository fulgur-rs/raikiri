use super::is_stylesheet_link;

#[test]
fn no_rel_attribute_is_not_a_stylesheet_link() {
    assert!(!is_stylesheet_link(None, None, None));
}

#[test]
fn rel_without_stylesheet_token_is_not_a_stylesheet_link() {
    assert!(!is_stylesheet_link(Some("icon"), None, None));
}

#[test]
fn rel_stylesheet_token_is_case_insensitive() {
    assert!(is_stylesheet_link(Some("StyleSheet"), None, None));
    assert!(is_stylesheet_link(Some("STYLESHEET"), None, None));
}

#[test]
fn rel_stylesheet_among_multiple_space_separated_tokens_matches() {
    assert!(is_stylesheet_link(Some("alternate stylesheet"), None, None));
    assert!(is_stylesheet_link(Some("stylesheet next"), None, None));
}

#[test]
fn absent_type_attribute_is_treated_as_stylesheet() {
    assert!(is_stylesheet_link(Some("stylesheet"), None, None));
}

#[test]
fn empty_type_attribute_is_treated_as_stylesheet_same_as_absent() {
    // `type=""` is "type unspecified", not "type is the empty MIME
    // essence" — it must gate identically to a wholly absent `type`
    // attribute (both `Some("")` and `None` reach this predicate now
    // that `Element::attr` distinguishes "present with empty value"
    // from "absent"; see the doc comment above `is_stylesheet_link`).
    assert!(is_stylesheet_link(Some("stylesheet"), Some(""), None));
}

#[test]
fn type_text_css_case_insensitive_is_treated_as_stylesheet() {
    assert!(is_stylesheet_link(
        Some("stylesheet"),
        Some("text/css"),
        None
    ));
    assert!(is_stylesheet_link(
        Some("stylesheet"),
        Some("Text/CSS"),
        None
    ));
}

#[test]
fn non_css_type_attribute_is_not_treated_as_stylesheet() {
    assert!(!is_stylesheet_link(
        Some("stylesheet"),
        Some("application/rss+xml"),
        None
    ));
}

#[test]
fn type_with_charset_mime_parameter_is_still_treated_as_stylesheet() {
    // browsers ignore MIME parameters (charset, etc.) when gating on the
    // `type` attribute's essence — only `text/css` (before any `;`)
    // matters.
    assert!(is_stylesheet_link(
        Some("stylesheet"),
        Some("text/css; charset=utf-8"),
        None
    ));
    assert!(is_stylesheet_link(
        Some("stylesheet"),
        Some("TEXT/CSS;charset=UTF-8"),
        None
    ));
}

#[test]
fn non_css_essence_with_mime_parameter_is_not_treated_as_stylesheet() {
    assert!(!is_stylesheet_link(
        Some("stylesheet"),
        Some("application/rss+xml; charset=utf-8"),
        None
    ));
}

#[test]
fn untitled_alternate_stylesheet_is_treated_as_stylesheet() {
    // CSSOM "add a CSS style sheet" step 5: an empty title unsets the
    // disabled flag unconditionally, regardless of the alternate flag.
    assert!(is_stylesheet_link(Some("alternate stylesheet"), None, None));
}

#[test]
fn empty_string_title_on_alternate_stylesheet_is_treated_as_stylesheet() {
    // Same as the `None` case above, but exercises `Some("")` directly
    // rather than relying on the call site's `Element::attr` contract
    // (which normalizes an empty attribute value to `None` before this
    // function ever sees it) to collapse the two.
    assert!(is_stylesheet_link(
        Some("alternate stylesheet"),
        None,
        Some("")
    ));
}

#[test]
fn titled_alternate_stylesheet_is_excluded() {
    // CSSOM "add a CSS style sheet" step 4-6: a titled alternate
    // stylesheet only has its disabled flag unset if it matches the
    // page's preferred/selected stylesheet set. This crate tracks
    // neither, so it can never legitimately be "selected" — excluded
    // unconditionally rather than applied as if always preferred.
    assert!(!is_stylesheet_link(
        Some("alternate stylesheet"),
        None,
        Some("High Contrast")
    ));
}

#[test]
fn titled_alternate_stylesheet_is_excluded_regardless_of_type_match() {
    assert!(!is_stylesheet_link(
        Some("stylesheet alternate"),
        Some("text/css"),
        Some("High Contrast")
    ));
}

#[test]
fn titled_non_alternate_stylesheet_link_still_applies() {
    // No `alternate` token in `rel` — a titled *non-alternate* link is
    // the preferred stylesheet (its title becomes the page's preferred
    // stylesheet set name, per CSSOM "add a CSS style sheet" step 4),
    // so it's unaffected by the alternate-only exclusion.
    assert!(is_stylesheet_link(
        Some("stylesheet"),
        None,
        Some("Default")
    ));
}
