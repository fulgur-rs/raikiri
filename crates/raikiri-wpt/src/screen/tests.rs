use super::*;

use std::collections::HashMap;

use crate::test_http_server::TestServer;

#[test]
fn viewport_units_keep_class_and_variable_names_and_resolve_inline_styles() {
    let server = TestServer::start(HashMap::from([
        ("/names.html", ("text/html", b"<style>html,body{margin:0}.foo100vw{--size100vw:100vw;width:var(--size100vw);height:100vh;background:green}</style><div class=foo100vw></div>".to_vec())),
        ("/inline.html", ("text/html", b"<style>html,body{margin:0}</style><div style='width:100vw;height:100vh;background:green'></div>".to_vec())),
    ]));
    let pixels: Vec<_> = ["names.html", "inline.html"]
        .into_iter()
        .map(|page| {
            let image =
                render_screen_url(&SystemHttpProvider::new(), server.url(page), 32, 24).unwrap();
            let offset = (23 * 32 + 31) * 4;
            image.rgba[offset..offset + 4].to_vec()
        })
        .collect();
    assert_eq!(pixels, [vec![0, 128, 0, 255], vec![0, 128, 0, 255]]);
    assert_eq!(server.finish(), ["/names.html", "/inline.html"]);
}

#[test]
fn rejects_an_oversized_viewport_before_fetching_the_document() {
    let error = match render_screen_url(
        &SystemHttpProvider::new(),
        raikiri::Url::parse("http://127.0.0.1:1/index.html").unwrap(),
        raikiri::MAX_RASTER_EDGE + 1,
        1,
    ) {
        Err(error) => error,
        Ok(image) => panic!(
            "oversized screen viewport unexpectedly rendered ({} bytes)",
            image.rgba.len()
        ),
    };
    let source = std::error::Error::source(&error).expect("raster error source");
    assert!(matches!(
        source.downcast_ref::<raikiri::RenderError>(),
        Some(raikiri::RenderError::LimitExceeded {
            kind: raikiri::LimitKind::RasterEdge,
            ..
        })
    ));
}

#[test]
fn invalid_utf8_document_reports_url_and_declared_encoding() {
    let server = TestServer::start(HashMap::from([(
        "/invalid.html",
        ("text/html; charset=utf-8", vec![0xff, 0xfe]),
    )]));
    let url = server.url("invalid.html");
    let error = render_screen_url(&SystemHttpProvider::new(), url.clone(), 32, 32)
        .unwrap_err()
        .to_string();
    assert!(error.contains("HTML parse failed"));
    assert!(error.contains(url.as_str()));
    assert!(error.contains("declared encoding: utf-8"));
    assert_eq!(server.finish(), ["/invalid.html"]);
}

fn red_png() -> Vec<u8> {
    let mut output = Vec::new();
    let mut encoder = png::Encoder::new(&mut output, 2, 2);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().expect("PNG header");
    writer
        .write_image_data(&[
            255, 0, 0, 255, 255, 0, 0, 255, 255, 0, 0, 255, 255, 0, 0, 255,
        ])
        .expect("PNG pixels");
    writer.finish().expect("finish PNG");
    output
}

fn contains_red_pixel(image: &RenderedImage) -> bool {
    image
        .rgba
        .chunks_exact(4)
        .any(|pixel| pixel[0] > 200 && pixel[1] < 50 && pixel[2] < 50 && pixel[3] > 200)
}

#[test]
fn renders_a_relative_img_resource_from_the_document_base_url() {
    let server = TestServer::start(HashMap::from([
        (
            "/index.html",
            (
                "text/html",
                b"<style>html,body{margin:0}img{display:block;width:8px;height:8px}</style><img src='red.png'>".to_vec(),
            ),
        ),
        ("/red.png", ("image/png", red_png())),
    ]));
    let provider = SystemHttpProvider::new();

    let image = render_screen_url(&provider, server.url("index.html"), 32, 32)
        .expect("render relative image");
    let requests = server.finish();

    assert!(requests.iter().any(|path| path == "/red.png"));
    assert!(contains_red_pixel(&image));
}

#[test]
fn renders_a_relative_css_background_from_the_document_base_url() {
    let server = TestServer::start(HashMap::from([
        (
            "/index.html",
            (
                "text/html",
                b"<style>html,body{margin:0}.tile{width:8px;height:8px;background-image:url('red.png')}</style><div class='tile'></div>".to_vec(),
            ),
        ),
        ("/red.png", ("image/png", red_png())),
    ]));
    let provider = SystemHttpProvider::new();

    let image = render_screen_url(&provider, server.url("index.html"), 32, 32)
        .expect("render relative background");
    let requests = server.finish();

    assert!(requests.iter().any(|path| path == "/red.png"));
    assert!(contains_red_pixel(&image));
}

#[test]
fn fetches_relative_font_face_sources_from_the_document_base_url() {
    let server = TestServer::start(HashMap::from([
        (
            "/index.html",
            (
                "text/html",
                b"<style>@font-face{font-family:Probe;src:url('probe.ttf')}body{font-family:Probe}</style><body>probe</body>".to_vec(),
            ),
        ),
        ("/probe.ttf", ("font/ttf", b"not-a-real-font".to_vec())),
    ]));
    let provider = SystemHttpProvider::new();

    render_screen_url(&provider, server.url("index.html"), 64, 32)
        .expect("render document with font-face");
    let requests = server.finish();

    assert!(requests.iter().any(|path| path == "/probe.ttf"));
}

#[test]
fn resolves_external_stylesheet_resources_from_the_stylesheet_url() {
    let server = TestServer::start(HashMap::from([
        (
            "/pages/index.html",
            (
                "text/html",
                b"<link rel='stylesheet' href='../styles/sheet.css'><div class='tile'>probe</div>"
                    .to_vec(),
            ),
        ),
        (
            "/styles/sheet.css",
            (
                "text/css",
                b"@font-face{font-family:Probe;src:url('probe.ttf')}.tile{width:8px;height:8px;background-image:url('red.png');font-family:Probe}"
                    .to_vec(),
            ),
        ),
        ("/styles/red.png", ("image/png", red_png())),
        (
            "/styles/probe.ttf",
            ("font/ttf", b"not-a-real-font".to_vec()),
        ),
    ]));
    let provider = SystemHttpProvider::new();

    let image = render_screen_url(&provider, server.url("pages/index.html"), 32, 32)
        .expect("render resources relative to external stylesheet");
    let requests = server.finish();

    assert!(requests.iter().any(|path| path == "/styles/red.png"));
    assert!(requests.iter().any(|path| path == "/styles/probe.ttf"));
    assert!(contains_red_pixel(&image));
}

#[test]
fn media_queries_use_the_requested_screen_viewport() {
    let server = TestServer::start(HashMap::from([(
        "/index.html",
        ("text/html", b"<style>html,body{margin:0}div{width:8px;height:8px;background:red}@media screen and (width:32px) and (height:24px){div{background:green}}</style><div></div>".to_vec()),
    )]));
    let image =
        render_screen_url(&SystemHttpProvider::new(), server.url("index.html"), 32, 24).unwrap();
    assert!(!contains_red_pixel(&image));
    assert_eq!(&image.rgba[..4], &[0, 128, 0, 255]);
    assert_eq!(server.finish(), ["/index.html"]);
}

#[test]
fn media_viewport_units_use_requested_screen_size_in_inline_and_external_css() {
    let server = TestServer::start(HashMap::from([
        ("/inline.html", ("text/html", b"<style>html,body{margin:0}div{width:8px;height:8px;background:red}@media screen and (width:100vw) and (height:100vh){div{background:green}}</style><div></div>".to_vec())),
        ("/external.html", ("text/html", b"<link rel=stylesheet href=sheet.css><div></div>".to_vec())),
        ("/sheet.css", ("text/css", b"html,body{margin:0}div{width:8px;height:8px;background:red}@media screen and (width:100vw) and (height:100vh){div{background:green}}".to_vec())),
        ("/attribute.html", ("text/html", b"<style>html,body{margin:0}div{width:8px;height:8px;background:red}</style><style media='screen and (width:100vw) and (height:100vh)'>div{background:green}</style><div></div>".to_vec())),
        ("/import.html", ("text/html", b"<style>@import url(imported.css) screen and (width:100vw) and (height:100vh);</style><div></div>".to_vec())),
        ("/imported.css", ("text/css", b"html,body{margin:0}div{width:8px;height:8px;background:green}".to_vec())),
    ]));
    for page in [
        "inline.html",
        "external.html",
        "attribute.html",
        "import.html",
    ] {
        let image =
            render_screen_url(&SystemHttpProvider::new(), server.url(page), 32, 24).unwrap();
        assert_eq!(&image.rgba[..4], &[0, 128, 0, 255], "{page}");
    }
    assert_eq!(
        server.finish(),
        [
            "/inline.html",
            "/external.html",
            "/sheet.css",
            "/attribute.html",
            "/import.html",
            "/imported.css"
        ]
    );
}
