use super::*;

use std::collections::HashMap;

use crate::test_http_server::TestServer;

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
