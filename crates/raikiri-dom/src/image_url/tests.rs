use super::*;

fn base() -> Url {
    Url::parse("https://example.test/docs/page.html").unwrap()
}

fn resolved(raw: &str, base_url: Option<&Url>) -> Option<String> {
    authored_image_url(raw, base_url).map(String::from)
}

#[test]
fn an_absolute_url_stands_for_itself_with_or_without_a_base() {
    let base = base();
    for base_url in [None, Some(&base)] {
        assert_eq!(
            resolved("https://images.test/a.png", base_url).as_deref(),
            Some("https://images.test/a.png")
        );
    }
}

#[test]
fn a_relative_url_resolves_against_the_base() {
    let base = base();
    for (raw, expected) in [
        ("a.png", "https://example.test/docs/a.png"),
        ("../a.png", "https://example.test/a.png"),
        ("/a.png", "https://example.test/a.png"),
        ("//images.test/a.png", "https://images.test/a.png"),
        ("  a.png  ", "https://example.test/docs/a.png"),
    ] {
        assert_eq!(
            resolved(raw, Some(&base)).as_deref(),
            Some(expected),
            "{raw:?}"
        );
    }
}

#[test]
fn a_relative_url_without_a_base_names_no_image() {
    assert_eq!(resolved("a.png", None), None);
}

#[test]
fn an_empty_or_fragment_only_url_names_no_image() {
    let base = base();
    for raw in ["", "   ", "\n", "#", "#target", "  #target"] {
        assert_eq!(resolved(raw, Some(&base)), None, "{raw:?}");
        assert_eq!(resolved(raw, None), None, "{raw:?}");
    }
}

#[test]
fn a_url_that_does_not_parse_names_no_image() {
    let base = base();
    assert_eq!(resolved("https://[broken/a.png", Some(&base)), None);
}

#[test]
fn the_fragment_does_not_name_another_image() {
    let base = base();
    assert_eq!(
        resolved("https://images.test/a.svg#view", None).as_deref(),
        Some("https://images.test/a.svg")
    );
    assert_eq!(
        resolved("a.svg#view", Some(&base)).as_deref(),
        Some("https://example.test/docs/a.svg")
    );
    assert_eq!(
        resolved("a.svg?size=2#view", Some(&base)).as_deref(),
        Some("https://example.test/docs/a.svg?size=2")
    );
}
