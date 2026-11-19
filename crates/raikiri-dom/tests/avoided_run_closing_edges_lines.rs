//! A run of boxes tied by avoided breaks (`break-after: avoid`) that ends its
//! block with a paragraph, when the paragraph's lines fit the page but the
//! padding and border that close the block do not. The run moves to the next
//! page from the free break before it, and the paragraph breaks between its
//! lines there when the edges still do not fit: a break that leaves fewer
//! lines than `widows`, or one inside a `break-inside: avoid` paragraph, beats
//! one that an avoided break forbids. A paragraph with lines enough for
//! `orphans` and `widows` breaks between them at once. The pages and
//! positions are the ones Chromium prints for the same markup: 200px by 100px
//! pages without margins, Ahem at 10px on 20px lines.
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
fn a_run_ending_with_a_paragraph_moves_or_splits_it_for_the_closing_edges() {
    let cases: &[Case] = &[
        (
            "<div style='padding-bottom:40px'><div style='height:40px'><span id=H>H</span></div><div style='break-after:avoid'></div><div><span id=a>a</span><br><span id=b>b</span></div></div><div><span id=Z>Z</span></div>",
            &[("H", 0, 5.0), ("a", 1, 5.0), ("b", 1, 25.0), ("Z", 1, 85.0)],
        ),
        (
            "<div style='padding-bottom:40px'><div style='height:20px'><span id=H>H</span></div><div style='break-after:avoid'></div><div><span id=a>a</span><br><span id=b>b</span><br><span id=c>c</span></div></div><div><span id=Z>Z</span></div>",
            &[
                ("H", 0, 5.0),
                ("a", 1, 5.0),
                ("b", 1, 25.0),
                ("c", 1, 45.0),
                ("Z", 2, 5.0),
            ],
        ),
        (
            "<div style='padding-bottom:40px'><div style='height:10px'><span id=H>H</span></div><div style='break-after:avoid'></div><div><span id=a>a</span><br><span id=b>b</span><br><span id=c>c</span><br><span id=d>d</span></div></div><div><span id=Z>Z</span></div>",
            &[
                ("H", 0, 5.0),
                ("a", 0, 15.0),
                ("b", 0, 35.0),
                ("c", 1, 5.0),
                ("d", 1, 25.0),
                ("Z", 1, 85.0),
            ],
        ),
        (
            "<div style='padding-bottom:40px'><div style='height:10px'><span id=H>H</span></div><div style='break-after:avoid;height:20px'><span id=M>M</span></div><div><span id=a>a</span><br><span id=b>b</span><br><span id=c>c</span></div></div><div><span id=Z>Z</span></div>",
            &[
                ("H", 0, 5.0),
                ("M", 1, 5.0),
                ("a", 1, 25.0),
                ("b", 1, 45.0),
                ("c", 2, 5.0),
                ("Z", 2, 65.0),
            ],
        ),
        (
            "<div style='padding-bottom:40px'><div style='height:5px'><span id=H>H</span></div><div style='break-after:avoid;height:10px'><span id=M>M</span></div><div><span id=a>a</span><br><span id=b>b</span><br><span id=c>c</span><br><span id=d>d</span></div></div><div><span id=Z>Z</span></div>",
            &[
                ("H", 0, 5.0),
                ("M", 0, 10.0),
                ("a", 0, 20.0),
                ("b", 0, 40.0),
                ("c", 1, 5.0),
                ("d", 1, 25.0),
                ("Z", 1, 85.0),
            ],
        ),
        (
            "<div style='padding-bottom:40px'><div style='height:20px'><span id=H>H</span></div><div style='break-after:avoid;height:20px'><span id=M>M</span></div><div><span id=a>a</span><br><span id=b>b</span></div></div><div><span id=Z>Z</span></div>",
            &[
                ("H", 0, 5.0),
                ("M", 1, 5.0),
                ("a", 1, 25.0),
                ("b", 1, 45.0),
                ("Z", 2, 5.0),
            ],
        ),
    ];
    for &(body, expected) in cases {
        assert_places(body, expected);
    }
}

#[test]
fn a_tied_paragraph_breaks_between_its_lines_rather_than_before_itself() {
    let cases: &[Case] = &[
        (
            "<div style='padding-bottom:40px'><div style='height:30px'><span id=H>H</span></div><div style='break-after:avoid;height:10px'><span id=M>M</span></div><div><span id=a>a</span><br><span id=b>b</span></div></div><div><span id=Z>Z</span></div>",
            &[
                ("H", 0, 5.0),
                ("M", 1, 5.0),
                ("a", 1, 15.0),
                ("b", 1, 35.0),
                ("Z", 2, 5.0),
            ],
        ),
        (
            "<div style='padding-bottom:30px'><div style='height:10px'><span id=H>H</span></div><div style='break-after:avoid;height:20px'><span id=M>M</span></div><div style='orphans:1;widows:1'><span id=a>a</span><br><span id=b>b</span><br><span id=c>c</span></div></div><div><span id=Z>Z</span></div>",
            &[
                ("H", 0, 5.0),
                ("M", 0, 15.0),
                ("a", 0, 35.0),
                ("b", 0, 55.0),
                ("c", 1, 5.0),
                ("Z", 1, 55.0),
            ],
        ),
        (
            "<div style='padding-bottom:40px'><div style='height:10px'><span id=H>H</span></div><div style='break-after:avoid;height:20px'><span id=M>M</span></div><div style='break-inside:avoid'><span id=a>a</span><br><span id=b>b</span><br><span id=c>c</span></div></div><div><span id=Z>Z</span></div>",
            &[
                ("H", 0, 5.0),
                ("M", 1, 5.0),
                ("a", 1, 25.0),
                ("b", 1, 45.0),
                ("c", 2, 5.0),
                ("Z", 2, 65.0),
            ],
        ),
        (
            "<div style='padding-bottom:40px'><div style='height:10px'><span id=H>H</span></div><div style='break-after:avoid;height:20px'><span id=M>M</span></div><div style='break-after:avoid;height:10px'><span id=N>N</span></div><div><span id=a>a</span><br><span id=b>b</span></div></div><div><span id=Z>Z</span></div>",
            &[
                ("H", 0, 5.0),
                ("M", 1, 5.0),
                ("a", 1, 35.0),
                ("b", 2, 5.0),
                ("Z", 2, 65.0),
            ],
        ),
        (
            "<div style='border-bottom:40px solid'><div style='height:20px'><span id=H>H</span></div><div style='break-after:avoid'></div><div><span id=a>a</span><br><span id=b>b</span><br><span id=c>c</span></div></div><div><span id=Z>Z</span></div>",
            &[
                ("H", 0, 5.0),
                ("a", 1, 5.0),
                ("b", 1, 25.0),
                ("c", 1, 45.0),
                ("Z", 2, 5.0),
            ],
        ),
        (
            "<div><div style='padding-bottom:40px'><div style='height:10px'><span id=H>H</span></div><div style='break-after:avoid;height:20px'><span id=M>M</span></div><div><span id=a>a</span><br><span id=b>b</span><br><span id=c>c</span></div></div></div><div><span id=Z>Z</span></div>",
            &[
                ("H", 0, 5.0),
                ("M", 1, 5.0),
                ("a", 1, 25.0),
                ("b", 1, 45.0),
                ("c", 2, 5.0),
                ("Z", 2, 65.0),
            ],
        ),
    ];
    for &(body, expected) in cases {
        assert_places(body, expected);
    }
}

#[test]
fn a_tied_paragraph_with_padding_keeps_its_page_when_its_lines_fit() {
    let cases: &[Case] = &[(
        "<div style='padding-bottom:20px'><div style='height:20px'><span id=S>S</span></div><div style='break-after:avoid;padding-bottom:5px'><span id=a>a</span><br><span id=b>b</span><br><span id=c>c</span></div><div style='height:5px'><span id=L>L</span></div></div><div><span id=Z>Z</span></div>",
        &[
            ("S", 0, 5.0),
            ("a", 0, 25.0),
            ("b", 0, 45.0),
            ("c", 0, 65.0),
            ("L", 0, 90.0),
            ("Z", 1, 25.0),
        ],
    )];
    for &(body, expected) in cases {
        assert_places(body, expected);
    }
}
