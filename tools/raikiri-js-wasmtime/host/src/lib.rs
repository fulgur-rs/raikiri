//! Local Linux trial of a metered guest DOM runtime.
mod bridge;
mod config;
mod limits;
mod loader;
mod page;
mod wasi;
pub use bridge::{Bridge, BridgeMetrics, checked_range};
pub use page::{EngineError, PageMetrics, SandboxOptions, WasmtimePage};
pub(crate) struct State {
    pub limits: limits::PageLimits,
    pub bridge: Bridge,
}
#[cfg(test)]
mod tests;
