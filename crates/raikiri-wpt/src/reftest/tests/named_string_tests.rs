use super::*;

const THREE: &str = r#"<h2>Introduction</h2><p>A</p><p style="break-before:page">B</p><h2 style="break-before:page">Background</h2><p>C</p>"#;
const FIVE: &str = r#"<h2>A1</h2><p>x</p><h2>A2</h2><p>x</p><p style="break-before:page">y</p><h2 style="break-before:page">C1</h2><p>z</p><h2>C2</h2><p>z</p><p style="break-before:page">w</p><h2>D1</h2><p style="break-before:page">v</p>"#;

/// The family the pages of [`render`] are drawn in: Ahem from the repository,
/// so that the pages do not depend on the host's fonts or on whether a WPT
/// checkout, whose `fonts/` folder would otherwise supply the generic
/// families, sits next to the build.
const FAMILY: &str = "NamedStringTest";

/// A directory laid out as the `fonts/` folder of a WPT checkout, holding the
/// Ahem font that [`FAMILY`] names; the font loader recognises a checkout by
/// that file.
fn font_root() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    let fonts = root.path().join("fonts");
    std::fs::create_dir(&fonts).unwrap();
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../raikiri-dom/tests/data/text-autospace/Ahem.ttf"),
        fonts.join("Ahem.ttf"),
    )
    .unwrap();
    root
}

fn render(body: &str, contents: [&str; 4], extra: &str, images: bool) -> RenderedDocument {
    let assignment = if contents.iter().any(|value| value.contains("string(")) {
        "h2{string-set:title content(text)}"
    } else {
        ""
    };
    let slots = ["top-left", "top-center", "top-right", "bottom-left"];
    let rules = slots
        .into_iter()
        .zip(contents)
        .map(|(slot, value)| format!("@{slot}{{content:{value};font:10px {FAMILY}}} "))
        .collect::<String>();
    let html = format!(
        "<!doctype html><style>@font-face{{font-family:{FAMILY};src:url(/fonts/Ahem.ttf)}}@page{{size:300px 200px;margin:40px;{rules}}}body{{margin:0;font:10px/10px {FAMILY}}}p,h2{{margin:0}}{assignment}{extra}</style>{body}"
    );
    let base = Path::new(env!("CARGO_MANIFEST_DIR"));
    let fonts = font_root();
    render_raikiri_pages_inner(&html, 800, 600, images.then_some(base), Some(fonts.path())).unwrap()
}

fn margin_pixels(image: &RenderedImage) -> Vec<u8> {
    let row = image.width as usize * 4;
    let mut result = image.rgba[..40 * row].to_vec();
    result.extend_from_slice(&image.rgba[(image.height as usize - 40) * row..]);
    result
}

fn body_pixels(image: &RenderedImage) -> &[u8] {
    let row = image.width as usize * 4;
    &image.rgba[40 * row..(image.height as usize - 40) * row]
}

#[test]
fn builtin_print_named_strings_match_literal_headers_on_three_pages() {
    for images in [false, true] {
        let actual = render(THREE, ["string(title)", "none", "none", "none"], "", images);
        assert_eq!(actual.pages.len(), 3);
        for (index, text) in ["Introduction", "Introduction", "Background"]
            .into_iter()
            .enumerate()
        {
            let literal = format!("'{text}'");
            let expected = render(THREE, [&literal, "none", "none", "none"], "", images);
            assert!(
                margin_pixels(&actual.pages[index]) == margin_pixels(&expected.pages[index]),
                "margin pixels differ on page {index}"
            );
            assert!(body_pixels(&actual.pages[index]) == body_pixels(&expected.pages[index]));
        }
    }
}

#[test]
fn builtin_print_named_strings_match_four_literal_modes_on_five_pages() {
    for images in [false, true] {
        let actual = render(
            FIVE,
            [
                "'F:' string(title,first)",
                "'S:' string(title,start)",
                "'L:' string(title,last)",
                "'X:' string(title,first-except)",
            ],
            "",
            images,
        );
        assert_eq!(actual.pages.len(), 5);
        for (index, texts) in [
            ["F:A1", "S:A1", "L:A2", "X:"],
            ["F:A2", "S:A2", "L:A2", "X:A2"],
            ["F:C1", "S:C1", "L:C2", "X:"],
            ["F:D1", "S:C2", "L:D1", "X:"],
            ["F:D1", "S:D1", "L:D1", "X:D1"],
        ]
        .into_iter()
        .enumerate()
        {
            let literals = texts.map(|text| format!("'{text}'"));
            let expected = render(FIVE, literals.each_ref().map(String::as_str), "", images);
            assert!(
                margin_pixels(&actual.pages[index]) == margin_pixels(&expected.pages[index]),
                "margin pixels differ on page {index}"
            );
            assert!(body_pixels(&actual.pages[index]) == body_pixels(&expected.pages[index]));
        }
    }
}

#[test]
fn builtin_print_named_strings_use_actual_width_continuations() {
    let body = format!(
        "<h2>ALPHA</h2><section style='page:narrow'><p>{}</p><h2>BETA</h2></section>",
        "<span class='atom'></span>".repeat(27)
    );
    let extra = "@page narrow{size:180px 200px}h2{font:10px/10px NamedStringTest}p{font-size:0}.atom{display:inline-block;width:40px;height:10px;vertical-align:top;background:green}";
    let actual = render(
        &body,
        [
            "'F:' string(title,first)",
            "'S:' string(title,start)",
            "'L:' string(title,last)",
            "'X:' string(title,first-except)",
        ],
        extra,
        false,
    );
    assert_eq!(
        actual
            .pages
            .iter()
            .map(|page| page.width)
            .collect::<Vec<_>>(),
        [300, 180, 180]
    );
    for (index, texts) in [
        ["F:ALPHA", "S:ALPHA", "L:ALPHA", "X:"],
        ["F:ALPHA", "S:ALPHA", "L:ALPHA", "X:ALPHA"],
        ["F:BETA", "S:ALPHA", "L:BETA", "X:"],
    ]
    .into_iter()
    .enumerate()
    {
        let literals = texts.map(|text| format!("'{text}'"));
        let expected = render(&body, literals.each_ref().map(String::as_str), extra, false);
        assert!(
            margin_pixels(&actual.pages[index]) == margin_pixels(&expected.pages[index]),
            "margin pixels differ on page {index}"
        );
        assert!(body_pixels(&actual.pages[index]) == body_pixels(&expected.pages[index]));
    }
}

#[test]
fn builtin_print_named_strings_use_height_only_page_geometry() {
    let extra = "@page :first {size:300px 220px;padding:5px}";
    let actual = render(
        THREE,
        ["string(title)", "none", "none", "none"],
        extra,
        false,
    );
    assert_eq!(
        actual
            .pages
            .iter()
            .map(|page| page.height)
            .collect::<Vec<_>>(),
        [220, 200, 200]
    );
    for (index, text) in ["Introduction", "Introduction", "Background"]
        .into_iter()
        .enumerate()
    {
        let literal = format!("'{text}'");
        let expected = render(THREE, [&literal, "none", "none", "none"], extra, false);
        assert!(margin_pixels(&actual.pages[index]) == margin_pixels(&expected.pages[index]));
        assert!(body_pixels(&actual.pages[index]) == body_pixels(&expected.pages[index]));
    }
}

#[test]
fn builtin_print_named_strings_projection_preserves_generated_marker_and_column_paint() {
    let body = "<h2>ALPHA</h2><ul><li>item</li></ul><div class='columns'><p>one</p><p>two</p></div><h2 style='break-before:page'>BETA</h2>";
    let extra = "p::before{content:'P'}ul{margin:0;padding-left:20px}.columns{columns:2;height:20px;column-rule:1px solid green}";
    let actual = render(
        body,
        ["string(title)", "none", "none", "none"],
        extra,
        false,
    );
    assert_eq!(actual.pages.len(), 2);
    for (index, text) in ["ALPHA", "BETA"].into_iter().enumerate() {
        let literal = format!("'{text}'");
        let expected = render(body, [&literal, "none", "none", "none"], extra, false);
        assert!(actual.pages[index].rgba == expected.pages[index].rgba);
    }
}

#[test]
fn builtin_print_named_strings_without_margin_boxes_keep_the_body_unchanged() {
    let render = |assignment: &str| {
        let html = format!(
            "<!doctype html><style>@page{{size:300px 200px;margin:40px}}body{{margin:0;font:10px/10px monospace}}h2{{margin:0;{assignment}}}</style><h2>ALPHA</h2><h2 style='break-before:page'>BETA</h2>"
        );
        render_raikiri_pages(&html, 800, 600).unwrap()
    };
    let actual = render("string-set:title content(text)");
    let expected = render("");
    assert_eq!(actual.pages.len(), 2);
    for (actual, expected) in actual.pages.iter().zip(&expected.pages) {
        assert!(actual.rgba == expected.rgba);
    }
}

#[test]
fn contents_named_strings_builtin_print_width_continuation() {
    let body = "<body><span style='display:contents'></span><h2>A1</h2><section style='page:narrow'><span style='display:contents'></span><h2>B1</h2><h2>B2</h2><p style='break-before:page'>tail</p></section></body>";
    let extra = "@page narrow{size:180px 200px}";
    for images in [false, true] {
        let actual = render(
            body,
            [
                "'F:' string(title,first)",
                "'S:' string(title,start)",
                "'L:' string(title,last)",
                "'X:' string(title,first-except)",
            ],
            extra,
            images,
        );
        assert_eq!(
            actual
                .pages
                .iter()
                .map(|page| page.width)
                .collect::<Vec<_>>(),
            [300, 180, 180]
        );
        for (index, values) in [
            ["F:A1", "S:A1", "L:A1", "X:"],
            ["F:B1", "S:B1", "L:B2", "X:"],
            ["F:B2", "S:B2", "L:B2", "X:B2"],
        ]
        .into_iter()
        .enumerate()
        {
            let literals = values.map(|text| format!("'{text}'"));
            let expected = render(body, literals.each_ref().map(String::as_str), extra, images);
            assert!(
                margin_pixels(&actual.pages[index]) == margin_pixels(&expected.pages[index]),
                "margin pixels differ page {index}"
            );
            assert!(body_pixels(&actual.pages[index]) == body_pixels(&expected.pages[index]));
        }
    }
}
