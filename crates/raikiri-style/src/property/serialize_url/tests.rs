use super::serialize_urls;

#[test]
fn top_level_urls_serialize_as_quoted_strings() {
    assert_eq!(
        serialize_urls("url(/favicon.ico)").as_deref(),
        Some(r#"url("/favicon.ico")"#)
    );
    assert_eq!(
        serialize_urls(r#"url('a"b.png'), none, URL(c.png)"#).as_deref(),
        Some(r#"url("a\"b.png"), none, url("c.png")"#)
    );
    assert_eq!(
        serialize_urls("url(hand.cur) 2 3, pointer").as_deref(),
        Some(r#"url("hand.cur") 2 3, pointer"#)
    );
    // Surrounding whitespace is not kept.
    assert_eq!(
        serialize_urls("  url(a.png) ").as_deref(),
        Some(r#"url("a.png")"#)
    );
}

#[test]
fn nested_urls_serialize_as_quoted_strings() {
    assert_eq!(
        serialize_urls("image-set(url(a.png) 1x, url(b.png) 2x)").as_deref(),
        Some(r#"image-set(url("a.png") 1x, url("b.png") 2x)"#)
    );
    assert_eq!(
        serialize_urls("light-dark(url(a.png), url('b.png'))").as_deref(),
        Some(r#"light-dark(url("a.png"), url("b.png"))"#)
    );
    assert_eq!(
        serialize_urls("drop-shadow(1px 1px red) (url(a.svg)) [url(b.svg)]").as_deref(),
        Some(r#"drop-shadow(1px 1px red) (url("a.svg")) [url("b.svg")]"#)
    );
}

#[test]
fn values_without_a_url_to_rewrite_are_kept() {
    // Already canonical, or not a lone URL string.
    assert_eq!(serialize_urls(r#"url("a.png")"#), None);
    assert_eq!(serialize_urls(r#"url("a.png" foo)"#), None);
    assert_eq!(serialize_urls("url()").as_deref(), Some(r#"url("")"#));
    assert_eq!(serialize_urls("linear-gradient(red, blue)"), None);
    assert_eq!(serialize_urls("'url(a.png)'"), None);
    // No `url` text at all: not tokenized.
    assert_eq!(serialize_urls("none"), None);
}

#[test]
fn substitution_functions_keep_the_value_as_written() {
    assert_eq!(serialize_urls("url(a.png), var(--x)"), None);
    assert_eq!(serialize_urls("image-set(url(a.png) 1x, ENV(x))"), None);
    assert_eq!(
        serialize_urls("light-dark(url(a.png), attr(data-x url))"),
        None
    );
}

#[test]
fn unbounded_values_are_kept() {
    let deep = format!("{}url(a.png){}", "(".repeat(512), ")".repeat(512));
    assert_eq!(serialize_urls(&deep), None);
}
