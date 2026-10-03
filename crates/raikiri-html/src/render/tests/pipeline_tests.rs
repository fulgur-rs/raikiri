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
    assert!(out.slices.len() >= 2, "fixture must paginate");
    assert_eq!(out.page_styles.len(), out.slices.len());
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

#[test]
fn pipeline_rejects_excess_pages_at_the_first_excess_page() {
    let mut html = String::from("<div style='height:10px'>first</div>");
    for _ in 0..8 {
        html.push_str("<div style='height:10px;break-before:page'>forced</div>");
    }
    let doc = parse(&html);
    let config = LayoutConfig::builder()
        .limits(
            raikiri_traits::RenderLimits::builder()
                .max_document_pages(Some(2))
                .build(),
        )
        .build();

    let result = run_pipeline(
        &doc,
        PageDefaults::default(),
        &config,
        PipelineInputs {
            resources: None,
            consumer_properties: &[],
            property_observer: None,
            preload_background_images: true,
        },
    );

    match result {
        Err(RenderError::LimitExceeded {
            kind: raikiri_traits::LimitKind::Pages,
            limit: 2,
            actual: 3,
        }) => {}
        Err(error) => panic!("expected rejection at page 3, got {error}"),
        Ok(PipelineRun::Completed(output)) => {
            panic!(
                "expected rejection, but generated {} pages",
                output.slices.len()
            )
        }
        Ok(PipelineRun::Aborted) => panic!("unexpected abort"),
    }
}

#[test]
fn pipeline_rejects_excess_pages_from_break_after() {
    let doc = parse(
        "<div style='height:10px;break-after:page'>first</div>\
         <div style='height:10px;break-after:page'>second</div>\
         <div style='height:10px'>third</div>",
    );
    let config = LayoutConfig::builder()
        .limits(
            raikiri_traits::RenderLimits::builder()
                .max_document_pages(Some(2))
                .build(),
        )
        .build();

    let result = run_pipeline(
        &doc,
        PageDefaults::default(),
        &config,
        PipelineInputs {
            resources: None,
            consumer_properties: &[],
            property_observer: None,
            preload_background_images: true,
        },
    );

    match result {
        Err(RenderError::LimitExceeded {
            kind: raikiri_traits::LimitKind::Pages,
            limit: 2,
            actual: 3,
        }) => {}
        Err(error) => panic!("expected the page limit error, got {error}"),
        Ok(PipelineRun::Completed(output)) => {
            panic!(
                "expected rejection, but generated {} pages",
                output.slices.len()
            )
        }
        Ok(PipelineRun::Aborted) => panic!("unexpected abort"),
    }
}

#[test]
fn pipeline_rejects_a_zero_page_limit_before_layout() {
    let doc = parse("<div>one page is still an excess</div>");
    let config = LayoutConfig::builder()
        .limits(
            raikiri_traits::RenderLimits::builder()
                .max_document_pages(Some(0))
                .build(),
        )
        .build();

    let result = run_pipeline(
        &doc,
        PageDefaults::default(),
        &config,
        PipelineInputs {
            resources: None,
            consumer_properties: &[],
            property_observer: None,
            preload_background_images: true,
        },
    );

    assert!(matches!(
        result,
        Err(RenderError::LimitExceeded {
            kind: raikiri_traits::LimitKind::Pages,
            limit: 0,
            actual: 1,
        })
    ));
}

#[test]
fn pipeline_returns_aborted_when_resolver_cancels_before_initial_pagination() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct AbortOnResolve {
        controller: raikiri_traits::AbortController,
        calls: AtomicUsize,
        abort_on_call: usize,
    }

    impl raikiri_traits::ReplacedResolver for AbortOnResolve {
        fn resolve(
            &self,
            _request: raikiri_traits::ResolverRequest<'_>,
        ) -> Result<raikiri_traits::ResolvedIntrinsic, raikiri_traits::ResolverError> {
            if self.calls.fetch_add(1, Ordering::SeqCst) + 1 == self.abort_on_call {
                self.controller.abort();
            }
            Ok(raikiri_traits::ResolvedIntrinsic {
                intrinsic: raikiri_traits::IntrinsicBox::new(1.0, 1.0),
                disposition: raikiri_traits::ResolveDisposition::Ok,
            })
        }
    }

    let controller = raikiri_traits::AbortController::new();
    let resolver = AbortOnResolve {
        controller: raikiri_traits::AbortController {
            signal: controller.signal.clone(),
        },
        calls: AtomicUsize::new(0),
        abort_on_call: 1,
    };
    let resources = RenderResources::new().replaced_resolver(&resolver);
    let doc = parse("<img src='https://example.invalid/image.png'>");
    let config = LayoutConfig::builder()
        .signal(Some(controller.signal.clone()))
        .build();

    let result = run_pipeline(
        &doc,
        PageDefaults::default(),
        &config,
        PipelineInputs {
            resources: Some(&resources),
            consumer_properties: &[],
            property_observer: None,
            preload_background_images: true,
        },
    );

    assert!(matches!(result, Ok(PipelineRun::Aborted)));
    assert!(resolver.calls.load(Ordering::SeqCst) >= 1);
}

#[test]
fn pipeline_returns_aborted_when_resolver_cancels_during_scheduled_pagination() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct AbortOnResolve {
        controller: raikiri_traits::AbortController,
        calls: AtomicUsize,
        abort_on_call: usize,
    }

    impl raikiri_traits::ReplacedResolver for AbortOnResolve {
        fn resolve(
            &self,
            _request: raikiri_traits::ResolverRequest<'_>,
        ) -> Result<raikiri_traits::ResolvedIntrinsic, raikiri_traits::ResolverError> {
            if self.calls.fetch_add(1, Ordering::SeqCst) + 1 == self.abort_on_call {
                self.controller.abort();
            }
            Ok(raikiri_traits::ResolvedIntrinsic {
                intrinsic: raikiri_traits::IntrinsicBox::new(1.0, 1.0),
                disposition: raikiri_traits::ResolveDisposition::Ok,
            })
        }
    }

    let controller = raikiri_traits::AbortController::new();
    let resolver = AbortOnResolve {
        controller: raikiri_traits::AbortController {
            signal: controller.signal.clone(),
        },
        calls: AtomicUsize::new(0),
        abort_on_call: 3,
    };
    let resources = RenderResources::new().replaced_resolver(&resolver);
    let doc = parse(
        "<style>@page{size:200px 100px;margin:0} @page :left{size:200px 50px;margin:0}</style>\
         <div style='height:45px'></div><div style='height:45px'></div>\
         <img src='https://example.invalid/image.png' style='display:block;height:30px'>\
         <div style='height:30px'></div><div style='height:30px'></div>",
    );
    let config = LayoutConfig::builder()
        .signal(Some(controller.signal.clone()))
        .build();

    let result = run_pipeline(
        &doc,
        PageDefaults::default(),
        &config,
        PipelineInputs {
            resources: Some(&resources),
            consumer_properties: &[],
            property_observer: None,
            preload_background_images: true,
        },
    );

    assert!(matches!(result, Ok(PipelineRun::Aborted)));
    assert!(
        resolver.calls.load(Ordering::SeqCst) >= 3,
        "scheduled pagination must invoke the resolver after the probe and initial pass"
    );
}

#[test]
fn pipeline_rejects_page_limit_during_scheduled_pagination() {
    let doc = parse(
        "<style>@page{size:200px 100px;margin:0} @page :left{size:200px 50px;margin:0}</style>\
         <div style='height:45px'></div><div style='height:45px'></div>\
         <div style='height:30px'></div><div style='height:30px'></div><div style='height:30px'></div>",
    );
    let config = LayoutConfig::builder()
        .limits(
            raikiri_traits::RenderLimits::builder()
                .max_document_pages(Some(2))
                .build(),
        )
        .build();

    let result = run_pipeline(
        &doc,
        PageDefaults::default(),
        &config,
        PipelineInputs {
            resources: None,
            consumer_properties: &[],
            property_observer: None,
            preload_background_images: true,
        },
    );

    assert!(matches!(
        result,
        Err(RenderError::LimitExceeded {
            kind: raikiri_traits::LimitKind::Pages,
            limit: 2,
            actual: 3,
        })
    ));
}

#[test]
fn pipeline_applies_page_limit_after_geometry_stabilizes() {
    let doc = parse(
        "<style>@page{size:200px 100px;margin:0} @page :left{size:200px 200px;margin:0}</style>\
         <div style='height:90px'></div><div style='height:90px'></div><div style='height:70px'></div>",
    );
    let config = LayoutConfig::builder()
        .limits(
            raikiri_traits::RenderLimits::builder()
                .max_document_pages(Some(2))
                .build(),
        )
        .build();

    let result = run_pipeline(
        &doc,
        PageDefaults::default(),
        &config,
        PipelineInputs {
            resources: None,
            consumer_properties: &[],
            property_observer: None,
            preload_background_images: true,
        },
    );

    match result {
        Ok(PipelineRun::Completed(output)) => assert_eq!(output.slices.len(), 2),
        Err(error) => panic!("expected the stabilized two-page layout, got {error}"),
        Ok(PipelineRun::Aborted) => panic!("unexpected abort"),
    }
}

#[test]
fn pipeline_does_not_count_distant_absolute_percentage_boxes_as_pages() {
    let doc = parse(
        "<style>@page{size:200px 100px;margin:0} @page :left{size:200px 200px;margin:0}</style>\
         <div style='height:90px'></div><div style='height:90px'></div><div style='height:70px'></div>\
         <div style='position:absolute;top:10000px;width:50%;height:10px'></div>",
    );
    let config = LayoutConfig::builder()
        .limits(
            raikiri_traits::RenderLimits::builder()
                .max_document_pages(Some(2))
                .build(),
        )
        .build();

    let result = run_pipeline(
        &doc,
        PageDefaults::default(),
        &config,
        PipelineInputs {
            resources: None,
            consumer_properties: &[],
            property_observer: None,
            preload_background_images: true,
        },
    );

    match result {
        Ok(PipelineRun::Completed(output)) => assert_eq!(output.slices.len(), 2),
        Err(error) => panic!("an out-of-flow box must not increase the page count: {error}"),
        Ok(PipelineRun::Aborted) => panic!("unexpected abort"),
    }
}

#[test]
fn converged_schedule_keeps_page_content_origins() {
    let doc = parse(
        "<style>@page{size:100px 100px;margin:0} @page :left{size:100px 240px;margin:0} @page wide{size:100px 180px;margin:0}</style><div style='height:200px'>first</div><div style='page:wide;break-before:page;height:10px'>wide</div><div style='height:300px'>tail</div>",
    );
    let out = run(&doc);
    assert_eq!(
        out.slices
            .iter()
            .map(|p| p.content_origin_y)
            .collect::<Vec<_>>(),
        vec![0.0, 100.0, 280.0, 380.0]
    );
}
