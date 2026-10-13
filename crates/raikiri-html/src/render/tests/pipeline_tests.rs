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
            resume: None,
        },
    )
    .expect("pipeline")
    {
        PipelineRun::Completed(out) => *out,
        PipelineRun::Aborted | PipelineRun::NotResumed => panic!("unexpected abort"),
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
            resume: None,
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
            resume: None,
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
        Ok(PipelineRun::Aborted) | Ok(PipelineRun::NotResumed) => panic!("unexpected abort"),
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
            resume: None,
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
        Ok(PipelineRun::Aborted) | Ok(PipelineRun::NotResumed) => panic!("unexpected abort"),
    }
}

#[test]
fn pipeline_reports_a_passed_cascade_limit() {
    let doc = parse("<div>styled by the user agent</div>");
    let config = LayoutConfig::builder()
        .limits(
            raikiri_traits::RenderLimits::builder()
                .max_cascade_declarations(Some(0))
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
            resume: None,
        },
    );

    assert!(matches!(
        result,
        Err(RenderError::LimitExceeded {
            kind: raikiri_traits::LimitKind::CascadeDeclarations,
            limit: 0,
            actual: 1..,
        })
    ));
}

#[test]
fn pipeline_reports_a_passed_rule_tree_limit() {
    let doc = parse("<style>div { color: red }</style><div>styled</div>");
    let config = LayoutConfig::builder()
        .limits(
            raikiri_traits::RenderLimits::builder()
                .max_style_declarations(Some(0))
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
            resume: None,
        },
    );

    assert!(matches!(
        result,
        Err(RenderError::LimitExceeded {
            kind: raikiri_traits::LimitKind::StyleDeclarations,
            limit: 0,
            actual: 1..,
        })
    ));
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
            resume: None,
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
            resume: None,
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
            resume: None,
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
            resume: None,
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
            resume: None,
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
        Ok(PipelineRun::Aborted) | Ok(PipelineRun::NotResumed) => panic!("unexpected abort"),
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
            resume: None,
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
            resume: None,
        },
    );

    match result {
        Ok(PipelineRun::Completed(output)) => assert_eq!(output.slices.len(), 2),
        Err(error) => panic!("expected the stabilized two-page layout, got {error}"),
        Ok(PipelineRun::Aborted) | Ok(PipelineRun::NotResumed) => panic!("unexpected abort"),
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
            resume: None,
        },
    );

    match result {
        Ok(PipelineRun::Completed(output)) => {
            assert_eq!(output.slices.len(), 2);
            assert_eq!(output.slices[1].page_name.as_deref(), Some("tall"));
        }
        Err(error) => panic!("expected the nested tall page to fit, got {error}"),
        Ok(PipelineRun::Aborted) | Ok(PipelineRun::NotResumed) => panic!("unexpected abort"),
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
            resume: None,
        },
    );

    match result {
        Ok(PipelineRun::Completed(output)) => {
            assert_eq!(output.slices.len(), 2);
            assert_eq!(output.slices[1].page_name.as_deref(), Some("tall"));
        }
        Err(error) => panic!("expected the named tall page after text to fit, got {error}"),
        Ok(PipelineRun::Aborted) | Ok(PipelineRun::NotResumed) => panic!("unexpected abort"),
    }
}

#[test]
fn pipeline_counts_distant_absolute_boxes_as_pages() {
    // An absolutely positioned box still needs a page to paint on, as in
    // browsers, so a distant one extends the page count and meets the page
    // limit like in-flow content does.
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
            resume: None,
        },
    );

    match result {
        Err(RenderError::LimitExceeded {
            kind: raikiri_traits::LimitKind::Pages,
            limit: 2,
            ..
        }) => {}
        Err(error) => panic!("expected a page limit rejection, got {error}"),
        Ok(PipelineRun::Completed(output)) => {
            panic!(
                "expected rejection, but generated {} pages",
                output.slices.len()
            )
        }
        Ok(PipelineRun::Aborted) | Ok(PipelineRun::NotResumed) => panic!("unexpected abort"),
    }
}

#[test]
fn converged_schedule_keeps_page_content_origins() {
    let doc = parse(
        "<style>@page{size:100px 100px;margin:0} @page :left{size:100px 240px;margin:0} @page wide{size:100px 180px;margin:0}</style><div style='height:200px'>first</div><div style='page:wide;break-before:page;height:10px'>wide</div><div style='height:300px'>tail</div>",
    );
    let out = run(&doc);
    // The first box runs onto the 240px left page, so the forced break opens
    // the third page (`wide`, 180px). The unnamed tail changes the page type
    // again and starts the fourth page; its overflow needs a fifth.
    assert_eq!(
        out.slices
            .iter()
            .map(|p| p.content_origin_y)
            .collect::<Vec<_>>(),
        vec![0.0, 100.0, 340.0, 520.0, 760.0]
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
fn pipeline_resolves_a_named_first_page_from_the_first_page_cascade() {
    let doc = parse(
        "<style>html { font-size: 20px } body { margin: 0 }\
             @page { size: 400px 300px; margin: 10px }\
             @page :first { margin-top: 2em }\
             @page cover { size: 500px 250px; margin-left: 1em }\
             @page cover:first { margin-bottom: 30px }\
             div { height: 10px; break-after: page }</style>\
             <div style='page: cover'>one</div><div>two</div>",
    );
    let out = run(&doc);
    let names: Vec<_> = out
        .slices
        .iter()
        .map(|slice| slice.page_name.as_deref())
        .collect();
    assert_eq!(names, [Some("cover"), None]);
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
            )
        })
        .collect();
    assert_eq!(
        summary,
        [
            // `cover:first` over `cover` over `:first` over `@page`; `2em`
            // and `1em` resolve against the root element's 20px.
            (500.0, 250.0, 40.0, 10.0, 30.0, 20.0),
            (400.0, 300.0, 10.0, 10.0, 10.0, 10.0),
        ]
    );

    // The page context derived for the named first page matches a full
    // cascade of the same query.
    let tree = crate::cascade::build_rule_tree_with_consumer_properties(&doc.uncascaded, &[]);
    let mut query = PageContextQuery::default();
    query.is_first = true;
    query.is_right = true;
    query.page_name = Some(Atom::from("cover"));
    let full = raikiri_style::cascade_with_media_context_for_page(
        &doc.uncascaded.dom,
        &tree,
        &LayoutConfig::default().media_context,
        &query,
    )
    .expect("cascade");
    assert_eq!(out.page_styles[0], full.page);
    assert_eq!(out.cascade.computed, full.computed);
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
            resume: None,
        },
    )
    .expect("pipeline")
    {
        PipelineRun::Completed(out) => *out,
        PipelineRun::Aborted | PipelineRun::NotResumed => panic!("unexpected abort"),
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

#[test]
fn pipeline_selects_font_faces_using_layout_media_dimensions() {
    #[derive(Default)]
    struct RecordingFontProvider(Mutex<Vec<String>>);
    impl raikiri_traits::NetworkProvider for RecordingFontProvider {
        fn fetch_one_hop(
            &self,
            request: raikiri_traits::Request,
        ) -> Result<raikiri_traits::FetchOutcome, raikiri_traits::NetworkError> {
            self.0.lock().unwrap().push(request.url.path().to_owned());
            Ok(raikiri_traits::FetchOutcome::Body(
                raikiri_traits::FetchedResource {
                    bytes: super::AHEM.into(),
                    content_type: Some("font/ttf".into()),
                    final_url: request.url,
                    encoding: None,
                },
            ))
        }
    }
    let doc = parse(
        "<style>@media print and (width:640px) and (height:480px){@font-face{font-family:MediaFont;src:url('https://fonts.test/matching.ttf')}}@media print and (width:480px){@font-face{font-family:OtherFont;src:url('https://fonts.test/other.ttf')}}body{font-family:MediaFont}</style><body>XX</body>",
    );
    let provider = RecordingFontProvider::default();
    let resources = RenderResources::new()
        .network_provider(&provider)
        .fonts(super::ahem_fonts());
    let config = LayoutConfig::builder()
        .media_context(MediaContext::with_viewport(
            crate::MediaType::Print,
            640,
            480,
        ))
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
            resume: None,
        },
    )
    .unwrap();
    assert!(matches!(result, PipelineRun::Completed(_)));
    assert_eq!(*provider.0.lock().unwrap(), ["/matching.ttf"]);
}

#[test]
fn pipeline_keeps_the_media_page_box_independent_of_authored_page_geometry() {
    let doc = parse(
        "<style>@page{size:200px 100px;margin:10px}body{color:red}@media print and (width:640px) and (height:480px){body{color:blue}}</style><body id='body'>text</body>",
    );
    let config = LayoutConfig::builder()
        .media_context(MediaContext::with_viewport(
            crate::MediaType::Print,
            640,
            480,
        ))
        .build();
    let out = match run_pipeline(
        &doc,
        PageDefaults::default(),
        &config,
        PipelineInputs {
            resources: None,
            consumer_properties: &[],
            property_observer: None,
            preload_background_images: true,
            resume: None,
        },
    )
    .unwrap()
    {
        PipelineRun::Completed(out) => out,
        PipelineRun::Aborted | PipelineRun::NotResumed => panic!("unexpected abort"),
    };
    assert_eq!(out.geometries[0].page_box.width, 200.0);
    assert_eq!(out.geometries[0].page_box.height, 100.0);
    let body = (0..out.document.node_count())
        .find(|&id| {
            out.document
                .get_node(id)
                .is_some_and(|node| node.tag_name() == Some("body"))
        })
        .unwrap();
    assert_eq!(
        out.cascade.computed[body].color,
        raikiri_style::CssColor {
            r: 0,
            g: 0,
            b: 255,
            a: 255
        }
    );
}

#[test]
fn pipeline_honors_cancellation_before_final_page_projection() {
    struct AbortOnPageBackground(raikiri_traits::AbortController);
    impl raikiri_traits::NetworkProvider for AbortOnPageBackground {
        fn fetch_one_hop(
            &self,
            request: raikiri_traits::Request,
        ) -> Result<raikiri_traits::FetchOutcome, raikiri_traits::NetworkError> {
            assert_eq!(request.url.path(), "/page.png");
            self.0.abort();
            Err(raikiri_traits::NetworkError::Aborted)
        }
    }
    let controller = raikiri_traits::AbortController::new();
    let provider = AbortOnPageBackground(raikiri_traits::AbortController {
        signal: controller.signal.clone(),
    });
    let resources = RenderResources::new().network_provider(&provider);
    let doc = parse(
        "<style>@page{size:100px 100px;background-image:url(https://images.test/page.png)}</style><p>text</p>",
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
            resume: None,
        },
    );
    assert!(controller.signal.is_aborted());
    assert!(matches!(result, Ok(PipelineRun::Aborted)));
}

#[test]
fn pipeline_reports_a_column_projection_work_limit() {
    // With 8 one-line columns, a cap of 32 admits the layout fragments but
    // not the column projection work, so the error comes from projection.
    // Reaching the built-in cap instead needs thousands of columns, whose
    // layout dominates the test's run time.
    let count = 8;
    let limit = 32;
    let html = format!(
        "<style>body{{margin:0}}.mc{{width:160px;column-count:{count};column-gap:0;orphans:1;widows:1}}p{{margin:0;font:10px/10px Ahem}}</style><div class=mc><p>{}</p></div>",
        vec!["A"; count].join("<br>")
    );
    let fonts = crate::FontCollectionBuilder::new()
        .font_bytes("Ahem", super::AHEM.to_vec())
        .build()
        .unwrap();
    let resources = RenderResources::new().fonts(fonts);
    let render = |doc: &HtmlDocument| {
        run_pipeline(
            doc,
            PageDefaults::default(),
            &LayoutConfig::default(),
            PipelineInputs {
                resources: Some(&resources),
                consumer_properties: &[],
                property_observer: None,
                preload_background_images: false,
                resume: None,
            },
        )
    };
    let mut doc = parse(&html);
    assert!(matches!(render(&doc), Ok(PipelineRun::Completed(_))));
    doc.uncascaded.dom.lower_layout_fragment_limit(limit);
    assert!(matches!(
        render(&doc),
        Err(RenderError::Layout(LayoutError::FragmentLimitExceeded { limit: actual }))
            if actual == limit
    ));
}

#[test]
fn many_columns_with_generated_decoration_prepare_one_source_piece() {
    let count = 1000;
    let html = format!(
        "<style>body{{margin:0}}.mc{{width:20000px;column-count:{count};column-gap:0;orphans:1;widows:1}}p{{margin:0;font:10px/10px Ahem}}p::before{{content:'X';background:red}}</style><div class=mc><p>{}</p></div>",
        vec!["A"; count].join("<br>")
    );
    let doc = parse(&html);
    let fonts = crate::FontCollectionBuilder::new()
        .font_bytes("Ahem", super::AHEM.to_vec())
        .build()
        .unwrap();
    let resources = RenderResources::new().fonts(fonts);
    let out = match run_pipeline(
        &doc,
        PageDefaults::default(),
        &LayoutConfig::default(),
        PipelineInputs {
            resources: Some(&resources),
            consumer_properties: &[],
            property_observer: None,
            preload_background_images: false,
            resume: None,
        },
    )
    .unwrap()
    {
        PipelineRun::Completed(out) => out,
        PipelineRun::Aborted | PipelineRun::NotResumed => panic!("completed layout"),
    };
    assert_eq!(
        out.document
            .page_paint_order(&out.cascade, 0, None)
            .iter()
            .filter(|event| matches!(event, raikiri_dom::PaintEvent::GeneratedBox(_)))
            .count(),
        1
    );
}

#[test]
fn pipeline_counts_absolute_boxes_inside_inline_elements_as_pages() {
    // The box inside the span is located from the paragraph's root, so it
    // still extends the document onto a third page.
    let doc = parse(
        "<style>@page{size:200px 100px;margin:0} body{margin:0}</style>\
         <div style='position:relative'><span>text\
         <span style='position:absolute;top:250px;width:10px;height:10px'></span></span></div>",
    );
    assert_eq!(run(&doc).slices.len(), 3);
}

#[test]
fn pipeline_clamps_body_height_by_max_height_when_counting_pages() {
    let doc = parse(
        "<style>@page{size:200px 100px;margin:0} body{margin:0;height:500px;max-height:150px}</style>\
         <div>text</div>",
    );
    assert_eq!(run(&doc).slices.len(), 2);
}

#[test]
fn pipeline_names_pages_opened_by_absolute_boxes_after_the_flow() {
    let doc = parse(
        "<style>@page{size:200px 100px;margin:0} @page wide{size:300px 100px;margin:0} body{margin:0}</style>\
         <div style='page:wide;position:relative'>text\
         <div style='position:absolute;top:150px;height:10px'></div></div>",
    );
    let out = run(&doc);
    assert_eq!(out.slices.len(), 2);
    assert!(
        out.slices
            .iter()
            .all(|slice| slice.page_name.as_deref() == Some("wide"))
    );
}

#[test]
fn pipeline_counts_the_body_after_the_root_margin() {
    let doc = parse(
        "<style>@page{size:200px 100px;margin:0} html{margin-top:60px} body{margin:0;height:60px}</style>",
    );
    assert_eq!(run(&doc).slices.len(), 2);
}

#[test]
fn pipeline_counts_the_block_end_padding_of_an_auto_height_body() {
    for content in ["", "<div style='height:20px'>x</div>"] {
        let doc = parse(&format!(
            "<style>@page{{size:200px 100px;margin:0}} body{{margin:0;padding-bottom:150px}}</style>{content}"
        ));
        assert_eq!(run(&doc).slices.len(), 2, "{content:?}");
    }
}

#[test]
fn pipeline_named_page_after_paper_fit_box_ignores_absolute_boxes() {
    // The 90px box overflows the 80px page area into the page padding but
    // fits the paper, so it stays on the first page without opening the
    // second. An absolutely positioned box reaching that page must not make
    // the flow look as if it did.
    let page_count = |absolute: &str| {
        let doc = parse(&format!(
            "<style>@page{{size:100px 100px;margin:0;padding:10px}}\
             @page wide{{size:200px 100px;margin:0;padding:10px}} body{{margin:0}}</style>{absolute}\
             <div style='height:90px'>first</div><div style='page:wide;height:10px'>wide</div>"
        ));
        run(&doc).slices.len()
    };
    let without = page_count("");
    let with = page_count("<div style='position:absolute;top:0;height:150px;width:10px'></div>");
    assert_eq!(with, without);
}

#[test]
fn pipeline_counts_boxes_under_boxless_overflow_hidden_elements() {
    // `display: contents` generates no box and so clips nothing.
    let doc = parse(
        "<style>@page{size:200px 100px;margin:0} body{margin:0}</style>\
         <div style='position:relative'>text<div style='display:contents;overflow:hidden'>\
         <div style='position:absolute;top:250px;width:10px;height:10px'></div></div></div>",
    );
    assert_eq!(run(&doc).slices.len(), 3);
}

#[test]
fn pipeline_resolves_body_percentage_height_against_a_definite_root() {
    let doc = parse(
        "<style>@page{size:200px 100px;margin:0} html{height:500px} body{margin:0;height:100%}</style>",
    );
    assert_eq!(run(&doc).slices.len(), 5);
}

#[test]
fn pipeline_names_continuation_pages_after_the_innermost_box() {
    // The outer box runs onto every page and names them `a`; the nested `b`
    // box starts the second page and runs onto the third and fourth, which
    // take its page type.
    let doc = parse(
        "<style>@page{size:200px 100px;margin:0} @page a{size:200px 100px;margin:0}\
         @page b{size:300px 100px;margin:0} body{margin:0}</style>\
         <div style='page:a;height:1000px'><div style='height:50px'></div><div style='page:b;height:250px'></div></div>",
    );
    let out = run(&doc);
    let names: Vec<_> = out
        .slices
        .iter()
        .map(|slice| slice.page_name.as_deref())
        .collect();
    assert_eq!(
        names[..5],
        [Some("a"), Some("b"), Some("b"), Some("b"), Some("a")],
        "{names:?}"
    );
}

#[test]
fn pipeline_keeps_a_definite_zero_body_height_before_its_padding() {
    // The 50px border box and the 80px child both fit the first page.
    let doc = parse(
        "<style>@page{size:200px 100px;margin:0} body{margin:0;height:0;padding-bottom:50px}</style>\
         <div style='height:80px'>x</div>",
    );
    assert_eq!(run(&doc).slices.len(), 1);
}

#[test]
fn pipeline_does_not_count_a_body_clipped_by_the_viewport() {
    let doc = parse(
        "<style>@page{size:200px 100px;margin:0} html{overflow:hidden} body{margin:0;height:1000px}</style>",
    );
    assert_eq!(run(&doc).slices.len(), 1);
}

#[test]
fn pipeline_resolves_body_percentage_height_against_a_calc_root() {
    let doc = parse(
        "<style>@page{size:200px 100px;margin:0} html{height:calc(500px)} body{margin:0;height:100%}</style>",
    );
    assert_eq!(run(&doc).slices.len(), 5);
}

#[test]
fn pipeline_constrains_the_auto_body_height_before_its_padding() {
    // The auto height (80px) beats `min-height`, and the padding then
    // reaches the second page; `max-height` caps it back onto the first.
    for (constraint, pages) in [("min-height:50px", 2), ("max-height:40px", 1)] {
        let doc = parse(&format!(
            "<style>@page{{size:200px 100px;margin:0}} body{{margin:0;{constraint};padding-bottom:40px}}</style>\
             <div style='height:80px'>x</div>"
        ));
        assert_eq!(run(&doc).slices.len(), pages, "{constraint}");
    }
}

#[test]
fn pipeline_resolves_body_percentages_against_the_root_used_height() {
    let doc = parse(
        "<style>@page{size:200px 100px;margin:0} html{height:500px;max-height:150px} body{margin:0;height:100%}</style>",
    );
    assert_eq!(run(&doc).slices.len(), 2);
}

#[test]
fn pipeline_names_tail_pages_after_the_flow_with_its_last_page_type() {
    // The absolutely positioned box reaches the fourth page before the
    // named box changes the page type on the second.
    let doc = parse(
        "<style>@page{size:200px 100px;margin:0} @page b{size:300px 100px;margin:0} body{margin:0}</style>\
         <div style='position:absolute;top:0;width:10px;height:350px'></div>\
         <div style='height:50px'></div><div style='page:b;height:10px'></div>",
    );
    let out = run(&doc);
    let names: Vec<_> = out
        .slices
        .iter()
        .map(|slice| slice.page_name.as_deref())
        .collect();
    assert_eq!(names, [None, Some("b"), Some("b"), Some("b")]);
}

#[test]
fn pipeline_measures_the_auto_body_from_its_flow_content() {
    // A `display: contents` wrapper's child counts as the body's own; a
    // relative offset does not move the body's end.
    for (content, pages) in [
        (
            "<div style='display:contents'><div style='height:80px'>x</div></div>",
            2,
        ),
        (
            "<div style='position:relative;top:150px;height:20px'>x</div>",
            2,
        ),
    ] {
        let doc = parse(&format!(
            "<style>@page{{size:200px 100px;margin:0}} body{{margin:0;padding-bottom:40px}}</style>{content}"
        ));
        assert_eq!(run(&doc).slices.len(), pages, "{content}");
    }
}

#[test]
fn pipeline_resolves_root_percentage_heights_against_the_page_area() {
    // The root's percentages resolve against the page area, 100px here; a
    // border-box root gives the body its content box.
    for (css, pages) in [
        ("html{height:200%} body{margin:0;height:100%}", 2),
        (
            "html{height:calc(150% + 50px);box-sizing:border-box;padding-bottom:100px} body{margin:0;height:100%}",
            2,
        ),
        (
            "body{margin:0;height:150px;box-sizing:border-box;padding-bottom:20px}",
            2,
        ),
    ] {
        let doc = parse(&format!(
            "<style>@page{{size:200px 100px;margin:0}} {css}</style>"
        ));
        assert_eq!(run(&doc).slices.len(), pages, "{css}");
    }
}

#[test]
fn pipeline_measures_the_auto_body_without_relative_offsets_and_with_negative_margins() {
    // The body ends after the static position of its last in-flow child,
    // including a negative block-end margin, followed by its padding. Boxes
    // outside the flow do not move that end, but still paint where they are.
    for (content, pages) in [
        (
            "<div style='position:relative;top:calc(150px + 1%);height:20px'>x</div>",
            2,
        ),
        (
            "<div style='display:none;height:500px'></div>\
             <div style='position:absolute;height:10px;width:10px'></div>\
             <div style='float:left;height:10px;width:10px'></div>\
             <div style='position:relative;bottom:-150px;height:20px'>x</div>",
            2,
        ),
        ("<div style='height:70px;margin-bottom:-30px'>x</div>", 1),
    ] {
        let doc = parse(&format!(
            "<style>@page{{size:200px 100px;margin:0}} body{{margin:0;padding-bottom:40px}}</style>{content}"
        ));
        assert_eq!(run(&doc).slices.len(), pages, "{content}");
    }
}

#[test]
fn pipeline_counts_the_pages_a_relatively_positioned_body_paints_on() {
    for (css, pages) in [
        ("body{margin:0;position:relative;top:150px;height:20px}", 2),
        (
            "body{margin:0;position:relative;bottom:-250px;height:20px}",
            3,
        ),
        ("body{margin:0;position:relative;top:10%;height:20px}", 1),
    ] {
        let doc = parse(&format!(
            "<style>@page{{size:200px 100px;margin:0}} {css}</style><div style='height:10px'>x</div>"
        ));
        assert_eq!(run(&doc).slices.len(), pages, "{css}");
    }
}

#[test]
fn pipeline_opens_a_new_page_for_a_page_type_change_after_overflowing_content() {
    // The tall box continues onto the second page, so the named box after it
    // opens a third, whether it starts partway down the second page or at
    // the top of the third.
    for height in [150, 200] {
        let doc = parse(&format!(
            "<style>@page{{size:100px 100px;margin:0}} @page wide{{size:200px 100px;margin:0}} body{{margin:0}}</style>\
             <div style='height:{height}px'></div><div style='page:wide;height:10px'>wide</div>"
        ));
        assert_eq!(run(&doc).slices.len(), 3, "{height}");
    }
}

#[test]
fn pipeline_caps_a_border_box_auto_body_by_max_height() {
    // Under `border-box` the 100px maximum covers the padding too.
    for (sizing, pages) in [("border-box", 1), ("content-box", 2)] {
        let doc = parse(&format!(
            "<style>@page{{size:200px 100px;margin:0}} body{{margin:0;box-sizing:{sizing};max-height:100px;padding-bottom:40px}}</style>\
             <div style='height:80px'>x</div>"
        ));
        assert_eq!(run(&doc).slices.len(), pages, "{sizing}");
    }
}

#[test]
fn pipeline_keeps_the_last_margin_of_a_body_with_min_height() {
    // A `min-height` keeps the child's block-end margin inside the body.
    for (min_height, pages) in [("80px", 2), ("0", 1)] {
        let doc = parse(&format!(
            "<style>@page{{size:200px 100px;margin:0}} body{{margin:0;min-height:{min_height}}}</style>\
             <div style='height:20px;margin-bottom:100px'>x</div>"
        ));
        assert_eq!(run(&doc).slices.len(), pages, "{min_height}");
    }
}

#[test]
fn pipeline_does_not_measure_a_display_contents_body() {
    let doc = parse(
        "<style>@page{size:200px 100px;margin:0} body{display:contents;height:1000px}</style>",
    );
    assert_eq!(run(&doc).slices.len(), 1);
}

#[test]
fn pipeline_measures_the_auto_body_from_its_collapsed_flow_and_enclosed_floats() {
    // Sibling margins collapse before the body's padding follows the last
    // child; a flow-root body also ends after its floats.
    for (css, content, pages) in [
        (
            "body{margin:0;padding-bottom:10px}",
            "<div style='height:20px;margin-bottom:200px'>x</div><div style='height:10px;margin-top:-200px'>y</div>",
            1,
        ),
        (
            "body{margin:0;padding-bottom:40px;display:flow-root}",
            "<div style='float:left;width:10px;height:80px'></div><div style='height:1px'></div>",
            2,
        ),
        (
            "body{margin:0;padding-bottom:40px}",
            "<div style='float:left;width:10px;height:80px'></div><div style='height:1px'></div>",
            1,
        ),
    ] {
        let doc = parse(&format!(
            "<style>@page{{size:200px 100px;margin:0}} {css}</style>{content}"
        ));
        assert_eq!(run(&doc).slices.len(), pages, "{css} {content}");
    }
}

#[test]
fn pipeline_counts_the_pages_a_relatively_positioned_float_paints_on() {
    let doc = parse(
        "<style>@page{size:200px 100px;margin:0} body{margin:0}</style>\
         <div style='float:left;position:relative;top:150px;width:10px;height:20px'></div>",
    );
    assert_eq!(run(&doc).slices.len(), 2);
}

#[test]
fn pipeline_places_floats_after_a_forced_break_clear_of_earlier_pages() {
    // Each page starts with a right float; the floats of earlier pages do
    // not reach the later ones, so every float sits at the right edge.
    let block = "<div style='break-before:page'>\
                 <div class='f' style='float:right;width:100px;height:100px'></div></div>";
    let doc = parse(&format!(
        "<style>@page {{ size: 400px 400px; margin: 0 }} body {{ margin: 0 }}</style>\
         <div><div class='f' style='float:right;width:100px;height:100px'></div></div>{}",
        block.repeat(2)
    ));
    let out = run(&doc);
    assert_eq!(out.slices.len(), 3);
    for page in 0..3 {
        let (document, _) = out.layout_for_page(page);
        let xs: Vec<f32> = document
            .page_fragments(page)
            .filter(|fragment| fragment.rect().width == 100.0)
            .map(|fragment| fragment.rect().x)
            .collect();
        assert_eq!(xs, vec![300.0], "page {page}");
    }
}

#[test]
fn pipeline_places_floats_after_a_named_page_change_clear_of_earlier_pages() {
    // A change of named page forces the break, so the floats before it do
    // not reach the next page either.
    let doc = parse(
        "<style>@page { size: 400px 400px; margin: 0 } body { margin: 0 }\
         .f { float: right; width: 100px; height: 100px }</style>\
         <div style='page:a'><div class='f'></div></div>\
         <div style='page:b'><div class='f'></div></div>\
         <div style='page:c'><div class='f'></div></div>",
    );
    let out = run(&doc);
    assert_eq!(out.slices.len(), 3);
    for page in 0..3 {
        let (document, _) = out.layout_for_page(page);
        let xs: Vec<f32> = document
            .page_fragments(page)
            .filter(|fragment| fragment.rect().width == 100.0)
            .map(|fragment| fragment.rect().x)
            .collect();
        assert_eq!(xs, vec![300.0], "page {page}");
    }
}

#[test]
fn pipeline_prints_a_hidden_root_on_one_page() {
    let doc = parse(
        "<html style='display:none'><body>\
         <div style='height:2000px'>a</div>\
         <div style='break-before:page'>b <a href='#x'>c</a></div></body></html>",
    );
    let out = run(&doc);
    assert_eq!(out.slices.len(), 1);
    // The hidden content leaves nothing on the page.
    let (document, cascade) = out.layout_for_page(0);
    assert_eq!(document.page_fragments(0).count(), 0);
    assert!(document.page_text_runs(cascade, 0).is_empty());
    assert_eq!(document.page_links(0).count(), 0);
}

#[test]
fn pipeline_clears_a_forced_break_inside_display_contents_on_both_sides() {
    // The box after the break sits in a `display: contents` wrapper and
    // clears only the left side; the right float of the earlier page still
    // does not reach it.
    let doc = parse(
        "<style>@page { size: 400px 400px; margin: 0 } body { margin: 0 }\
         .f { float: right; width: 100px; height: 100px }</style>\
         <div><div class='f'></div></div>\
         <div style='display:contents'>\
         <div style='break-before:page; clear:left'><div class='f'></div></div></div>",
    );
    let out = run(&doc);
    assert_eq!(out.slices.len(), 2);
    for page in 0..2 {
        let (document, _) = out.layout_for_page(page);
        let xs: Vec<f32> = document
            .page_fragments(page)
            .filter(|fragment| fragment.rect().width == 100.0)
            .map(|fragment| fragment.rect().x)
            .collect();
        assert_eq!(xs, vec![300.0], "page {page}");
    }
}

#[test]
fn pipeline_clears_the_box_after_a_nested_break_after() {
    // The forced break sits on the last box inside a wrapper; the wrapper's
    // next sibling starts the new page clear of the earlier float.
    let doc = parse(
        "<style>@page { size: 400px 400px; margin: 0 } body { margin: 0 }\
         .f { float: right; width: 100px; height: 100px }</style>\
         <div><div class='f'></div><div style='break-after:page'>a</div></div>\
         <div><div class='f'></div></div>",
    );
    let out = run(&doc);
    assert_eq!(out.slices.len(), 2);
    for page in 0..2 {
        let (document, _) = out.layout_for_page(page);
        let xs: Vec<f32> = document
            .page_fragments(page)
            .filter(|fragment| fragment.rect().width == 100.0)
            .map(|fragment| fragment.rect().x)
            .collect();
        assert_eq!(xs, vec![300.0], "page {page}");
    }
}

#[test]
fn pipeline_clears_a_float_right_after_a_forced_break() {
    // The float right after the break goes to the new page clear of the
    // earlier page's float, and the block after it follows it there.
    let doc = parse(
        "<style>@page { size: 400px 400px; margin: 0 } body { margin: 0 }\
         .f { float: right; width: 100px; height: 100px }</style>\
         <div><div class='f'></div></div>\
         <div style='break-after:page'>a</div>\
         <div class='f'></div><div style='height:50px'>b</div>",
    );
    let out = run(&doc);
    assert_eq!(out.slices.len(), 2);
    let (document, _) = out.layout_for_page(1);
    let floats: Vec<f32> = document
        .page_fragments(1)
        .filter(|fragment| fragment.rect().width == 100.0)
        .map(|fragment| fragment.rect().x)
        .collect();
    assert_eq!(floats, vec![300.0]);
    assert!(
        document
            .page_fragments(1)
            .any(|fragment| fragment.rect().height == 50.0),
        "the block follows the float onto the new page"
    );
}

#[test]
fn pipeline_lines_up_floats_after_a_forced_break_on_the_new_page() {
    // Only the first float after the break clears the earlier page; the
    // second lines up beside it.
    let doc = parse(
        "<style>@page { size: 400px 400px; margin: 0 } body { margin: 0 }\
         .f { float: right; width: 100px; height: 100px }</style>\
         <div><div class='f'></div></div>\
         <div style='break-after:page'>a</div>\
         <div class='f'></div><div class='f'></div>",
    );
    let out = run(&doc);
    assert_eq!(out.slices.len(), 2);
    let (document, _) = out.layout_for_page(1);
    let mut floats: Vec<(f32, f32)> = document
        .page_fragments(1)
        .filter(|fragment| fragment.rect().width == 100.0)
        .map(|fragment| (fragment.rect().x, fragment.rect().y))
        .collect();
    floats.sort_by(|a, b| a.0.total_cmp(&b.0));
    assert_eq!(floats.len(), 2);
    assert_eq!(floats[0].0, 200.0);
    assert_eq!(floats[1].0, 300.0);
    assert_eq!(floats[0].1, floats[1].1);
}

#[test]
fn pipeline_clears_a_named_page_after_leading_text() {
    // The text takes the body's page, so the box on page `b` after it starts
    // a new page clear of the float beside the text.
    let doc = parse(
        "<style>@page { size: 400px 400px; margin: 0 } body { margin: 0 }\
         .f { float: right; width: 100px; height: 100px }</style>\
         <div class='f'></div>text\
         <div style='page:b'><div class='f'></div></div>",
    );
    let out = run(&doc);
    assert_eq!(out.slices.len(), 2);
    let (document, _) = out.layout_for_page(1);
    let xs: Vec<f32> = document
        .page_fragments(1)
        .filter(|fragment| fragment.rect().width == 100.0)
        .map(|fragment| fragment.rect().x)
        .collect();
    assert_eq!(xs, vec![300.0]);
}

#[test]
fn pipeline_clears_a_named_page_after_leading_generated_content() {
    // Generated content before the boxes takes the body's page like text.
    let doc = parse(
        "<style>@page { size: 400px 400px; margin: 0 } body { margin: 0 }\
         body::before { content: 'x' }\
         .f { float: right; width: 100px; height: 100px }</style>\
         <div class='f'></div>\
         <div style='page:b'><div class='f'></div></div>",
    );
    let out = run(&doc);
    assert_eq!(out.slices.len(), 2);
    let (document, _) = out.layout_for_page(1);
    let xs: Vec<f32> = document
        .page_fragments(1)
        .filter(|fragment| fragment.rect().width == 100.0)
        .map(|fragment| fragment.rect().x)
        .collect();
    assert_eq!(xs, vec![300.0]);
}

#[test]
fn pipeline_clears_a_named_page_after_generated_content_of_display_contents() {
    // The wrapper generates no box, but its `::before` content still takes
    // the body's page before the box on page `b`.
    let doc = parse(
        "<style>@page { size: 400px 400px; margin: 0 } body { margin: 0 }\
         .w { display: contents } .w::before { content: 'x' }\
         .f { float: right; width: 100px; height: 100px }</style>\
         <div class='f'></div>\
         <div class='w'><div style='page:b'><div class='f'></div></div></div>",
    );
    let out = run(&doc);
    assert_eq!(out.slices.len(), 2);
    let (document, _) = out.layout_for_page(1);
    let xs: Vec<f32> = document
        .page_fragments(1)
        .filter(|fragment| fragment.rect().width == 100.0)
        .map(|fragment| fragment.rect().x)
        .collect();
    assert_eq!(xs, vec![300.0]);
}

#[test]
fn pipeline_clears_a_named_page_after_a_float_on_the_parent_page() {
    // The float between the boxes on page `b` takes the body's page, so the
    // second box on page `b` starts a new page clear of it.
    let doc = parse(
        "<style>@page { size: 400px 400px; margin: 0 } body { margin: 0 }\
         .f { float: right; width: 100px; height: 100px }</style>\
         <div style='page:b'>a</div><div class='f'></div>\
         <div style='page:b'><div class='f'></div></div>",
    );
    let out = run(&doc);
    assert_eq!(out.slices.len(), 3);
    let (document, _) = out.layout_for_page(2);
    let xs: Vec<f32> = document
        .page_fragments(2)
        .filter(|fragment| fragment.rect().width == 100.0)
        .map(|fragment| fragment.rect().x)
        .collect();
    assert_eq!(xs, vec![300.0]);
}

#[test]
fn pipeline_ignores_forced_breaks_on_inline_boxes() {
    // Pagination does not break after an inline box, so the block after it
    // stays beside the float on the same page.
    let doc = parse(
        "<style>@page { size: 400px 400px; margin: 0 } body { margin: 0 }\
         .f { float: right; width: 100px; height: 100px }</style>\
         <div class='f'></div><span style='break-after:page'>a</span>\
         <div style='display:flow-root; height:50px'>b</div>",
    );
    let out = run(&doc);
    assert_eq!(out.slices.len(), 1);
    let (document, _) = out.layout_for_page(0);
    let block = document
        .page_fragments(0)
        .find(|fragment| fragment.rect().height == 50.0)
        .expect("the block is on the page");
    assert_eq!(block.rect().width, 300.0);
}

#[test]
fn pipeline_clears_a_float_with_its_own_break_before() {
    // The float's own break moves it to the new page, clear of the earlier
    // page's float.
    let doc = parse(
        "<style>@page { size: 400px 400px; margin: 0 } body { margin: 0 }\
         .f { float: right; width: 100px; height: 100px }</style>\
         <div><div class='f'></div></div><div>a</div>\
         <div class='f' style='break-before:page'></div>",
    );
    let out = run(&doc);
    assert_eq!(out.slices.len(), 2);
    let (document, _) = out.layout_for_page(1);
    let floats: Vec<f32> = document
        .page_fragments(1)
        .filter(|fragment| fragment.rect().width == 100.0)
        .map(|fragment| fragment.rect().x)
        .collect();
    assert_eq!(floats, vec![300.0]);
}

#[test]
fn pipeline_clears_the_box_after_a_float_with_break_after() {
    // The box after the float starts the new page and does not shorten
    // beside the earlier page's floats.
    let doc = parse(
        "<style>@page { size: 400px 400px; margin: 0 } body { margin: 0 }\
         .f { float: right; width: 100px; height: 100px }</style>\
         <div><div class='f'></div></div>\
         <div class='f' style='break-after:page'></div>\
         <div style='height:50px; display:flow-root'>b</div>",
    );
    let out = run(&doc);
    assert_eq!(out.slices.len(), 2);
    let (document, _) = out.layout_for_page(1);
    let block = document
        .page_fragments(1)
        .find(|fragment| fragment.rect().height == 50.0)
        .expect("the block is on the new page");
    assert_eq!(block.rect().width, 400.0);
}
