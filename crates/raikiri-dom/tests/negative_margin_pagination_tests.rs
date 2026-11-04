//! Mixed-sign collapsed margins choose the first page edge after content.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/data/text-autospace/Ahem.ttf"
));

fn position(previous: &str, empty: Option<&str>, next: &str) -> (u32, Vec<(u32, f32)>) {
    let empty = empty.map_or(String::new(), |style| {
        format!("<div style='{style}'></div>")
    });
    let html = format!(
        "<!doctype html><style>@page{{size:100px 100px;margin:0}}\
         html,body{{margin:0}}body{{font:20px/20px Ahem}}</style>\
         <div style='{previous}'>A</div>{empty}<div style='height:20px;{next}'>B</div>"
    );
    let fonts = FontCollectionBuilder::new()
        .font_bytes("Ahem", AHEM.to_vec())
        .build()
        .unwrap();
    let resources = RenderResources::new().fonts(fonts);
    let parsed = parse_html_with_resources(html.as_bytes(), &resources).unwrap();
    let LayoutStatus::Completed(result) = layout(
        &parsed,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new().resources(&resources),
    )
    .unwrap() else {
        panic!("completed layout")
    };
    let mut positions = Vec::new();
    for page in result.pages() {
        for run in page.text_runs() {
            if run.text == "B" {
                let origin = (run.origin.0, run.origin.1 - run.ascent);
                let top = run
                    .transform
                    .map_or(origin, |transform| transform.apply(origin))
                    .1;
                positions.push((page.index(), top));
            }
        }
    }
    (result.page_count(), positions)
}

#[test]
fn an_empty_previous_box_with_a_negative_top_margin_does_not_pull_text_a_page_back() {
    assert_eq!(
        position(
            "height:250px",
            Some("height:0;margin-top:-100px;margin-bottom:300px"),
            "margin-top:-200px"
        ),
        (4, vec![(3, 0.0)]),
    );
}

#[test]
fn negative_previous_bottom_and_positive_next_top_use_the_same_collapse() {
    assert_eq!(
        position(
            "height:250px",
            Some("height:0;margin-top:-100px;margin-bottom:-200px"),
            "margin-top:300px"
        ),
        (4, vec![(3, 0.0)]),
    );
}

#[test]
fn the_first_margin_edge_is_not_tied_to_a_particular_page_number() {
    assert_eq!(
        position(
            "height:350px",
            Some("height:0;margin-top:-100px;margin-bottom:300px"),
            "margin-top:-200px"
        ),
        (5, vec![(4, 0.0)]),
    );
}

#[test]
fn ordinary_mixed_margins_still_truncate_at_the_next_page() {
    assert_eq!(
        position("height:95px;margin-bottom:10px", None, "margin-top:-5px"),
        (2, vec![(1, 0.0)])
    );
}

#[test]
fn negative_only_margins_still_overlap_without_clamping() {
    assert_eq!(
        position("height:80px;margin-bottom:-10px", None, "margin-top:-5px"),
        (1, vec![(0, 70.0)])
    );
}

#[test]
fn positive_only_margins_keep_the_existing_truncation() {
    assert_eq!(
        position("height:95px;margin-bottom:20px", None, "margin-top:10px"),
        (2, vec![(1, 0.0)])
    );
}
