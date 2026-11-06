//! Row flex alignment preserves whole boxes that extend above a page.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults, PaintRect};

fn boxes(
    writing_mode: &str,
    alignment: &str,
    offset: f32,
    with_sibling: bool,
    size: &str,
    text: &str,
) -> Vec<(String, PaintRect, PaintRect)> {
    let fonts = FontCollectionBuilder::new()
        .font_bytes(
            "Ahem",
            include_bytes!("data/text-autospace/Ahem.ttf").to_vec(),
        )
        .build()
        .expect("fonts");
    let resources = RenderResources::new().fonts(fonts);
    let sibling = if with_sibling {
        "<div id=b style='width:10px;height:50px'>x</div>"
    } else {
        ""
    };
    let html = format!(
        "<!DOCTYPE html><style>@page{{size:800px 600px;margin:0}}\
         body{{margin:0;font:10px/10px Ahem}}div{{box-sizing:border-box}}</style>\
         <div style='height:{offset}px'></div>\
         <div style='display:flex;align-items:{alignment};height:80px'>{sibling}\
         <div id=a style='width:10px;{size};writing-mode:{writing_mode}'>{text}</div></div>"
    );
    let doc = parse_html_with_resources(html.as_bytes(), &resources).expect("parse");
    let LayoutStatus::Completed(result) = layout(
        &doc,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new().resources(&resources),
    )
    .expect("layout") else {
        panic!("incomplete layout")
    };
    let page = result.pages().next().expect("page");
    page.fragments()
        .filter_map(|fragment| {
            let id = page.dom().attr(fragment.node(), "id")?;
            Some((
                id.to_owned(),
                fragment.rect(),
                fragment.content_rect().expect("whole content box"),
            ))
        })
        .collect()
}

// Whole boxes are Chromium DOM rectangles. The top-of-page flex-end case
// also paints a 10x80 blue rectangle in both its screenshot and print PDF.
// Returning an uncut rect, or shrinking the actual item, breaks this test.
#[test]
fn row_cross_alignment_page_cuts_keep_whole_content_geometry() {
    for writing_mode in ["horizontal-tb", "vertical-rl", "vertical-lr"] {
        for (alignment, tall_y, short_y, first_fragment_height) in [
            ("flex-start", 0.0, 0.0, 170.0),
            ("flex-end", -90.0, 30.0, 80.0),
            ("center", -45.0, 15.0, 125.0),
            ("stretch", 0.0, 0.0, 170.0),
        ] {
            for (offset, fragment_y, fragment_height) in [
                (0.0, 0.0, first_fragment_height),
                (200.0, tall_y + 200.0, 170.0),
            ] {
                for with_sibling in [false, true] {
                    let actual = boxes(
                        writing_mode,
                        alignment,
                        offset,
                        with_sibling,
                        "height:170px",
                        "x",
                    );
                    assert_eq!(actual.len(), if with_sibling { 2 } else { 1 });
                    let x = if with_sibling { 10.0 } else { 0.0 };
                    let mut expected = vec![(
                        "a",
                        PaintRect::new(x, fragment_y, 10.0, fragment_height),
                        PaintRect::new(x, tall_y + offset, 10.0, 170.0),
                    )];
                    if with_sibling {
                        let short = PaintRect::new(0.0, short_y + offset, 10.0, 50.0);
                        expected.push(("b", short, short));
                    }
                    for (id, visible, whole) in expected {
                        let (_, actual_visible, actual_whole) = actual
                            .iter()
                            .find(|(name, _, _)| name == id)
                            .expect("item fragment");
                        assert_eq!(
                            (*actual_visible, *actual_whole),
                            (visible, whole),
                            "{writing_mode}/{alignment}/offset={offset}/sibling={with_sibling}/#{id}"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn vertical_intrinsic_inline_sizes_keep_the_same_whole_box_above_the_page() {
    for writing_mode in ["vertical-rl", "vertical-lr"] {
        for keyword in ["max-content", "fit-content"] {
            for (alignment, whole_y, first_height) in
                [("flex-end", -90.0, 80.0), ("center", -45.0, 125.0)]
            {
                for (offset, fragment_y, fragment_height) in
                    [(0.0, 0.0, first_height), (200.0, whole_y + 200.0, 170.0)]
                {
                    let actual = boxes(
                        writing_mode,
                        alignment,
                        offset,
                        false,
                        &format!("inline-size:{keyword}"),
                        "xxxxxxxxxxxxxxxxx",
                    );
                    assert_eq!(
                        actual,
                        vec![(
                            "a".to_owned(),
                            PaintRect::new(0.0, fragment_y, 10.0, fragment_height),
                            PaintRect::new(0.0, whole_y + offset, 10.0, 170.0),
                        )],
                        "{writing_mode}/{keyword}/{alignment}/offset={offset}"
                    );
                }
            }
        }
    }
}
