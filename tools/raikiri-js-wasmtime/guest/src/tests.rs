use super::*;
use raikiri_dom::Document;
use raikiri_js::runtime::DocumentHost;
use raikiri_js_wasmtime_protocol::*;
#[test]
fn rpc_flush_and_services() {
    let mut host = RpcDocumentHost::new(
        Document::new(),
        Some("file:///page".into()),
        100,
        Box::new(|op| match op {
            HostOperation::Flush(s) => {
                assert_eq!(s.nodes.len(), 1);
                Ok(HostValue::Unit)
            }
            HostOperation::Fetch(url) => Ok(HostValue::Text(Some(url))),
            HostOperation::LoadImage(url) if url == "ok.png" => Ok(HostValue::Unit),
            HostOperation::LoadImage(_) => Err("missing image".into()),
            HostOperation::Computed(_, _) => Ok(HostValue::Text(Some("red".into()))),
            HostOperation::Geometry(_) => Ok(HostValue::Geometry(None)),
            HostOperation::Rendered(node) => Ok(HostValue::Flag(node == 0)),
            HostOperation::Visible(node) => Ok(HostValue::Flag(node == 1)),
            HostOperation::Viewport => Ok(HostValue::Size(Some((800.0, 600.0)))),
            HostOperation::FixedToViewport(node) => Ok(HostValue::Flag(node == 1)),
            HostOperation::ImagePixels(url, size) => Ok(HostValue::Image(match url.as_str() {
                "ok.png" if size == (4.0, 2.0) => Some(ImageDto {
                    size: (4.0, 2.0),
                    width: 1,
                    height: 1,
                    rgba: vec![1, 2, 3, 4],
                }),
                "corrupt.png" => Some(ImageDto {
                    size: (2.0, 2.0),
                    width: 2,
                    height: 2,
                    rgba: vec![0; 4],
                }),
                _ => None,
            })),
            HostOperation::Fragment { .. } | HostOperation::BudgetedFragment { .. } => {
                Ok(HostValue::Document(Document::new().logical_snapshot()))
            }
        }),
    );
    host.flush().unwrap();
    assert_eq!(host.fetch_script("source").unwrap(), "source");
    assert!(host.load_image("ok.png").is_ok());
    assert!(host.load_image("missing.png").is_err());
    assert_eq!(host.computed_value(0, "color").unwrap(), Some("red".into()));
    assert!(host.box_geometry(0).unwrap().is_none());
    assert!(host.is_being_rendered(0).unwrap());
    assert!(!host.is_being_rendered(1).unwrap());
    assert!(!host.is_visible(0).unwrap());
    assert!(host.is_visible(1).unwrap());
    assert_eq!(host.viewport_size().unwrap(), Some((800.0, 600.0)));
    assert!(!host.is_fixed_to_viewport(0).unwrap());
    assert!(host.is_fixed_to_viewport(1).unwrap());
    let pixels = host.image_pixels("ok.png", (4.0, 2.0)).unwrap();
    assert_eq!(pixels.size, (4.0, 2.0));
    assert_eq!(pixels.image.rgba, [1, 2, 3, 4]);
    assert!(host.image_pixels("corrupt.png", (4.0, 2.0)).is_none());
    assert!(host.image_pixels("missing.png", (4.0, 2.0)).is_none());
    assert_eq!(host.parse_fragment("div", "", "x").unwrap().node_count(), 1);
    assert_eq!(host.document_url(), Some("file:///page".into()));
    // The downcast hooks recover the concrete host from its boxed form.
    let boxed: Box<dyn DocumentHost> = Box::new(host);
    assert!(boxed.downcast_ref::<RpcDocumentHost>().is_some());
    let mut boxed = boxed;
    assert!(boxed.downcast_mut::<RpcDocumentHost>().is_some());
    assert!(boxed.downcast::<RpcDocumentHost>().is_ok());
}

#[test]
fn hostile_wrong_response_kinds_are_rejected() {
    // Each service rejects a response of the wrong kind, and transport
    // failures surface as host errors rather than panics.
    let mut wrong_kind = RpcDocumentHost::new(
        Document::new(),
        None,
        100,
        Box::new(|op| {
            Ok(match op {
                HostOperation::Flush(_) => HostValue::Geometry(None),
                HostOperation::Geometry(_) => HostValue::Unit,
                HostOperation::Computed(_, _) => HostValue::Unit,
                HostOperation::Fetch(_) => HostValue::Unit,
                HostOperation::LoadImage(_) => HostValue::Text(None),
                HostOperation::Rendered(_) => HostValue::Unit,
                HostOperation::Visible(_) => HostValue::Unit,
                HostOperation::Viewport => HostValue::Unit,
                HostOperation::FixedToViewport(_) => HostValue::Unit,
                HostOperation::ImagePixels(..) => HostValue::Unit,
                HostOperation::Fragment { .. } | HostOperation::BudgetedFragment { .. } => {
                    HostValue::Unit
                }
            })
        }),
    );
    assert!(wrong_kind.flush().is_err());
    assert!(wrong_kind.box_geometry(0).is_err());
    assert!(wrong_kind.computed_value(0, "color").is_err());
    assert!(wrong_kind.fetch_script("x").is_err());
    assert!(wrong_kind.load_image("x").is_err());
    assert!(wrong_kind.is_being_rendered(0).is_err());
    assert!(wrong_kind.is_visible(0).is_err());
    assert!(wrong_kind.viewport_size().is_err());
    assert!(wrong_kind.is_fixed_to_viewport(0).is_err());
    assert!(wrong_kind.image_pixels("x", (1.0, 1.0)).is_none());
    assert!(wrong_kind.parse_fragment("div", "", "x").is_err());

    let mut failing = RpcDocumentHost::new(
        Document::new(),
        None,
        100,
        Box::new(|_| Err("transport down".to_owned())),
    );
    assert!(failing.flush().is_err());
    assert!(failing.box_geometry(0).is_err());
    assert!(failing.computed_value(0, "color").is_err());
    assert!(failing.fetch_script("x").is_err());
    assert!(failing.load_image("x").is_err());
    assert!(failing.image_pixels("x", (1.0, 1.0)).is_none());
    assert!(failing.parse_fragment("div", "", "x").is_err());
}

#[test]
fn hostile_geometry_and_fragment_validation_are_rejected() {
    // Invalid geometry DTO and invalid snapshot in a fragment response
    // must be rejected without panicking the guest.
    let mut bad_geometry = RpcDocumentHost::new(
        Document::new(),
        None,
        100,
        Box::new(|op| {
            Ok(match op {
                HostOperation::Geometry(_) => {
                    HostValue::Geometry(Some(raikiri_js_wasmtime_protocol::GeometryDto {
                        border: [f64::NAN; 6],
                        padding: [0.0; 6],
                        scroll: [0.0; 2],
                        position: 0,
                        is_inline: false,
                    }))
                }
                _ => HostValue::Unit,
            })
        }),
    );
    assert!(bad_geometry.box_geometry(0).is_err());

    let mut bad_fragment = RpcDocumentHost::new(
        Document::new(),
        None,
        100,
        Box::new(|op| {
            Ok(match op {
                HostOperation::Fragment { .. } | HostOperation::BudgetedFragment { .. } => {
                    HostValue::Document(raikiri_dom::snapshot::LogicalSnapshot {
                        root: 0,
                        nodes: Vec::new(),
                        quirks: 0,
                        stylesheets: Vec::new(),
                        base_url: None,
                        selected_image_urls: Vec::new(),
                        focused: None,
                        focus_hidden: false,
                    })
                }
                _ => HostValue::Unit,
            })
        }),
    );
    assert!(bad_fragment.parse_fragment("div", "", "x").is_err());

    // Fetch with None text is a missing script, not an empty one.
    let mut missing = RpcDocumentHost::new(
        Document::new(),
        None,
        100,
        Box::new(|op| {
            Ok(match op {
                HostOperation::Fetch(_) => HostValue::Text(None),
                _ => HostValue::Unit,
            })
        }),
    );
    assert!(missing.fetch_script("x").is_err());
}

#[test]
fn fragment_budget_crosses_rpc_and_nodes_error_stays_typed() {
    let mut host = RpcDocumentHost::new(
        Document::new(),
        None,
        100,
        Box::new(|operation| match operation {
            HostOperation::BudgetedFragment {
                remaining_nodes,
                tag,
                ..
            } => {
                assert_eq!(remaining_nodes, 2);
                assert_eq!(tag, "div");
                Ok(HostValue::FragmentNodes)
            }
            _ => panic!("expected budgeted fragment"),
        }),
    );
    assert!(matches!(
        host.parse_fragment_with_budget("div", "", "huge", 2),
        Err(raikiri_js::runtime::FragmentParseError::Nodes)
    ));
}

#[test]
fn oversized_budgeted_fragment_response_is_a_node_abort() {
    let mut fragment = Document::new();
    for _ in 0..10 {
        fragment.create_detached_element("b").unwrap();
    }
    let snapshot = fragment.logical_snapshot();
    let mut host = RpcDocumentHost::new(
        Document::new(),
        None,
        100,
        Box::new(move |_| Ok(HostValue::Document(snapshot.clone()))),
    );
    assert!(matches!(
        host.parse_fragment_with_budget("div", "", "", 1),
        Err(raikiri_js::runtime::FragmentParseError::Nodes)
    ));
}
