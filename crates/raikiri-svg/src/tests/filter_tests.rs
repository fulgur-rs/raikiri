//! Filter effects: drawn by the raster path, and bounded before resvg runs
//! them.

use std::sync::Arc;

use crate::MAX_FILTER_ELEMENTS;
use crate::{
    SvgDocument, SvgError, SvgFilterBudget, SvgFontRequest, SvgFontSource, SvgImageAccess,
    SvgRootStyle, SvgSceneError, SvgSceneFeature, SvgViewport,
};
use raikiri_traits::DecodedImage;
use raikiri_traits::vector::FontFace;

const AHEM: &[u8] = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/Ahem.ttf"));

const SVG_NAMESPACE: &str = "http://www.w3.org/2000/svg";

/// Gives every request the Ahem face, in which each letter is a full square.
struct Ahem;

impl SvgFontSource for Ahem {
    fn select(&self, _request: &SvgFontRequest) -> Option<FontFace> {
        let data: Arc<dyn AsRef<[u8]> + Send + Sync> = Arc::new(AHEM);
        Some(FontFace::new(data, 0))
    }

    fn fallback(&self, _request: &SvgFontRequest, _ch: char) -> Option<FontFace> {
        None
    }
}

/// A document of `width` by `height` whose content is `body`.
fn document(width: u32, height: u32, body: &str) -> SvgDocument {
    SvgDocument::parse(
        format!("<svg xmlns='{SVG_NAMESPACE}' width='{width}' height='{height}'>{body}</svg>")
            .as_bytes(),
    )
    .unwrap()
}

fn rasterize(svg: &SvgDocument, width: u32, height: u32) -> Result<DecodedImage, SvgError> {
    svg.rasterize(
        SvgViewport {
            width: width as f32,
            height: height as f32,
        },
        SvgRootStyle::default(),
        None,
    )
}

fn count(image: &DecodedImage, wanted: impl Fn([u8; 4]) -> bool) -> usize {
    image
        .rgba
        .chunks_exact(4)
        .filter(|pixel| wanted([pixel[0], pixel[1], pixel[2], pixel[3]]))
        .count()
}

fn pure_red(pixel: [u8; 4]) -> bool {
    pixel == [255, 0, 0, 255]
}

fn visible(pixel: [u8; 4]) -> bool {
    pixel[3] > 0
}

/// A 60 by 30 red rectangle, as the filter-bearing inline SVG of the issue
/// that asked for filters to be drawn.
const BLURRED_RECT: &str = "<filter id='f'><feGaussianBlur stdDeviation='3'/></filter>\
    <rect x='10' y='10' width='60' height='30' fill='red' filter='url(#f)'/>";

#[test]
fn a_blur_softens_the_rectangle_it_filters() {
    let sharp = document(
        100,
        60,
        "<rect x='10' y='10' width='60' height='30' fill='red'/>",
    );
    let image = rasterize(&sharp, 100, 60).unwrap();
    assert_eq!(count(&image, pure_red), 60 * 30);
    assert_eq!(count(&image, visible), 60 * 30);

    let blurred = rasterize(&document(100, 60, BLURRED_RECT), 100, 60).unwrap();
    // The edge gives red to the pixels around it, so fewer pixels are solid
    // and more are visible than in the rectangle itself.
    let (solid, shown) = (count(&blurred, pure_red), count(&blurred, visible));
    assert!(solid < 60 * 30 && solid > 300, "{solid} solid pixels");
    assert!(shown > 60 * 30 + 400, "{shown} visible pixels");
    // The middle stays red and a corner of the page stays empty.
    let at = |x: usize, y: usize| {
        let start = (y * 100 + x) * 4;
        [
            blurred.rgba[start],
            blurred.rgba[start + 1],
            blurred.rgba[start + 2],
            blurred.rgba[start + 3],
        ]
    };
    assert_eq!(at(40, 25), [255, 0, 0, 255]);
    assert_eq!(at(0, 0), [0; 4]);
}

#[test]
fn a_filter_is_drawn_whichever_way_it_is_named() {
    let named = [
        // The `filter` property of a style attribute, with a CSS function.
        "<rect x='10' y='10' width='60' height='30' fill='red' style='filter: blur(3px)'/>",
        // A style sheet that names a filter of the document.
        "<style>rect { filter: url(#f) }</style><filter id='f'><feGaussianBlur stdDeviation='3'/></filter>\
         <rect x='10' y='10' width='60' height='30' fill='red'/>",
    ];
    for body in named {
        let image = rasterize(&document(100, 60, body), 100, 60).unwrap();
        assert!(count(&image, visible) > 60 * 30 + 400, "{body}");
    }
}

#[test]
fn a_filter_that_is_none_or_unused_changes_nothing() {
    let plain = "<rect x='10' y='10' width='60' height='30' fill='red'/>";
    let expected = rasterize(&document(100, 60, plain), 100, 60).unwrap();
    for body in [
        "<rect x='10' y='10' width='60' height='30' fill='red' style='filter:none'/>",
        "<filter id='unused'><feGaussianBlur stdDeviation='3'/></filter>\
         <rect x='10' y='10' width='60' height='30' fill='red'/>",
    ] {
        let image = rasterize(&document(100, 60, body), 100, 60).unwrap();
        assert_eq!(image.rgba, expected.rgba, "{body}");
    }
}

#[test]
fn the_parser_keeps_an_active_filter_in_the_tree() {
    let source =
        format!("<svg xmlns='{SVG_NAMESPACE}' width='100' height='60'>{BLURRED_RECT}</svg>");
    let tree = super::super::parse_tree(&source).unwrap();
    assert_eq!(tree.filters().len(), 1);
}

/// `n` primitives that each hold a buffer of the filter region.
fn offsets(n: usize) -> String {
    "<feOffset dx='1'/>".repeat(n)
}

#[test]
fn filters_over_the_memory_budget_fail_before_resvg_runs() {
    // The region is 1.2 times the canvas on each side: 5.8 million pixels in
    // each of 21 buffers is more than eight output-sized buffers.
    let body = format!(
        "<filter id='f'>{}</filter><rect width='2000' height='2000' filter='url(#f)'/>",
        offsets(20)
    );
    let error = rasterize(&document(2000, 2000, &body), 2000, 2000).unwrap_err();
    assert!(
        matches!(
            error,
            SvgError::FilterLimitExceeded {
                budget: SvgFilterBudget::Pixels,
                requested,
                limit,
            } if requested > limit
        ),
        "{error:?}"
    );
    // A handful of primitives over the same canvas is within the budget.
    let fewer = format!(
        "<filter id='f'>{}</filter><rect width='2000' height='2000' filter='url(#f)'/>",
        offsets(3)
    );
    rasterize(&document(2000, 2000, &fewer), 2000, 2000).unwrap();
}

/// A filter whose primitive is `primitive`, on a rectangle of `side` pixels.
fn filtered(side: u32, primitive: &str) -> String {
    format!(
        "<filter id='f'>{primitive}</filter><rect width='{side}' height='{side}' filter='url(#f)'/>"
    )
}

fn assert_over_the_work_budget(side: u32, body: &str) {
    let error = rasterize(&document(side, side, body), side, side).unwrap_err();
    assert!(
        matches!(
            error,
            SvgError::FilterLimitExceeded {
                budget: SvgFilterBudget::Work,
                requested,
                limit,
            } if requested > limit
        ),
        "{error:?}"
    );
}

#[test]
fn a_primitive_whose_time_grows_with_a_number_of_the_document_is_bounded() {
    // A wide morphology compares each pixel with a kernel as large as the layer.
    assert_over_the_work_budget(
        1000,
        &filtered(1000, "<feMorphology operator='dilate' radius='300'/>"),
    );
    // Turbulence adds a layer of noise for each octave.
    assert_over_the_work_budget(
        100,
        &filtered(
            100,
            "<feTurbulence baseFrequency='0.05' numOctaves='100000000'/>",
        ),
    );
    // A merge composites every one of its inputs over the result.
    let merge = format!(
        "<filter id='f'><feMerge>{}</feMerge></filter>{}",
        "<feMergeNode in='SourceGraphic'/>".repeat(100),
        "<rect width='1000' height='1000' filter='url(#f)'/>".repeat(40),
    );
    assert_over_the_work_budget(1000, &merge);
    // A region smaller than a pixel is still drawn as a whole pixel.
    assert_over_the_work_budget(
        100,
        "<filter id='f' filterUnits='userSpaceOnUse' x='0' y='0' width='0.001' height='0.001'>\
         <feTurbulence baseFrequency='0.05' numOctaves='1000000000'/></filter>\
         <rect width='100' height='100' filter='url(#f)'/>",
    );
    // A convolution multiplies each pixel with every cell of its matrix.
    let matrix = vec!["1"; 60 * 60].join(" ");
    assert_over_the_work_budget(
        1000,
        &filtered(
            1000,
            &format!("<feConvolveMatrix order='60' kernelMatrix='{matrix}'/>"),
        ),
    );
}

#[test]
fn the_work_of_all_the_filters_of_a_picture_adds_up() {
    let morphology = "<feMorphology operator='dilate' radius='10'/>";
    let rect = "<rect width='1000' height='1000' filter='url(#f)'/>";
    let one = format!("<filter id='f'>{morphology}</filter>{rect}");
    rasterize(&document(1000, 1000, &one), 1000, 1000).unwrap();
    let eight = format!("<filter id='f'>{morphology}</filter>{}", rect.repeat(8));
    assert_over_the_work_budget(1000, &eight);
}

#[test]
fn a_radius_counts_in_canvas_pixels_whatever_scales_it() {
    let morphology = "<filter id='f'><feMorphology operator='dilate' radius='5'/></filter>";
    let rect = |side: u32| format!("<rect width='{side}' height='{side}' filter='url(#f)'/>");
    // Five pixels is a small kernel.
    let plain = format!("{morphology}{}", rect(300));
    rasterize(&document(300, 300, &plain), 300, 300).unwrap();
    // A transform of the group enlarges the radius to a hundred pixels.
    let scaled = format!("{morphology}<g transform='scale(20)'>{}</g>", rect(15));
    assert_over_the_work_budget(300, &scaled);
    // So does the scale of the raster.
    let small = format!("{}{}", morphology.replace("'5'", "'10'"), rect(30));
    let error = document(30, 30, &small)
        .rasterize_with_fonts(
            SvgViewport {
                width: 30.0,
                height: 30.0,
            },
            SvgRootStyle::default(),
            None,
            10.0,
            None,
        )
        .unwrap_err();
    assert!(
        matches!(error, SvgError::FilterLimitExceeded { .. }),
        "{error:?}"
    );
}

#[test]
fn filters_met_through_masks_and_patterns_are_bounded_too() {
    let heavy = "<filter id='f'><feMorphology operator='dilate' radius='300'/></filter>";
    let inner = "<rect width='1000' height='1000' filter='url(#f)'/>";
    let cases = [
        format!(
            "{heavy}<mask id='m'>{inner}</mask><rect width='1000' height='1000' mask='url(#m)'/>"
        ),
        format!(
            "{heavy}<pattern id='p' width='1000' height='1000' patternUnits='userSpaceOnUse'>{inner}</pattern>\
             <rect width='1000' height='1000' fill='url(#p)'/>"
        ),
    ];
    for body in cases {
        let error = rasterize(&document(1000, 1000, &body), 1000, 1000).unwrap_err();
        assert!(
            matches!(error, SvgError::FilterLimitExceeded { .. }),
            "{body}: {error:?}"
        );
    }
}

#[test]
fn a_filter_at_the_end_of_a_chain_of_masks_is_bounded() {
    // Masks link to one another, and resvg draws the whole chain.
    let chain = |last: &str| {
        let mask = |id: &str, next: Option<&str>, content: &str| {
            let link = next.map_or(String::new(), |next| format!("mask='url(#{next})'"));
            format!(
                "<mask id='{id}' maskUnits='userSpaceOnUse' x='0' y='0' width='1000' height='1000' {link}>\
                 {content}</mask>"
            )
        };
        let plain = "<rect width='1000' height='1000' fill='white'/>";
        format!(
            "<filter id='f'><feMorphology operator='dilate' radius='300'/></filter>{}{}{}\
             <rect width='1000' height='1000' mask='url(#m1)'/>",
            mask("m1", Some("m2"), plain),
            mask("m2", Some("m3"), plain),
            mask("m3", None, last),
        )
    };
    rasterize(
        &document(
            1000,
            1000,
            &chain("<rect width='1000' height='1000' fill='white'/>"),
        ),
        1000,
        1000,
    )
    .unwrap();
    assert_over_the_work_budget(
        1000,
        &chain("<rect width='1000' height='1000' fill='white' filter='url(#f)'/>"),
    );
}

#[test]
fn filters_of_one_group_all_work_on_the_union_of_their_regions() {
    // The second filter has a region of one pixel, but resvg hands it the
    // layer of the first, which is the whole canvas.
    let chain = |filters: &str| {
        format!(
            "<filter id='a' filterUnits='userSpaceOnUse' x='0' y='0' width='1000' height='1000'>\
             <feFlood flood-color='red'/></filter>\
             <filter id='b' filterUnits='userSpaceOnUse' x='0' y='0' width='1' height='1'>\
             <feMorphology operator='dilate' radius='300'/></filter>\
             <rect width='1000' height='1000' filter='{filters}'/>"
        )
    };
    rasterize(&document(1000, 1000, &chain("url(#b)")), 1000, 1000).unwrap();
    assert_over_the_work_budget(1000, &chain("url(#a) url(#b)"));
}

#[test]
fn a_wrapped_convolution_pays_for_the_steps_that_bring_a_coordinate_back() {
    // resvg adds the width of the image to a negative coordinate until it is
    // back in, so a kernel much wider than a one pixel image takes a step for
    // each of its columns, on every cell.
    let convolve = |edge_mode: &str| {
        let order = 70_000;
        let kernel = vec!["1"; order].join(" ");
        format!(
            "<filter id='f' filterUnits='userSpaceOnUse' x='0' y='0' width='1' height='1'>\
             <feConvolveMatrix order='{order} 1' targetX='{target}' kernelMatrix='{kernel}' \
             edgeMode='{edge_mode}'/></filter>\
             <rect width='1' height='1' filter='url(#f)'/>",
            target = order - 1,
        )
    };
    rasterize(&document(1, 1, &convolve("none")), 1, 1).unwrap();
    assert_over_the_work_budget(1, &convolve("wrap"));
}

#[test]
fn a_filter_with_too_many_elements_is_refused_before_usvg_reads_it() {
    // usvg checks each name a primitive uses against all that came before it,
    // so reading a long chain takes the square of its length. The parse stops
    // it first, on the size alone.
    let chain = |count: usize| {
        let offsets: String = (0..count)
            .map(|index| format!("<feOffset in='r{index}' result='r{}'/>", index + 1))
            .collect();
        format!(
            "<svg xmlns='{SVG_NAMESPACE}' width='1' height='1'>\
             <filter id='f'><feFlood result='r0'/>{offsets}</filter>\
             <rect width='1' height='1' filter='url(#f)'/></svg>"
        )
    };
    SvgDocument::parse(chain(MAX_FILTER_ELEMENTS - 1).as_bytes()).unwrap();
    for count in [MAX_FILTER_ELEMENTS, 200_000] {
        let error = SvgDocument::parse(chain(count).as_bytes()).unwrap_err();
        assert!(
            matches!(
                error,
                SvgError::FilterLimitExceeded {
                    budget: SvgFilterBudget::Elements,
                    requested,
                    limit,
                } if requested == count as u64 + 1 && limit == MAX_FILTER_ELEMENTS as u64
            ),
            "{count}: {error:?}"
        );
    }
}

#[test]
fn an_fe_image_that_refers_to_an_element_is_refused_at_parse() {
    // Two filters whose pictures use one another's elements overflow the
    // stack of usvg, which only breaks a filter that uses its own.
    let svg = |filters: &str, elements: &str| {
        format!(
            "<svg xmlns='{SVG_NAMESPACE}' xmlns:xlink='http://www.w3.org/1999/xlink' \
             width='1' height='1'>{filters}{elements}</svg>"
        )
    };
    let elements = "<g id='g1' filter='url(#a)'><rect width='1' height='1'/></g>\
                    <g id='g2' filter='url(#b)'><rect width='1' height='1'/></g>";
    for (a, b) in [
        ("href='#g2'", "href='#g1'"),
        ("xlink:href='#g2'", "xlink:href='#g1'"),
        ("href=' #g2'", "href='#g1'"),
        // Both attributes are given, and the one that counts comes last.
        (
            "href='x.png' xlink:href='#g2'",
            "xlink:href='x.png' href='#g1'",
        ),
    ] {
        let filters =
            format!("<filter id='a'><feImage {a}/></filter><filter id='b'><feImage {b}/></filter>");
        let error = SvgDocument::parse(svg(&filters, elements).as_bytes()).unwrap_err();
        assert!(
            matches!(error, SvgError::InvalidDocument(_)),
            "{a}: {error:?}"
        );
    }
    // A picture that is not an element of the document is left out, not refused.
    let filters = "<filter id='a'><feImage href='data:image/png;base64,AAAA'/></filter>";
    SvgDocument::parse(svg(filters, "<rect width='1' height='1' filter='url(#a)'/>").as_bytes())
        .unwrap();
}

#[test]
fn a_document_with_a_filter_and_a_picture_has_no_picture() {
    // Only a scene draws pictures and only the raster draws filters.
    let source = format!(
        "<svg xmlns='{SVG_NAMESPACE}' width='20' height='20'>{BLURRED_RECT}\
         <image href='data:image/png;base64,AAAA' width='4' height='4'/></svg>"
    );
    let svg = SvgDocument::parse_with_images(source.as_bytes(), SvgImageAccess::Data).unwrap();
    let viewport = SvgViewport {
        width: 20.0,
        height: 20.0,
    };
    assert!(matches!(
        svg.scene(viewport, SvgRootStyle::default()),
        Err(SvgSceneError::Unsupported(SvgSceneFeature::Filter))
    ));
    assert!(matches!(
        svg.rasterize(viewport, SvgRootStyle::default(), None),
        Err(SvgError::UnsupportedImages)
    ));
}

#[test]
fn a_pattern_tile_is_drawn_at_the_scale_of_its_own_transform() {
    let pattern = |transform: &str| {
        format!(
            "<filter id='f'><feMorphology operator='dilate' radius='3'/></filter>\
             <pattern id='p' width='10' height='10' patternUnits='userSpaceOnUse' {transform}>\
             <rect width='10' height='10' filter='url(#f)'/></pattern>\
             <rect width='300' height='300' fill='url(#p)'/>"
        )
    };
    // Three pixels of radius over a tile of ten.
    rasterize(&document(300, 300, &pattern("")), 300, 300).unwrap();
    // A hundred times the scale makes the radius three hundred pixels.
    assert_over_the_work_budget(300, &pattern("patternTransform='scale(100)'"));
}

#[test]
fn filters_in_the_pattern_that_paints_a_text_are_bounded_too() {
    // resvg draws a text as the paths it flattens to, and the pattern that
    // fills them is drawn with its filter.
    let body = |radius: u32| {
        format!(
            "<filter id='f'><feMorphology operator='dilate' radius='{radius}'/></filter>\
             <pattern id='p' width='1000' height='1000' patternUnits='userSpaceOnUse'>\
             <rect width='1000' height='1000' filter='url(#f)'/></pattern>\
             <text x='0' y='8' font-family='Ahem' font-size='8' fill='url(#p)'>A</text>"
        )
    };
    let draw = |radius: u32| {
        document(1000, 1000, &body(radius)).rasterize_with_fonts(
            SvgViewport {
                width: 1000.0,
                height: 1000.0,
            },
            SvgRootStyle::default(),
            Some(&Ahem),
            1.0,
            None,
        )
    };
    draw(1).unwrap();
    assert!(
        matches!(
            draw(300),
            Err(SvgError::FilterLimitExceeded {
                budget: SvgFilterBudget::Work,
                ..
            })
        ),
        "a text filled by a heavy filter must be over the work budget"
    );
}
