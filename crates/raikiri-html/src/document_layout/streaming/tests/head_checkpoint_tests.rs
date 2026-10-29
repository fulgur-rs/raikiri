use super::*;

#[test]
fn long_heads_do_not_trace_or_snapshot_before_a_body_candidate() {
    for xhtml in [false, true] {
        for n in [128, 256, 512] {
            let mut head = String::from("<!DOCTYPE html><html><head>");
            for i in 0..n {
                head.push_str(&format!(
                    "<meta name='m{i}' content='{}'/>",
                    "x".repeat(950)
                ));
            }
            head.push_str("</head>");
            let resources = RenderResources::new();
            let mut record = Record::default();
            let mut stream = if xhtml {
                StreamingLayout::new_xhtml(
                    &resources,
                    PageDefaults::default(),
                    LayoutConfig::default(),
                    &mut record,
                )
            } else {
                StreamingLayout::new(
                    &resources,
                    PageDefaults::default(),
                    LayoutConfig::default(),
                    &mut record,
                )
            };
            for chunk in head.as_bytes().chunks(4096) {
                stream.feed(chunk).expect("head feed");
            }
            assert_eq!(stream.input.sink().snapshot_calls.get(), 0);
            assert_eq!(stream.input.sink().traced_handle_calls.get(), 0);
            drop(stream);
            assert!(record.pages.is_empty());
        }
    }
}

fn head_stream<'r, 'a>(
    xhtml: bool,
    resources: &'r RenderResources<'a>,
    record: &'r mut Record,
) -> StreamingLayout<'r, 'a, &'r mut Record> {
    if xhtml {
        StreamingLayout::new_xhtml(
            resources,
            PageDefaults::default(),
            LayoutConfig::default(),
            record,
        )
    } else {
        StreamingLayout::new(
            resources,
            PageDefaults::default(),
            LayoutConfig::default(),
            record,
        )
    }
}

#[test]
fn head_text_that_mentions_body_does_not_trigger_a_snapshot_and_finishes_like_batch() {
    for xhtml in [false, true] {
        for tag in ["comment", "style", "script", "title"] {
            let contents = match tag {
                "style" => format!("/* <body><frameset> {} */", " ".repeat(140_000)),
                "script" => format!("var x = '<body><frameset>{}';", " ".repeat(140_000)),
                _ => format!("<body><frameset>{}", " ".repeat(140_000)),
            };
            let element = if tag == "comment" {
                format!("<!--{contents}-->")
            } else if xhtml {
                format!("<{tag}><![CDATA[{contents}]]></{tag}>")
            } else {
                format!("<{tag}>{contents}</{tag}>")
            };
            let head = format!(
                "<!DOCTYPE html><html xmlns='http://www.w3.org/1999/xhtml'><head>{element}</head>"
            );
            let resources = RenderResources::new();
            let mut record = Record::default();
            let mut stream = head_stream(xhtml, &resources, &mut record);
            for chunk in head.as_bytes().chunks(4096) {
                stream.feed(chunk).expect("head text");
            }
            assert_eq!(stream.input.sink().snapshot_calls.get(), 0);
            assert_eq!(stream.input.sink().traced_handle_calls.get(), 0);
            let tail = "<body><div>visible</div></body></html>";
            stream.feed(tail.as_bytes()).expect("body");
            let summary = completed(stream.finish().expect("finish"));
            let full = format!("{head}{tail}");
            let parsed = if xhtml {
                crate::parse_xhtml_with_resources(full.as_bytes(), &resources).expect("batch XML")
            } else {
                crate::parse_html_with_resources(full.as_bytes(), &resources).expect("batch HTML")
            };
            let LayoutStatus::Completed(batch) = layout(
                &parsed,
                PageDefaults::default(),
                LayoutConfig::default(),
                LayoutOptions::new().resources(&resources),
            )
            .expect("batch layout") else {
                panic!("batch aborted")
            };
            let pages: PageLog = batch
                .pages()
                .map(|p| (p.index(), p.fragments().map(|f| f.node()).collect()))
                .collect();
            assert_eq!(record.pages, pages);
            assert_eq!(summary.page_count as usize, record.pages.len());
        }
    }
}

#[test]
fn long_head_then_body_preserves_early_delivery_and_final_pages() {
    let head = format!(
        "<!DOCTYPE html><html><head><style>h2 {{ break-after: avoid }} .keep {{ break-inside: avoid }}</style><!--{}--></head>",
        "x".repeat(140_000)
    );
    let original = long_document();
    let (_, content) = original.split_once("<body>").expect("body fixture");
    let body = format!("<body>{content}</html>");
    let full = format!("{head}{body}");
    let expected = batch_pages(&full);
    assert!(expected.len() > 1);
    let resources = RenderResources::new();
    let mut record = Record::default();
    let mut stream = head_stream(false, &resources, &mut record).checkpoint_bytes(4096);
    stream.feed(head.as_bytes()).expect("head");
    assert_eq!(stream.input.sink().snapshot_calls.get(), 0);
    for chunk in body.as_bytes().chunks(4096) {
        stream.feed(chunk).expect("body");
    }
    assert!(!stream.sink.pages.is_empty());
    assert!(stream.input.sink().snapshot_calls.get() > 0);
    completed(stream.finish().expect("finish"));
    assert_eq!(record.pages, expected);
}

#[test]
fn finishing_a_head_only_document_still_creates_an_implicit_body() {
    let html = "<!DOCTYPE html><html><head><title>head only</title></head>";
    let expected = batch_pages(html);
    let resources = RenderResources::new();
    let mut record = Record::default();
    let mut stream = head_stream(false, &resources, &mut record).checkpoint_bytes(1);
    stream.feed(html.as_bytes()).expect("head");
    assert_eq!(stream.input.sink().snapshot_calls.get(), 0);
    completed(stream.finish().expect("implicit body at EOF"));
    assert_eq!(record.pages, expected);
}

#[test]
fn pending_head_text_reserves_its_node_budget_before_snapshot() {
    for xhtml in [false, true] {
        for tag in ["style", "script"] {
            let limits = RenderLimits::builder().max_dom_nodes(Some(4)).build();
            let resources = RenderResources::new().render_limits(limits);
            let mut record = Record::default();
            let mut stream = head_stream(xhtml, &resources, &mut record).checkpoint_bytes(32);
            let prefix = format!("<!DOCTYPE html><html><head><{tag}>");
            stream
                .feed(prefix.as_bytes())
                .expect("four nodes; doctypes are not stored");
            let error = stream
                .feed(" ".repeat(140_000).as_bytes())
                .expect_err("text exceeds budget");
            assert!(matches!(
                error,
                RenderError::LimitExceeded {
                    kind: LimitKind::DomNodes,
                    limit: 4,
                    actual: 5
                }
            ));
            assert_eq!(stream.input.sink().snapshot_calls.get(), 0);
            assert!(matches!(
                stream.finish(),
                Err(RenderError::Configuration(_))
            ));
        }
    }
}

#[test]
fn cancellation_before_body_is_reported_at_finish_without_snapshot() {
    let controller = AbortController::new();
    let config = LayoutConfig::builder()
        .signal(Some(controller.signal.clone()))
        .build();
    let resources = RenderResources::new();
    let mut record = Record::default();
    let mut stream = StreamingLayout::new(&resources, PageDefaults::default(), config, &mut record);
    stream
        .feed(format!("<!DOCTYPE html><html><head><!--{}-->", "x".repeat(140_000)).as_bytes())
        .expect("head");
    assert_eq!(stream.input.sink().snapshot_calls.get(), 0);
    controller.abort();
    assert!(matches!(
        stream.finish().expect("abort"),
        StreamStatus::Aborted
    ));
    assert!(record.pages.is_empty());
    assert!(!record.finished);
}

#[test]
fn explicit_implicit_frameset_and_xml_candidates_reach_the_snapshot_path() {
    for (xhtml, input, supported) in [
        (
            false,
            "<!doctype html><html><head></head><body><p>x</p>",
            true,
        ),
        (
            false,
            "<!doctype html><html><head></head><p>implicit body</p>",
            true,
        ),
        (
            false,
            "<!doctype html><html><head></head><frameset><frame>",
            true,
        ),
        (
            true,
            "<html xmlns='http://www.w3.org/1999/xhtml'><head/><body><p>x</p>",
            true,
        ),
        (
            true,
            "<html xmlns='http://www.w3.org/1999/xhtml'><head/><BODY><p>x</p>",
            false,
        ),
        (
            true,
            "<html><head/><f:frameset xmlns:f='urn:foreign'><f:frame/>",
            false,
        ),
    ] {
        let resources = RenderResources::new();
        let mut record = Record::default();
        let mut stream = head_stream(xhtml, &resources, &mut record).checkpoint_bytes(1);
        let result = stream.feed(input.as_bytes());
        if supported {
            result.expect("supported body candidate");
        } else {
            assert!(
                matches!(result, Err(RenderError::Layout(_))),
                "{input}: {result:?}"
            );
        }
        assert!(stream.input.sink().has_created_body_candidate());
        assert!(stream.input.sink().snapshot_calls.get() > 0);
        assert!(stream.input.sink().traced_handle_calls.get() > 0);
    }
}

#[test]
fn input_and_xml_depth_limits_are_checked_without_a_body_candidate() {
    let limits = RenderLimits::builder().max_input_bytes(Some(80)).build();
    let resources = RenderResources::new().render_limits(limits);
    let mut record = Record::default();
    let mut stream = head_stream(false, &resources, &mut record).checkpoint_bytes(8);
    stream.feed(b"<!doctype html><html><head>").expect("head");
    let error = stream
        .feed(" ".repeat(100).as_bytes())
        .expect_err("input cap");
    assert!(matches!(
        error,
        RenderError::LimitExceeded {
            kind: LimitKind::InputBytes,
            ..
        }
    ));
    assert_eq!(stream.input.sink().snapshot_calls.get(), 0);
    drop(stream);
    let resources = RenderResources::new();
    let mut stream = head_stream(true, &resources, &mut record).checkpoint_bytes(8);
    let error = stream
        .feed(format!("<html><head>{}", "<x>".repeat(255)).as_bytes())
        .expect_err("depth cap");
    assert!(matches!(
        error,
        RenderError::LimitExceeded {
            kind: LimitKind::DomDepth,
            limit: 256,
            actual: 257
        }
    ));
    assert_eq!(stream.input.sink().snapshot_calls.get(), 0);
}

#[test]
fn a_detached_candidate_keeps_the_reachable_body_guard() {
    let resources = RenderResources::new();
    let mut record = Record::default();
    let mut stream = head_stream(false, &resources, &mut record).checkpoint_bytes(16);
    html5ever::interface::TreeSink::create_element(
        stream.input.sink(),
        html5ever::QualName::new(None, markup5ever::ns!(html), "body".into()),
        vec![],
        html5ever::interface::ElementFlags::default(),
    );
    stream
        .feed(b"<!doctype html><html><head><title>still head</title>")
        .expect("detached body must not reach layout");
    assert!(stream.input.sink().snapshot_calls.get() > 0);
    assert!(stream.sink.pages.is_empty());
}
