//! raikiri-traits — foundation traits and neutral model types.
//!
//! Collects all traits and neutral model types from design spec §4 for
//! raikiri-html / raikiri-style / raikiri-dom / raikiri-paint / raikiri-net /
//! raikiri-js. This shared type layer contains no implementations.
//!
//! ## Module tour
//!
//! - [`dom`]      — DOM abstraction trait + identifier newtypes (Symbol, NodeId)
//! - [`page`]     — Page-related opaque model types (PageBox, ...)
//! - [`paint`]    — Shared paint geometry and color values
//! - [`policy`]   — ResourcePolicy trait + violation types
//! - [`net`]      — NetworkProvider trait + Request / FetchedResource types
//! - [`resolver`] — ReplacedResolver trait + intrinsic size types
//! - [`script`]   — ScriptExecutor trait + ScriptExecution outcome
//! - [`error`]    — RenderError taxonomy + status / summary types
//! - [`strategy`] — Strategy traits (LookaheadPolicy, TargetResolver, ReflowPolicy)
//! - [`config`]   — Entry point configs (RenderLimits, LookaheadConfig, ...)
//! - [`plan`]     — `plan()` output types (DocumentPlan, PageSummary)
//! - [`io`]       — bounded regular-file read primitive
//!
//! ## Spec authority
//!
//! The authoritative source for these type definitions is the design doc (from the
//! Workspace Layout scope and spec drift protocol design).

pub mod config;
pub mod consumer;
pub mod dom;
pub mod error;
pub mod image;
pub mod io;
pub mod net;
pub mod page;
pub mod paint;
pub mod plan;
pub mod policy;
pub mod resolver;
pub mod script;
pub mod strategy;

// Main types re-exported at the crate root (so consumers can use `use raikiri_traits::*`).
pub use config::{
    BatchConfig, BatchConfigBuilder, LayoutConfig, LayoutConfigBuilder, LookaheadConfig,
    LookaheadConfigBuilder, RenderLimits, RenderLimitsBuilder,
};
pub use consumer::{ConsumerPropertyEvent, ConsumerPropertyObserver, ConsumerPropertyValue};
pub use dom::{Dom, Element, Node, NodeId, NodeKind, QuirksMode, StylesheetKind, Symbol};
pub use error::{
    CascadeError, EmittedSlotInfo, ExhaustionPolicy, LayoutError, LimitKind, ParseError,
    RenderError, RenderSummary, RenderWarning, TargetDiscrepancy, TargetKind, TargetSlotId,
    UnresolvedReason, UnresolvedTarget, WarningKind,
};
pub use image::{DecodedImage, ImageIntrinsicSize, ImagePixelSource, ImageRasterSize};
pub use io::{OversizePhase, RejectReason, read_bounded_regular_file};
pub use net::{
    AbortController, AbortSignal, Body, FetchOutcome, FetchedResource, HeaderMap,
    MAX_AUTO_REDIRECT_HOPS, Method, NetworkError, NetworkProvider, Request,
};
pub use page::{
    ContentSource, ContentValueConvertError, ContentValueItem, CounterStack, FormData,
    GcpmDirective, LayoutBuffer, NamedStringState, PageBox, PageContext, PageDefaults,
    PageDefaultsBuilder, PendingResolution, ResolveOutcome, RunningTemplate, RunningTemplateId,
    TargetInfo, TargetRegistry, resolve_content_component,
};
pub use paint::{PaintClip, PaintColor, PaintInsets, PaintRect};
pub use plan::{BreakReason, DocumentPlan, PageSummary, TargetDefinition};
pub use policy::{PolicyViolation, ResourceKind, ResourcePolicy, ViolationType};
pub use resolver::{
    IntrinsicBox, ReplacedResolver, ResolveDisposition, ResolvedIntrinsic, ResolverError,
    ResolverRequest,
};
pub use script::{ScriptExecution, ScriptExecutor};
pub use strategy::{
    ContainerOverflowFallback, DirtyDeadline, LookaheadPolicy, ProbeContext, ReflowAction,
    ReflowPolicy, ResolvedTarget, TargetRequest, TargetResolver,
};

#[cfg(test)]
mod tests;
