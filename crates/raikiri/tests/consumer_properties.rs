//! Integration coverage for the neutral consumer-property observer.

use raikiri::{
    ConsumerPropertyEvent, ConsumerPropertyObserver, ConsumerPropertyRegistration,
    ConsumerPropertyValue, PageBox, PageDefaults, PageFragment, RenderSink, RenderStatus,
    RenderSummary, ReplacedResolver, ResolverError, ResolverRequest, StreamingConfig, parse_html,
    render_streaming_with_consumer_properties,
};

struct NoopResolver;

impl ReplacedResolver for NoopResolver {
    fn resolve(
        &self,
        _request: ResolverRequest<'_>,
    ) -> Result<raikiri::ResolvedIntrinsic, ResolverError> {
        unreachable!("test document contains no replaced element")
    }
}

#[derive(Default)]
struct Sink {
    pages: Vec<PageFragment>,
    summary: Option<RenderSummary>,
}

impl RenderSink for Sink {
    fn accept_page(&mut self, page: PageFragment) -> Result<(), std::io::Error> {
        self.pages.push(page);
        Ok(())
    }

    fn finish_render(&mut self, summary: RenderSummary) -> Result<(), std::io::Error> {
        self.summary = Some(summary);
        Ok(())
    }
}

#[derive(Default)]
struct Observer {
    events: Vec<ConsumerPropertyEvent>,
    fail: bool,
}

impl ConsumerPropertyObserver for Observer {
    fn observe_event(&mut self, event: ConsumerPropertyEvent) -> Result<(), std::io::Error> {
        if self.fail {
            return Err(std::io::Error::other("consumer observer failure"));
        }
        self.events.push(event);
        Ok(())
    }
}

fn defaults() -> PageDefaults {
    let mut defaults = PageDefaults::default();
    let mut page_box = PageBox::new();
    page_box.width = 300.0;
    page_box.height = 300.0;
    defaults.page_box = page_box;
    defaults
}

fn options() -> raikiri::ParseOptions<'static> {
    raikiri::ParseOptions {
        extra_stylesheets: &[],
        network: None,
        base_url: None,
    }
}

#[test]
fn resolved_consumer_properties_are_neutral_and_document_ordered() {
    let doc = parse_html(
        &br#"<html><head><style>
            h1 { bookmark-level: 1; bookmark-label: "Chapter " attr(data-label); }
            .second { bookmark-level: 2; bookmark-label: "Second"; }
        </style></head><body>
            <h1 data-label="One">First</h1>
            <div style="bookmark-level: 3; bookmark-label: content(text)">
                <h1 class="second">Second</h1>
            </div>
        </body></html>"#[..],
        &options(),
    )
    .expect("parse");
    let registrations = [
        ConsumerPropertyRegistration::integer("bookmark-level"),
        ConsumerPropertyRegistration::text("bookmark-label"),
    ];
    let mut sink = Sink::default();
    let mut observer = Observer::default();
    let status = render_streaming_with_consumer_properties(
        &doc,
        defaults(),
        &NoopResolver,
        StreamingConfig::default(),
        &registrations,
        &mut sink,
        &mut observer,
    )
    .expect("render");

    assert!(matches!(status, RenderStatus::Completed(_)));
    assert!(sink.summary.is_some());
    assert_eq!(observer.events.len(), 6);
    assert_eq!(
        observer
            .events
            .iter()
            .map(|event| (event.node_id, event.property_name.as_str()))
            .collect::<Vec<_>>(),
        vec![
            (observer.events[0].node_id, "bookmark-level"),
            (observer.events[0].node_id, "bookmark-label"),
            (observer.events[2].node_id, "bookmark-level"),
            (observer.events[2].node_id, "bookmark-label"),
            (observer.events[4].node_id, "bookmark-level"),
            (observer.events[4].node_id, "bookmark-label"),
        ]
    );
    assert!(
        observer
            .events
            .windows(2)
            .all(|events| events[0].source_order <= events[1].source_order)
    );
    assert!(
        observer
            .events
            .iter()
            .all(|event| event.parent_id.is_some())
    );
    assert!(matches!(
        observer.events[0].value,
        ConsumerPropertyValue::Integer(1)
    ));
    assert_eq!(
        observer.events[1].value,
        ConsumerPropertyValue::Text("Chapter One".to_owned())
    );
    assert_eq!(
        observer.events[3].value,
        ConsumerPropertyValue::Text("Second".to_owned())
    );
    assert!(
        sink.pages
            .iter()
            .flat_map(|page| page.items.iter())
            .any(|item| {
                observer
                    .events
                    .iter()
                    .any(|event| event.node_id == item.node_id)
            })
    );
}

#[test]
fn explicit_none_is_a_neutral_value_and_observer_errors_are_structured() {
    let doc = parse_html(
        &br#"<html><body><h1 style="bookmark-level: none">Hidden</h1></body></html>"#[..],
        &options(),
    )
    .expect("parse");
    let registrations = [ConsumerPropertyRegistration::integer_or_none(
        "bookmark-level",
    )];
    let mut sink = Sink::default();
    let mut observer = Observer {
        fail: true,
        ..Observer::default()
    };
    let error = render_streaming_with_consumer_properties(
        &doc,
        defaults(),
        &NoopResolver,
        StreamingConfig::default(),
        &registrations,
        &mut sink,
        &mut observer,
    )
    .expect_err("observer failure must stop the render");
    assert!(matches!(error, raikiri::RenderError::Sink(_)));
    assert!(sink.pages.is_empty());
    assert!(sink.summary.is_none());

    let mut sink = Sink::default();
    let mut observer = Observer::default();
    render_streaming_with_consumer_properties(
        &doc,
        defaults(),
        &NoopResolver,
        StreamingConfig::default(),
        &registrations,
        &mut sink,
        &mut observer,
    )
    .expect("none render");
    assert_eq!(observer.events.len(), 1);
    assert_eq!(observer.events[0].value, ConsumerPropertyValue::None);
}

#[test]
fn registrations_use_var_resolution_media_and_explicit_inheritance() {
    let extra = [r#"section { bookmark-level: 1; }"#];
    let doc = parse_html(
        &br#"<html><head><style>
            :root { --level: 7; --label: "Root"; --invalid-level: nope; }
            @media all { section { bookmark-level: var(--level); bookmark-label: var(--label); } }
            div { bookmark-level: 9; }
            .via-var { bookmark-level: var(--invalid-level); }
        </style></head><body>
            <section><div>local</div><p style="bookmark-level: invalid">invalid</p>
                <p class="via-var">invalid var</p>
            </section>
        </body></html>"#[..],
        &raikiri::ParseOptions {
            extra_stylesheets: &extra,
            network: None,
            base_url: None,
        },
    )
    .expect("parse");
    let registrations = [
        ConsumerPropertyRegistration::integer("bookmark-level"),
        ConsumerPropertyRegistration::text("bookmark-label").inherited(),
    ];
    let mut sink = Sink::default();
    let mut observer = Observer::default();
    render_streaming_with_consumer_properties(
        &doc,
        defaults(),
        &NoopResolver,
        StreamingConfig::default(),
        &registrations,
        &mut sink,
        &mut observer,
    )
    .expect("render");

    let levels = observer
        .events
        .iter()
        .filter(|event| event.property_name == "bookmark-level")
        .map(|event| event.value.clone())
        .collect::<Vec<_>>();
    assert_eq!(
        levels,
        vec![
            ConsumerPropertyValue::Integer(7),
            ConsumerPropertyValue::Integer(9)
        ]
    );
    let labels = observer
        .events
        .iter()
        .filter(|event| event.property_name == "bookmark-label")
        .map(|event| event.value.clone())
        .collect::<Vec<_>>();
    assert_eq!(
        labels,
        vec![
            ConsumerPropertyValue::Text("Root".to_owned()),
            ConsumerPropertyValue::Text("Root".to_owned()),
            ConsumerPropertyValue::Text("Root".to_owned()),
            ConsumerPropertyValue::Text("Root".to_owned()),
        ]
    );
}

#[test]
fn content_text_ignores_non_rendered_subtrees() {
    let doc = parse_html(
        &br#"<html><head><style>
            body { bookmark-label: content(text); }
            h1 { bookmark-label: attr(DATA-TITLE); }
        </style><title>Hidden title</title></head>
        <body><script>hidden script</script><style>hidden style</style><h1 data-title="Case label">Visible heading</h1></body></html>"#[..],
        &options(),
    )
    .expect("parse");
    let registrations = [ConsumerPropertyRegistration::text("bookmark-label")];
    let mut sink = Sink::default();
    let mut observer = Observer::default();
    render_streaming_with_consumer_properties(
        &doc,
        defaults(),
        &NoopResolver,
        StreamingConfig::default(),
        &registrations,
        &mut sink,
        &mut observer,
    )
    .expect("render");

    assert_eq!(observer.events.len(), 2);
    assert_eq!(
        observer.events[0].value,
        ConsumerPropertyValue::Text("Visible heading".to_owned())
    );
    assert_eq!(
        observer.events[1].value,
        ConsumerPropertyValue::Text("Case label".to_owned())
    );
}

#[test]
fn consumer_property_edge_values_and_closure_observer() {
    let doc = parse_html(
        &br#"<html><head><style>
            p { bookmark-level: 4; bookmark-label: attr(missing, "Fallback"); }
        </style></head><body><p>Edge</p></body></html>"#[..],
        &options(),
    )
    .expect("parse");
    let registrations = [
        ConsumerPropertyRegistration::integer_or_none("bookmark-level").non_inherited(),
        ConsumerPropertyRegistration::text("bookmark-label"),
    ];
    assert!(!registrations[0].inherits());
    let mut sink = Sink::default();
    let mut events = Vec::new();
    let mut observer = |event: ConsumerPropertyEvent| {
        events.push(event);
        Ok::<_, std::io::Error>(())
    };
    render_streaming_with_consumer_properties(
        &doc,
        defaults(),
        &NoopResolver,
        StreamingConfig::default(),
        &registrations,
        &mut sink,
        &mut observer,
    )
    .expect("render");

    assert_eq!(
        events
            .iter()
            .map(|event| (&event.property_name, &event.value))
            .collect::<Vec<_>>(),
        vec![
            (
                &"bookmark-level".to_owned(),
                &ConsumerPropertyValue::Integer(4),
            ),
            (
                &"bookmark-label".to_owned(),
                &ConsumerPropertyValue::Text("Fallback".to_owned()),
            ),
        ]
    );
}

#[test]
fn empty_consumer_registration_keeps_rendering_compatible() {
    let doc = parse_html(
        &br#"<html><body><p>No consumer properties</p></body></html>"#[..],
        &options(),
    )
    .expect("parse");
    let registrations: [ConsumerPropertyRegistration; 0] = [];
    let mut sink = Sink::default();
    let mut observer = Observer::default();
    let status = render_streaming_with_consumer_properties(
        &doc,
        defaults(),
        &NoopResolver,
        StreamingConfig::default(),
        &registrations,
        &mut sink,
        &mut observer,
    )
    .expect("render");

    assert!(matches!(status, RenderStatus::Completed(_)));
    assert!(observer.events.is_empty());
    assert!(sink.summary.is_some());
}
