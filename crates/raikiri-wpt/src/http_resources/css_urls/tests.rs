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
fn rewrites_src_and_image_set_string_urls() {
    let base = Url::parse("http://web-platform.test/styles/sheet.css").unwrap();
    let source = r#"p{a:src("a.png");b:image-set("b.png" 1x,'c.png' type("image/png") 2x);c:-webkit-image-set("d.png" 1x);d:url("e.png" cross-origin(anonymous));e:fn("f.png")}"#;
    assert_eq!(
        absolutize_stylesheet_urls(source, &base),
        r#"p{a:src("http://web-platform.test/styles/a.png");b:image-set("http://web-platform.test/styles/b.png" 1x,"http://web-platform.test/styles/c.png" type("image/png") 2x);c:-webkit-image-set("http://web-platform.test/styles/d.png" 1x);d:url("http://web-platform.test/styles/e.png" cross-origin(anonymous));e:fn("f.png")}"#
    );
}

#[test]
fn keeps_fragment_only_and_empty_urls() {
    let base = Url::parse("http://web-platform.test/styles/sheet.css").unwrap();
    for source in [
        "p{clip-path:url(#clip)}",
        r##"p{mask:url("#mask")}"##,
        r##"p{a:src("#a");b:image-set("#b" 1x)}"##,
        // An empty URL names no resource.
        r#"p{a:src("");b:image-set("" 1x);c:url("")}"#,
        // URL parsing strips surrounding whitespace first.
        r##"p{a:src(" #icon");b:image-set(" " 1x)}"##,
    ] {
        assert_eq!(absolutize_stylesheet_urls(source, &base), source);
    }
}

#[test]
fn rewritten_urls_round_trip_as_css_strings() {
    // An opaque `data:` path keeps `"` and `\` unencoded, so the rewritten
    // string must escape them to read back as the same URL.
    let base = Url::parse("http://web-platform.test/styles/sheet.css").unwrap();
    let source = r#"p{a:url('data:text/plain,"\\')}"#;
    let output = absolutize_stylesheet_urls(source, &base);
    assert_eq!(output, r#"p{a:url("data:text/plain,\"\\")}"#);
    let mut input = ParserInput::new(&output);
    let mut parser = Parser::new(&mut input);
    let mut replacements = Vec::new();
    collect_urls(&mut parser, &mut replacements, false).unwrap();
    assert_eq!(replacements[0].value, r#"data:text/plain,"\"#);
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
