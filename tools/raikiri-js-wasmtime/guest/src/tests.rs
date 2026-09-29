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
