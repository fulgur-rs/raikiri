use super::*;
use raikiri_traits::{
    IntrinsicBox, ResolveDisposition, ResolvedIntrinsic, ResolverError, ResolverRequest,
};
use std::cell::{Cell, RefCell};

#[derive(Default)]
struct RecordingResolver {
    urls: RefCell<Vec<String>>,
    fails: bool,
}

impl ReplacedResolver for RecordingResolver {
    fn resolve(&self, request: ResolverRequest<'_>) -> Result<ResolvedIntrinsic, ResolverError> {
        self.urls.borrow_mut().push(request.url().to_string());
        if self.fails {
            return Err(ResolverError::Decode("test image".into()));
        }
        Ok(ResolvedIntrinsic {
            intrinsic: IntrinsicBox::new(10.0, 20.0),
            disposition: ResolveDisposition::Ok,
        })
    }
}

fn image_document(root_css: &str, src: &str) -> (Document, raikiri_style::CascadeResult) {
    let mut document = Document::new();
    let html = document.append_element(Some(0), "html", Style::default(), Some(root_css));
    let body = document.append_element(
        Some(html),
        "body",
        Style::default(),
        Some("display:block;margin:0"),
    );
    let image = document.append_element(Some(body), "img", Style::default(), None::<&str>);
    document.set_element_attributes(image, vec![("src".into(), src.into())]);
    let rules = raikiri_style::build_rule_tree(&document);
    let cascade = raikiri_style::cascade(&document, &rules).unwrap();
    (document, cascade)
}

fn forced_pages(writing_mode: &str) -> (Document, raikiri_style::CascadeResult, Vec<usize>) {
    let mut document = Document::new();
    let html = document.append_element(
        Some(0),
        "html",
        Style::default(),
        Some(format!("display:block;writing-mode:{writing_mode}").as_str()),
    );
    let body = document.append_element(
        Some(html),
        "body",
        Style::default(),
        Some("display:block;margin:0"),
    );
    let children = (0..3)
        .map(|_| {
            document.append_element(
                Some(body),
                "div",
                Style::default(),
                Some("display:block;height:10px;width:50%;break-before:page"),
            )
        })
        .collect();
    let rules = raikiri_style::build_rule_tree(&document);
    let cascade = raikiri_style::cascade(&document, &rules).unwrap();
    (document, cascade, children)
}

fn truncated_control() -> PageLayoutControl<'static> {
    let (mut document, cascade, _) = forced_pages("horizontal-tb");
    let control = PageLayoutControl::for_geometry_discovery(Some(1));
    let pages = layout_pages_with_options(
        &mut document,
        &cascade,
        page_box_800x600(),
        PageLayoutOptions {
            control: Some(&control),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(pages.len(), 1);
    assert!(control.page_limit_reached());
    control
}

#[test]
fn options_default_produces_one_page() {
    let (mut document, cascade) = hello_world_doc();
    let pages = layout_pages_with_options(
        &mut document,
        &cascade,
        page_box_800x600(),
        PageLayoutOptions::default(),
    )
    .unwrap();
    assert_eq!(
        pages,
        vec![PageSlice {
            page_index: 0,
            content_origin_y: 0.0,
            page_name: None,
        }]
    );
}

#[test]
fn options_hidden_root_skips_images() {
    let (mut document, cascade) = image_document("display:none", "https://example.test/image.png");
    let resolver = RecordingResolver {
        fails: true,
        ..Default::default()
    };
    let pages = layout_pages_with_options(
        &mut document,
        &cascade,
        page_box_800x600(),
        PageLayoutOptions {
            images: Some(PageImageOptions {
                resolver: &resolver,
                base_url: None,
            }),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(pages.len(), 1);
    assert!(resolver.urls.borrow().is_empty());
}

#[test]
fn options_cancel_before_resolver() {
    let (mut document, cascade) = image_document("display:block", "https://example.test/image.png");
    let resolver = RecordingResolver::default();
    let abort = || true;
    let control = PageLayoutControl::new(Some(10)).with_abort_check(&abort);
    let result = layout_pages_with_options(
        &mut document,
        &cascade,
        page_box_800x600(),
        PageLayoutOptions {
            images: Some(PageImageOptions {
                resolver: &resolver,
                base_url: None,
            }),
            control: Some(&control),
            ..Default::default()
        },
    );
    assert!(matches!(result, Err(LayoutError::Aborted)));
    assert!(resolver.urls.borrow().is_empty());
}

#[test]
fn options_zero_page_limit_before_resolver() {
    let (mut document, cascade) = image_document("display:block", "https://example.test/image.png");
    let resolver = RecordingResolver::default();
    let control = PageLayoutControl::new(Some(0));
    let result = layout_pages_with_options(
        &mut document,
        &cascade,
        page_box_800x600(),
        PageLayoutOptions {
            images: Some(PageImageOptions {
                resolver: &resolver,
                base_url: None,
            }),
            control: Some(&control),
            ..Default::default()
        },
    );
    assert!(matches!(
        result,
        Err(LayoutError::PageLimitExceeded {
            limit: 0,
            actual: 1
        })
    ));
    assert!(resolver.urls.borrow().is_empty());
}

#[test]
fn options_reuses_discovery_control() {
    let control = truncated_control();
    let (mut document, cascade) = hello_world_doc();
    let pages = layout_pages_with_options(
        &mut document,
        &cascade,
        page_box_800x600(),
        PageLayoutOptions {
            control: Some(&control),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(pages.len(), 1);
    assert!(!control.page_limit_reached());
}

#[test]
fn options_preserves_preflight_reset_order() {
    for images in [false, true] {
        let control = truncated_control();
        let abort = || true;
        let control = control.with_abort_check(&abort);
        let resolver = RecordingResolver::default();
        let (mut document, cascade) =
            image_document("display:block", "https://example.test/image.png");
        let result = layout_pages_with_options(
            &mut document,
            &cascade,
            page_box_800x600(),
            PageLayoutOptions {
                images: images.then_some(PageImageOptions {
                    resolver: &resolver,
                    base_url: None,
                }),
                control: Some(&control),
                ..Default::default()
            },
        );
        assert!(matches!(result, Err(LayoutError::Aborted)));
        assert_eq!(control.page_limit_reached(), images);
        assert!(resolver.urls.borrow().is_empty());
    }
}

#[test]
fn options_resolver_error_preserves_previous_limit_state() {
    let control = truncated_control();
    let resolver = RecordingResolver {
        fails: true,
        ..Default::default()
    };
    let (mut document, cascade) = image_document("display:block", "https://example.test/image.png");
    let result = layout_pages_with_options(
        &mut document,
        &cascade,
        page_box_800x600(),
        PageLayoutOptions {
            images: Some(PageImageOptions {
                resolver: &resolver,
                base_url: None,
            }),
            control: Some(&control),
            ..Default::default()
        },
    );
    assert!(matches!(result, Err(LayoutError::Resolver(_))));
    assert!(control.page_limit_reached());
    assert_eq!(
        &*resolver.urls.borrow(),
        &["https://example.test/image.png"]
    );
}

#[test]
fn options_resolves_relative_urls_and_borrows_short_lived_settings() {
    let resolver = RecordingResolver::default();
    let base = url::Url::parse("https://example.test/docs/").unwrap();
    let (mut document, cascade) = image_document("display:block", "image.png");
    let polls = Cell::new(0);
    {
        let abort = || {
            polls.set(polls.get() + 1);
            false
        };
        let control = PageLayoutControl::new(Some(10)).with_abort_check(&abort);
        let steps = [100.0, 200.0];
        let widths = [80.0, 120.0];
        let pages = layout_pages_with_options(
            &mut document,
            &cascade,
            page_box_800x600(),
            PageLayoutOptions {
                page_steps: &steps,
                page_widths: &widths,
                images: Some(PageImageOptions {
                    resolver: &resolver,
                    base_url: Some(&base),
                }),
                control: Some(&control),
            },
        )
        .unwrap();
        assert_eq!(pages.len(), 1);
        assert!(!control.page_limit_reached());
    }
    assert!(polls.get() > 0);
    assert_eq!(
        &*resolver.urls.borrow(),
        &["https://example.test/docs/image.png"]
    );
}

#[test]
fn options_preserves_variable_page_geometry() {
    let (mut document, cascade, children) = forced_pages("horizontal-tb");
    let mut page_box = PageBox::new();
    page_box.width = 100.0;
    page_box.height = 100.0;
    let pages = layout_pages_with_options(
        &mut document,
        &cascade,
        page_box,
        PageLayoutOptions {
            page_steps: &[100.0, 200.0],
            page_widths: &[80.0, 120.0],
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        pages
            .iter()
            .map(|page| page.content_origin_y)
            .collect::<Vec<_>>(),
        [0.0, 100.0, 300.0]
    );
    assert_eq!(
        children
            .iter()
            .map(|&id| document.nodes[id].unrounded_layout.size.width)
            .collect::<Vec<_>>(),
        [50.0, 75.0, 50.0]
    );
}

#[test]
fn options_vertical_pages_use_the_width_schedule() {
    for writing_mode in ["vertical-rl", "vertical-lr"] {
        let (mut document, mut cascade, children) = forced_pages(writing_mode);
        // Avoid cyclic percentage block sizes in an auto-width vertical body.
        for child in children {
            cascade.computed[child].width = raikiri_style::ComputedLengthPercentageOrAuto::Px(30.0);
        }
        let pages = layout_pages_with_options(
            &mut document,
            &cascade,
            page_box_800x600(),
            PageLayoutOptions {
                page_steps: &[100.0, 200.0],
                page_widths: &[80.0, 120.0],
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            pages
                .iter()
                .map(|page| page.content_origin_y)
                .collect::<Vec<_>>(),
            [0.0, 80.0, 200.0]
        );
    }
}

#[test]
fn options_preserves_one_sided_geometry_schedules() {
    for (steps, widths, origins, sizes) in [
        (
            &[100.0, 200.0][..],
            &[][..],
            [0.0, 100.0, 300.0],
            [50.0, 50.0, 50.0],
        ),
        (
            &[][..],
            &[80.0, 120.0][..],
            [0.0, 100.0, 200.0],
            [50.0, 75.0, 50.0],
        ),
    ] {
        let (mut document, cascade, children) = forced_pages("horizontal-tb");
        let mut page_box = PageBox::new();
        page_box.width = 100.0;
        page_box.height = 100.0;
        let pages = layout_pages_with_options(
            &mut document,
            &cascade,
            page_box,
            PageLayoutOptions {
                page_steps: steps,
                page_widths: widths,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            pages
                .iter()
                .map(|page| page.content_origin_y)
                .collect::<Vec<_>>(),
            origins
        );
        assert_eq!(
            children
                .iter()
                .map(|&id| document.nodes[id].unrounded_layout.size.width)
                .collect::<Vec<_>>(),
            sizes
        );
    }
}

#[test]
fn options_vertical_error_restores_physical_style() {
    let (mut document, mut cascade, children) = forced_pages("vertical-rl");
    for &id in &children {
        cascade.computed[id].width = raikiri_style::ComputedLengthPercentageOrAuto::Px(30.0);
    }
    let control = PageLayoutControl::new(Some(1));
    let result = layout_pages_with_options(
        &mut document,
        &cascade,
        page_box_800x600(),
        PageLayoutOptions {
            control: Some(&control),
            ..Default::default()
        },
    );
    assert!(matches!(result, Err(LayoutError::PageLimitExceeded { .. })));
    for child in children {
        let size = document.layout_style(child).unwrap().size;
        assert_eq!(size.width, Dimension::length(30.0));
        assert_eq!(size.height, Dimension::length(10.0));
    }
}
