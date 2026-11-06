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

#[derive(Clone, Copy)]
enum LegacyRoute {
    Plain,
    Images,
    NamedImagesAndHighlights,
}

fn legacy_margin_glyphs(rule: &str, route: LegacyRoute) -> Vec<String> {
    legacy_margin_glyphs_for_body("<p>before</p><h2>A1</h2><h2>A2</h2>", rule, route)
}

fn legacy_margin_glyphs_for_body(body: &str, rule: &str, route: LegacyRoute) -> Vec<String> {
    let (document, cascade, page, _) = fixture(body, rule, false);
    let mut scene = Scene::new();
    match route {
        LegacyRoute::Images => {
            crate::paint_single_page_with_images(
                &mut scene,
                &document,
                &cascade,
                page,
                &EmptyPixels,
            )
            .unwrap();
        }
        LegacyRoute::Plain => {
            crate::paint_single_page_with_origin(&mut scene, &document, &cascade, page, 0.0)
                .unwrap();
        }
        LegacyRoute::NamedImagesAndHighlights => {
            crate::paint_single_page_with_origin_and_page_context_named_with_fixed_page_width_and_images_and_highlights(
                &mut scene, &document, &cascade, page, 0.0, 0, 1, false, None,
                None, f32::NAN, &EmptyPixels, &[],
            ).unwrap();
        }
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
    for route in [
        LegacyRoute::Plain,
        LegacyRoute::Images,
        LegacyRoute::NamedImagesAndHighlights,
    ] {
        for (mode, expected) in [
            ("first", "A1"),
            ("start", ""),
            ("last", "A2"),
            ("first-except", ""),
        ] {
            assert_eq!(
                legacy_margin_glyphs(&format!("'V:' string(title,{mode})"), route),
                legacy_margin_glyphs(&format!("'V:{expected}'"), route)
            );
        }
    }
}

#[test]
fn builtin_margin_named_strings_start_ignores_leading_html_whitespace() {
    for prefix in ["\n   ", "\t", ""] {
        let body = format!("<body>{prefix}<h2>A1</h2><h2>A2</h2></body>");
        for paged in [true, false] {
            assert_eq!(
                texts(&body, "string(title,start)", paged),
                ["A1"],
                "paged={paged}, prefix={prefix:?}"
            );
        }
    }
}

#[test]
fn contents_named_strings_empty_wrappers_do_not_precede_heading() {
    for prefix in [
        "<span style='display:contents'></span>",
        "<span style='display:contents'><span style='display:contents'></span></span>",
    ] {
        for paged in [false, true] {
            let body = format!("<body>{prefix}<h2>A1</h2><h2>A2</h2></body>");
            assert_eq!(
                texts(&body, "string(title,start)", paged),
                ["A1"],
                "paged={paged}, prefix={prefix}"
            );
        }
    }
}

#[test]
fn contents_named_strings_preserve_real_content() {
    for prefix in [
        "<span style='display:contents'>before</span>",
        "<div style='display:contents'><p>before</p></div>",
        "<div style='width:0;height:0'></div>",
    ] {
        for paged in [false, true] {
            let body = format!("<body>{prefix}<h2>A1</h2><h2>A2</h2></body>");
            assert_eq!(
                texts(&body, "string(title,start)", paged),
                [""],
                "paged={paged}, prefix={prefix}"
            );
        }
    }
}

#[test]
fn contents_named_strings_preserve_single_page_generated_content() {
    for pseudo in ["before", "after"] {
        for declaration in [
            "content:'text'",
            "content:'';padding:5px",
            "content:'';display:inline-block;width:10px;height:10px",
            "content:'';display:block;padding:5px",
        ] {
            let body = format!(
                "<body><style>.c::{pseudo}{{{declaration}}}</style><span class=c style='display:contents'></span><h2>A1</h2><h2>A2</h2></body>"
            );
            assert_eq!(
                texts(&body, "string(title,start)", false),
                [""],
                "{pseudo}: {declaration}"
            );
        }
    }
}

#[test]
fn contents_named_strings_absent_pseudo_does_not_precede_heading() {
    for pseudo in ["before", "after"] {
        for declaration in ["content:none", "content:''", "content:'text';display:none"] {
            for paged in [false, true] {
                let body = format!(
                    "<body><style>.c::{pseudo}{{{declaration}}}</style><span class=c style='display:contents'></span><h2>A1</h2><h2>A2</h2></body>"
                );
                assert_eq!(
                    texts(&body, "string(title,start)", paged),
                    ["A1"],
                    "paged={paged}, {pseudo}: {declaration}"
                );
            }
        }
    }
}

#[test]
fn contents_named_strings_carry_in_yields_to_next_page_heading() {
    let body = "<body><h2>OLD</h2><section style='break-before:page'><span style='display:contents'></span><h2>NEW</h2><h2>LAST</h2></section><p style='break-before:page'>tail</p></body>";
    for (mode, expected) in [
        ("first", ["OLD", "NEW", "LAST"]),
        ("start", ["OLD", "NEW", "LAST"]),
        ("last", ["OLD", "LAST", "LAST"]),
        ("first-except", ["", "", "LAST"]),
    ] {
        assert_eq!(
            texts(body, &format!("string(title,{mode})"), true),
            expected,
            "{mode}"
        );
    }
}

#[test]
fn contents_named_strings_running_element_uses_next_real_page() {
    let body = "<body><p>OLD</p><div id=r style='position:running(hdr)'>RUN</div><span style='display:contents'></span><p style='break-before:page'>NEXT</p></body>";
    let (document, cascade, _, count) = fixture(body, "element(hdr)", true);
    assert_eq!(count, 2);
    let running = document
        .preorder(document.root_index())
        .find(|&node| document.element_attribute(node, "id") == Some("r"))
        .unwrap();
    let index = raikiri_dom::RunningIndex::build(
        &document,
        &cascade,
        (0..count).map(|page| (&document, page)),
    );
    assert_eq!(
        index.select("hdr", raikiri_style::property::StringFetchMode::First, 0),
        None
    );
    assert_eq!(
        index.select("hdr", raikiri_style::property::StringFetchMode::Start, 1),
        Some(raikiri_traits::NodeId(running as u64))
    );
}

#[test]
fn contents_named_strings_single_paint_matches_literal() {
    let body = "<body><span style='display:contents'></span><h2>A1</h2><h2>A2</h2></body>";
    for route in [
        LegacyRoute::Plain,
        LegacyRoute::Images,
        LegacyRoute::NamedImagesAndHighlights,
    ] {
        assert_eq!(
            legacy_margin_glyphs_for_body(body, "'V:' string(title,start)", route),
            legacy_margin_glyphs_for_body(body, "'V:A1'", route)
        );
    }
}

#[test]
fn contents_named_strings_preserve_painted_pseudo_overlays() {
    fn glyph_count(body: &str) -> usize {
        let (document, cascade, page, _) = fixture(body, "'V:'", false);
        let mut scene = Scene::new();
        crate::paint_single_page_with_origin(&mut scene, &document, &cascade, page, 0.0).unwrap();
        scene
            .commands
            .iter()
            .map(|command| match command {
                RenderCommand::GlyphRun(run) => run.glyphs.len(),
                _ => 0,
            })
            .sum()
    }
    let mut observed = Vec::new();
    for pseudo in ["before", "after"] {
        for placement in ["float:left", "position:absolute", "position:fixed"] {
            let body = format!(
                "<body><style>.c::{pseudo}{{content:'X';{placement}}}</style><span class=c style='display:contents'></span><h2>A1</h2><h2>A2</h2></body>"
            );
            let control = body.replace("content:'X'", "content:none");
            let painted = glyph_count(&body) > glyph_count(&control);
            let header = texts(&body, "string(title,start)", false);
            observed.push((pseudo, placement, painted, header));
        }
    }
    println!("overlay observations: {observed:?}");
    assert!(observed.iter().all(|(_, _, painted, _)| *painted));
    assert!(
        observed
            .iter()
            .all(|(_, _, _, header)| header == &[String::new()])
    );
}

#[test]
fn contents_named_strings_absent_overlay_does_not_precede_heading() {
    for pseudo in ["before", "after"] {
        for placement in ["float:left", "position:absolute", "position:fixed"] {
            for content in ["content:none", "content:'X';display:none"] {
                let body = format!(
                    "<body><style>.c::{pseudo}{{{content};{placement}}}</style><span class=c style='display:contents'></span><h2>A1</h2><h2>A2</h2></body>"
                );
                assert_eq!(
                    texts(&body, "string(title,start)", false),
                    ["A1"],
                    "{pseudo} {placement} {content}"
                );
            }
        }
    }
}
