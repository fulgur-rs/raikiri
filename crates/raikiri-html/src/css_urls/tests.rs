use super::*;

fn resolve(source: &str) -> String {
    let base = Url::parse("https://example.test/css/print.css").unwrap();
    absolutize_urls(source, &base).into_owned()
}

#[test]
fn relative_urls_resolve_against_the_stylesheet() {
    assert_eq!(
        resolve("div { background: url(bg.png) no-repeat }"),
        r#"div { background: url("https://example.test/css/bg.png") no-repeat }"#
    );
    assert_eq!(
        resolve(r#"li { list-style-image: URL( "../img/a b.png" ) }"#),
        r#"li { list-style-image: url("https://example.test/img/a%20b.png") }"#
    );
    assert_eq!(
        resolve("@media print { p { content: url('x.svg') } }"),
        r#"@media print { p { content: url("https://example.test/css/x.svg") } }"#
    );
    assert_eq!(
        resolve("@font-face { src: url(f.woff2) format('woff2') }"),
        r#"@font-face { src: url("https://example.test/css/f.woff2") format('woff2') }"#
    );
}

#[test]
fn absolute_fragment_empty_and_namespace_urls_are_kept() {
    for source in [
        "a { background: url(https://cdn.test/a.png) }",
        "a { background: url(data:image/png;base64,AAAA) }",
        "a { clip-path: url(#clip) }",
        "a { background: url() }",
        "@namespace svg url(svg-ns); a { color: red }",
        "a { content: 'url(x.png)' }",
    ] {
        assert_eq!(resolve(source), source);
    }
    assert_eq!(
        resolve("@namespace url(ns); a { background: url(a.png) }"),
        r#"@namespace url(ns); a { background: url("https://example.test/css/a.png") }"#
    );
}

#[test]
fn namespace_blocks_and_unjoinable_bases_are_left_alone() {
    // A block after `@namespace` ends the prelude; urls inside still resolve.
    assert_eq!(
        resolve("@namespace x { a { background: url(a.png) } }"),
        r#"@namespace x { a { background: url("https://example.test/css/a.png") } }"#
    );
    // Nested function arguments in the prelude are skipped as a unit.
    let source = "@namespace x foo(url(ns)); a { color: red }";
    assert_eq!(resolve(source), source);
    let data = Url::parse("data:text/css,a{}").unwrap();
    let source = "a { background: url(a.png) }";
    assert_eq!(absolutize_urls(source, &data), source);
}
