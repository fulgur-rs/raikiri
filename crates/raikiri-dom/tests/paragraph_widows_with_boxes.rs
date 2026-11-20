//! A paragraph whose lines cross the page end breaks between them so that
//! its last `widows` lines go to the next page, when the paragraph has a set
//! height, holds inline boxes among its lines, or has floats or out-of-flow
//! boxes of its own: the lines move down inside its box, the inline boxes
//! with the lines that hold them. The pages and positions are the ones
//! Chromium prints for the same markup: 200px by 100px pages without margins,
//! Ahem at 10px on 20px lines.
use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/data/text-autospace/Ahem.ttf"
));

/// A document body and where its letters land: (id, page, top).
type Case<'a> = (&'a str, &'a [(&'a str, usize, f32)]);

/// The page index, top and bottom on that page of each fragment of the
/// element with the id `id`.
fn fragments_of(body: &str, id: &str) -> Vec<(usize, f32, f32)> {
    let html = format!(
        "<!DOCTYPE html><style>@page{{size:200px 100px;margin:0}}\
         body{{margin:0;font:10px/20px Ahem}}</style>{body}"
    );
    let fonts = FontCollectionBuilder::new()
        .font_bytes("Ahem", AHEM.to_vec())
        .build()
        .expect("fonts");
    let resources = RenderResources::new().fonts(fonts);
    let doc = parse_html_with_resources(html.as_bytes(), &resources).expect("parse");
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
    let mut fragments = Vec::new();
    for (index, page) in result.pages().enumerate() {
        for fragment in page.fragments() {
            if page.dom().attr(fragment.node(), "id") == Some(id) {
                let rect = fragment.rect();
                fragments.push((index, rect.y, rect.y + rect.height));
            }
        }
    }
    fragments
}

/// Checks the page and the top on that page of each glyph `(id, page, top)`,
/// in the order of its fragments.
fn assert_places(body: &str, expected: &[(&str, usize, f32)]) {
    let mut ids: Vec<&str> = expected.iter().map(|&(id, ..)| id).collect();
    ids.dedup();
    for id in ids {
        let want: Vec<(usize, f32)> = expected
            .iter()
            .filter(|&&(other, ..)| other == id)
            .map(|&(_, page, top)| (page, top))
            .collect();
        let got: Vec<(usize, f32)> = fragments_of(body, id)
            .into_iter()
            .map(|(page, top, _)| (page, top))
            .collect();
        let same = got.len() == want.len()
            && got
                .iter()
                .zip(&want)
                .all(|(&(page, top), &(want_page, want_top))| {
                    let off = (top - want_top).abs();
                    page == want_page && !off.is_nan() && off < 0.01
                });
        assert!(same, "#{id} at {got:?}, expected {want:?} in {body}");
    }
}

#[test]
fn a_paragraph_with_inline_boxes_moves_its_last_lines_for_widows() {
    let cases: &[Case] = &[
        (
            "<div style='height:40px'><span id=S>S</span></div><div><span id=a>a</span><br><span id=b>b</span><span style='display:inline-block;width:10px;height:10px'></span><br><span id=c>c</span><br><span id=d>d</span></div><div><span id=Z>Z</span></div>",
            &[
                ("S", 0, 5.0),
                ("a", 0, 45.0),
                ("b", 0, 65.0),
                ("c", 1, 5.0),
                ("d", 1, 25.0),
                ("Z", 1, 45.0),
            ],
        ),
        (
            "<div style='height:40px'><span id=S>S</span></div><div><span id=a>a</span><br><span id=b>b</span><br><span id=c>c</span><span style='display:inline-block;width:10px;height:10px'></span><br><span id=d>d</span></div><div><span id=Z>Z</span></div>",
            &[
                ("S", 0, 5.0),
                ("a", 0, 45.0),
                ("b", 0, 65.0),
                ("c", 1, 5.0),
                ("d", 1, 25.0),
                ("Z", 1, 45.0),
            ],
        ),
        (
            "<div style='height:40px'><span id=S>S</span></div><div><span id=a>a</span><br><span id=b>b</span><br><span style='display:inline-block'><span id=c>c</span></span><br><span id=d>d</span></div><div><span id=Z>Z</span></div>",
            &[
                ("S", 0, 5.0),
                ("a", 0, 45.0),
                ("b", 0, 65.0),
                ("c", 1, 5.0),
                ("d", 1, 25.0),
                ("Z", 1, 45.0),
            ],
        ),
        (
            "<div style='height:40px'><span id=S>S</span></div><div><span id=a>a</span><br><span id=b>b</span><br><span id=c>c</span> <span style='display:inline-block;vertical-align:top'><span id=X>X</span><br><span id=Y>Y</span></span><br><span id=d>d</span></div><div><span id=Z>Z</span></div>",
            &[
                ("S", 0, 5.0),
                ("a", 0, 45.0),
                ("b", 0, 65.0),
                ("c", 1, 5.0),
                ("d", 1, 45.0),
                ("Z", 1, 65.0),
                ("X", 1, 5.0),
                ("Y", 1, 25.0),
            ],
        ),
        (
            "<div style='height:40px'><span id=S>S</span></div><div style='height:80px'><span id=a>a</span><br><span id=b>b</span><br><span id=c>c</span><img style='width:10px;height:10px'><br><span id=d>d</span></div><div><span id=Z>Z</span></div>",
            &[
                ("S", 0, 5.0),
                ("a", 0, 45.0),
                ("b", 0, 65.0),
                ("c", 1, 5.0),
                ("Z", 1, 25.0),
                ("d", 1, 25.0),
            ],
        ),
        (
            "<div style='height:20px'><span id=S>S</span></div><div style='break-after:avoid'><span id=H>H</span></div><div><span id=a>a</span><br><span id=b>b</span><br><span id=c>c</span><span style='display:inline-block;width:5px;height:5px'></span><br><span id=d>d</span><br><span id=e>e</span><br><span id=K>K</span></div><div><span id=Z>Z</span></div>",
            &[
                ("S", 0, 5.0),
                ("H", 0, 25.0),
                ("a", 0, 45.0),
                ("b", 0, 65.0),
                ("c", 0, 85.0),
                ("d", 1, 5.0),
                ("e", 1, 25.0),
                ("K", 1, 45.0),
                ("Z", 1, 65.0),
            ],
        ),
        (
            "<div style='height:40px'><span id=S>S</span></div><table style='border-spacing:0'><tr><td style='padding:0'><span id=a>a</span><br><span id=b>b</span><span style='display:inline-block'><span id=X>X</span></span><br><span id=c>c</span><br><span id=d>d</span></td></tr></table><div><span id=Z>Z</span></div>",
            &[
                ("S", 0, 5.0),
                ("a", 0, 45.0),
                ("b", 0, 65.0),
                ("X", 0, 65.0),
                ("c", 1, 5.0),
                ("d", 1, 25.0),
                ("Z", 1, 45.0),
            ],
        ),
        (
            "<div style='height:40px'><span id=S>S</span></div><div><span id=a>a</span><br><span id=b>b</span><span style='display:inline-block'><span id=X>X</span></span><br><span id=c>c</span><br><span id=d>d</span><div><span id=W>W</span></div></div><div><span id=Z>Z</span></div>",
            &[
                ("S", 0, 5.0),
                ("a", 0, 45.0),
                ("b", 0, 65.0),
                ("X", 0, 65.0),
                ("c", 1, 5.0),
                ("d", 1, 25.0),
                ("W", 1, 45.0),
                ("Z", 1, 65.0),
            ],
        ),
        (
            "<div style='height:40px'><span id=S>S</span></div><div><span id=a>a</span><br><span id=b>b</span><br><span id=c>c</span><span style='display:inline-block'><span id=X>X</span></span><br><span id=d>d</span></div><div><span id=Z>Z</span></div>",
            &[
                ("S", 0, 5.0),
                ("a", 0, 45.0),
                ("b", 0, 65.0),
                ("c", 1, 5.0),
                ("X", 1, 5.0),
                ("d", 1, 25.0),
                ("Z", 1, 45.0),
            ],
        ),
        (
            "<div style='height:40px'><span id=S>S</span></div><div><span id=a>a</span><br><span id=b>b</span><br><span id=c>c</span><br><span id=d>d</span><span style='display:inline-block'><span id=X>X</span></span></div><div><span id=Z>Z</span></div>",
            &[
                ("S", 0, 5.0),
                ("a", 0, 45.0),
                ("b", 0, 65.0),
                ("c", 1, 5.0),
                ("d", 1, 25.0),
                ("X", 1, 25.0),
                ("Z", 1, 45.0),
            ],
        ),
        (
            "<div style='height:40px'><span id=S>S</span></div><div><span id=a>a</span><br><span id=b>b</span><br><span id=c>c</span><br><span><span id=d>d</span><span style='display:inline-block'><span id=X>X</span></span></span></div><div><span id=Z>Z</span></div>",
            &[
                ("S", 0, 5.0),
                ("a", 0, 45.0),
                ("b", 0, 65.0),
                ("c", 1, 5.0),
                ("d", 1, 25.0),
                ("X", 1, 25.0),
                ("Z", 1, 45.0),
            ],
        ),
        (
            "<div style='height:40px'><span id=S>S</span></div><div><span id=a>a</span><br><span id=b>b</span><br><span id=c>c</span><span style='display:inline-block'><span id=X>X</span></span><br><span id=d>d</span><div><span id=W>W</span></div></div><div><span id=Z>Z</span></div>",
            &[
                ("S", 0, 5.0),
                ("a", 0, 45.0),
                ("b", 0, 65.0),
                ("c", 1, 5.0),
                ("X", 1, 5.0),
                ("d", 1, 25.0),
                ("W", 1, 45.0),
                ("Z", 1, 65.0),
            ],
        ),
    ];
    for &(body, expected) in cases {
        assert_places(body, expected);
    }
}

#[test]
fn a_paragraph_with_a_set_height_or_boxes_of_its_own_moves_its_last_lines_for_widows() {
    let cases: &[Case] = &[
        (
            "<div style='height:40px'><span id=S>S</span></div><div style='height:80px'><span id=a>a</span><br><span id=b>b</span><br><span id=c>c</span><br><span id=d>d</span></div><div><span id=Z>Z</span></div>",
            &[
                ("S", 0, 5.0),
                ("a", 0, 45.0),
                ("b", 0, 65.0),
                ("c", 1, 5.0),
                ("Z", 1, 25.0),
                ("d", 1, 25.0),
            ],
        ),
        (
            "<div style='height:40px'><span id=S>S</span></div><div style='min-height:80px'><span id=a>a</span><br><span id=b>b</span><br><span id=c>c</span><br><span id=d>d</span></div><div><span id=Z>Z</span></div>",
            &[
                ("S", 0, 5.0),
                ("a", 0, 45.0),
                ("b", 0, 65.0),
                ("c", 1, 5.0),
                ("d", 1, 25.0),
                ("Z", 1, 45.0),
            ],
        ),
        (
            "<div style='height:40px'><span id=S>S</span></div><div style='height:100px'><span id=a>a</span><br><span id=b>b</span><br><span id=c>c</span><br><span id=d>d</span></div><div><span id=Z>Z</span></div>",
            &[
                ("S", 0, 5.0),
                ("a", 0, 45.0),
                ("b", 0, 65.0),
                ("c", 1, 5.0),
                ("d", 1, 25.0),
                ("Z", 1, 45.0),
            ],
        ),
        (
            "<div style='height:20px'><span id=S>S</span></div><div style='break-after:avoid'><span id=H>H</span></div><div style='height:120px'><span id=a>a</span><br><span id=b>b</span><br><span id=c>c</span><br><span id=d>d</span><br><span id=e>e</span><br><span id=K>K</span></div><div><span id=Z>Z</span></div>",
            &[
                ("S", 0, 5.0),
                ("H", 0, 25.0),
                ("a", 0, 45.0),
                ("b", 0, 65.0),
                ("c", 0, 85.0),
                ("d", 1, 5.0),
                ("e", 1, 25.0),
                ("K", 1, 45.0),
                ("Z", 1, 65.0),
            ],
        ),
        (
            "<div style='height:40px'><span id=S>S</span></div><div style='max-height:80px'><span id=a>a</span><br><span id=b>b</span><br><span id=c>c</span><br><span id=d>d</span></div><div><span id=Z>Z</span></div>",
            &[
                ("S", 0, 5.0),
                ("a", 0, 45.0),
                ("b", 0, 65.0),
                ("c", 1, 5.0),
                ("Z", 1, 25.0),
                ("d", 1, 25.0),
            ],
        ),
        (
            "<div style='height:40px'><span id=S>S</span></div><div><span style='float:right;width:10px;height:10px'></span><span id=a>a</span><br><span id=b>b</span><br><span id=c>c</span><br><span id=d>d</span></div><div><span id=Z>Z</span></div>",
            &[
                ("S", 0, 5.0),
                ("a", 0, 45.0),
                ("b", 0, 65.0),
                ("c", 1, 5.0),
                ("d", 1, 25.0),
                ("Z", 1, 45.0),
            ],
        ),
        (
            "<div style='height:40px'><span id=S>S</span></div><div style='position:relative'><span style='position:absolute;top:0;width:10px;height:10px'></span><span id=a>a</span><br><span id=b>b</span><br><span id=c>c</span><br><span id=d>d</span></div><div><span id=Z>Z</span></div>",
            &[
                ("S", 0, 5.0),
                ("a", 0, 45.0),
                ("b", 0, 65.0),
                ("c", 1, 5.0),
                ("d", 1, 25.0),
                ("Z", 1, 45.0),
            ],
        ),
        (
            "<div style='height:40px'><span id=S>S</span></div><div style='contain:size;height:80px'><span id=a>a</span><br><span id=b>b</span><br><span id=c>c</span><br><span id=d>d</span></div><div><span id=Z>Z</span></div>",
            &[
                ("S", 0, 5.0),
                ("a", 1, 5.0),
                ("b", 1, 25.0),
                ("c", 1, 45.0),
                ("d", 1, 65.0),
                ("Z", 1, 85.0),
            ],
        ),
    ];
    for &(body, expected) in cases {
        assert_places(body, expected);
    }
}

#[test]
fn inline_boxes_before_the_first_text_of_a_paragraph_keep_their_own_lines() {
    // Lines that hold only inline boxes before the paragraph's first text are
    // placed before the text moves its lines, so the lines must not move
    // without them: each letter keeps a line of its own.
    let body = "<div style='height:40px'>S</div><div>\
                <span style='display:inline-block'><span id=a>a</span></span><br>\
                <span style='display:inline-block'><span id=b>b</span></span><br>\
                <span style='display:inline-block'><span id=c>c</span></span><br>\
                <span id=d>d</span></div><div>Z</div>";
    let mut tops: Vec<(usize, i32)> = ["a", "b", "c", "d"]
        .iter()
        .flat_map(|id| fragments_of(body, id))
        .map(|(page, top, _)| (page, top.round() as i32))
        .collect();
    assert_eq!(tops.len(), 4, "{tops:?}");
    tops.sort_unstable();
    tops.dedup();
    assert_eq!(tops.len(), 4, "letters overlap: {tops:?}");
}
