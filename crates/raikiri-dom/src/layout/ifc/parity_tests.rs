use crate::layout::page_pipeline::layout_single_page;
use crate::layout::test_support::with_ahem;
use crate::layout::test_support::{ahem_paragraph, ifc_ahem_fonts, page_box_800x600};

fn size(text: &str, css: &str) -> (f32, f32) {
    let (mut doc, cascade, root) = ahem_paragraph(text, css);
    doc.set_font_collection_with_limits(ifc_ahem_fonts(), shodo::limits::Limits::default());
    layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600()).expect("layout");
    let size = doc.nodes[root].unrounded_layout.size;
    (size.width, size.height)
}

#[test]
fn simple_paragraphs_have_their_hand_computed_box_geometry() {
    let cases = [
        ("aaaa bbbb cccc", "width:50px;line-height:10px"),
        ("aaaa bbbb cccc", "width:200px;line-height:14px"),
        ("aaaa\nbbbb", "width:200px;line-height:10px"),
        (
            "aa bb cc dd",
            "width:35px;line-height:12px;text-align:center",
        ),
    ];
    let mut expected_28 = [(50.0, 30.0), (200.0, 14.0), (200.0, 20.0), (35.0, 48.0)].into_iter();
    for (text, css) in cases {
        assert_eq!(
            expected_28.next().expect("a value per case"),
            size(text, css),
            "{text} / {css}"
        );
    }
}

/// Shapes the former text layout got wrong; the CSS-expected heights are
/// pinned.
#[test]
fn consecutive_breaks_and_a_shrunk_float_have_their_css_heights() {
    // Two consecutive <br> make an empty middle line: three 10px lines.
    let text = "aaaa\n\nbbbb";
    let css = "width:200px;line-height:10px";
    assert_eq!(size(text, css), (200.0, 30.0));

    // A float shrinks to its max-width (30px), which forces the second word
    // onto its own line.
    let text = "aaaa bbbb";
    let css = "float:left;max-width:30px;line-height:10px";
    assert_eq!(size(text, css).1, 20.0);
}

#[test]
fn normal_line_height_matches_for_ahem() {
    let (text, css) = ("aaaa bbbb cccc", "width:50px;line-height:normal");
    assert_eq!((50.0, 30.0), size(text, css));
}
