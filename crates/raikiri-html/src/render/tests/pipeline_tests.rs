use super::super::*;
use crate::parse_html_with_resources;

fn parse(html: &str) -> HtmlDocument {
    parse_html_with_resources(html.as_bytes(), &RenderResources::new()).expect("parse")
}

fn run(doc: &HtmlDocument) -> PipelineOutput {
    match run_pipeline(
        doc,
        PageDefaults::default(),
        &LayoutConfig::default(),
        PipelineInputs {
            resources: None,
            consumer_properties: &[],
            property_observer: None,
            preload_background_images: true,
        },
    )
    .expect("pipeline")
    {
        PipelineRun::Completed(out) => *out,
        PipelineRun::Aborted => panic!("unexpected abort"),
    }
}

#[test]
fn pipeline_collects_one_page_style_per_page() {
    let doc = parse(
        "<style>@page { size: 200px 100px; margin: 10px }\
             @page :first { margin: 30px } div { height: 150px }</style>\
             <div></div><div></div>",
    );
    let out = run(&doc);
    assert!(out.pages.len() >= 2, "fixture must paginate");
    assert_eq!(out.page_styles.len(), out.pages.len());
}

#[test]
fn pipeline_page_styles_follow_the_page_context() {
    let doc = parse(
        "<style>@page { size: 200px 100px; margin: 10px }\
             @page :first { margin: 30px } div { height: 150px }</style>\
             <div></div><div></div>",
    );
    let out = run(&doc);
    let first = out.page_styles[0].declarations();
    let second = out.page_styles[1].declarations();
    assert_ne!(
        format!("{first:?}"),
        format!("{second:?}"),
        ":first must change the first page's page cascade"
    );
}
