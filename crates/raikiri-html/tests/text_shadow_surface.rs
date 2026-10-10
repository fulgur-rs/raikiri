//! Used text shadows available to downstream PDF painters.

use raikiri_html::computed::CssColor;
use raikiri_html::{
    DocumentLayout, FontCollectionBuilder, LayoutConfig, LayoutOptions, LayoutStatus, PageDefaults,
    RenderResources, RunSource, layout, parse_html_with_resources,
};

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../raikiri-dom/tests/data/text-autospace/Ahem.ttf"
));

fn document(body: &str, css: &str) -> DocumentLayout {
    let fonts = FontCollectionBuilder::new()
        .system_fonts(false)
        .font_bytes("Ahem", AHEM.to_vec())
        .build()
        .unwrap();
    let resources = RenderResources::new().fonts(fonts);
    let html = format!(
        "<!doctype html><style>@page {{size:200px 200px;margin:0}} \
         body {{margin:0;font:20px/20px Ahem}} p {{margin:0}} {css}</style>{body}"
    );
    let parsed = parse_html_with_resources(html.as_bytes(), &resources).unwrap();
    let LayoutStatus::Completed(result) = layout(
        &parsed,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new().resources(&resources),
    )
    .unwrap() else {
        panic!("layout aborted")
    };
    result
}

const RED: CssColor = CssColor {
    r: 255,
    g: 0,
    b: 0,
    a: 255,
};
const BLUE: CssColor = CssColor {
    r: 0,
    g: 0,
    b: 255,
    a: 255,
};

#[test]
fn shadows_are_listed_in_declaration_order_with_used_values() {
    let doc = document(
        "<p>ab</p>",
        "p {color:blue;text-shadow:2px 3px 4px red, -1px 0.5em currentcolor}",
    );
    let page = doc.page(0).unwrap();
    let runs = page.text_runs();
    assert_eq!(runs.len(), 1);
    let shadows = &runs[0].shadows;
    assert_eq!(shadows.len(), 2);
    assert_eq!(shadows[0].offset, (2.0, 3.0));
    assert_eq!(shadows[0].blur_radius, 4.0);
    assert_eq!(shadows[0].color, RED);
    // `em` is absolutized against the font size; `currentcolor` takes the
    // text color.
    assert_eq!(shadows[1].offset, (-1.0, 10.0));
    assert_eq!(shadows[1].blur_radius, 0.0);
    assert_eq!(shadows[1].color, BLUE);
}

#[test]
fn shadows_are_inherited_and_none_clears_them() {
    let doc = document(
        "<p>a<span>b</span></p>",
        "p {text-shadow:1px 1px red} span {color:blue;text-shadow:none}",
    );
    let page = doc.page(0).unwrap();
    let runs = page.text_runs();
    assert_eq!(runs.len(), 2);
    assert_eq!(runs[0].shadows.len(), 1);
    assert!(runs[1].shadows.is_empty());

    let doc = document("<p>a<span>b</span></p>", "p {text-shadow:1px 1px}");
    let page = doc.page(0).unwrap();
    let runs = page.text_runs();
    assert!(runs.iter().all(|run| run.shadows.len() == 1));
}

#[test]
fn generated_text_and_markers_carry_shadows() {
    let doc = document(
        "<ol><li>a</li></ol><p>b</p>",
        "body {text-shadow:1px 2px red} p::before {content:'x'}",
    );
    let page = doc.page(0).unwrap();
    let runs = page.text_runs();
    let marker = runs
        .iter()
        .find(|run| run.is_standalone_marker())
        .expect("marker run");
    assert_eq!(marker.shadows.len(), 1);
    assert_eq!(marker.shadows[0].offset, (1.0, 2.0));
    assert!(
        runs.iter()
            .any(|run| matches!(run.source, RunSource::Generated(..))
                && !run.is_standalone_marker()
                && run.shadows.len() == 1)
    );
}
