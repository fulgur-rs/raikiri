use crate::layout::page_pipeline::layout_single_page;
use crate::layout::test_support::{
    ahem_font_context, ahem_paragraph, ifc_ahem_fonts, page_box_800x600,
};

fn size(text: &str, css: &str, ifc: bool) -> (f32, f32) {
    let (mut doc, cascade, root) = ahem_paragraph(text, css);
    if ifc {
        doc.enable_inline_formatting(ifc_ahem_fonts(), shodo::limits::Limits::default());
    }
    layout_single_page(&mut doc, &cascade, page_box_800x600(), ahem_font_context())
        .expect("layout");
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
    for (text, css) in cases {
        assert_eq!(
            size(text, css, false),
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
    // Two consecutive <br> make an empty middle line: three 10px lines. The
    // parley path drops that line.
    let text = "aaaa\n\nbbbb";
    let css = "width:200px;line-height:10px";
    assert_eq!(size(text, css, false), (200.0, 20.0));
    assert_eq!(size(text, css, true), (200.0, 30.0));

    // A float shrinks to its max-width (30px), which forces the second word
    // onto its own line. The parley path does not re-break at that width.
    let text = "aaaa bbbb";
    let css = "float:left;max-width:30px;line-height:10px";
    assert_eq!(size(text, css, false).1, 10.0);
    assert_eq!(size(text, css, true).1, 20.0);
}

#[test]
fn normal_line_height_matches_for_ahem() {
    let (text, css) = ("aaaa bbbb cccc", "width:50px;line-height:normal");
    assert_eq!(size(text, css, false), size(text, css, true));
}
