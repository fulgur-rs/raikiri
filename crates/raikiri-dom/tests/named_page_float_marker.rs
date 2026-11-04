//! Named-page flow follows in-flow content, including inside marker boxes.

use raikiri_html::{
    DocumentLayout, FontCollectionBuilder, GeneratedKind, LayoutOptions, LayoutStatus,
    RenderResources, RunSource, layout, parse_html_with_resources,
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

fn layout_document(css: &str, body: &str, resources: &RenderResources<'_>) -> DocumentLayout {
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
    result
}

fn pages(css: &str, body: &str, resources: &RenderResources<'_>) -> Pages {
    let result = layout_document(css, body, resources);
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
    check_oracle(oracle());
}

#[test]
fn marker_display_keeps_the_printed_content_and_page_contexts() {
    check_oracle(serde_json::from_str(include_str!("data/marker-display.json")).unwrap());
}

#[test]
fn marker_display_keeps_nested_quote_strings_and_following_quotes() {
    let resources = resources();
    let body = "<section><ol><li>X</li></ol><div class=probe>Y</div></section>";
    for position in ["inside", "outside"] {
        for display in ["", "display:none"] {
            let css = format!(
                "@page{{size:200px 200px;margin:10px}} \
                 body{{margin:0;font:10px/10px Ahem}}ol{{margin:0;padding-left:50px}} \
                 section{{quotes:'<' '>' '[' ']' '{{' '}}'}} \
                 section::before{{content:open-quote}} \
                 section::after{{content:close-quote close-quote close-quote}} \
                 li{{list-style-position:{position}}} \
                 li::marker{{content:open-quote;{display}}} \
                 .probe::before{{content:open-quote}}"
            );
            let document = layout_document(&css, body, &resources);
            let mut generated: Vec<_> = document
                .pages()
                .flat_map(|page| page.text_runs())
                .filter_map(|run| {
                    let kind = match run.source {
                        RunSource::Generated(_, GeneratedKind::Marker) => "marker",
                        RunSource::Generated(_, GeneratedKind::Before) => "before",
                        RunSource::Generated(_, GeneratedKind::After) => "after",
                        _ => return None,
                    };
                    Some((kind.to_owned(), run.text.to_owned()))
                })
                .collect();
            generated.sort();
            assert_eq!(
                generated,
                [
                    ("after".to_owned(), "}]>".to_owned()),
                    ("before".to_owned(), "<".to_owned()),
                    ("before".to_owned(), "{".to_owned()),
                    ("marker".to_owned(), "[".to_owned()),
                ],
                "{position} / {display}"
            );
        }
    }
}

fn check_oracle(oracle: serde_json::Value) {
    // Chromium's printed page widths identify the page names. PDF glyph bounds
    // are converted from points to CSS pixels; Ahem advances each character by
    // 10px. The same font is supplied directly to the public layout API here.
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
