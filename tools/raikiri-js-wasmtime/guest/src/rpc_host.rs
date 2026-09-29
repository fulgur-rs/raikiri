use std::any::Any;

use raikiri_dom::Document;
use raikiri_js::runtime::{BoxGeometry, DocumentHost, HostError};
use raikiri_js_wasmtime_protocol::*;
pub type Transport = Box<dyn FnMut(HostOperation) -> Result<HostValue, String>>;
pub struct RpcDocumentHost {
    document: Document,
    url: Option<String>,
    max_nodes: usize,
    transport: Transport,
}
impl RpcDocumentHost {
    pub fn new(
        document: Document,
        url: Option<String>,
        max_nodes: usize,
        transport: Transport,
    ) -> Self {
        Self {
            document,
            url,
            max_nodes,
            transport,
        }
    }
    fn call(&mut self, op: HostOperation) -> Result<HostValue, HostError> {
        (self.transport)(op).map_err(HostError)
    }
}
impl DocumentHost for RpcDocumentHost {
    fn document(&self) -> &Document {
        &self.document
    }
    fn document_mut(&mut self) -> &mut Document {
        &mut self.document
    }
    fn document_url(&self) -> Option<String> {
        self.url.clone()
    }
    fn flush(&mut self) -> Result<(), HostError> {
        match self.call(HostOperation::Flush(self.document.logical_snapshot()))? {
            HostValue::Unit => Ok(()),
            _ => Err(HostError("flush response kind".into())),
        }
    }
    fn box_geometry(&mut self, node: usize) -> Result<Option<BoxGeometry>, HostError> {
        match self.call(HostOperation::Geometry(node))? {
            HostValue::Geometry(v) => v
                .map(GeometryDto::into_native)
                .transpose()
                .map_err(HostError),
            _ => Err(HostError("geometry response kind".into())),
        }
    }
    fn computed_value(&mut self, node: usize, property: &str) -> Result<Option<String>, HostError> {
        match self.call(HostOperation::Computed(node, property.into()))? {
            HostValue::Text(v) => Ok(v),
            _ => Err(HostError("computed response kind".into())),
        }
    }
    fn fetch_script(&mut self, url: &str) -> Result<String, HostError> {
        match self.call(HostOperation::Fetch(url.into()))? {
            HostValue::Text(Some(v)) => Ok(v),
            _ => Err(HostError("fetch response kind".into())),
        }
    }
    fn parse_fragment(
        &mut self,
        tag: &str,
        namespace: &str,
        markup: &str,
    ) -> Result<Document, HostError> {
        match self.call(HostOperation::Fragment {
            tag: tag.into(),
            namespace: namespace.into(),
            markup: markup.into(),
        })? {
            HostValue::Document(s) => Document::from_logical_snapshot(s, self.max_nodes)
                .map_err(|e| HostError(e.to_string())),
            _ => Err(HostError("fragment response kind".into())),
        }
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
    fn into_any(self: Box<Self>) -> Box<dyn Any> {
        self
    }
}
