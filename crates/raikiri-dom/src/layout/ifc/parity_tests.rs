use crate::layout::page_pipeline::layout_single_page;
use crate::layout::test_support::with_ahem;
use crate::layout::test_support::{ahem_paragraph, ifc_ahem_fonts, page_box_800x600};

fn size(text: &str, css: &str, ifc: bool) -> (f32, f32) {
    let (mut doc, cascade, root) = ahem_paragraph(text, css);
    if ifc {
        doc.enable_inline_formatting(ifc_ahem_fonts(), shodo::limits::Limits::default());
    }
    layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600()).expect("layout");
    let size = doc.nodes[root].unrounded_layout.size;
    (size.width, size.height)
}

#[test]
fn simple_paragraphs_have_the_same_box_geometry_with_and_without_the_switch() {
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
            size(text, css, true),
            "{text} / {css}"
        );
    }
}

/// Shapes where the parley path is the one that disagrees with CSS. The
/// shodo path gives the CSS-expected height; these are pinned so a change on
/// either side is noticed instead of being read as an unpainted-text
/// difference.
#[test]
fn known_divergences_where_the_parley_path_is_wrong() {
    // Two consecutive <br> make an empty middle line: three 10px lines.
    let text = "aaaa\n\nbbbb";
    let css = "width:200px;line-height:10px";
    assert_eq!(size(text, css, true), (200.0, 30.0));

    // A float shrinks to its max-width (30px), which forces the second word
    // onto its own line.
    let text = "aaaa bbbb";
    let css = "float:left;max-width:30px;line-height:10px";
    assert_eq!(size(text, css, true).1, 20.0);
}

#[test]
fn normal_line_height_matches_for_ahem() {
    let (text, css) = ("aaaa bbbb cccc", "width:50px;line-height:normal");
    assert_eq!((50.0, 30.0), size(text, css, true));
}
