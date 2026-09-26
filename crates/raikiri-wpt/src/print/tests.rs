use std::collections::HashMap;

use raikiri_net::SystemHttpProvider;

use super::render_print_url;
use crate::test_http_server::{TestResponse, TestServer};

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
