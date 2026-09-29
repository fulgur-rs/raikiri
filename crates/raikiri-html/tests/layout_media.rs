//! Contract tests for the media context used by document layout.

use raikiri_html::{
    DocumentLayout, LayoutConfig, LayoutOptions, LayoutStatus, MediaContext, MediaType, Page,
    PageDefaults, RenderError, RenderResources, layout, parse_html_with_resources,
};
use raikiri_style::ComputedLengthPercentageOrAuto;

const WIDTH_CSS: &str = "body{margin:0} p{margin:0;width:10px;height:10px} \
    @media print and (min-width:261px){p{width:30px}}";

fn completed(html: &str, media: MediaContext) -> DocumentLayout {
    let resources = RenderResources::new();
    let document = parse_html_with_resources(html.as_bytes(), &resources).unwrap();
    match layout(
        &document,
        PageDefaults::default(),
        LayoutConfig::builder().media_context(media).build(),
        LayoutOptions::new().resources(&resources),
    )
    .unwrap()
    {
        LayoutStatus::Completed(result) => result,
        _ => panic!("expected a completed layout"),
    }
}

fn assert_paragraph_width(page: Page<'_>, expected: f32) {
    let paragraph = page
        .fragments()
        .find(|fragment| page.dom().local_name(fragment.node()) == Some("p"))
        .expect("paragraph fragment");
    assert_eq!(paragraph.rect().width, expected);
    assert_eq!(
        page.computed(paragraph.node()).unwrap().width,
        ComputedLengthPercentageOrAuto::Px(expected)
    );
}

#[test]
fn screen_media_is_rejected() {
    let resources = RenderResources::new();
    let document = parse_html_with_resources(b"<p></p>".as_slice(), &resources).unwrap();
    assert!(matches!(
        layout(
            &document,
            PageDefaults::default(),
            LayoutConfig::builder()
                .media_context(MediaContext::screen())
                .build(),
            LayoutOptions::new().resources(&resources),
        ),
        Err(RenderError::Configuration(_))
    ));
    assert!(matches!(
        layout(
            &document,
            PageDefaults::default(),
            LayoutConfig::default(),
            LayoutOptions::new().resources(&resources),
        )
        .unwrap(),
        LayoutStatus::Completed(_)
    ));
}

#[test]
fn print_media_viewport_changes_computed_width() {
    let html = format!("<style>{WIDTH_CSS}</style><p></p>");
    for (width, expected) in [(260, 10.0), (261, 30.0)] {
        let result = completed(
            &html,
            MediaContext::with_viewport(MediaType::Print, width, 160),
        );
        assert_paragraph_width(result.page(0).unwrap(), expected);
    }
}

#[test]
fn rounded_viewport_matches_media_boundary() {
    let html = format!("<style>{WIDTH_CSS}</style><p></p>");
    for (width, rounded, expected) in [(260.49_f32, 260, 10.0), (260.5_f32, 261, 30.0)] {
        let viewport_width = width.round() as u32;
        assert_eq!(viewport_width, rounded);
        let result = completed(
            &html,
            MediaContext::with_viewport(MediaType::Print, viewport_width, 160),
        );
        assert_paragraph_width(result.page(0).unwrap(), expected);
    }
}

#[test]
fn media_applies_to_named_page_and_later_page() {
    let html = format!(
        "<style>{WIDTH_CSS} @page chapter {{size:300px 200px;margin:20px}} \
         p{{page:chapter}}</style><p></p><p style='break-before:page'></p>"
    );
    let result = completed(
        &html,
        MediaContext::with_viewport(MediaType::Print, 261, 160),
    );
    assert_eq!(result.page_count(), 2);
    for page in result.pages() {
        assert_eq!(page.name(), Some("chapter"));
        assert_paragraph_width(page, 30.0);
    }
}

#[test]
fn media_applies_to_page_geometry_and_schedule() {
    let html = "<style>body{margin:0} p{page:small;margin:0;height:10px} \
        @page small{size:280px 180px;margin:10px} \
        @page chapter{size:300px 200px;margin:20px} \
        @page chapter:left{size:320px 220px;margin:25px} \
        @media print and (min-width:261px){p{page:chapter}} \
        </style><p></p><p style='break-before:page'></p>";
    for (width, name, expected) in [
        (260, "small", [(280.0, 180.0, 10.0), (280.0, 180.0, 10.0)]),
        (261, "chapter", [(300.0, 200.0, 20.0), (320.0, 220.0, 25.0)]),
    ] {
        let result = completed(
            html,
            MediaContext::with_viewport(MediaType::Print, width, 160),
        );
        assert_eq!(result.page_count(), 2);
        for (page, (width, height, margin)) in result.pages().zip(expected) {
            let geometry = page.geometry();
            assert_eq!(page.name(), Some(name));
            assert_eq!(geometry.page_box.width, width);
            assert_eq!(geometry.page_box.height, height);
            assert_eq!(geometry.margins.top, margin);
            assert_eq!(geometry.content_box.x, margin);
            assert_eq!(geometry.content_box.y, margin);
        }
    }
}

#[test]
fn background_preload_uses_configured_media() {
    use raikiri_traits::{FetchOutcome, NetworkError, NetworkProvider, Request};
    use std::sync::Mutex;

    #[derive(Default)]
    struct RecordingNetwork(Mutex<Vec<String>>);

    impl NetworkProvider for RecordingNetwork {
        fn fetch_one_hop(&self, request: Request) -> Result<FetchOutcome, NetworkError> {
            self.0.lock().unwrap().push(request.url.to_string());
            Err(NetworkError::Aborted)
        }
    }

    let html = b"<style>@page{size:300px 200px;margin:20px} \
        @media print and (min-width:261px){ \
            p{width:10px;height:10px;background-image:url('https://example.test/page.png')} \
        }</style><p></p>";
    for (width, expected) in [(260, vec![]), (261, vec!["https://example.test/page.png"])] {
        let network = RecordingNetwork::default();
        let resources = RenderResources::new().network_provider(&network);
        let document = parse_html_with_resources(html.as_slice(), &resources).unwrap();
        assert!(matches!(
            layout(
                &document,
                PageDefaults::default(),
                LayoutConfig::builder()
                    .media_context(MediaContext::with_viewport(MediaType::Print, width, 160))
                    .build(),
                LayoutOptions::new().resources(&resources),
            )
            .unwrap(),
            LayoutStatus::Completed(_)
        ));
        assert_eq!(*network.0.lock().unwrap(), expected);
    }
}
