//! Named-page flow follows in-flow content, including inside marker boxes.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

const AHEM: &[u8] = include_bytes!("data/text-autospace/Ahem.ttf");
type Glyph = (u32, String, f32, f32);
type Pages = (Vec<Option<String>>, Vec<Glyph>);

fn resources() -> RenderResources<'static> {
    let fonts = FontCollectionBuilder::new()
        .font_bytes("Ahem", AHEM.to_vec())
        .build()
        .unwrap();
    RenderResources::new().fonts(fonts)
}

fn sort_glyphs(glyphs: &mut [Glyph]) {
    glyphs.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then(a.1.cmp(&b.1))
            .then(a.2.total_cmp(&b.2))
            .then(a.3.total_cmp(&b.3))
    });
}

fn pages(css: &str, body: &str, resources: &RenderResources<'_>) -> Pages {
    let html = format!("<!doctype html><style>{css}</style>{body}");
    let parsed = parse_html_with_resources(html.as_bytes(), resources).unwrap();
    let LayoutStatus::Completed(result) = layout(
        &parsed,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new().resources(resources),
    )
    .unwrap() else {
        panic!("incomplete layout")
    };
    let names = result
        .pages()
        .map(|page| page.name().map(str::to_owned))
        .collect();
    let mut glyphs = Vec::new();
    for page in result.pages() {
        for run in page.text_runs() {
            for (offset, letter) in run.text.chars().enumerate() {
                if !letter.is_ascii_alphabetic() {
                    continue;
                }
                let origin = (
                    run.origin.0 + offset as f32 * 10.0,
                    run.origin.1 - run.ascent,
                );
                let (x, y) = run
                    .transform
                    .map_or(origin, |transform| transform.apply(origin));
                glyphs.push((page.index(), letter.to_string(), x, y));
            }
        }
    }
    sort_glyphs(&mut glyphs);
    (names, glyphs)
}

fn oracle() -> serde_json::Value {
    serde_json::from_str(include_str!("data/named-page-float-marker.json")).unwrap()
}

#[test]
fn floats_and_markers_keep_their_printed_page_contexts() {
    // Chromium's printed page widths identify the page names. PDF glyph bounds
    // are converted from points to CSS pixels; Ahem advances each character by
    // 10px. The same font is supplied directly to the public layout API here.
    let oracle = oracle();
    let resources = resources();
    let mut mismatches = Vec::new();
    for case in oracle["cases"].as_array().unwrap() {
        let label = case["label"].as_str().unwrap();
        let css = format!(
            "{}{}",
            oracle["css"].as_str().unwrap(),
            case["extra_css"].as_str().unwrap()
        );
        let (names, glyphs) = pages(&css, case["body"].as_str().unwrap(), &resources);
        let expected_names: Vec<Option<String>> =
            serde_json::from_value(case["page_names"].clone()).unwrap();
        if names != expected_names {
            mismatches.push(format!(
                "{label}: pages {names:?}, expected {expected_names:?}"
            ));
        }
        let mut expected: Vec<Glyph> = serde_json::from_value(case["glyphs"].clone()).unwrap();
        sort_glyphs(&mut expected);
        if glyphs.len() != expected.len()
            || glyphs.iter().zip(&expected).any(|(got, want)| {
                got.0 != want.0
                    || got.1 != want.1
                    || (got.2 - want.2).abs() > 0.05
                    || (got.3 - want.3).abs() > 0.05
            })
        {
            mismatches.push(format!("{label}: glyphs {glyphs:?}, expected {expected:?}"));
        }
    }
    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
}

#[test]
fn ineffective_float_declarations_keep_the_named_flex_item_context() {
    // RAIKIRI-1449 tracks the independent row-flex page-boundary difference.
    // Compare page names with float:none instead of pinning that existing bug.
    let oracle = oracle();
    let resources = resources();
    let css = oracle["css"].as_str().unwrap();
    let body = oracle["ineffective_float_control"]["body"]
        .as_str()
        .unwrap();
    let (baseline, _) = pages(css, &body.replace("float:left;", "float:none;"), &resources);
    assert!(baseline.iter().any(|name| name.as_deref() == Some("a")));
    for float in ["left", "right", "inline-start", "inline-end"] {
        let (names, _) = pages(
            css,
            &body.replace("float:left;", &format!("float:{float};")),
            &resources,
        );
        assert_eq!(names, baseline, "{float}");
    }
}
