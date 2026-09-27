//! Owned, bounded protocol for the local integration trial.
use raikiri_dom::snapshot::LogicalSnapshot;
use raikiri_js::runtime::{
    Abort, BoxGeometry, DomRect, Limits, PositionKind, RunOptions, RunReport,
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
pub const VERSION: u32 = 1;
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
}
#[derive(Debug, Serialize, Deserialize)]
pub struct Response {
    pub version: u32,
    pub result: Result<HostValue, String>,
}
#[derive(Debug, Serialize, Deserialize)]
pub enum HostValue {
    Unit,
    Geometry(Option<GeometryDto>),
    Text(Option<String>),
    Document(LogicalSnapshot),
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
