use super::*;

#[test]
fn rewrites_unquoted_urls_inside_nested_css_blocks() {
    let base = Url::parse("http://web-platform.test/styles/sheet.css").unwrap();
    let source = r#".tile{background:image-set(url(red.png) 1x,url("../blue.png") 2x);--asset:[url(../green.png)]}"#;
    assert_eq!(
        absolutize_stylesheet_urls(source, &base),
        r#".tile{background:image-set(url("http://web-platform.test/styles/red.png") 1x,url("http://web-platform.test/blue.png") 2x);--asset:[url("http://web-platform.test/green.png")]}"#
    );
}

#[test]
fn preserves_comments_strings_and_invalid_urls_while_rewriting_valid_urls() {
    let base = Url::parse("http://web-platform.test/styles/sheet.css").unwrap();
    let source = r#"/* url(comment.png) */p{content:"url(text.png)";background:url("http://[");mask:url(mask.png)}"#;
    assert_eq!(
        absolutize_stylesheet_urls(source, &base),
        r#"/* url(comment.png) */p{content:"url(text.png)";background:url("http://[");mask:url("http://web-platform.test/styles/mask.png")}"#
    );
}

#[test]
fn preserves_stylesheets_without_valid_url_tokens() {
    let base = Url::parse("http://web-platform.test/styles/sheet.css").unwrap();
    for source in [
        "/* unchanged */p{color:red}",
        r#"p{background:url("one.png" "two.png")}"#,
        r#"p{background:url(bad url)}"#,
    ] {
        assert_eq!(absolutize_stylesheet_urls(source, &base), source);
    }
}

#[test]
fn keeps_absolute_and_data_urls_and_resolves_protocol_relative_urls() {
    let base = Url::parse("http://web-platform.test/styles/sheet.css").unwrap();
    assert_eq!(
        absolutize_stylesheet_urls(
            r#"p{a:url(https://cdn.test/a.png);b:url(data:image/png;base64,AA==);c:URL("//cdn.test/b.png")}"#,
            &base,
        ),
        r#"p{a:url("https://cdn.test/a.png");b:url("data:image/png;base64,AA==");c:url("http://cdn.test/b.png")}"#,
    );
}
