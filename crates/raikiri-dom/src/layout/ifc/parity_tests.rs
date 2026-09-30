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
