use std::collections::HashMap;

use raikiri_net::SystemHttpProvider;

use super::render_print_url;
use crate::reftest::RenderedImage;
use crate::test_http_server::{TestResponse, TestServer};

#[test]
fn rejects_zero_fallback_dimensions_before_fetching() {
    let server = TestServer::start(HashMap::<&str, TestResponse>::new());
    let provider = SystemHttpProvider::new();
    for (width, height) in [(0, 32), (32, 0), (0, 0)] {
        let error = render_print_url(&provider, server.url("index.html"), width, height)
            .unwrap_err()
            .to_string();
        assert!(error.contains("dimensions must be positive"));
    }
    assert!(server.finish().is_empty());
}

#[test]
fn invalid_utf8_document_reports_base_parse_error_and_url() {
    let server = TestServer::start(HashMap::from([(
        "/invalid.html",
        ("text/html", vec![0xff, 0xfe]),
    )]));
    let url = server.url("invalid.html");
    let error = render_print_url(&SystemHttpProvider::new(), url.clone(), 32, 32)
        .unwrap_err()
        .to_string();
    assert!(error.contains("HTML base URL parse failed"));
    assert!(error.contains(url.as_str()));
    assert_eq!(server.finish(), ["/invalid.html"]);
}

#[test]
fn prepares_background_resources_for_the_resolved_first_named_page() {
    let server = TestServer::start(HashMap::from([
        (
            "/pages/index.html",
            TestResponse::ok(
                "text/html",
                br#"<!doctype html><style>
                    @page wide {size:48px 48px;margin:0}
                    @page narrow {size:32px 32px;margin:0;background-image:url(red.png)}
                    body {display:grid;grid-template-columns:100%;grid-template-rows:auto auto;margin:0}
                </style><body>
                    <div style="grid-row:2;order:0;page:wide;height:8px">wide</div>
                    <div style="grid-row:1;order:1;page:narrow;height:8px">narrow</div>
                </body>"#.to_vec(),
            ),
        ),
        ("/pages/red.png", TestResponse::ok("image/png", red_png())),
    ]));
    let document = render_print_url(
        &SystemHttpProvider::new(),
        server.url("pages/index.html"),
        64,
        64,
    )
    .unwrap();
    assert_eq!(
        (document.pages[0].width, document.pages[0].height),
        (32, 32)
    );
    assert!(contains_red_pixel(&document.pages[0]));
    assert!(server.finish().iter().any(|path| path == "/pages/red.png"));
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
fn renders_ordered_pages_with_authored_geometry() {
    let server = TestServer::start(HashMap::from([(
        "/index.html",
        (
            "text/html",
            br#"<style>
                @page:first { size:120px 100px; margin:0 }
                @page { size:140px 110px; margin:0 }
                html,body { margin:0 }
                div { height:180px }
            </style><div>two pages</div>"#
                .to_vec(),
        ),
    )]));

    let document = render_print_url(
        &SystemHttpProvider::new(),
        server.url("index.html"),
        480,
        288,
    )
    .expect("render print document");
    server.finish();

    assert!(document.pages.len() >= 2);
    assert_eq!(
        (document.pages[0].width, document.pages[0].height),
        (120, 100)
    );
    assert_eq!(
        (document.pages[1].width, document.pages[1].height),
        (140, 110)
    );
}

#[test]
fn resolves_external_stylesheet_resources_for_print() {
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
                b"@page{size:120px 100px;margin:0}@font-face{font-family:Probe;src:url('probe.ttf')}.tile{width:8px;height:8px;background-image:url('red.png');font-family:Probe}"
                    .to_vec(),
            ),
        ),
        ("/styles/red.png", ("image/png", red_png())),
        (
            "/styles/probe.ttf",
            ("font/ttf", b"not-a-real-font".to_vec()),
        ),
    ]));

    render_print_url(
        &SystemHttpProvider::new(),
        server.url("pages/index.html"),
        480,
        288,
    )
    .expect("render external print resources");
    let requests = server.finish();

    assert!(requests.iter().any(|path| path == "/styles/sheet.css"));
    assert!(requests.iter().any(|path| path == "/styles/red.png"));
    assert!(requests.iter().any(|path| path == "/styles/probe.ttf"));
}

#[test]
fn uses_effective_document_base_for_print_resources() {
    let server = TestServer::start(HashMap::from([
        (
            "/start.html",
            TestResponse::redirect("/pages/index.html"),
        ),
        (
            "/pages/index.html",
            TestResponse::ok(
                "text/html",
                b"<base href='/assets/'><style>@page{size:120px 100px;margin:0}img{width:8px;height:8px}</style><img src='image.png'>"
                    .to_vec(),
            ),
        ),
        (
            "/assets/image.png",
            TestResponse::ok("image/png", red_png()),
        ),
    ]));

    render_print_url(
        &SystemHttpProvider::new(),
        server.url("start.html"),
        480,
        288,
    )
    .expect("render redirected print document");
    let requests = server.finish();

    assert!(requests.iter().any(|path| path == "/assets/image.png"));
}

#[test]
fn paints_relative_img_from_the_effective_document_base() {
    let server = TestServer::start(HashMap::from([
        (
            "/pages/index.html",
            TestResponse::ok(
                "text/html",
                b"<base href='/assets/'><style>@page:first{size:32px 32px;margin:0}@page{size:36px 36px;margin:0}html,body{margin:0}img{display:block;width:8px;height:8px}.spacer{height:60px}</style><img src='red.png'><div class='spacer'></div>"
                    .to_vec(),
            ),
        ),
        (
            "/assets/red.png",
            TestResponse::ok("image/png", red_png()),
        ),
    ]));

    let document = render_print_url(
        &SystemHttpProvider::new(),
        server.url("pages/index.html"),
        32,
        32,
    )
    .expect("render relative image");
    let requests = server.finish();

    assert!(requests.iter().any(|path| path == "/assets/red.png"));
    assert!(contains_red_pixel(&document.pages[0]));
}

#[test]
fn resolves_relative_base_once_for_external_stylesheets() {
    let server = TestServer::start(HashMap::from([
        (
            "/pages/index.html",
            TestResponse::ok(
                "text/html",
                b"<base href='assets/'><link rel='stylesheet' href='sheet.css'><p>probe</p>"
                    .to_vec(),
            ),
        ),
        (
            "/pages/assets/sheet.css",
            TestResponse::ok("text/css", b"@page{size:32px 32px;margin:0}".to_vec()),
        ),
    ]));

    render_print_url(
        &SystemHttpProvider::new(),
        server.url("pages/index.html"),
        32,
        32,
    )
    .expect("render stylesheet with relative base");
    let requests = server.finish();

    assert!(
        requests
            .iter()
            .any(|path| path == "/pages/assets/sheet.css")
    );
    assert!(!requests.iter().any(|path| path.contains("assets/assets")));
}

#[test]
fn paints_relative_page_background_image() {
    let server = TestServer::start(HashMap::from([
        (
            "/pages/index.html",
            TestResponse::ok(
                "text/html",
                b"<style>@page{size:32px 32px;margin:0;background-image:url('red.png')}</style>"
                    .to_vec(),
            ),
        ),
        ("/pages/red.png", TestResponse::ok("image/png", red_png())),
    ]));

    let document = render_print_url(
        &SystemHttpProvider::new(),
        server.url("pages/index.html"),
        32,
        32,
    )
    .expect("render page background image");
    let requests = server.finish();

    assert!(requests.iter().any(|path| path == "/pages/red.png"));
    assert!(contains_red_pixel(&document.pages[0]));
}

#[test]
fn paints_relative_page_margin_box_background_image() {
    let server = TestServer::start(HashMap::from([
        (
            "/pages/index.html",
            TestResponse::ok(
                "text/html",
                b"<style>@page{size:64px 64px;margin:16px;@top-left{content:'x';background-image:url('red.png')}}</style>"
                    .to_vec(),
            ),
        ),
        ("/pages/red.png", TestResponse::ok("image/png", red_png())),
    ]));

    let document = render_print_url(
        &SystemHttpProvider::new(),
        server.url("pages/index.html"),
        64,
        64,
    )
    .expect("render margin box background image");
    let requests = server.finish();

    assert!(requests.iter().any(|path| path == "/pages/red.png"));
    assert!(contains_red_pixel(&document.pages[0]));
}
