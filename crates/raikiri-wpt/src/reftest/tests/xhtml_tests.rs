use super::*;

#[test]
fn xhtml_self_closing_flex_items_match_an_html_reference() {
    let dir = tempfile::tempdir().unwrap();
    let source = r#"<?xml version="1.0"?><html xmlns="http://www.w3.org/1999/xhtml"><head><link rel="match" href="ref.html"/><style>html,body{margin:0}.flex{display:flex;flex-direction:column;height:40px;float:left}.item{width:20px;height:20px;background:green}</style></head><body><div class="flex"><div class="item"/><div class="item"/></div></body></html>"#;
    let reference = "<!doctype html><style>html,body{margin:0}</style><div style='width:20px;height:40px;background:green'></div>";
    let path = dir.path().join("test.xhtml");
    std::fs::write(&path, source).unwrap();
    std::fs::write(dir.path().join("ref.html"), reference).unwrap();
    let pairs = discover_pairs_for_file(&path).unwrap();
    let result = run_pair(&pairs[0], ReftestConfig::default()).unwrap();
    assert_eq!(result.mismatched_pixels, 0);
}

#[test]
fn malformed_xhtml_is_repaired_by_the_xml_parser() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("broken.xhtml");
    // The unclosed `<p>` is repaired at the end tag of its parent, and the
    // self-closing `<div/>` elements stay siblings as they would in XML
    // (HTML recovery would nest the second one inside the first).
    std::fs::write(
        &path,
        r#"<html xmlns="http://www.w3.org/1999/xhtml"><head><style>@page{size:80px 60px;margin:0}html,body{margin:0}div{width:20px;height:20px;background:green}</style></head><body><div/><div/><p></body></html>"#,
    )
    .unwrap();
    let document = render_raikiri_file_pages(&path, 80, 60).unwrap();
    let page = &document.pages[0];
    for top in [10, 30] {
        let offset = (top * page.width as usize + 10) * 4;
        assert_eq!(&page.rgba[offset..offset + 4], &[0, 128, 0, 255]);
    }
}

#[test]
fn xhtml_geometry_reparse_keeps_self_closing_siblings() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("pages.xhtml");
    std::fs::write(&path, r#"<html xmlns="http://www.w3.org/1999/xhtml"><head><style>@page{size:200px 80px;margin:0}@page:first{size:100px 80px}html,body{margin:0}div{height:60px;background:green}div+div{break-before:page}</style></head><body><div/><div/></body></html>"#).unwrap();
    let document = render_raikiri_file_pages(&path, 800, 600).unwrap();
    assert_eq!(document.pages.len(), 2);
    assert_eq!(
        (document.pages[0].width, document.pages[1].width),
        (100, 200)
    );
    for page in &document.pages {
        let offset = (10 * page.width as usize + 10) * 4;
        assert_eq!(&page.rgba[offset..offset + 4], &[0, 128, 0, 255]);
    }
}

#[test]
fn xhtml_reference_accepts_mirrored_author_page_margin() {
    let dir = tempfile::tempdir().unwrap();
    let source = r#"<html xmlns="http://www.w3.org/1999/xhtml"><head><link rel="match" href="ref.xhtml"/><style>@page{size:200px 160px}html,body{margin:0}div{width:20px;height:20px;background:green}</style></head><body><div/></body></html>"#;
    let reference = r#"<?xml version="1.0"?><html xmlns="http://www.w3.org/1999/xhtml"><head><style>html,body{margin:0}div{width:20px;height:20px;background:green}</style></head><body><div/></body></html>"#;
    let path = dir.path().join("test.xhtml");
    std::fs::write(&path, source).unwrap();
    std::fs::write(dir.path().join("ref.xhtml"), reference).unwrap();
    let config = ReftestConfig {
        width: 200,
        height: 160,
        ..Default::default()
    };
    let result = run_pair(&discover_pairs_for_file(&path).unwrap()[0], config).unwrap();
    assert_eq!(result.mismatched_pixels, 0);
}

#[test]
fn dynamic_xhtml_uses_xml_initially_and_html_after_serialization() {
    let dir = tempfile::tempdir().unwrap();
    let source = r#"<html xmlns="http://www.w3.org/1999/xhtml"><head><link rel="match" href="ref.html"/></head><body style="margin:0"><div id="a"/><div id="b"/><script><![CDATA[if (document.getElementById('a').parentNode !== document.getElementById('b').parentNode) {throw new Error('nested');} document.body.style.background='green';]]></script></body></html>"#;
    let path = dir.path().join("script.xhtml");
    std::fs::write(&path, source).unwrap();
    std::fs::write(
        dir.path().join("ref.html"),
        "<!doctype html><body style='margin:0;background:green'></body>",
    )
    .unwrap();
    let prepared = dynamic::prepare(source, &path, "", ReftestConfig::default()).unwrap();
    assert_eq!(prepared.syntax, DocumentSyntax::Html);
    let result = run_pair(
        &discover_pairs_for_file(&path).unwrap()[0],
        ReftestConfig::default(),
    )
    .unwrap();
    assert_eq!(result.mismatched_pixels, 0);
}

#[test]
fn http_screen_uses_xhtml_mime_type_instead_of_the_url_extension() {
    use crate::test_http_server::{TestResponse, TestServer};
    use std::collections::HashMap;
    let source = br#"<html xmlns="http://www.w3.org/1999/xhtml"><head><style>html,body{margin:0}div{width:20px;height:20px}#a{background:green}#b{background:red}</style></head><body><div id="a"/><div id="b"/></body></html>"#;
    let server = TestServer::start(HashMap::from([
        (
            "/xml.html",
            TestResponse::ok("application/xhtml+xml; charset=utf-8", source.to_vec()),
        ),
        (
            "/html.xhtml",
            TestResponse::ok("text/html", source.to_vec()),
        ),
    ]));
    for (path, top) in [
        ("xml.html", [0, 128, 0, 255]),
        ("html.xhtml", [255, 0, 0, 255]),
    ] {
        let image = crate::render_screen_url(
            &raikiri_net::SystemHttpProvider::new(),
            server.url(path),
            40,
            40,
        )
        .unwrap();
        let offset = (10 * 40 + 10) * 4;
        assert_eq!(&image.rgba[offset..offset + 4], &top);
    }
    assert_eq!(server.finish(), ["/xml.html", "/html.xhtml"]);
}

#[test]
fn http_print_uses_xhtml_for_base_probe_and_rendering() {
    use crate::test_http_server::{TestResponse, TestServer};
    use std::collections::HashMap;
    let source = br#"<html xmlns="http://www.w3.org/1999/xhtml"><head><style>@page{size:40px 40px;margin:0}html,body{margin:0}div{width:20px;height:20px}#a{background:green}#b{background:red}</style></head><body><div id="a"/><div id="b"/></body></html>"#;
    let server = TestServer::start(HashMap::from([(
        "/xml.html",
        TestResponse::ok("application/xhtml+xml", source.to_vec()),
    )]));
    let document = crate::render_print_url(
        &raikiri_net::SystemHttpProvider::new(),
        server.url("xml.html"),
        800,
        600,
    )
    .unwrap();
    assert_eq!(document.pages.len(), 1);
    let offset = (10 * 40 + 10) * 4;
    assert_eq!(
        &document.pages[0].rgba[offset..offset + 4],
        &[0, 128, 0, 255]
    );
    assert_eq!(server.finish(), ["/xml.html"]);
}

#[test]
fn legacy_xml_extensions_keep_the_existing_html_compatibility_path() {
    let dir = tempfile::tempdir().unwrap();
    let source = "<html><head><title>border&ndash;radius</title></head><body></body></html>";
    for extension in ["xht", "xml"] {
        let path = dir.path().join(format!("legacy.{extension}"));
        std::fs::write(&path, source).unwrap();
        assert!(render_raikiri_file_pages(&path, 80, 60).is_ok());
    }
}
