use super::super::*;
use crate::parse_html_with_resources;
use raikiri_style::ComputedLengthPercentageOrAuto;

fn run(html: &str) -> PipelineOutput {
    let doc = parse_html_with_resources(html.as_bytes(), &RenderResources::new()).expect("parse");
    match run_pipeline(
        &doc,
        PageDefaults::default(),
        &LayoutConfig::default(),
        PipelineInputs {
            resources: None,
            consumer_properties: &[],
            property_observer: None,
            preload_background_images: false,
        },
    )
    .expect("pipeline")
    {
        PipelineRun::Completed(out) => *out,
        PipelineRun::Aborted => panic!("unexpected abort"),
    }
}

/// The computed `(width, height)` of the element whose `id` is `id`.
fn size_of(
    out: &PipelineOutput,
    id: &str,
) -> (
    ComputedLengthPercentageOrAuto,
    ComputedLengthPercentageOrAuto,
) {
    let node = (0..out.document.node_count())
        .find(|&node| out.document.element_attribute(node, "id") == Some(id))
        .expect("element");
    let computed = &out.cascade.computed[node];
    (computed.width, computed.height)
}

#[test]
fn viewport_lengths_resolve_against_the_first_page_area() {
    let out = run("<style>\
        @page { size: 400px 200px; margin: 20px }\
        body { margin: 0 }\
        #a { width: 50vw; height: 100vh }\
        #b { width: 10vmin; height: 10vmax }\
        #c { width: 100svw; height: 50dvh }\
        </style><div id=a></div><div id=b></div><div id=c></div>");
    use ComputedLengthPercentageOrAuto::Px;
    assert_eq!(size_of(&out, "a"), (Px(180.0), Px(160.0)));
    assert_eq!(size_of(&out, "b"), (Px(16.0), Px(36.0)));
    assert_eq!(size_of(&out, "c"), (Px(360.0), Px(80.0)));
}

#[test]
fn a_full_viewport_block_fills_one_page() {
    let out = run("<style>\
        @page { size: 400px 200px; margin: 20px }\
        body { margin: 0 } div { height: 100vh }\
        </style><div></div><div></div>");
    assert_eq!(out.slices.len(), 2);
}

#[test]
fn the_viewport_is_the_first_page_area_inside_its_padding_and_border() {
    let out = run("<style>\
        @page { size: 400px 200px; margin: 20px }\
        @page :first { padding: 10px; border: 5px solid }\
        body { margin: 0 } #a { width: 100vw; height: 100vh }\
        </style><div id=a></div>");
    use ComputedLengthPercentageOrAuto::Px;
    assert_eq!(size_of(&out, "a"), (Px(330.0), Px(130.0)));
}

#[test]
fn a_named_first_page_gives_the_viewport() {
    let out = run("<style>\
        @page { size: 400px 200px; margin: 20px }\
        @page cover { margin: 50px }\
        body { margin: 0 } #a { page: cover; width: 100vw; height: 100vh }\
        </style><div id=a></div>");
    use ComputedLengthPercentageOrAuto::Px;
    assert_eq!(size_of(&out, "a"), (Px(300.0), Px(100.0)));
}

#[test]
fn page_context_declarations_with_viewport_lengths_are_dropped() {
    let out = run("<style>\
        @page { size: 400px 200px; margin: 20px; margin: 10vh }\
        body { margin: 0 } #a { width: 100vw }\
        </style><div id=a></div>");
    use ComputedLengthPercentageOrAuto::Px;
    assert_eq!(size_of(&out, "a").0, Px(360.0));
}

#[test]
fn marker_images_are_sized_in_the_final_viewport() {
    // A viewBox-only SVG marker is one em square, and the em here is
    // relative to the named first page's area, not the provisional one.
    let out = run("<style>\
        @page { size: 400px 200px; margin: 20px }\
        @page cover { margin: 50px }\
        body { margin: 0 }\
        li { page: cover; font-size: 10vh; list-style: inside url('data:image/svg+xml,%3Csvg xmlns=%22http://www.w3.org/2000/svg%22 viewBox=%220 0 10 10%22%3E%3Crect width=%2210%22 height=%2210%22/%3E%3C/svg%3E') }\
        </style><li>one</li>");
    let item = out
        .cascade
        .computed
        .iter()
        .position(|cv| cv.display == raikiri_style::DisplayValue::ListItem)
        .expect("list item");
    let size = out
        .document
        .list_marker_image_size(item)
        .expect("prepared marker");
    assert_eq!((size.width, size.height), (10.0, 10.0));
}
