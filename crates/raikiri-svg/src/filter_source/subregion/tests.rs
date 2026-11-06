use crate::filter_source::like_browsers;
use crate::{SvgDocument, SvgRootStyle, SvgViewport};
use std::borrow::Cow;

/// Turns what is there to red where the source has an alpha, so that a
/// subregion shows as a red rectangle on the rectangle the filter is on.
const RED: &str = "type='matrix' values='0 0 0 0 1  0 0 0 0 0  0 0 0 0 0  0 0 0 1 0'";

/// A rectangle of 40 by 40 at 20, 20 under the filter `f`, which is made of
/// `filter` (its attributes) and `body` (its primitives).
fn scene(filter: &str, body: &str) -> String {
    format!(
        "<svg xmlns='http://www.w3.org/2000/svg' width='100' height='100'>\
         <filter id='f' {filter}>{body}</filter>\
         <rect x='20' y='20' width='40' height='40' fill='green' filter='url(#f)'/></svg>"
    )
}

/// [`scene`] in the units of the bounding box, with one primitive `primitive`.
fn fractions(primitive: &str) -> String {
    scene("primitiveUnits='objectBoundingBox'", primitive)
}

/// [`scene`] in the units of the user space, with one primitive `primitive`.
fn pixels(primitive: &str) -> String {
    scene("primitiveUnits='userSpaceOnUse'", primitive)
}

fn rasterized(source: &str) -> Vec<u8> {
    let viewport = SvgViewport {
        width: 100.0,
        height: 100.0,
    };
    SvgDocument::parse(source.as_bytes())
        .unwrap()
        .rasterize(viewport, SvgRootStyle::default(), None)
        .unwrap()
        .rgba
}

fn pixel(image: &[u8], x: usize, y: usize) -> [u8; 4] {
    image[(y * 100 + x) * 4..][..4].try_into().unwrap()
}

const CLEAR: [u8; 4] = [0, 0, 0, 0];
const SOLID_RED: [u8; 4] = [255, 0, 0, 255];

/// Draws `fractions` and `pixels` and asserts they are the same picture, which
/// is not an empty one. Returns it.
fn same_picture(fractions: &str, pixels: &str) -> Vec<u8> {
    let (drawn, wanted) = (rasterized(fractions), rasterized(pixels));
    assert!(
        wanted.chunks_exact(4).any(|pixel| pixel[3] > 0),
        "the reference is empty: {pixels}"
    );
    assert!(drawn == wanted, "{fractions}\n{pixels}");
    wanted
}

#[test]
fn a_subregion_is_a_fraction_of_the_bounding_box() {
    // The box is 20, 20, 40, 40: a quarter and a half of it is 30, 30, 20, 20.
    for (x, y, width, height) in [
        ("25%", "25%", "50%", "50%"),
        ("0.25", "0.25", "0.5", "0.5"),
        ("25%", "0.25", "0.5", "50%"),
    ] {
        let drawn = same_picture(
            &fractions(&format!(
                "<feColorMatrix {RED} x='{x}' y='{y}' width='{width}' height='{height}'/>"
            )),
            &pixels(&format!(
                "<feColorMatrix {RED} x='30' y='30' width='20' height='20'/>"
            )),
        );
        assert_eq!(pixel(&drawn, 35, 35), SOLID_RED);
        assert_eq!(pixel(&drawn, 25, 25), CLEAR);
        assert_eq!(pixel(&drawn, 55, 55), CLEAR);
    }
}

#[test]
fn a_subregion_reaches_beyond_the_box() {
    // From -50% to 150% of the box, which the filter region cuts at 4 and 76.
    same_picture(
        &fractions(&format!(
            "<feColorMatrix {RED} x='-50%' y='0' width='200%' height='25%'/>"
        )),
        &pixels(&format!(
            "<feColorMatrix {RED} x='0' y='20' width='80' height='10'/>"
        )),
    );
}

#[test]
fn every_primitive_but_a_flood_or_an_image_is_clipped() {
    // The same primitive in the units of the box and in those of the user
    // space, which is 40 by 40 here: a size of a fortieth is a unit.
    for (box_units, user_units) in [
        (
            "<feBlend in2='SourceGraphic' mode='multiply' ",
            "<feBlend in2='SourceGraphic' mode='multiply' ",
        ),
        (
            "<feComposite in2='SourceGraphic' operator='over' ",
            "<feComposite in2='SourceGraphic' operator='over' ",
        ),
        (
            "<feGaussianBlur stdDeviation='0' ",
            "<feGaussianBlur stdDeviation='0' ",
        ),
        ("<feMorphology radius='0.025' ", "<feMorphology radius='1' "),
        ("<feComponentTransfer ", "<feComponentTransfer "),
        (
            "<feDropShadow dx='0' dy='0' stdDeviation='0' flood-color='red' ",
            "<feDropShadow dx='0' dy='0' stdDeviation='0' flood-color='red' ",
        ),
        (
            "<feConvolveMatrix order='1' kernelMatrix='1' ",
            "<feConvolveMatrix order='1' kernelMatrix='1' ",
        ),
        (
            "<feDisplacementMap in2='SourceGraphic' scale='0' ",
            "<feDisplacementMap in2='SourceGraphic' scale='0' ",
        ),
    ] {
        let end = if box_units.starts_with("<feComponentTransfer") {
            "></feComponentTransfer>"
        } else {
            "/>"
        };
        let drawn = same_picture(
            &fractions(&format!(
                "{box_units}x='25%' y='25%' width='50%' height='50%'{end}"
            )),
            &pixels(&format!(
                "{user_units}x='30' y='30' width='20' height='20'{end}"
            )),
        );
        assert_eq!(pixel(&drawn, 25, 25), CLEAR, "{box_units}");
        assert_eq!(pixel(&drawn, 35, 35)[3], 255, "{box_units}");
    }
}

#[test]
fn an_offset_is_clipped_too() {
    // `usvg` does not clip it in the units of the user space, where the whole
    // box is shifted; here the subregion is a clip like any other.
    let drawn = rasterized(&fractions(
        "<feOffset dx='0' dy='0' x='25%' y='25%' width='50%' height='50%'/>",
    ));
    assert_eq!(pixel(&drawn, 35, 35), [0, 128, 0, 255]);
    assert_eq!(pixel(&drawn, 25, 25), CLEAR);
    assert_eq!(pixel(&drawn, 55, 55), CLEAR);
}

#[test]
fn a_merge_is_clipped_with_the_nodes_it_holds() {
    same_picture(
        &fractions(
            "<feMerge x='25%' y='25%' width='50%' height='50%'>\
             <feMergeNode in='SourceGraphic'/><feMergeNode in='SourceGraphic'/></feMerge>",
        ),
        &pixels(
            "<feMerge x='30' y='30' width='20' height='20'>\
             <feMergeNode in='SourceGraphic'/><feMergeNode in='SourceGraphic'/></feMerge>",
        ),
    );
}

#[test]
fn a_clipped_result_goes_by_its_name() {
    // The primitive after reads the clipped result, by name and by place.
    let by_name = same_picture(
        &fractions(&format!(
            "<feColorMatrix {RED} x='25%' y='25%' width='50%' height='50%' result='a'/>\
             <feOffset in='a' dx='0.25' dy='0'/>"
        )),
        &pixels(&format!(
            "<feColorMatrix {RED} x='30' y='30' width='20' height='20' result='a'/>\
             <feOffset in='a' dx='10' dy='0'/>"
        )),
    );
    assert_eq!(pixel(&by_name, 55, 35), SOLID_RED);
    assert_eq!(pixel(&by_name, 35, 35), CLEAR);
    same_picture(
        &fractions(&format!(
            "<feColorMatrix {RED} x='25%' y='25%' width='50%' height='50%'/>\
             <feOffset dx='0.25' dy='0'/>"
        )),
        &pixels(&format!(
            "<feColorMatrix {RED} x='30' y='30' width='20' height='20'/>\
             <feOffset dx='10' dy='0'/>"
        )),
    );
}

#[test]
fn a_subregion_that_leaves_values_out_takes_them_from_the_filter_region() {
    // The filter region is -10% to 110% of the box, which is 16 to 64.
    for (left_out, wanted) in [
        (
            "x='25%' width='50%'",
            "x='30' width='20' y='16' height='48'",
        ),
        (
            "y='25%' height='50%'",
            "y='30' height='20' x='16' width='48'",
        ),
        ("x='50%'", "x='40' y='16' width='48' height='48'"),
        ("width='50%'", "x='16' y='16' width='20' height='48'"),
    ] {
        same_picture(
            &fractions(&format!("<feColorMatrix {RED} {left_out}/>")),
            &pixels(&format!("<feColorMatrix {RED} {wanted}/>")),
        );
    }
    // The region of the filter is its own.
    same_picture(
        &scene(
            "primitiveUnits='objectBoundingBox' x='0' y='-50%' width='1' height='2'",
            &format!("<feColorMatrix {RED} x='25%' width='50%'/>"),
        ),
        &scene(
            "primitiveUnits='userSpaceOnUse' filterUnits='userSpaceOnUse' x='20' y='0' width='40' height='80'",
            &format!("<feColorMatrix {RED} x='30' width='20' y='0' height='80'/>"),
        ),
    );
}

#[test]
fn a_merge_or_a_turbulence_that_leaves_values_out_takes_them_from_the_filter_region() {
    // The nodes of a merge are its inputs, and a turbulence has none.
    same_picture(
        &fractions(
            "<feMerge x='25%' width='50%'>\
             <feMergeNode in='SourceGraphic'/><feMergeNode in='SourceAlpha'/></feMerge>",
        ),
        &pixels(
            "<feMerge x='30' width='20' y='16' height='48'>\
             <feMergeNode in='SourceGraphic'/><feMergeNode in='SourceAlpha'/></feMerge>",
        ),
    );
    same_picture(
        &fractions("<feTurbulence type='fractalNoise' baseFrequency='0' x='25%' width='50%'/>"),
        &pixels(
            "<feTurbulence type='fractalNoise' baseFrequency='0' x='30' width='20' y='16' height='48'/>",
        ),
    );
    // A node that reads a result keeps the merge out, as an input left out
    // after the first primitive does.
    let source = fractions(
        "<feColorMatrix type='saturate' result='a'/>\
         <feMerge x='25%' width='50%'><feMergeNode in='a'/></feMerge>",
    );
    assert!(matches!(like_browsers(&source), Cow::Borrowed(_)));
    let source = fractions(
        "<feColorMatrix type='saturate'/>\
         <feMerge x='25%' width='50%'><feMergeNode/></feMerge>",
    );
    assert!(matches!(like_browsers(&source), Cow::Borrowed(_)));
}

#[test]
fn a_flood_or_an_image_that_leaves_values_out_takes_them_from_the_filter_region() {
    // The subregion reaches to 78, and the filter region stops at 64.
    let drawn = same_picture(
        &fractions("<feFlood flood-color='red' x='25%' y='25%'/>"),
        &pixels("<feFlood flood-color='red' x='30' y='30' width='48' height='48'/>"),
    );
    assert_eq!(pixel(&drawn, 60, 60), SOLID_RED);
    assert_eq!(pixel(&drawn, 70, 70), CLEAR);
    assert_eq!(pixel(&drawn, 25, 25), CLEAR);
    // A flood that tells its four values is read as it is.
    same_picture(
        &fractions("<feFlood flood-color='red' x='25%' y='25%' width='50%' height='50%'/>"),
        &pixels("<feFlood flood-color='red' x='30' y='30' width='20' height='20'/>"),
    );
    // The values that an image leaves out are added too, and the ones that it
    // tells are kept.
    let source = fractions("<feImage href='#a' width='10%'/>");
    assert_eq!(
        like_browsers(&source),
        fractions("<feImage x=\"-0.1\" y=\"-0.1\" height=\"1.2\" href='#a' width='10%'/>")
    );
    let source = fractions("<feImage href='#a'/>");
    assert!(matches!(like_browsers(&source), Cow::Borrowed(_)));
    // The region of the filter has to be one of fractions.
    let source = scene(
        "primitiveUnits='objectBoundingBox' filterUnits='userSpaceOnUse'",
        "<feFlood x='25%'/>",
    );
    assert!(matches!(like_browsers(&source), Cow::Borrowed(_)));
}

#[test]
fn what_is_left_out_is_only_known_for_the_standard_inputs() {
    // The primitive reads a result, whose subregion the default is made of.
    let source = fractions(&format!(
        "<feColorMatrix {RED} result='a'/><feColorMatrix {RED} in='a' x='25%' width='50%'/>"
    ));
    assert!(matches!(like_browsers(&source), Cow::Borrowed(_)));
    // Nothing comes before it, so it reads the source graphic.
    let source = fractions(&format!("<feColorMatrix {RED} x='25%' width='50%'/>"));
    assert!(matches!(like_browsers(&source), Cow::Owned(_)));
    // Both inputs of a blend count.
    let source = fractions("<feBlend in='SourceGraphic' in2='SourceAlpha' x='25%' width='50%'/>");
    assert!(matches!(like_browsers(&source), Cow::Owned(_)));
    let source = fractions(
        "<feColorMatrix type='saturate' result='a'/>\
         <feBlend in='SourceGraphic' in2='a' x='25%' width='50%'/>",
    );
    assert!(matches!(like_browsers(&source), Cow::Borrowed(_)));
    // A subregion of four values does not depend on that.
    let source = fractions(&format!(
        "<feColorMatrix {RED} result='a'/>\
         <feColorMatrix {RED} in='a' x='25%' y='25%' width='50%' height='50%'/>"
    ));
    assert!(matches!(like_browsers(&source), Cow::Owned(_)));
}

#[test]
fn what_is_left_out_is_only_known_for_a_region_of_fractions() {
    let source = scene(
        "primitiveUnits='objectBoundingBox' filterUnits='userSpaceOnUse' x='0' y='0' width='100' height='100'",
        &format!("<feColorMatrix {RED} x='25%' width='50%'/>"),
    );
    assert!(matches!(like_browsers(&source), Cow::Borrowed(_)));
    // A region of user space leaves nothing out when the subregion has four.
    let source = scene(
        "primitiveUnits='objectBoundingBox' filterUnits='userSpaceOnUse' x='0' y='0' width='100' height='100'",
        &format!("<feColorMatrix {RED} x='25%' y='25%' width='50%' height='50%'/>"),
    );
    assert!(matches!(like_browsers(&source), Cow::Owned(_)));
    for region in [
        "x='5px'",
        "width='0'",
        "height='-1'",
        "x='em'",
        "width='100%' height='-1%'",
    ] {
        let source = scene(
            &format!("primitiveUnits='objectBoundingBox' {region}"),
            &format!("<feColorMatrix {RED} x='25%' width='50%'/>"),
        );
        assert!(
            matches!(like_browsers(&source), Cow::Borrowed(_)),
            "{region}"
        );
    }
}

#[test]
fn a_subregion_that_is_not_made_of_fractions_is_left_alone() {
    for values in [
        // Too large for a single precision number, which `usvg` reads.
        "x='1e39' y='25%' width='50%' height='50%'",
        "x='25%' y='25%' width='1e41%' height='50%'",
        "x='5px' y='25%' width='50%' height='50%'",
        "x='25%' y='25%' width='calc(50%)' height='50%'",
        "x='25%' y='25%' width='50%' height='em'",
        // A size of nothing disables the primitive.
        "x='25%' y='25%' width='0' height='50%'",
        "x='25%' y='25%' width='50%' height='-10%'",
        "x='25%' y='25%' width='1e999' height='50%'",
    ] {
        let source = fractions(&format!("<feColorMatrix {RED} {values}/>"));
        assert!(
            matches!(like_browsers(&source), Cow::Borrowed(_)),
            "{values}"
        );
    }
}

#[test]
fn only_a_filter_in_the_units_of_the_bounding_box_is_read() {
    for filter in [
        "",
        "primitiveUnits='userSpaceOnUse'",
        "primitiveUnits='objectboundingbox'",
        "primitiveUnits=' objectBoundingBox'",
    ] {
        let source = scene(
            filter,
            &format!("<feColorMatrix {RED} x='25' y='25' width='50' height='50'/>"),
        );
        assert!(
            matches!(like_browsers(&source), Cow::Borrowed(_)),
            "{filter}"
        );
    }
}

#[test]
fn a_filter_in_a_chain_is_left_alone() {
    let primitive = format!("<feColorMatrix {RED} x='25%' y='25%' width='50%' height='50%'/>");
    // It takes from another filter.
    for link in [
        "href='#g'",
        "xlink:href='#g' xmlns:xlink='http://www.w3.org/1999/xlink'",
    ] {
        let source = format!(
            "<svg xmlns='http://www.w3.org/2000/svg' width='100' height='100'>\
             <filter id='g'/><filter id='f' primitiveUnits='objectBoundingBox' {link}>{primitive}</filter></svg>"
        );
        assert!(matches!(like_browsers(&source), Cow::Borrowed(_)), "{link}");
    }
    // Another takes from it.
    let source = format!(
        "<svg xmlns='http://www.w3.org/2000/svg' width='100' height='100'>\
         <filter id='g' href='#f'/><filter id='f' primitiveUnits='objectBoundingBox'>{primitive}</filter></svg>"
    );
    assert!(matches!(like_browsers(&source), Cow::Borrowed(_)));
    // Another one that does not.
    let source = format!(
        "<svg xmlns='http://www.w3.org/2000/svg' width='100' height='100'>\
         <filter id='g' href='#h'/><filter id='f' primitiveUnits='objectBoundingBox'>{primitive}</filter></svg>"
    );
    assert!(matches!(like_browsers(&source), Cow::Owned(_)));
}

#[test]
fn a_tile_repeats_the_subregion_so_the_filter_is_left_alone() {
    let source = fractions(&format!(
        "<feColorMatrix {RED} x='25%' y='25%' width='50%' height='50%' result='a'/><feTile in='a'/>"
    ));
    assert!(matches!(like_browsers(&source), Cow::Borrowed(_)));
}

#[test]
fn the_clip_is_written_after_the_primitive() {
    let source = fractions(
        "<feColorMatrix type='saturate' x='25%' y='25%' width='50%' height='50%' result='a'/>",
    );
    assert_eq!(
        like_browsers(&source),
        fractions(
            "<feColorMatrix type='saturate' result='raikiri-subregion-0'/>\
             <feFlood x=\"0.25\" y=\"0.25\" width=\"0.5\" height=\"0.5\" \
             result=\"raikiri-subregion-1\"/>\
             <feComposite in=\"raikiri-subregion-0\" in2=\"raikiri-subregion-1\" operator=\"in\" result=\"a\"/>"
        )
    );
}

#[test]
fn an_attribute_taken_out_may_follow_the_name_of_the_element() {
    // Another rewrite takes the zero divisor out, next to the name, where the
    // result goes too.
    let source = fractions(
        "<feConvolveMatrix divisor='0' x='25%' y='25%' width='50%' height='50%' order='1' kernelMatrix='1'/>",
    );
    assert_eq!(
        like_browsers(&source),
        fractions(
            "<feConvolveMatrix result=\"raikiri-subregion-0\" order='1' kernelMatrix='1'/>\
             <feFlood x=\"0.25\" y=\"0.25\" width=\"0.5\" height=\"0.5\" \
             result=\"raikiri-subregion-1\"/>\
             <feComposite in=\"raikiri-subregion-0\" in2=\"raikiri-subregion-1\" operator=\"in\"/>"
        )
    );
}

#[test]
fn a_primitive_without_a_name_gets_one() {
    let source = fractions("<feOffset dx='1' x='25%' y='25%' width='50%' height='50%'></feOffset>");
    assert_eq!(
        like_browsers(&source),
        fractions(
            "<feOffset result=\"raikiri-subregion-0\" dx='1'></feOffset>\
             <feFlood x=\"0.25\" y=\"0.25\" width=\"0.5\" height=\"0.5\" \
             result=\"raikiri-subregion-1\"/>\
             <feComposite in=\"raikiri-subregion-0\" in2=\"raikiri-subregion-1\" operator=\"in\"/>"
        )
    );
}

#[test]
fn the_names_of_the_clips_are_not_names_the_filter_uses() {
    let source = fractions(&format!(
        "<feColorMatrix {RED} x='25%' y='25%' width='50%' height='50%' result='raikiri-subregion-0'/>\
         <feOffset in='raikiri-subregion-0' dx='1' result='raikiri-subregion-2'/>\
         <feBlend in='raikiri-subregion-3' in2='raikiri-subregion-0' x='0' y='0' width='1' height='1'/>"
    ));
    let rewritten = like_browsers(&source).into_owned();
    // 1 and 4 are free, and 2 and 3 are not.
    assert!(
        rewritten.contains("<feComposite in=\"raikiri-subregion-1\" in2=\"raikiri-subregion-4\""),
        "{rewritten}"
    );
    assert!(rewritten.contains("result=\"raikiri-subregion-0\"/><feOffset"));
}

#[test]
fn the_clip_is_read_in_the_color_space_of_the_primitive() {
    let source = fractions(
        "<feColorMatrix color-interpolation-filters='sRGB' class='a b' style='opacity:.5' \
         x='25%' y='25%' width='50%' height='50%'/>",
    );
    let rewritten = like_browsers(&source).into_owned();
    assert!(
        rewritten.contains(
            "operator=\"in\" color-interpolation-filters=\"sRGB\" class=\"a b\" style=\"opacity:.5\"/>"
        ),
        "{rewritten}"
    );
    // The flood does not take them.
    assert!(!rewritten.contains("<feFlood color"));
}

#[test]
fn quotes_in_what_is_copied_are_escaped() {
    let source = fractions(
        "<feColorMatrix class='a\"b&amp;&lt;' x='25%' y='25%' width='50%' height='50%'/>",
    );
    let rewritten = like_browsers(&source).into_owned();
    assert!(
        rewritten.contains("class=\"a&quot;b&amp;&lt;\""),
        "{rewritten}"
    );
    assert!(SvgDocument::parse(rewritten.as_bytes()).is_ok());
}

#[test]
fn prefixed_elements_get_prefixed_clips() {
    let source = "<s:svg xmlns:s='http://www.w3.org/2000/svg' width='100' height='100'>\
        <s:filter id='f' primitiveUnits='objectBoundingBox'>\
        <s:feColorMatrix type='matrix' values='0 0 0 0 1  0 0 0 0 0  0 0 0 0 0  0 0 0 1 0' \
        x='25%' y='25%' width='50%' height='50%'/></s:filter>\
        <s:rect x='20' y='20' width='40' height='40' fill='green' filter='url(#f)'/></s:svg>";
    let rewritten = like_browsers(source).into_owned();
    assert!(rewritten.contains("<s:feFlood"), "{rewritten}");
    assert!(rewritten.contains("<s:feComposite"), "{rewritten}");
    let drawn = rasterized(source);
    assert_eq!(pixel(&drawn, 35, 35), SOLID_RED);
    assert_eq!(pixel(&drawn, 25, 25), CLEAR);
}

#[test]
fn a_primitive_that_sets_a_namespace_is_left_alone() {
    // The clip would be written outside the scope of the declaration, where
    // the filter and its elements have a prefix and no default namespace.
    let source = "<s:svg xmlns:s='http://www.w3.org/2000/svg' width='100' height='100'>\
        <s:filter id='f' primitiveUnits='objectBoundingBox'>\
        <feColorMatrix xmlns='http://www.w3.org/2000/svg' type='saturate' \
        x='25%' y='25%' width='50%' height='50%'/></s:filter></s:svg>";
    assert!(matches!(like_browsers(source), Cow::Borrowed(_)));
    // So does a declaration that changes nothing for the clip: telling it
    // apart is not worth the care.
    let source = fractions(
        "<feColorMatrix xmlns:o='urn:other' type='saturate' x='25%' y='25%' width='50%' height='50%'/>",
    );
    assert!(matches!(like_browsers(&source), Cow::Borrowed(_)));
    // As many bindings as the filter has, but the default one is another: the
    // clip would be written in the namespace of the filter, which is not SVG.
    let source = "<s:svg xmlns:s='http://www.w3.org/2000/svg' width='100' height='100'>\
        <s:filter id='f' xmlns='urn:other' primitiveUnits='objectBoundingBox'>\
        <feColorMatrix xmlns='http://www.w3.org/2000/svg' type='saturate' \
        x='25%' y='25%' width='50%' height='50%'/></s:filter>\
        <s:rect x='20' y='20' width='40' height='40' fill='green' filter='url(#f)'/></s:svg>";
    assert!(matches!(like_browsers(source), Cow::Borrowed(_)));
    // The same binding again changes nothing, so the primitive is clipped.
    let source = fractions(&format!(
        "<feColorMatrix xmlns='http://www.w3.org/2000/svg' {RED} x='25%' y='25%' width='50%' height='50%'/>"
    ));
    let drawn = rasterized(&source);
    assert_eq!(pixel(&drawn, 35, 35), SOLID_RED);
    assert_eq!(pixel(&drawn, 25, 25), CLEAR);
}

#[test]
fn a_primitive_of_another_namespace_is_left_alone() {
    let source = fractions(&format!(
        "<other:feColorMatrix xmlns:other='urn:other' {RED} x='25%' y='25%' width='50%' height='50%'/>"
    ));
    assert!(matches!(like_browsers(&source), Cow::Borrowed(_)));
}

#[test]
fn what_a_selector_can_tell_is_left_alone() {
    let primitive =
        format!("<feColorMatrix {RED} x='25%' y='25%' width='50%' height='50%' result='a'/>");
    for css in [
        // The attributes that the edits change.
        "feColorMatrix[result] {}",
        "feColorMatrix[ x] {}",
        "[height='50%'] {}",
        // The place among the siblings, which the clip changes.
        "feColorMatrix:first-child {}",
        ":nth-child(2) {}",
        ":last-of-type {}",
        ":only-child {}",
        "feColorMatrix + feFlood {}",
        "feColorMatrix ~ feFlood {}",
        // The color space of the primitive, which the clip is to share.
        "feColorMatrix { color-interpolation-filters: sRGB }",
        "#a { COLOR-INTERPOLATION-FILTERS: linearRGB }",
    ] {
        let source = format!(
            "<svg xmlns='http://www.w3.org/2000/svg' width='100' height='100'>\
             <style>{css}</style><filter id='f' primitiveUnits='objectBoundingBox'>{primitive}</filter></svg>"
        );
        assert!(matches!(like_browsers(&source), Cow::Borrowed(_)), "{css}");
    }
    // A rule about something else changes nothing.
    let source = format!(
        "<svg xmlns='http://www.w3.org/2000/svg' width='100' height='100'>\
         <style>feColorMatrix {{ lighting-color: red }}</style>\
         <filter id='f' primitiveUnits='objectBoundingBox'>{primitive}</filter></svg>"
    );
    assert!(matches!(like_browsers(&source), Cow::Owned(_)));
}

#[test]
fn a_rule_for_the_color_or_the_opacity_of_a_flood_leaves_the_sheet_alone() {
    // `usvg` lets an important declaration of the sheet win over the important
    // declaration of a style attribute, so the flood of the clip could not be
    // kept as black and opaque as it is by default.
    let primitive = format!("<feColorMatrix {RED} x='25%' y='25%' width='50%' height='50%'/>");
    for css in [
        "feFlood { flood-opacity: 0 !important }",
        "* { FLOOD-COLOR: blue }",
        "#a { flood-opacity: .5 }",
    ] {
        let source = format!(
            "<svg xmlns='http://www.w3.org/2000/svg' width='100' height='100'>\
             <style>{css}</style><filter id='f' primitiveUnits='objectBoundingBox'>{primitive}</filter></svg>"
        );
        assert!(matches!(like_browsers(&source), Cow::Borrowed(_)), "{css}");
    }
}

#[test]
fn a_document_of_many_filters_is_read_in_one_pass() {
    let one = format!(
        "<filter id='f' primitiveUnits='objectBoundingBox'><feColorMatrix {RED} x='25%' y='25%' width='50%' height='50%'/></filter>"
    );
    let source = format!(
        "<svg xmlns='http://www.w3.org/2000/svg' width='100' height='100'>{}</svg>",
        one.repeat(2000)
    );
    let rewritten = like_browsers(&source);
    assert_eq!(rewritten.matches("<feComposite").count(), 2000);
}

#[test]
fn the_work_of_a_math_function_and_a_clip_go_together() {
    // The width of the rectangle is worked out, the subregion is clipped.
    let source = "<svg xmlns='http://www.w3.org/2000/svg' width='100' height='100'>\
        <filter id='f' primitiveUnits='objectBoundingBox'>\
        <feColorMatrix type='matrix' values='0 0 0 0 1  0 0 0 0 0  0 0 0 0 0  0 0 0 1 0' \
        x='25%' y='25%' width='50%' height='50%'/></filter>\
        <rect x='20' y='20' width='calc(8 * 5)' height='40' fill='green' filter='url(#f)'/></svg>";
    let drawn = rasterized(source);
    assert_eq!(pixel(&drawn, 35, 35), SOLID_RED);
    assert_eq!(pixel(&drawn, 25, 25), CLEAR);
}
