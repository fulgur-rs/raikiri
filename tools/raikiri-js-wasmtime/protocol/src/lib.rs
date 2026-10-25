//! Owned, bounded protocol for the local integration trial.
use raikiri_dom::snapshot::LogicalSnapshot;
use raikiri_js::runtime::{
    Abort, BoxGeometry, DomRect, Limits, PositionKind, RunOptions, RunReport,
};
pub use raikiri_js::runtime::{DecodedImage, ImagePixels};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
// cov:ignore: exercised by raikiri-js-wasmtime-protocol tests; tools/ is outside the workspace llvm-cov run.
pub const VERSION: u32 = 7;
pub const MAX_MESSAGE: usize = 16 * 1024 * 1024;

pub fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>, String> {
    struct Bounded(Vec<u8>);
    impl std::io::Write for Bounded {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            if b.len() > MAX_MESSAGE.saturating_sub(self.0.len()) {
                return Err(std::io::Error::other("transport cap"));
            }
            self.0.extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut writer = Bounded(Vec::new());
    serde_json::to_writer(&mut writer, value).map_err(|e| e.to_string())?;
    Ok(writer.0)
}
pub fn decode<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, String> {
    if bytes.len() > MAX_MESSAGE {
        return Err("transport cap".into());
    }
    serde_json::from_slice(bytes).map_err(|e| e.to_string())
}
#[derive(Debug, Serialize, Deserialize)]
pub struct Request {
    pub version: u32,
    pub operation: HostOperation,
}
impl Request {
    pub fn check_version(&self) -> Result<(), String> {
        if self.version == VERSION {
            Ok(())
        } else {
            Err("protocol version".into())
        }
    }
}
#[derive(Debug, Serialize, Deserialize)]
pub enum HostOperation {
    Flush(LogicalSnapshot),
    Geometry(usize),
    Computed(usize, String),
    Fragment {
        tag: String,
        namespace: String,
        markup: String,
    },
    Fetch(String),
    // cov:ignore: exercised by raikiri-js-wasmtime-protocol users; tools/ is outside the workspace llvm-cov run.
    LoadImage(String),
    // cov:ignore: exercised by raikiri-js-wasmtime-protocol tests; tools/ is outside the workspace llvm-cov run.
    Rendered(usize),
    // cov:ignore: exercised by raikiri-js-wasmtime-protocol tests; tools/ is outside the workspace llvm-cov run.
    Visible(usize),
    // cov:ignore: exercised by raikiri-js-wasmtime-protocol tests; tools/ is outside the workspace llvm-cov run.
    Viewport,
    // cov:ignore: exercised by raikiri-js-wasmtime-protocol tests; tools/ is outside the workspace llvm-cov run.
    FixedToViewport(usize),
    /// The pixels of a loaded image, sized against a default object size
    /// (width, height) in CSS px.
    // cov:ignore: exercised by raikiri-js-wasmtime-protocol tests; tools/ is outside the workspace llvm-cov run.
    ImagePixels(String, (f64, f64)),
}
#[derive(Debug, Serialize, Deserialize)]
pub struct Response {
    pub version: u32,
    pub result: Result<HostValue, String>,
}
// cov:ignore: exercised by raikiri-js-wasmtime-protocol tests; tools/ is outside the workspace llvm-cov run.
#[derive(Debug, Serialize, Deserialize)]
pub enum HostValue {
    Unit,
    Geometry(Option<GeometryDto>),
    Text(Option<String>),
    Document(LogicalSnapshot),
    // cov:ignore: exercised by raikiri-js-wasmtime-protocol tests; tools/ is outside the workspace llvm-cov run.
    Flag(bool),
    // cov:ignore: exercised by raikiri-js-wasmtime-protocol tests; tools/ is outside the workspace llvm-cov run.
    Size(Option<(f64, f64)>),
    // cov:ignore: exercised by raikiri-js-wasmtime-protocol tests; tools/ is outside the workspace llvm-cov run.
    Image(Option<ImageDto>),
}
/// An image's concrete object size in CSS px and its decoded straight-alpha
/// RGBA8 raster.
// cov:ignore: exercised by raikiri-js-wasmtime-protocol tests; tools/ is outside the workspace llvm-cov run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageDto {
    pub size: (f64, f64),
    pub width: u32,
    pub height: u32,
    /// Sent as base64: as a JSON array of numbers, an ordinary 1024x1024
    /// image alone would exceed [`MAX_MESSAGE`].
    #[serde(with = "base64")]
    pub rgba: Vec<u8>,
}
// cov:ignore: exercised by raikiri-js-wasmtime-protocol tests; tools/ is outside the workspace llvm-cov run.
impl From<&ImagePixels> for ImageDto {
    // cov:ignore: exercised by raikiri-js-wasmtime-protocol tests; tools/ is outside the workspace llvm-cov run.
    fn from(pixels: &ImagePixels) -> Self {
        Self {
            size: pixels.size,
            width: pixels.image.width,
            height: pixels.image.height,
            rgba: pixels.image.rgba.clone(),
        }
    }
}
// cov:ignore: exercised by raikiri-js-wasmtime-protocol tests; tools/ is outside the workspace llvm-cov run.
impl ImageDto {
    /// The native image, or an error when the byte count does not match
    /// the dimensions.
    // cov:ignore: exercised by raikiri-js-wasmtime-protocol tests; tools/ is outside the workspace llvm-cov run.
    pub fn into_native(self) -> Result<ImagePixels, String> {
        let expected = (self.width as usize)
            .checked_mul(self.height as usize)
            .and_then(|pixels| pixels.checked_mul(4));
        if expected != Some(self.rgba.len()) {
            return Err("image byte length".into());
        }
        Ok(ImagePixels {
            size: self.size,
            image: std::sync::Arc::new(DecodedImage::new(self.width, self.height, self.rgba)),
        })
    }
}
/// Padded standard base64 (RFC 4648 §4) for a byte field.
// cov:ignore: exercised by raikiri-js-wasmtime-protocol tests; tools/ is outside the workspace llvm-cov run.
pub mod base64 {
    use serde::{Deserialize, Deserializer, Serializer, de::Error};

    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    // cov:ignore: exercised by raikiri-js-wasmtime-protocol tests; tools/ is outside the workspace llvm-cov run.
    pub fn serialize<S: Serializer>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&encode(bytes))
    }
    // cov:ignore: exercised by raikiri-js-wasmtime-protocol tests; tools/ is outside the workspace llvm-cov run.
    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<u8>, D::Error> {
        let text = String::deserialize(deserializer)?;
        decode(&text).ok_or_else(|| D::Error::custom("invalid base64"))
    }
    // cov:ignore: exercised by raikiri-js-wasmtime-protocol tests; tools/ is outside the workspace llvm-cov run.
    pub fn encode(bytes: &[u8]) -> String {
        let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
        for chunk in bytes.chunks(3) {
            let byte = |i: usize| u32::from(chunk.get(i).copied().unwrap_or(0));
            let group = byte(0) << 16 | byte(1) << 8 | byte(2);
            for i in 0..4 {
                if i <= chunk.len() {
                    out.push(char::from(ALPHABET[(group >> (18 - 6 * i) & 63) as usize]));
                } else {
                    out.push('=');
                }
            }
        }
        out
    }
    /// The bytes `text` encodes, or `None` when it is not padded base64.
    // cov:ignore: exercised by raikiri-js-wasmtime-protocol tests; tools/ is outside the workspace llvm-cov run.
    pub fn decode(text: &str) -> Option<Vec<u8>> {
        let text = text.as_bytes();
        if text.len() % 4 != 0 {
            return None;
        }
        let groups = text.len() / 4;
        let mut out = Vec::with_capacity(groups * 3);
        for (index, quad) in text.chunks(4).enumerate() {
            let padding = quad.iter().rev().take_while(|&&c| c == b'=').count();
            // Only the last group may be padded, and by at most two.
            if padding > 2 || (padding > 0 && index + 1 != groups) {
                return None;
            }
            let mut group = 0u32;
            for &c in &quad[..4 - padding] {
                group = group << 6 | u32::from(value(c)?);
            }
            group <<= 6 * padding as u32;
            out.extend_from_slice(&group.to_be_bytes()[1..4 - padding]);
        }
        Some(out)
    }
    // cov:ignore: exercised by raikiri-js-wasmtime-protocol tests; tools/ is outside the workspace llvm-cov run.
    fn value(c: u8) -> Option<u8> {
        match c {
            b'A'..=b'Z' => Some(c - b'A'),
            b'a'..=b'z' => Some(c - b'a' + 26),
            b'0'..=b'9' => Some(c - b'0' + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LimitsDto {
    pub virtual_time: f64,
    pub tasks: u64,
    pub loops: u64,
    pub recursion: usize,
    pub nodes: usize,
}
impl From<RunOptions> for LimitsDto {
    fn from(o: RunOptions) -> Self {
        let l = o.limits;
        Self {
            virtual_time: l.max_virtual_time_ms,
            tasks: l.max_tasks,
            loops: l.max_loop_iterations,
            recursion: l.max_recursion,
            nodes: l.max_nodes,
        }
    }
}
impl LimitsDto {
    pub fn into_options(self) -> RunOptions {
        RunOptions {
            limits: Limits {
                max_virtual_time_ms: self.virtual_time,
                max_tasks: self.tasks,
                max_loop_iterations: self.loops,
                max_recursion: self.recursion,
                max_nodes: self.nodes,
            },
        }
    }
}
#[derive(Debug, Serialize, Deserialize)]
pub struct CreatePage {
    pub document: LogicalSnapshot,
    pub document_url: Option<String>,
    pub limits: LimitsDto,
}
#[derive(Debug, Serialize, Deserialize)]
pub enum GuestOperation {
    Create(CreatePage),
    Evaluate(String),
    RunDocument,
    ProbeTimeout(ReportDto),
    TakeResults,
    TakeDocument,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct GuestRequest {
    pub version: u32,
    pub operation: GuestOperation,
}
#[derive(Debug, Serialize, Deserialize)]
pub enum GuestValue {
    Unit,
    Report(ReportDto),
    Delivery(Option<DeliveryDto>),
    Document(LogicalSnapshot),
}
#[derive(Debug, Serialize, Deserialize)]
pub struct GuestResponse {
    pub version: u32,
    pub result: Result<GuestValue, String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestOutcomeDto {
    pub name: String,
    pub passed: bool,
    pub message: String,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct DeliveryDto {
    pub tests: Vec<TestOutcomeDto>,
    #[serde(serialize_with = "serialize_harness_status")]
    pub harness_status: f64,
    pub harness_message: String,
}
fn serialize_harness_status<S: serde::Serializer>(
    status: &f64,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    // Native classifies all nonfinite statuses as ERROR, just like status 1.
    // JSON cannot represent these numbers; preserve the classification.
    serializer.serialize_f64(if status.is_finite() { *status } else { 1.0 })
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AbortDto {
    VirtualTime,
    Tasks,
    LoopIterations,
    Recursion,
    Nodes,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReportDto {
    pub scripts_run: usize,
    pub uncaught_errors: Vec<String>,
    pub fetch_errors: Vec<String>,
    pub aborted: Option<AbortDto>,
    pub host_failures: Vec<String>,
    pub console: Vec<(String, String)>,
}
impl From<RunReport> for ReportDto {
    fn from(r: RunReport) -> Self {
        Self {
            scripts_run: r.scripts_run,
            uncaught_errors: r.uncaught_errors,
            fetch_errors: r.fetch_errors,
            aborted: r.aborted.map(|a| match a {
                Abort::VirtualTime => AbortDto::VirtualTime,
                Abort::Tasks => AbortDto::Tasks,
                Abort::LoopIterations => AbortDto::LoopIterations,
                Abort::Recursion => AbortDto::Recursion,
                Abort::Nodes => AbortDto::Nodes,
            }),
            host_failures: r.host_failures,
            console: r.console,
        }
    }
}
impl ReportDto {
    pub fn into_native(self) -> RunReport {
        RunReport {
            scripts_run: self.scripts_run,
            uncaught_errors: self.uncaught_errors,
            fetch_errors: self.fetch_errors,
            aborted: self.aborted.map(|a| match a {
                AbortDto::VirtualTime => Abort::VirtualTime,
                AbortDto::Tasks => Abort::Tasks,
                AbortDto::LoopIterations => Abort::LoopIterations,
                AbortDto::Recursion => Abort::Recursion,
                AbortDto::Nodes => Abort::Nodes,
            }),
            host_failures: self.host_failures,
            console: self.console,
        }
    }
}
#[derive(Debug, Serialize, Deserialize)]
pub struct GeometryDto {
    pub border: [f64; 6],
    pub padding: [f64; 6],
    pub scroll: [f64; 2],
    pub position: u8,
    pub is_inline: bool,
}
fn rect(r: DomRect) -> [f64; 6] {
    [r.left, r.top, r.right, r.bottom, r.width, r.height]
}
fn native_rect(r: [f64; 6]) -> DomRect {
    DomRect {
        left: r[0],
        top: r[1],
        right: r[2],
        bottom: r[3],
        width: r[4],
        height: r[5],
    }
}
impl From<BoxGeometry> for GeometryDto {
    fn from(g: BoxGeometry) -> Self {
        Self {
            border: rect(g.border_box),
            padding: rect(g.padding_box),
            scroll: [g.scroll_width, g.scroll_height],
            position: match g.position {
                PositionKind::Static => 0,
                PositionKind::Relative => 1,
                PositionKind::Absolute => 2,
                PositionKind::Fixed => 3,
                PositionKind::Sticky => 4,
            },
            is_inline: g.is_inline,
        }
    }
}
impl GeometryDto {
    pub fn into_native(self) -> Result<BoxGeometry, String> {
        if self.position > 4
            || self
                .border
                .iter()
                .chain(&self.padding)
                .chain(&self.scroll)
                .any(|n| !n.is_finite())
        {
            return Err("invalid geometry".into());
        }
        Ok(BoxGeometry {
            border_box: native_rect(self.border),
            padding_box: native_rect(self.padding),
            scroll_width: self.scroll[0],
            scroll_height: self.scroll[1],
            position: match self.position {
                0 => PositionKind::Static,
                1 => PositionKind::Relative,
                2 => PositionKind::Absolute,
                3 => PositionKind::Fixed,
                _ => PositionKind::Sticky,
            },
            is_inline: self.is_inline,
        })
    }
}
#[cfg(test)]
mod tests;
