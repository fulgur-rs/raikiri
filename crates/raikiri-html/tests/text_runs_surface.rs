//! External-consumer coverage for the positioned glyph runs a PDF painter
//! reads.
//!
//! Every type below is named through `raikiri_html` (and `raikiri_traits`)
//! only, the way a downstream painter that does not depend on the layout or
//! text crates would write it.

use raikiri_html::computed::CssColor;
use raikiri_html::{
    FontBlob, FontCollectionBuilder, FontId, FontRef, FontVariation, GeneratedKind, Glyph,
    LayoutOptions, LayoutStatus, PositionedGlyphRun, RenderResources, RunSource, Synthesis, Tag,
    WarningKind, layout, parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};
use std::ops::Range;
use std::sync::Arc;

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../raikiri-dom/tests/data/text-autospace/Ahem.ttf"
));

#[test]
fn inside_marker_before_body_and_after_keep_distinct_sources() {
    let html = br#"<!doctype html><style>
      @page { size: 300px 200px; margin: 0 }
      body { margin: 0; font: 10px/10px Ahem }
      li { list-style: "M " inside }
      li::before { content: "B " }
      li::after { content: " A" }
    </style><li id="item">body</li>"#;
    let fonts = FontCollectionBuilder::new()
        .font_bytes("Ahem", AHEM.to_vec())
        .build()
        .expect("fonts");
    let resources = RenderResources::new().fonts(fonts);
    let doc = parse_html_with_resources(&html[..], &resources).expect("parse");
    let LayoutStatus::Completed(result) = layout(
        &doc,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new().resources(&resources),
    )
    .expect("layout") else {
        panic!("expected a complete layout");
    };
    let page = result.pages().next().expect("one page");
    let runs = page.text_runs();
    let marker = runs.iter().find(|run| run.text == "M").expect("marker");
    let RunSource::Generated(element, GeneratedKind::Marker) = marker.source else {
        panic!("expected a ::marker run, got {:?}", marker.source);
    };
    assert_eq!(page.dom().attr(element, "id"), Some("item"));
    let before = runs.iter().find(|run| run.text == "B ").expect("before");
    assert_eq!(
        before.source,
        RunSource::Generated(element, GeneratedKind::Before)
    );
    let after = runs.iter().find(|run| run.text == " A").expect("after");
    assert_eq!(
        after.source,
        RunSource::Generated(element, GeneratedKind::After)
    );
    let body = runs.iter().find(|run| run.text == "body").expect("body");
    let RunSource::Text(text_node) = body.source else {
        panic!("expected the body text node");
    };
    assert_eq!(page.dom().text(text_node), Some("body"));
}

/// What a painter keeps of one glyph: its id, position and source text.
fn place(run: &PositionedGlyphRun<'_>) -> Vec<(u32, (f32, f32), String)> {
    let mut pen = run.origin.0;
    run.glyphs
        .iter()
        .map(|glyph: &Glyph| {
            let range: Range<usize> = glyph.text_range.clone();
            let placed = (
                glyph.id,
                (pen + glyph.x_offset, run.origin.1 + glyph.y_offset),
                run.text[range].to_owned(),
            );
            pen += glyph.advance;
            placed
        })
        .collect()
}

#[test]
fn painter_reads_text_runs_through_raikiri_html_only() {
    let html = br#"<!doctype html>
        <style>
          @page { size: 300px 200px; margin: 10px }
          body { margin: 0; font: 10px/10px Ahem }
          #p::before { content: "go "; color: rgb(0, 0, 255) }
        </style>
        <p id="p" style="margin: 0">ab <span style="color: rgb(0, 128, 0)">cd</span></p>
        <p style="writing-mode: vertical-rl">up</p>"#;
    let fonts = FontCollectionBuilder::new()
        .font_bytes("Ahem", AHEM.to_vec())
        .build()
        .expect("fonts");
    let resources = RenderResources::new().fonts(fonts);
    let doc = parse_html_with_resources(&html[..], &resources).expect("parse");
    let status = layout(
        &doc,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new().resources(&resources),
    )
    .expect("layout");
    let LayoutStatus::Completed(result) = status else {
        panic!("expected a complete layout");
    };
    let page = result.pages().next().expect("one page");
    let runs: Vec<PositionedGlyphRun<'_>> = page.text_runs();
    let texts: Vec<&str> = runs.iter().map(|run| run.text).collect();
    assert_eq!(texts, ["go ", "ab ", "cd"]);

    let before = &runs[0];
    let RunSource::Generated(element, GeneratedKind::Before) = before.source else {
        panic!("expected a ::before run, got {:?}", before.source);
    };
    assert_eq!(page.dom().attr(element, "id"), Some("p"));
    let blue: CssColor = before.color;
    assert_eq!((blue.r, blue.g, blue.b, blue.a), (0, 0, 255, 255));
    // The first line's baseline is 8px (Ahem's ascent) below the content
    // box, which starts at the 10px page margin.
    assert_eq!(before.origin, (10.0, 18.0));
    let placed: Vec<((f32, f32), String)> = place(before)
        .into_iter()
        .map(|(_, position, text)| (position, text))
        .collect();
    assert_eq!(
        placed,
        [
            ((10.0, 18.0), "g".to_owned()),
            ((20.0, 18.0), "o".to_owned()),
            ((30.0, 18.0), " ".to_owned()),
        ]
    );

    let RunSource::Text(text_node) = runs[1].source else {
        panic!("expected a text run");
    };
    assert_eq!(page.dom().text(text_node), Some("ab "));
    let green = runs[2].color;
    assert_eq!((green.r, green.g, green.b), (0, 128, 0));
    assert_eq!(runs[2].origin, (70.0, 18.0));
    assert_eq!(runs[2].advance, 20.0);

    let font: &FontRef = &runs[1].font;
    let id: FontId = font.id;
    assert!(runs.iter().all(|run| run.font.id == id));
    let data: &FontBlob = &font.data;
    assert_eq!(data.as_bytes(), AHEM);
    let shared: Arc<dyn AsRef<[u8]> + Send + Sync> = data.to_arc();
    assert_eq!((*shared).as_ref().len(), AHEM.len());
    assert_eq!(font.index, 0);
    assert!(format!("{data:?}").contains("len"));

    let synthesis: Synthesis = runs[1].synthesis;
    assert!(!synthesis.embolden);
    assert_eq!(synthesis.skew, None);
    let variations: &[FontVariation] = &runs[1].variations;
    assert!(
        variations
            .iter()
            .all(|v| v.tag != Tag(*b"wght") || v.value > 0.0)
    );
    let _coords: &[i16] = &runs[1].normalized_coords;
    assert_eq!(runs[1].font_size, 10.0);
    assert_eq!((runs[1].ascent, runs[1].descent), (8.0, 2.0));

    let omitted = result
        .warnings()
        .iter()
        .filter(|warning| matches!(warning.kind, WarningKind::TextRunsOmitted))
        .count();
    assert_eq!(omitted, 1, "the vertical paragraph is reported as omitted");
}

#[test]
fn first_letter_runs_keep_original_sources_and_resolved_paint() {
    for generated in [false, true] {
        let before = if generated {
            "li::before {content:'XY';color:blue}"
        } else {
            ""
        };
        let html = format!(
            r#"<!doctype html><style>
          @page {{size:300px 200px;margin:0}}
          body {{margin:0;font:10px/30px Ahem;color:black}}
          li {{list-style:"M " inside}}
          li::first-letter {{font-size:20px;color:red}}
          {before}
        </style><li id="item">AB</li>"#
        );
        let fonts = FontCollectionBuilder::new()
            .font_bytes("Ahem", AHEM.to_vec())
            .build()
            .unwrap();
        let resources = RenderResources::new().fonts(fonts);
        let doc = parse_html_with_resources(html.as_bytes(), &resources).unwrap();
        let LayoutStatus::Completed(result) = layout(
            &doc,
            PageDefaults::default(),
            LayoutConfig::default(),
            LayoutOptions::new().resources(&resources),
        )
        .unwrap() else {
            panic!("expected complete layout")
        };
        let page = result.pages().next().unwrap();
        let runs = page.text_runs();
        let marker = runs.iter().find(|run| run.text == "M").unwrap();
        assert_eq!(marker.font_size, 10.0);
        let RunSource::Generated(item, GeneratedKind::Marker) = marker.source else {
            panic!("marker source")
        };
        assert_eq!(page.dom().attr(item, "id"), Some("item"));
        let letter = runs.iter().find(|run| run.font_size == 20.0).unwrap();
        assert_eq!(letter.text, if generated { "X" } else { "A" });
        assert_eq!(
            letter.color,
            CssColor {
                r: 255,
                g: 0,
                b: 0,
                a: 255
            }
        );
        if generated {
            assert_eq!(
                letter.source,
                RunSource::Generated(item, GeneratedKind::Before)
            );
            let rest = runs.iter().find(|run| run.text == "Y").unwrap();
            assert_eq!(rest.source, letter.source);
            assert_eq!(
                rest.color,
                CssColor {
                    r: 0,
                    g: 0,
                    b: 255,
                    a: 255
                }
            );
        } else {
            let RunSource::Text(text) = letter.source else {
                panic!("original DOM text source")
            };
            assert_eq!(page.dom().text(text), Some("AB"));
            let rest = runs.iter().find(|run| run.text == "B").unwrap();
            assert_eq!(rest.source, letter.source);
            assert_eq!(
                rest.color,
                CssColor {
                    r: 0,
                    g: 0,
                    b: 0,
                    a: 255
                }
            );
        }
    }
}

#[test]
fn first_letter_across_generated_sources_preserves_each_owner_kind_and_remainder() {
    let html = br#"<!doctype html><style>
        @page {size:300px 200px;margin:0}
        body {margin:0;font:10px/40px Ahem;color:black}
        div::before {content:'(';color:red;font-size:20px}
        span::before {content:'A';color:blue;font-size:30px}
        span::after {content:')Y';color:green;font-size:40px}
        div::first-letter {font-size:50%}
        </style><div id="outer"><span id="inner"></span>Z</div>"#;
    let fonts = FontCollectionBuilder::new()
        .font_bytes("Ahem", AHEM.to_vec())
        .build()
        .unwrap();
    let resources = RenderResources::new().fonts(fonts);
    let doc = parse_html_with_resources(&html[..], &resources).unwrap();
    let LayoutStatus::Completed(result) = layout(
        &doc,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new().resources(&resources),
    )
    .unwrap() else {
        panic!("expected complete layout");
    };
    let page = result.pages().next().unwrap();
    let runs = page.text_runs();
    for (text, size, id, kind, color) in [
        (
            "(",
            10.0,
            "outer",
            GeneratedKind::Before,
            CssColor {
                r: 255,
                g: 0,
                b: 0,
                a: 255,
            },
        ),
        (
            "A",
            15.0,
            "inner",
            GeneratedKind::Before,
            CssColor {
                r: 0,
                g: 0,
                b: 255,
                a: 255,
            },
        ),
        (
            ")",
            20.0,
            "inner",
            GeneratedKind::After,
            CssColor {
                r: 0,
                g: 128,
                b: 0,
                a: 255,
            },
        ),
        (
            "Y",
            40.0,
            "inner",
            GeneratedKind::After,
            CssColor {
                r: 0,
                g: 128,
                b: 0,
                a: 255,
            },
        ),
    ] {
        let run = runs.iter().find(|run| run.text == text).unwrap();
        assert_eq!(run.font_size, size);
        assert_eq!(run.color, color);
        let RunSource::Generated(owner, actual_kind) = run.source else {
            panic!("original generated source");
        };
        assert_eq!(actual_kind, kind);
        assert_eq!(page.dom().attr(owner, "id"), Some(id));
    }
    let body = runs.iter().find(|run| run.text == "Z").unwrap();
    let RunSource::Text(owner) = body.source else {
        panic!("original DOM source");
    };
    assert_eq!(page.dom().text(owner), Some("Z"));
}
