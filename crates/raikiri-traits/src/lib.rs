//! raikiri-traits — foundation traits and neutral model types.
//!
//! 設計仕様書 §4 に定義された全 trait / 中立モデル型を集約する。実装は持たず、
//! raikiri-html / raikiri-style / raikiri-dom / raikiri-paint / raikiri-net が
//! 参照する共通型層。
//!
//! ## Module tour
//!
//! - [`dom`]      — DOM abstraction trait + identifier newtypes (Symbol, NodeId)
//! - [`page`]     — Page-related opaque model types (PageBox, ...)
//! - [`paint`]    — Shared paint geometry and color values
//! - [`policy`]   — ResourcePolicy trait + violation types
//! - [`net`]      — NetworkProvider trait + Request / FetchedResource types
//! - [`resolver`] — ReplacedResolver trait + intrinsic size types
//! - [`error`]    — RenderError taxonomy + status / summary types
//! - [`strategy`] — Strategy traits (LookaheadPolicy, TargetResolver, ReflowPolicy)
//! - [`config`]   — Entry point configs (RenderLimits, LookaheadConfig, ...)
//! - [`plan`]     — `plan()` output types (DocumentPlan, PageSummary)
//! - [`io`]       — bounded regular-file read primitive
//!
//! ## Spec authority
//!
//! 型定義の authoritative source は design doc (Workspace Layout scope +
//! spec drift protocol の design 由来)。

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
pub mod strategy;

// 主要型 crate-root re-export (Consumer が `use raikiri_traits::*` で足りる shape)
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
pub use strategy::{
    ContainerOverflowFallback, DirtyDeadline, LookaheadPolicy, ProbeContext, ReflowAction,
    ReflowPolicy, ResolvedTarget, TargetRequest, TargetResolver,
};

#[cfg(test)]
mod tests;
