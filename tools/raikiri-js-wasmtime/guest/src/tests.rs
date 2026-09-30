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
            HostOperation::Computed(_, _) => Ok(HostValue::Text(Some("red".into()))),
            HostOperation::Geometry(_) => Ok(HostValue::Geometry(None)),
            HostOperation::Fragment { .. } => {
                Ok(HostValue::Document(Document::new().logical_snapshot()))
            }
        }),
    );
    host.flush().unwrap();
    assert_eq!(host.fetch_script("source").unwrap(), "source");
    assert_eq!(host.computed_value(0, "color").unwrap(), Some("red".into()));
    assert!(host.box_geometry(0).unwrap().is_none());
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
                HostOperation::Fragment { .. } => HostValue::Unit,
            })
        }),
    );
    assert!(wrong_kind.flush().is_err());
    assert!(wrong_kind.box_geometry(0).is_err());
    assert!(wrong_kind.computed_value(0, "color").is_err());
    assert!(wrong_kind.fetch_script("x").is_err());
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
                HostOperation::Fragment { .. } => {
                    HostValue::Document(raikiri_dom::snapshot::LogicalSnapshot {
                        root: 0,
                        nodes: Vec::new(),
                        quirks: 0,
                        stylesheets: Vec::new(),
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
