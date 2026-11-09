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
        Ok(ResolvedIntrinsic::new(
            IntrinsicBox::new(10.0, 20.0),
            ResolveDisposition::Ok,
        ))
    }
}

fn image_document(root_css: &str, src: &str) -> (Document, raikiri_style::CascadeResult) {
    let mut document = Document::new();
    let html = document.append_element(Some(0), "html", Some(root_css));
    let body = document.append_element(Some(html), "body", Some("display:block;margin:0"));
    let image = document.append_element(Some(body), "img", None::<&str>);
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
        Some(format!("display:block;writing-mode:{writing_mode}").as_str()),
    );
    let body = document.append_element(Some(html), "body", Some("display:block;margin:0"));
    let children = (0..3)
        .map(|_| {
            document.append_element(
                Some(body),
                "div",
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
            .map(|&id| document.nodes[id].layout.size.width)
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
                .map(|&id| document.nodes[id].layout.size.width)
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
        let size = crate::layout::test_support::bridged_style(&mut document, &cascade, child).size;
        assert_eq!(size.width, Dimension::length(30.0));
        assert_eq!(size.height, Dimension::length(10.0));
    }
}

/// `body > [div(padding-bottom) > div(height), div(height:10px)?]` on pages
/// 600px tall.
fn padded_card_document(
    height: u32,
    padding: u32,
    block_after: bool,
) -> (Document, raikiri_style::CascadeResult) {
    let mut document = Document::new();
    let html = document.append_element(Some(0), "html", Some("display:block"));
    let body = document.append_element(Some(html), "body", Some("display:block;margin:0"));
    let card = document.append_element(
        Some(body),
        "div",
        Some(format!("display:block;padding-bottom:{padding}px").as_str()),
    );
    document.append_element(
        Some(card),
        "div",
        Some(format!("display:block;height:{height}px").as_str()),
    );
    if block_after {
        document.append_element(Some(body), "div", Some("display:block;height:10px"));
    }
    let rules = raikiri_style::build_rule_tree(&document);
    let cascade = raikiri_style::cascade(&document, &rules).unwrap();
    (document, cascade)
}

#[test]
fn options_page_limit_stops_the_flow_where_edges_start_the_next_page() {
    // Without the limit the padding takes the second page; with it the flow
    // stops there, whether a block follows the card or the card ends the flow.
    for block_after in [false, true] {
        let (mut document, cascade) = padded_card_document(560, 100, block_after);
        let pages = layout_pages_with_options(
            &mut document,
            &cascade,
            page_box_800x600(),
            PageLayoutOptions::default(),
        )
        .unwrap();
        assert_eq!(pages.len(), 2, "{block_after}");

        let (mut document, cascade) = padded_card_document(560, 100, block_after);
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
        assert_eq!(pages.len(), 1, "{block_after}");
        assert!(control.page_limit_reached(), "{block_after}");
    }
}

#[test]
fn options_page_limit_stops_the_flow_inside_edges_that_cross_pages() {
    // The 1500px of padding start the second page and end on the fourth: the
    // limit of two pages lets the flow reach the second and no further.
    let (mut document, cascade) = padded_card_document(560, 1500, false);
    let control = PageLayoutControl::for_geometry_discovery(Some(2));
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
    assert_eq!(pages.len(), 2);
    assert!(control.page_limit_reached());

    // A strict limit reports the page that only the pushed padding reaches:
    // the card ends at 1200 without the push and at 1210 with it.
    let (mut document, cascade) = padded_card_document(590, 610, false);
    let control = PageLayoutControl::new(Some(2));
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
}

/// `body > div(1200px, overflow) > div(padding-bottom:360px) > div(padding-bottom:360px) > div(480px)`
/// on pages 600px tall: the inner padding starts the second page and the outer
/// padding is left with nothing above the end of the box around them.
fn nested_padded_cards_document(clip: &str) -> (Document, raikiri_style::CascadeResult) {
    let mut document = Document::new();
    let html = document.append_element(Some(0), "html", Some("display:block"));
    let body = document.append_element(Some(html), "body", Some("display:block;margin:0"));
    let outer = document.append_element(
        Some(body),
        "div",
        Some(format!("display:block;height:1200px;{clip}").as_str()),
    );
    let card = document.append_element(
        Some(outer),
        "div",
        Some("display:block;padding-bottom:360px"),
    );
    let inner = document.append_element(
        Some(card),
        "div",
        Some("display:block;padding-bottom:360px"),
    );
    document.append_element(Some(inner), "div", Some("display:block;height:480px"));
    let rules = raikiri_style::build_rule_tree(&document);
    let cascade = raikiri_style::cascade(&document, &rules).unwrap();
    (document, cascade)
}

#[test]
fn options_page_limit_ignores_pushed_edges_that_a_clip_hides() {
    // The padding pushed to the second page's end lies past the box that clips
    // it, so it does not reach a third page; unclipped it does.
    let (mut document, cascade) = nested_padded_cards_document("overflow:hidden");
    let control = PageLayoutControl::new(Some(2));
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
    assert_eq!(pages.len(), 2);

    let (mut document, cascade) = nested_padded_cards_document("");
    let control = PageLayoutControl::new(Some(2));
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
}

/// A table whose second cell spans three rows, the middle one styled
/// `middle_row`, and holds a block that paints 2000px past the cell.
fn spanning_cell_with_overflow_document(
    middle_row: &str,
) -> (Document, raikiri_style::CascadeResult) {
    let mut document = Document::new();
    let html = document.append_element(Some(0), "html", Some("display:block"));
    let body = document.append_element(Some(html), "body", Some("display:block;margin:0"));
    let table = document.append_element(Some(body), "div", Some("display:table"));
    let group = document.append_element(Some(table), "div", Some("display:table-row-group"));
    for (index, row_css) in ["display:table-row", middle_row, "display:table-row"]
        .into_iter()
        .enumerate()
    {
        let row = document.append_element(Some(group), "div", Some(row_css));
        document.append_element(
            Some(row),
            "div",
            Some("display:table-cell;width:20px;height:20px;padding:0"),
        );
        if index == 0 {
            let span = document.append_element(
                Some(row),
                "div",
                Some("display:table-cell;width:20px;padding:0"),
            );
            document.set_element_attributes(span, vec![("rowspan".into(), "3".into())]);
            document.append_element(
                Some(span),
                "div",
                Some("display:block;height:2010px;margin-bottom:-2000px"),
            );
        }
    }
    document.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&document);
    let cascade = raikiri_style::cascade(&document, &rules).unwrap();
    (document, cascade)
}

#[test]
fn options_page_limit_ignores_overflow_a_collapsed_row_span_clips() {
    // The cell spanning a collapsed row clips what it holds to its border box,
    // so the block painting past it starts no page; without the collapsed row
    // the block paints onto three more pages.
    for (middle_row, fits) in [
        ("display:table-row;visibility:collapse", true),
        ("display:table-row", false),
    ] {
        let (mut document, cascade) = spanning_cell_with_overflow_document(middle_row);
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
        assert_eq!(result.is_ok(), fits, "{middle_row}");
    }
}
