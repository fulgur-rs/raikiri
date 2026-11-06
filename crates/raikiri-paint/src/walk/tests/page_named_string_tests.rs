use super::*;

const THREE: &str = r#"<h2>Introduction</h2><p>A</p><p style="break-before:page">B</p><h2 style="break-before:page">Background</h2><p>C</p>"#;
const FIVE: &str = r#"<h2>A1</h2><p>x</p><h2>A2</h2><p>x</p><p style="break-before:page">y</p><h2 style="break-before:page">C1</h2><p>z</p><h2>C2</h2><p>z</p><p style="break-before:page">w</p><h2>D1</h2><p style="break-before:page">v</p>"#;

fn fixture(body: &str, rule: &str, paged: bool) -> (Document, CascadeResult, PageBox, u32) {
    let html = format!(
        "<!doctype html><style>@page{{size:300px 200px;margin:40px;@top-left{{content:{rule};font:10px Ahem}}}}body{{margin:0;font:10px/10px Ahem}}p,h2{{margin:0}}h2{{string-set:title content(text)}}</style>{body}"
    );
    let mut parsed =
        raikiri_html::parse(html.as_bytes(), &raikiri_html::ParseOptions::new()).unwrap();
    parsed.dom.set_font_collection(
        raikiri_dom::build_wpt_font_collection(std::path::Path::new(FONT_DIR)).unwrap(),
    );
    let cascade = raikiri_html::build_cascaded(&parsed).unwrap();
    let mut page = PageBox::A4;
    page.width = 300.0;
    page.height = 200.0;
    let count = if paged {
        let slices = raikiri_dom::layout_pages(&mut parsed.dom, &cascade, page).unwrap();
        let geometry = (
            page,
            raikiri_dom::page_margins_for_page(&cascade.page, page),
            raikiri_dom::page_content_insets_for_page(&cascade.page, page),
        );
        parsed
            .dom
            .project_pages(&cascade, page, &slices, &vec![geometry; slices.len()])
            .unwrap();
        slices.len() as u32
    } else {
        raikiri_dom::layout_single_page(&mut parsed.dom, &cascade, page).unwrap();
        1
    };
    (parsed.dom, cascade, page, count)
}

fn texts(body: &str, rule: &str, paged: bool) -> Vec<String> {
    let (document, cascade, page, count) = fixture(body, rule, paged);
    (0..count)
        .map(|index| {
            layout_page_margin_boxes(
                &document,
                &cascade,
                page,
                index,
                count,
                index % 2 == 1,
                None,
                None,
            )
            .into_iter()
            .map(|margin| margin.content)
            .collect::<String>()
        })
        .collect()
}

#[test]
fn builtin_margin_named_strings_follow_three_page_assignments() {
    assert_eq!(
        texts(THREE, "string(title)", true),
        ["Introduction", "Introduction", "Background"]
    );
}

#[test]
fn builtin_margin_named_strings_select_all_four_modes_on_five_pages() {
    for (mode, expected) in [
        ("first", ["A1", "A2", "C1", "D1", "D1"]),
        ("start", ["A1", "A2", "C1", "C2", "D1"]),
        ("last", ["A2", "A2", "C2", "D1", "D1"]),
        ("first-except", ["", "A2", "", "", "D1"]),
    ] {
        assert_eq!(
            texts(FIVE, &format!("string(title,{mode})"), true),
            expected
        );
    }
}

#[test]
fn builtin_margin_named_strings_use_real_unfragmented_single_page_order() {
    for (mode, expected) in [
        ("first", "A1"),
        ("start", ""),
        ("last", "A2"),
        ("first-except", ""),
    ] {
        assert_eq!(
            texts(
                "<p>before</p><h2>A1</h2><h2>A2</h2>",
                &format!("string(title,{mode})"),
                false
            ),
            [expected]
        );
    }
}

struct EmptyPixels;
impl raikiri_traits::ImagePixelSource for EmptyPixels {
    fn get_decoded(&self, _url: &url::Url) -> Option<std::sync::Arc<raikiri_traits::DecodedImage>> {
        None
    }
}

fn legacy_margin_glyphs(rule: &str, images: bool) -> Vec<String> {
    let (document, cascade, page, _) = fixture("<p>before</p><h2>A1</h2><h2>A2</h2>", rule, false);
    let mut scene = Scene::new();
    if images {
        crate::paint_single_page_with_images(&mut scene, &document, &cascade, page, &EmptyPixels)
            .unwrap();
    } else {
        crate::paint_single_page_with_origin(&mut scene, &document, &cascade, page, 0.0).unwrap();
    }
    scene
        .commands
        .iter()
        .filter_map(|command| match command {
            RenderCommand::GlyphRun(run) if run.transform.translation().y < 40.0 => {
                Some(format!("{:?} {:?}", run.transform, run.glyphs))
            }
            _ => None,
        })
        .collect()
}

#[test]
fn builtin_margin_named_strings_reach_legacy_image_and_plain_painters() {
    for images in [false, true] {
        for (mode, expected) in [
            ("first", "A1"),
            ("start", ""),
            ("last", "A2"),
            ("first-except", ""),
        ] {
            assert_eq!(
                legacy_margin_glyphs(&format!("'V:' string(title,{mode})"), images),
                legacy_margin_glyphs(&format!("'V:{expected}'"), images)
            );
        }
    }
}

#[test]
fn builtin_margin_named_strings_start_ignores_leading_html_whitespace() {
    assert_eq!(
        texts("\n   <h2>A1</h2><h2>A2</h2>", "string(title,start)", false),
        ["A1"]
    );
}
