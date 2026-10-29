use std::any::Any;

use raikiri_dom::Document;
use raikiri_js::runtime::{BoxGeometry, DocumentHost, FragmentParseError, HostError};
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
    // cov:ignore: exercised by raikiri-js-wasmtime-guest tests; tools/ is outside the workspace llvm-cov run.
    fn is_being_rendered(&mut self, node: usize) -> Result<bool, HostError> {
        match self.call(HostOperation::Rendered(node))? {
            HostValue::Flag(v) => Ok(v),
            _ => Err(HostError("rendered response kind".into())),
        }
    }
    // cov:ignore: exercised by raikiri-js-wasmtime-guest tests; tools/ is outside the workspace llvm-cov run.
    fn is_visible(&mut self, node: usize) -> Result<bool, HostError> {
        match self.call(HostOperation::Visible(node))? {
            HostValue::Flag(v) => Ok(v),
            _ => Err(HostError("visible response kind".into())),
        }
    }
    // cov:ignore: exercised by raikiri-js-wasmtime-guest tests; tools/ is outside the workspace llvm-cov run.
    fn is_fixed_to_viewport(&mut self, node: usize) -> Result<bool, HostError> {
        match self.call(HostOperation::FixedToViewport(node))? {
            HostValue::Flag(v) => Ok(v),
            _ => Err(HostError("fixed-to-viewport response kind".into())),
        }
    }
    // cov:ignore: exercised by raikiri-js-wasmtime-guest tests; tools/ is outside the workspace llvm-cov run.
    fn viewport_size(&mut self) -> Result<Option<(f64, f64)>, HostError> {
        match self.call(HostOperation::Viewport)? {
            HostValue::Size(v) => Ok(v),
            _ => Err(HostError("viewport response kind".into())),
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
    // cov:ignore: exercised by raikiri-js-wasmtime-guest tests; tools/ is outside the workspace llvm-cov run.
    fn load_image(&mut self, url: &str) -> Result<(), HostError> {
        match self.call(HostOperation::LoadImage(url.into()))? {
            HostValue::Unit => Ok(()),
            _ => Err(HostError("image response kind".into())),
        }
    }
    /// Pixels that fail to arrive or decode draw nothing, the same as a
    /// host that keeps none.
    // cov:ignore: exercised by raikiri-js-wasmtime-guest tests; tools/ is outside the workspace llvm-cov run.
    fn image_pixels(&mut self, url: &str, default_size: (f64, f64)) -> Option<ImagePixels> {
        match self
            .call(HostOperation::ImagePixels(url.into(), default_size))
            .ok()?
        {
            HostValue::Image(image) => image?.into_native().ok(),
            _ => None,
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
    // cov:ignore: verified by guest and real-AOT host tests; this tools workspace is outside the native llvm-cov run.
    fn parse_fragment_with_budget(
        &mut self,
        tag: &str,
        namespace: &str,
        markup: &str,
        remaining_nodes: usize,
    ) -> Result<Document, FragmentParseError> {
        match self
            .call(HostOperation::BudgetedFragment {
                tag: tag.into(),
                namespace: namespace.into(),
                markup: markup.into(),
                remaining_nodes: u64::try_from(remaining_nodes).unwrap_or(u64::MAX),
            })
            .map_err(FragmentParseError::Host)?
        {
            HostValue::FragmentNodes => Err(FragmentParseError::Nodes),
            HostValue::Document(snapshot) => {
                let arena_cap = remaining_nodes.min(self.max_nodes).saturating_add(4);
                if snapshot.nodes.len() > arena_cap {
                    return Err(FragmentParseError::Nodes);
                }
                Document::from_logical_snapshot(snapshot, arena_cap)
                    .map_err(|error| FragmentParseError::Host(HostError(error.to_string())))
            }
            _ => Err(FragmentParseError::Host(HostError(
                "fragment response kind".into(),
            ))),
        }
    }
    // cov:ignore: exercised by raikiri-js-wasmtime-guest tests; tools/ is outside the workspace llvm-cov run.
    fn as_any(&self) -> &dyn Any {
        self
    }
    // cov:ignore: exercised by raikiri-js-wasmtime-guest tests; tools/ is outside the workspace llvm-cov run.
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
    // cov:ignore: exercised by raikiri-js-wasmtime-guest tests; tools/ is outside the workspace llvm-cov run.
    fn into_any(self: Box<Self>) -> Box<dyn Any> {
        self
    }
}
