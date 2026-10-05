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
fn pipeline_propagates_a_resolver_error_during_initial_pagination() {
    struct FailingResolver;

    impl raikiri_traits::ReplacedResolver for FailingResolver {
        fn resolve(
            &self,
            _request: raikiri_traits::ResolverRequest<'_>,
        ) -> Result<raikiri_traits::ResolvedIntrinsic, raikiri_traits::ResolverError> {
            Err(raikiri_traits::ResolverError::Decode(
                "injected resolver failure".to_owned(),
            ))
        }
    }

    let resolver = FailingResolver;
    let resources = RenderResources::new().replaced_resolver(&resolver);
    let doc = parse("<img src='https://example.invalid/image.png'>");

    let result = run_pipeline(
        &doc,
        PageDefaults::default(),
        &LayoutConfig::default(),
        PipelineInputs {
            resources: Some(&resources),
            consumer_properties: &[],
            property_observer: None,
            preload_background_images: true,
        },
    );

    assert!(matches!(
        result,
        Err(RenderError::Resolver(raikiri_traits::ResolverError::Decode(message)))
            if message == "injected resolver failure"
    ));
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
fn pipeline_returns_aborted_when_resolver_cancels_during_strict_page_confirmation() {
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
        .limits(
            raikiri_traits::RenderLimits::builder()
                .max_document_pages(Some(2))
                .build(),
        )
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

    assert!(
        matches!(result, Ok(PipelineRun::Aborted)),
        "strict page confirmation must abort; calls: {}",
        resolver.calls.load(Ordering::SeqCst)
    );
    assert!(
        resolver.calls.load(Ordering::SeqCst) >= 3,
        "strict page confirmation must reach the aborting resolver"
    );
}

#[test]
fn pipeline_rechecks_the_page_limit_when_strict_confirmation_changes_page_names() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct ChangingResolver {
        calls: AtomicUsize,
    }

    impl raikiri_traits::ReplacedResolver for ChangingResolver {
        fn resolve(
            &self,
            _request: raikiri_traits::ResolverRequest<'_>,
        ) -> Result<raikiri_traits::ResolvedIntrinsic, raikiri_traits::ResolverError> {
            let call = self.calls.fetch_add(1, Ordering::SeqCst);
            let height = if call < 2 { 350.0 } else { 10.0 };
            Ok(raikiri_traits::ResolvedIntrinsic {
                intrinsic: raikiri_traits::IntrinsicBox::new(10.0, height),
                disposition: raikiri_traits::ResolveDisposition::Ok,
            })
        }
    }

    let resolver = ChangingResolver {
        calls: AtomicUsize::new(0),
    };
    let resources = RenderResources::new().replaced_resolver(&resolver);
    let doc = parse(
        "<style>@page{size:200px 100px;margin:0} @page :left{size:200px 200px;margin:0} @page narrow{size:200px 50px;margin:0}</style>\
         <div style='height:40px'></div><img src='https://example.invalid/image.png' style='display:block'>\
         <div style='page:narrow;height:100px'></div>",
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
            resources: Some(&resources),
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
        Err(error) => panic!("expected the recomputed page limit, got {error}"),
        Ok(PipelineRun::Completed(output)) => panic!(
            "expected the recomputed narrow-page schedule to exceed the limit, got {} pages; resolver calls: {}",
            output.slices.len(),
            resolver.calls.load(Ordering::SeqCst)
        ),
        Ok(PipelineRun::Aborted) => panic!("unexpected abort"),
    }
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
fn pipeline_discovers_nested_named_page_before_enforcing_page_limit() {
    let doc = parse(
        "<style>@page{size:200px 100px;margin:0} @page tall{size:200px 200px;margin:0}</style>\
         <div><div style='height:90px'></div><div style='page:tall;height:150px'></div></div>",
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
        Ok(PipelineRun::Completed(output)) => {
            assert_eq!(output.slices.len(), 2);
            assert_eq!(output.slices[1].page_name.as_deref(), Some("tall"));
        }
        Err(error) => panic!("expected the nested tall page to fit, got {error}"),
        Ok(PipelineRun::Aborted) => panic!("unexpected abort"),
    }
}

#[test]
fn pipeline_discovers_named_page_after_oversized_text_before_enforcing_page_limit() {
    let doc = parse(
        "<style>@page{size:200px 100px;margin:0} @page tall{size:200px 200px;margin:0}</style>\
         <div style='height:250px;line-height:250px'>text</div>\
         <div style='margin-top:-160px;page:tall;height:10px'></div>",
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
        Ok(PipelineRun::Completed(output)) => {
            assert_eq!(output.slices.len(), 2);
            assert_eq!(output.slices[1].page_name.as_deref(), Some("tall"));
        }
        Err(error) => panic!("expected the named tall page after text to fit, got {error}"),
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

#[test]
fn pipeline_resolves_root_inherited_left_right_and_named_page_geometry_per_page() {
    let doc = parse(
        "<style>html { font-size: 20px } body { margin: 0 }\
             @page { size: 400px 300px; margin: 2em }\
             @page :left { margin-left: 10px }\
             @page :right { margin-right: 1.5em }\
             @page wide { size: 600px 300px; padding: 0.5em }\
             div { height: 10px; break-after: page }</style>\
             <div>one</div><div>two</div><div>three</div>\
             <div style='page: wide'>four</div><div>five</div>",
    );
    let out = run(&doc);
    let names: Vec<_> = out
        .slices
        .iter()
        .map(|slice| slice.page_name.as_deref())
        .collect();
    assert_eq!(names, [None, None, None, Some("wide"), None]);
    let summary: Vec<_> = out
        .geometries
        .iter()
        .map(|geometry| {
            (
                geometry.page_box.width,
                geometry.page_box.height,
                geometry.margins.top,
                geometry.margins.right,
                geometry.margins.bottom,
                geometry.margins.left,
                geometry.content_insets.top,
            )
        })
        .collect();
    assert_eq!(
        summary,
        [
            // Right pages inherit `font-size: 20px` from the root element.
            (400.0, 300.0, 40.0, 30.0, 40.0, 40.0, 0.0),
            (400.0, 300.0, 40.0, 40.0, 40.0, 10.0, 0.0),
            (400.0, 300.0, 40.0, 30.0, 40.0, 40.0, 0.0),
            (600.0, 300.0, 40.0, 40.0, 40.0, 10.0, 10.0),
            (400.0, 300.0, 40.0, 30.0, 40.0, 40.0, 0.0),
        ]
    );
    assert_eq!(out.page_styles.len(), out.slices.len());
}

#[test]
fn pipeline_preloads_element_backgrounds_once_then_page_backgrounds_in_page_order() {
    #[derive(Default)]
    struct RecordingSvgProvider {
        requests: Mutex<Vec<String>>,
    }

    impl raikiri_traits::NetworkProvider for RecordingSvgProvider {
        fn fetch_one_hop(
            &self,
            request: raikiri_traits::Request,
        ) -> Result<raikiri_traits::FetchOutcome, raikiri_traits::NetworkError> {
            self.requests
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(request.url.path().to_owned());
            Ok(raikiri_traits::FetchOutcome::Body(
                raikiri_traits::FetchedResource {
                    bytes: br#"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"/>"#
                        .as_slice()
                        .into(),
                    content_type: Some("image/svg+xml".into()),
                    final_url: request.url,
                    encoding: None,
                },
            ))
        }
    }

    let provider = RecordingSvgProvider::default();
    let resources = RenderResources::new().network_provider(&provider);
    let doc = parse(
        "<style>body { margin: 0 }\
             @page { size: 200px 100px; margin: 10px;\
                     background-image: url('https://images.test/page.svg');\
                     @top-left { color: red } }\
             @page :right { @top-left { background-image: url('https://images.test/top.svg') } }\
             @page :left { background-image: url('https://images.test/left.svg') }\
             div { height: 10px; break-after: page;\
                   background-image: url('https://images.test/element.svg') }</style>\
             <div>one</div><div>two</div><div>three</div>",
    );
    let out = match run_pipeline(
        &doc,
        PageDefaults::default(),
        &LayoutConfig::default(),
        PipelineInputs {
            resources: Some(&resources),
            consumer_properties: &[],
            property_observer: None,
            preload_background_images: true,
        },
    )
    .expect("pipeline")
    {
        PipelineRun::Completed(out) => *out,
        PipelineRun::Aborted => panic!("unexpected abort"),
    };
    assert_eq!(out.slices.len(), 3);
    let requests = provider
        .requests
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    assert_eq!(
        requests,
        ["/element.svg", "/page.svg", "/top.svg", "/left.svg"]
    );
}
