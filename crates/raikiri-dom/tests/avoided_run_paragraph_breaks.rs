//! A run of boxes tied by avoided breaks (`break-after: avoid`,
//! `break-before: avoid`) that holds a paragraph breaks between the lines of
//! that paragraph when the page ends inside the run, rather than before the
//! run: the paragraph's last lines go to the next page with the boxes tied
//! after it, and with the padding and border that close their block. The
//! pages and positions are the ones Chromium prints for the same markup:
//! 200px by 100px pages without margins, Ahem at 10px on 20px lines.
use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/data/text-autospace/Ahem.ttf"
));

/// The page index, top and bottom on that page of each fragment of the
/// element with the id `id`.
/// A document body and where its letters land: (id, page, top).
type Case<'a> = (&'a str, &'a [(&'a str, usize, f32)]);

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
fn a_tied_paragraph_breaks_between_its_lines_for_the_edges_that_close_its_block() {
    let cases: &[Case] = &[
        (
            "<div style='padding-bottom:20px'><div style='height:20px'><span id=S>S</span></div><div style='break-after:avoid'><span id=a>a</span><br><span id=b>b</span><br><span id=c>c</span></div><div style='height:10px'><span id=L>L</span></div></div><div><span id=Z>Z</span></div>",
            &[
                ("S", 0, 5.0),
                ("a", 1, 5.0),
                ("b", 1, 25.0),
                ("c", 1, 45.0),
                ("L", 1, 65.0),
                ("Z", 2, 5.0),
            ],
        ),
        (
            "<div style='padding-bottom:20px'><div style='break-after:avoid'><span id=a>a</span><br><span id=b>b</span><br><span id=c>c</span><br><span id=d>d</span></div><div style='height:10px'><span id=L>L</span></div></div><div><span id=Z>Z</span></div>",
            &[
                ("a", 0, 5.0),
                ("b", 0, 25.0),
                ("c", 1, 5.0),
                ("d", 1, 25.0),
                ("L", 1, 45.0),
                ("Z", 1, 75.0),
            ],
        ),
        (
            "<div style='border-bottom:20px solid'><div style='height:20px'><span id=S>S</span></div><div style='break-after:avoid'><span id=a>a</span><br><span id=b>b</span><br><span id=c>c</span></div><div style='height:10px'><span id=L>L</span></div></div><div><span id=Z>Z</span></div>",
            &[
                ("S", 0, 5.0),
                ("a", 1, 5.0),
                ("b", 1, 25.0),
                ("c", 1, 45.0),
                ("L", 1, 65.0),
                ("Z", 2, 5.0),
            ],
        ),
        (
            "<div style='padding-bottom:20px'><div style='height:10px'><span id=S>S</span></div><div style='break-after:avoid'><span id=a>a</span><br><span id=b>b</span><br><span id=c>c</span></div><div style='break-after:avoid;height:10px'><span id=M>M</span></div><div style='height:10px'><span id=L>L</span></div></div><div><span id=Z>Z</span></div>",
            &[
                ("S", 0, 5.0),
                ("a", 1, 5.0),
                ("b", 1, 25.0),
                ("c", 1, 45.0),
                ("M", 1, 65.0),
                ("L", 1, 75.0),
                ("Z", 2, 5.0),
            ],
        ),
        (
            "<div style='padding-bottom:20px'><div style='height:20px'><span id=S>S</span></div><div><span id=a>a</span><br><span id=b>b</span><br><span id=c>c</span></div><div style='height:10px;break-before:avoid'><span id=L>L</span></div></div><div><span id=Z>Z</span></div>",
            &[
                ("S", 0, 5.0),
                ("a", 1, 5.0),
                ("b", 1, 25.0),
                ("c", 1, 45.0),
                ("L", 1, 65.0),
                ("Z", 2, 5.0),
            ],
        ),
        (
            "<div style='padding-bottom:20px'><div style='height:40px'><span id=S>S</span></div><div style='break-after:avoid'><span id=a>a</span><br><span id=b>b</span></div><div style='height:10px'><span id=L>L</span></div></div><div><span id=Z>Z</span></div>",
            &[
                ("S", 0, 5.0),
                ("a", 1, 5.0),
                ("b", 1, 25.0),
                ("L", 1, 45.0),
                ("Z", 1, 75.0),
            ],
        ),
        (
            "<div style='padding-bottom:20px'><div style='height:20px'><span id=S>S</span></div><div style='break-after:avoid'><span id=a>a</span><br><span id=b>b</span><br><span id=c>c</span></div><div style='height:10px'><span id=L>L</span></div></div><div style='height:0'></div><div><span id=Z>Z</span></div>",
            &[
                ("S", 0, 5.0),
                ("a", 1, 5.0),
                ("b", 1, 25.0),
                ("c", 1, 45.0),
                ("L", 1, 65.0),
                ("Z", 2, 5.0),
            ],
        ),
        (
            "<div style='padding-bottom:20px'><div style='height:10px'><span id=S>S</span></div><div style='break-after:avoid'><span id=a>a</span><br><span id=b>b</span><br><span id=c>c</span><br><span id=d>d</span></div><div style='height:10px'></div></div><div><span id=Z>Z</span></div>",
            &[
                ("S", 0, 5.0),
                ("a", 0, 15.0),
                ("b", 0, 35.0),
                ("c", 1, 5.0),
                ("d", 1, 25.0),
                ("Z", 1, 75.0),
            ],
        ),
        (
            "<div style='padding-bottom:20px'><div style='height:10px;break-after:avoid'><span id=S>S</span></div><div style='break-after:avoid'><span id=a>a</span><br><span id=b>b</span><br><span id=c>c</span><br><span id=d>d</span></div><div style='height:10px'></div></div><div><span id=Z>Z</span></div>",
            &[
                ("S", 0, 5.0),
                ("a", 0, 15.0),
                ("b", 0, 35.0),
                ("c", 1, 5.0),
                ("d", 1, 25.0),
                ("Z", 1, 75.0),
            ],
        ),
        (
            "<div style='padding-bottom:20px'><div style='height:10px'><span id=S>S</span></div><div style='break-after:avoid;orphans:1;widows:1'><span id=a>a</span><br><span id=b>b</span><br><span id=c>c</span><br><span id=d>d</span></div><div style='height:10px'></div></div><div><span id=Z>Z</span></div>",
            &[
                ("S", 0, 5.0),
                ("a", 0, 15.0),
                ("b", 0, 35.0),
                ("c", 0, 55.0),
                ("d", 1, 5.0),
                ("Z", 1, 55.0),
            ],
        ),
        (
            "<div style='padding-bottom:20px'><div style='height:10px'><span id=S>S</span></div><div style='break-after:avoid'><span id=a>a</span><br><span id=b>b</span><br><span id=c>c</span><br><span id=d>d</span></div><div style='break-after:avoid;height:5px'></div><div style='height:5px'></div></div><div><span id=Z>Z</span></div>",
            &[
                ("S", 0, 5.0),
                ("a", 0, 15.0),
                ("b", 0, 35.0),
                ("c", 1, 5.0),
                ("d", 1, 25.0),
                ("Z", 1, 75.0),
            ],
        ),
        (
            "<div style='padding-bottom:20px'><div style='break-after:avoid'><span id=a>a</span><br><span id=b>b</span><br><span id=c>c</span><br><span id=d>d</span></div><div style='height:50px'><span id=L>L</span></div></div><div><span id=Z>Z</span></div>",
            &[
                ("a", 0, 5.0),
                ("b", 0, 25.0),
                ("c", 0, 45.0),
                ("d", 0, 65.0),
                ("L", 0, 85.0),
                ("Z", 1, 55.0),
            ],
        ),
        (
            "<div style='padding-bottom:20px'><div style='height:20px'><span id=S>S</span></div><div style='break-after:avoid'><span id=a>a</span><br><span id=b>b</span><br><span id=c>c</span></div><div style='height:20px'><span id=L>L</span></div></div><div><span id=Z>Z</span></div>",
            &[
                ("S", 0, 5.0),
                ("a", 1, 5.0),
                ("b", 1, 25.0),
                ("c", 1, 45.0),
                ("L", 1, 65.0),
                ("Z", 2, 5.0),
            ],
        ),
        (
            "<div style='padding-bottom:30px'><div style='height:30px'><span id=S>S</span></div><div style='break-after:avoid'><span id=a>a</span><br><span id=b>b</span><br><span id=c>c</span><br><span id=d>d</span></div><div style='height:20px'><span id=L>L</span></div></div><div><span id=Z>Z</span></div>",
            &[
                ("S", 0, 5.0),
                ("a", 0, 35.0),
                ("b", 0, 55.0),
                ("c", 1, 5.0),
                ("d", 1, 25.0),
                ("L", 1, 45.0),
                ("Z", 2, 5.0),
            ],
        ),
        (
            "<div style='padding-bottom:30px'><div style='break-after:avoid'><span id=a>a</span><br><span id=b>b</span><br><span id=c>c</span><br><span id=d>d</span></div><div style='height:40px'><span id=L>L</span></div></div><div><span id=Z>Z</span></div>",
            &[
                ("a", 0, 5.0),
                ("b", 0, 25.0),
                ("c", 0, 45.0),
                ("d", 0, 65.0),
                ("L", 0, 85.0),
                ("Z", 1, 55.0),
            ],
        ),
        (
            "<div style='padding-bottom:20px'><div style='height:20px'><span id=S>S</span></div><div style='break-after:avoid'><span id=a>a</span><br><span id=b>b</span><br><span id=c>c</span><br><span id=d>d</span><br><span id=e>e</span></div><div style='height:20px'><span id=L>L</span></div></div><div><span id=Z>Z</span></div>",
            &[
                ("S", 0, 5.0),
                ("a", 0, 25.0),
                ("b", 0, 45.0),
                ("c", 0, 65.0),
                ("d", 1, 5.0),
                ("e", 1, 25.0),
                ("L", 1, 45.0),
                ("Z", 1, 85.0),
            ],
        ),
        (
            "<div style='border-bottom:30px solid'><div style='height:30px'><span id=S>S</span></div><div style='break-after:avoid'><span id=a>a</span><br><span id=b>b</span><br><span id=c>c</span><br><span id=d>d</span></div><div style='height:20px'><span id=L>L</span></div></div><div><span id=Z>Z</span></div>",
            &[
                ("S", 0, 5.0),
                ("a", 0, 35.0),
                ("b", 0, 55.0),
                ("c", 1, 5.0),
                ("d", 1, 25.0),
                ("L", 1, 45.0),
                ("Z", 2, 5.0),
            ],
        ),
        (
            "<div style='padding-bottom:30px'><div style='height:30px'><span id=S>S</span></div><div style='break-after:avoid'><span id=a>a</span><br><span id=b>b</span><br><span id=c>c</span><br><span id=d>d</span></div><div style='break-after:avoid;height:10px'><span id=M>M</span></div><div style='height:10px'><span id=L>L</span></div></div><div><span id=Z>Z</span></div>",
            &[
                ("S", 0, 5.0),
                ("a", 0, 35.0),
                ("b", 0, 55.0),
                ("c", 1, 5.0),
                ("d", 1, 25.0),
                ("M", 1, 45.0),
                ("L", 1, 55.0),
                ("Z", 2, 5.0),
            ],
        ),
        (
            "<div style='padding-bottom:30px'><div style='height:30px'><span id=S>S</span></div><div><span id=a>a</span><br><span id=b>b</span><br><span id=c>c</span><br><span id=d>d</span></div><div style='height:20px;break-before:avoid'><span id=L>L</span></div></div><div><span id=Z>Z</span></div>",
            &[
                ("S", 0, 5.0),
                ("a", 0, 35.0),
                ("b", 0, 55.0),
                ("c", 1, 5.0),
                ("d", 1, 25.0),
                ("L", 1, 45.0),
                ("Z", 2, 5.0),
            ],
        ),
    ];
    for &(body, expected) in cases {
        assert_places(body, expected);
    }
}

#[test]
fn a_tied_paragraph_breaks_between_its_lines_for_the_boxes_tied_after_it() {
    let cases: &[Case] = &[
        (
            "<div><div style='height:30px'><span id=S>S</span></div><div style='break-after:avoid'><span id=a>a</span><br><span id=b>b</span><br><span id=c>c</span><br><span id=d>d</span></div><div style='height:20px'><span id=L>L</span></div></div><div><span id=Z>Z</span></div>",
            &[
                ("S", 0, 5.0),
                ("a", 0, 35.0),
                ("b", 0, 55.0),
                ("c", 1, 5.0),
                ("d", 1, 25.0),
                ("L", 1, 45.0),
                ("Z", 1, 65.0),
            ],
        ),
        (
            "<div><div style='height:30px'><span id=S>S</span></div><div><span id=a>a</span><br><span id=b>b</span><br><span id=c>c</span><br><span id=d>d</span></div><div style='height:20px'><span id=L>L</span></div></div><div><span id=Z>Z</span></div>",
            &[
                ("S", 0, 5.0),
                ("a", 0, 35.0),
                ("b", 0, 55.0),
                ("c", 1, 5.0),
                ("d", 1, 25.0),
                ("L", 1, 45.0),
                ("Z", 1, 65.0),
            ],
        ),
        (
            "<div style='padding-bottom:30px'><div style='height:30px'><span id=S>S</span></div><div><span id=a>a</span><br><span id=b>b</span><br><span id=c>c</span><br><span id=d>d</span></div><div style='height:20px'><span id=L>L</span></div></div><div><span id=Z>Z</span></div>",
            &[
                ("S", 0, 5.0),
                ("a", 0, 35.0),
                ("b", 0, 55.0),
                ("c", 1, 5.0),
                ("d", 1, 25.0),
                ("L", 1, 45.0),
                ("Z", 2, 5.0),
            ],
        ),
        (
            "<div style='padding-bottom:30px'><div style='height:30px'><span id=S>S</span></div><div style='height:80px'><span id=a>a</span><br><span id=b>b</span><br><span id=c>c</span><br><span id=d>d</span></div><div style='height:20px'><span id=L>L</span></div></div><div><span id=Z>Z</span></div>",
            &[
                ("S", 0, 5.0),
                ("a", 0, 35.0),
                ("b", 0, 55.0),
                ("c", 1, 5.0),
                ("L", 1, 15.0),
                ("d", 1, 25.0),
                ("Z", 1, 65.0),
            ],
        ),
        (
            "<div><div style='height:20px'><span id=S>S</span></div><div style='break-after:avoid'><span id=a>a</span><br><span id=b>b</span><br><span id=c>c</span><br><span id=d>d</span></div><div style='height:20px'><span id=L>L</span></div></div><div><span id=Z>Z</span></div>",
            &[
                ("S", 0, 5.0),
                ("a", 0, 25.0),
                ("b", 0, 45.0),
                ("c", 1, 5.0),
                ("d", 1, 25.0),
                ("L", 1, 45.0),
                ("Z", 1, 65.0),
            ],
        ),
        (
            "<div><div style='height:20px'><span id=S>S</span></div><div><span id=a>a</span><br><span id=b>b</span><br><span id=c>c</span><br><span id=d>d</span></div><div style='height:20px;break-before:avoid'><span id=L>L</span></div></div><div><span id=Z>Z</span></div>",
            &[
                ("S", 0, 5.0),
                ("a", 0, 25.0),
                ("b", 0, 45.0),
                ("c", 1, 5.0),
                ("d", 1, 25.0),
                ("L", 1, 45.0),
                ("Z", 1, 65.0),
            ],
        ),
        (
            "<div><div style='height:40px'><span id=S>S</span></div><div style='break-after:avoid'><span id=a>a</span><br><span id=b>b</span><br><span id=c>c</span></div><div style='height:20px'><span id=L>L</span></div></div><div><span id=Z>Z</span></div>",
            &[
                ("S", 0, 5.0),
                ("a", 1, 5.0),
                ("b", 1, 25.0),
                ("c", 1, 45.0),
                ("L", 1, 65.0),
                ("Z", 1, 85.0),
            ],
        ),
        (
            "<div><div style='height:30px'><span id=S>S</span></div><div style='break-after:avoid'><span id=a>a</span><br><span id=b>b</span><br><span id=c>c</span><br><span id=d>d</span></div><div style='break-after:avoid;height:20px'><span id=M>M</span></div><div style='height:20px'><span id=L>L</span></div></div><div><span id=Z>Z</span></div>",
            &[
                ("S", 0, 5.0),
                ("a", 0, 35.0),
                ("b", 0, 55.0),
                ("c", 1, 5.0),
                ("d", 1, 25.0),
                ("M", 1, 45.0),
                ("L", 1, 65.0),
                ("Z", 1, 85.0),
            ],
        ),
        (
            "<div><div style='height:10px;break-after:avoid'><span id=S>S</span></div><div style='break-after:avoid'><span id=a>a</span><br><span id=b>b</span><br><span id=c>c</span><br><span id=d>d</span></div><div style='height:20px'><span id=L>L</span></div></div><div><span id=Z>Z</span></div>",
            &[
                ("S", 0, 5.0),
                ("a", 0, 15.0),
                ("b", 0, 35.0),
                ("c", 1, 5.0),
                ("d", 1, 25.0),
                ("L", 1, 45.0),
                ("Z", 1, 65.0),
            ],
        ),
        (
            "<div><div style='height:30px'><span id=S>S</span></div><div style='break-after:avoid'><span id=a>a</span><br><span id=b>b</span><br><span id=c>c</span><br><span id=d>d</span><br><span id=e>e</span></div><div style='height:20px'><span id=L>L</span></div></div><div><span id=Z>Z</span></div>",
            &[
                ("S", 0, 5.0),
                ("a", 0, 35.0),
                ("b", 0, 55.0),
                ("c", 0, 75.0),
                ("d", 1, 5.0),
                ("e", 1, 25.0),
                ("L", 1, 45.0),
                ("Z", 1, 65.0),
            ],
        ),
        (
            "<div><div style='break-after:avoid'><span id=a>a</span><br><span id=b>b</span><br><span id=c>c</span><br><span id=d>d</span></div><div style='height:50px'><span id=L>L</span></div></div><div><span id=Z>Z</span></div>",
            &[
                ("a", 0, 5.0),
                ("b", 0, 25.0),
                ("c", 0, 45.0),
                ("d", 0, 65.0),
                ("L", 0, 85.0),
                ("Z", 1, 35.0),
            ],
        ),
        (
            "<div><div style='height:10px'><span id=S>S</span></div><div style='break-after:avoid'><span id=a>a</span><br><span id=b>b</span><br><span id=c>c</span><br><span id=d>d</span></div><div style='height:30px'><span id=L>L</span></div></div><div><span id=Z>Z</span></div>",
            &[
                ("S", 0, 5.0),
                ("a", 0, 15.0),
                ("b", 0, 35.0),
                ("c", 1, 5.0),
                ("d", 1, 25.0),
                ("L", 1, 45.0),
                ("Z", 1, 75.0),
            ],
        ),
        (
            "<div><div style='height:10px'><span id=S>S</span></div><div style='break-after:avoid'><span id=a>a</span><br><span id=b>b</span><br><span id=c>c</span><br><span id=d>d</span></div><div><span id=L>L</span><br><span id=M>M</span><br><span id=Q>Q</span></div></div><div><span id=Z>Z</span></div>",
            &[
                ("S", 0, 5.0),
                ("a", 0, 15.0),
                ("b", 0, 35.0),
                ("c", 1, 5.0),
                ("d", 1, 25.0),
                ("L", 1, 45.0),
                ("M", 1, 65.0),
                ("Q", 1, 85.0),
                ("Z", 2, 5.0),
            ],
        ),
        (
            "<div><div style='height:10px'><span id=S>S</span></div><div style='break-after:avoid'><span id=a>a</span><br><span id=b>b</span><br><span id=c>c</span><br><span id=d>d</span></div><div><div style='height:20px'><span id=L>L</span></div></div></div><div><span id=Z>Z</span></div>",
            &[
                ("S", 0, 5.0),
                ("a", 0, 15.0),
                ("b", 0, 35.0),
                ("c", 1, 5.0),
                ("d", 1, 25.0),
                ("L", 1, 45.0),
                ("Z", 1, 65.0),
            ],
        ),
    ];
    for &(body, expected) in cases {
        assert_places(body, expected);
    }
}

#[test]
fn a_tied_paragraph_among_the_boxes_of_the_body_breaks_between_its_lines() {
    let cases: &[Case] = &[
        (
            "<div style='height:30px'><span id=S>S</span></div><div style='break-after:avoid'><span id=a>a</span><br><span id=b>b</span><br><span id=c>c</span><br><span id=d>d</span></div><div style='height:20px'><span id=L>L</span></div><div><span id=Z>Z</span></div>",
            &[
                ("S", 0, 5.0),
                ("a", 0, 35.0),
                ("b", 0, 55.0),
                ("c", 1, 5.0),
                ("d", 1, 25.0),
                ("L", 1, 45.0),
                ("Z", 1, 65.0),
            ],
        ),
        (
            "<div style='height:20px'><span id=S>S</span></div><div style='break-after:avoid'><span id=a>a</span><br><span id=b>b</span><br><span id=c>c</span><br><span id=d>d</span></div><div style='height:20px'><span id=L>L</span></div><div><span id=Z>Z</span></div>",
            &[
                ("S", 0, 5.0),
                ("a", 0, 25.0),
                ("b", 0, 45.0),
                ("c", 1, 5.0),
                ("d", 1, 25.0),
                ("L", 1, 45.0),
                ("Z", 1, 65.0),
            ],
        ),
        (
            "<div style='height:10px;break-after:avoid'><span id=S>S</span></div><div style='break-after:avoid'><span id=a>a</span><br><span id=b>b</span><br><span id=c>c</span><br><span id=d>d</span></div><div style='height:20px'><span id=L>L</span></div><div><span id=Z>Z</span></div>",
            &[
                ("S", 0, 5.0),
                ("a", 0, 15.0),
                ("b", 0, 35.0),
                ("c", 1, 5.0),
                ("d", 1, 25.0),
                ("L", 1, 45.0),
                ("Z", 1, 65.0),
            ],
        ),
        (
            "<div style='height:40px'><span id=S>S</span></div><div style='break-after:avoid'><span id=a>a</span><br><span id=b>b</span><br><span id=c>c</span></div><div style='height:20px'><span id=L>L</span></div><div><span id=Z>Z</span></div>",
            &[
                ("S", 0, 5.0),
                ("a", 1, 5.0),
                ("b", 1, 25.0),
                ("c", 1, 45.0),
                ("L", 1, 65.0),
                ("Z", 1, 85.0),
            ],
        ),
        (
            "<div style='height:30px'><span id=S>S</span></div><div style='break-after:avoid'><span id=a>a</span><br><span id=b>b</span><br><span id=c>c</span><br><span id=d>d</span></div><div style='height:20px;break-after:avoid'><span id=M>M</span></div><div style='height:20px'><span id=L>L</span></div><div><span id=Z>Z</span></div>",
            &[
                ("S", 0, 5.0),
                ("a", 0, 35.0),
                ("b", 0, 55.0),
                ("c", 1, 5.0),
                ("d", 1, 25.0),
                ("M", 1, 45.0),
                ("L", 1, 65.0),
                ("Z", 1, 85.0),
            ],
        ),
        (
            "<div style='height:10px'><span id=S>S</span></div><div style='break-after:avoid'><span id=a>a</span><br><span id=b>b</span><br><span id=c>c</span><br><span id=d>d</span></div><div style='height:30px'><span id=L>L</span></div><div><span id=Z>Z</span></div>",
            &[
                ("S", 0, 5.0),
                ("a", 0, 15.0),
                ("b", 0, 35.0),
                ("c", 1, 5.0),
                ("d", 1, 25.0),
                ("L", 1, 45.0),
                ("Z", 1, 75.0),
            ],
        ),
        (
            "<div style='height:10px'><span id=S>S</span></div><div><span id=a>a</span><br><span id=b>b</span><br><span id=c>c</span><br><span id=d>d</span></div><div style='break-before:avoid'><div style='height:20px'><span id=L>L</span></div></div><div><span id=Z>Z</span></div>",
            &[
                ("S", 0, 5.0),
                ("a", 0, 15.0),
                ("b", 0, 35.0),
                ("c", 1, 5.0),
                ("d", 1, 25.0),
                ("L", 1, 45.0),
                ("Z", 1, 65.0),
            ],
        ),
    ];
    for &(body, expected) in cases {
        assert_places(body, expected);
    }
}
