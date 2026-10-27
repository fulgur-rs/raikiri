use raikiri_dom::Document;
use raikiri_js::runtime::DocumentHost;
use raikiri_js_wasmtime_protocol::*;
#[derive(Debug, Default, Clone)]
pub struct BridgeMetrics {
    pub calls: u64,
    pub request_bytes: u64,
    pub response_bytes: u64,
}
pub struct Bridge {
    pub host: Box<dyn DocumentHost>,
    pending: Option<Vec<u8>>,
    pub(crate) max_nodes: usize,
    pub metrics: BridgeMetrics,
}
impl Bridge {
    pub fn new(host: Box<dyn DocumentHost>, max_nodes: usize) -> Self {
        Self {
            host,
            pending: None,
            max_nodes,
            metrics: Default::default(),
        }
    }
    pub fn request(&mut self, bytes: &[u8]) -> Result<usize, String> {
        if self.pending.is_some() {
            return Err("unconsumed response".into());
        }
        let r: Request = decode(bytes)?;
        r.check_version()?;
        self.metrics.calls += 1;
        self.metrics.request_bytes += bytes.len() as u64;
        let result = self.execute(r.operation);
        let response = encode(&Response {
            version: VERSION,
            result,
        })?;
        let n = response.len();
        self.metrics.response_bytes += n as u64;
        self.pending = Some(response);
        Ok(n)
    }
    fn execute(&mut self, operation: HostOperation) -> Result<HostValue, String> {
        match operation {
            HostOperation::Flush(s) => {
                let doc = Document::from_logical_snapshot(s, self.max_nodes)
                    .map_err(|e| e.to_string())?;
                *self.host.document_mut() = doc;
                self.host.flush().map_err(|e| e.to_string())?;
                Ok(HostValue::Unit)
            }
            HostOperation::Geometry(n) => {
                if n >= self.host.document().node_count() {
                    return Err("geometry node bounds".into());
                }
                self.host
                    .box_geometry(n)
                    .map(|v| HostValue::Geometry(v.map(Into::into)))
                    .map_err(|e| e.to_string())
            }
            HostOperation::Computed(n, p) => {
                if n >= self.host.document().node_count() {
                    return Err("computed node bounds".into());
                }
                self.host
                    .computed_value(n, &p)
                    .map(HostValue::Text)
                    .map_err(|e| e.to_string())
            }
            HostOperation::Fragment {
                tag,
                namespace,
                markup,
            } => self
                .host
                .parse_fragment(&tag, &namespace, &markup)
                .map(|d| HostValue::Document(d.logical_snapshot()))
                .map_err(|e| e.to_string()),
            HostOperation::BudgetedFragment {
                tag,
                namespace,
                markup,
                remaining_nodes,
            } => {
                let remaining = usize::try_from(remaining_nodes)
                    .unwrap_or(usize::MAX)
                    .min(self.max_nodes);
                match self
                    .host
                    .parse_fragment_with_budget(&tag, &namespace, &markup, remaining)
                {
                    Ok(document) if document.node_count() > remaining.saturating_add(4) => {
                        Ok(HostValue::FragmentNodes)
                    }
                    Ok(document) => Ok(HostValue::Document(document.logical_snapshot())),
                    Err(raikiri_js::runtime::FragmentParseError::Nodes) => {
                        Ok(HostValue::FragmentNodes)
                    }
                    Err(raikiri_js::runtime::FragmentParseError::Host(error)) => {
                        Err(error.to_string())
                    }
                }
            }
            HostOperation::Fetch(url) => self
                .host
                .fetch_script(&url)
                .map(|s| HostValue::Text(Some(s)))
                .map_err(|e| e.to_string()),
            // cov:ignore: exercised by raikiri-js-wasmtime-host tests; tools/ is outside the workspace llvm-cov run.
            HostOperation::LoadImage(url) => self.load_image(&url),
            // cov:ignore: exercised by raikiri-js-wasmtime-host tests; tools/ is outside the workspace llvm-cov run.
            HostOperation::Visible(n) => self.is_visible(n),
            // cov:ignore: exercised by raikiri-js-wasmtime-host tests; tools/ is outside the workspace llvm-cov run.
            HostOperation::Viewport => self.viewport_size(),
            // cov:ignore: exercised by raikiri-js-wasmtime-host tests; tools/ is outside the workspace llvm-cov run.
            HostOperation::FixedToViewport(n) => self.is_fixed_to_viewport(n),
            // cov:ignore: exercised by raikiri-js-wasmtime-host tests; tools/ is outside the workspace llvm-cov run.
            HostOperation::Rendered(n) => self.is_being_rendered(n),
            // cov:ignore: exercised by raikiri-js-wasmtime-host tests; tools/ is outside the workspace llvm-cov run.
            HostOperation::ImagePixels(url, default_size) => Ok(HostValue::Image(
                self.host
                    .image_pixels(&url, default_size)
                    .map(|pixels| ImageDto::from(&pixels)),
            )),
        }
    }
    // cov:ignore: exercised by raikiri-js-wasmtime-host tests; tools/ is outside the workspace llvm-cov run.
    fn load_image(&mut self, url: &str) -> Result<HostValue, String> {
        self.host
            .load_image(url)
            .map(|()| HostValue::Unit)
            .map_err(|e| e.to_string())
    }
    // cov:ignore: exercised by raikiri-js-wasmtime-host tests; tools/ is outside the workspace llvm-cov run.
    fn is_being_rendered(&mut self, node: usize) -> Result<HostValue, String> {
        if node >= self.host.document().node_count() {
            return Err("rendered node bounds".into());
        }
        self.host
            .is_being_rendered(node)
            .map(HostValue::Flag)
            .map_err(|e| e.to_string())
    }
    // cov:ignore: exercised by raikiri-js-wasmtime-host tests; tools/ is outside the workspace llvm-cov run.
    fn is_visible(&mut self, node: usize) -> Result<HostValue, String> {
        if node >= self.host.document().node_count() {
            return Err("visible node bounds".into());
        }
        self.host
            .is_visible(node)
            .map(HostValue::Flag)
            .map_err(|e| e.to_string())
    }
    // cov:ignore: exercised by raikiri-js-wasmtime-host tests; tools/ is outside the workspace llvm-cov run.
    fn is_fixed_to_viewport(&mut self, node: usize) -> Result<HostValue, String> {
        if node >= self.host.document().node_count() {
            return Err("fixed-to-viewport node bounds".into());
        }
        self.host
            .is_fixed_to_viewport(node)
            .map(HostValue::Flag)
            .map_err(|e| e.to_string())
    }
    // cov:ignore: exercised by raikiri-js-wasmtime-host tests; tools/ is outside the workspace llvm-cov run.
    fn viewport_size(&mut self) -> Result<HostValue, String> {
        self.host
            .viewport_size()
            .map(HostValue::Size)
            .map_err(|e| e.to_string())
    }
    pub fn take_response(&mut self, capacity: usize) -> Result<Vec<u8>, String> {
        let bytes = self.pending.as_ref().ok_or("no pending response")?;
        if capacity < bytes.len() || capacity > MAX_MESSAGE {
            return Err("response capacity".into());
        }
        Ok(self.pending.take().expect("checked pending response"))
    }
    pub fn copy_response(&mut self, out: &mut [u8]) -> Result<usize, String> {
        let bytes = self.take_response(out.len())?;
        out[..bytes.len()].copy_from_slice(&bytes);
        Ok(bytes.len())
    }
}
pub fn checked_range(
    ptr: u32,
    len: u32,
    memory_len: usize,
) -> Result<std::ops::Range<usize>, String> {
    if len as usize > MAX_MESSAGE {
        return Err("message cap".into());
    }
    let start = ptr as usize;
    let end = start.checked_add(len as usize).ok_or("pointer overflow")?;
    if end > memory_len {
        return Err("memory bounds".into());
    }
    Ok(start..end)
}
