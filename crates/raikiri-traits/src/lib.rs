//! raikiri-traits — foundation traits and neutral model types.
//!
//! 設計仕様書 §4 に定義された全 trait / 中立モデル型を集約する。実装は持たず、
//! raikiri-html / raikiri-style / raikiri-dom / raikiri-paint / raikiri-net が
//! 参照する共通型層。詳細は `docs/superpowers/specs/2026-07-13-raikiri-rebuild-design.md`
//! §4 を参照。

pub mod dom;
pub mod error;
pub mod net;
pub mod page;
pub mod policy;
pub mod resolver;
pub mod sink;
pub mod strategy;

pub use dom::{Dom, Element, Node, NodeId, Symbol};
pub use error::{
    CascadeError, EmittedSlotInfo, ExhaustionPolicy, LayoutError, LimitKind, ParseError,
    RenderError, RenderStatus, RenderSummary, RenderWarning, TargetDiscrepancy, TargetKind,
    TargetSlotId, UnresolvedReason, UnresolvedTarget, WarningKind,
};
pub use net::{
    AbortController, AbortSignal, Body, FetchedResource, HeaderMap, Method, NetworkError,
    NetworkProvider, Request,
};
pub use page::{
    ContentValueItem, FormData, GcpmDirective, LayoutBuffer, PageBox, PageContext, PageFragment,
    RunningTemplate, TargetRegistry,
};
pub use policy::{PolicyViolation, ResourceKind, ResourcePolicy, ViolationType};
pub use resolver::{
    IntrinsicBox, ReplacedResolver, ResolveDisposition, ResolvedIntrinsic, ResolverError,
    ResolverRequest,
};
pub use sink::RenderSink;
pub use strategy::{
    ContainerOverflowFallback, DirtyDeadline, EmissionPolicy, LookaheadPolicy, ProbeContext,
    ReflowAction, ReflowPolicy, ResolvedTarget, TargetRequest, TargetResolver,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn symbol_construct_from_str() {
        let s = Symbol::from("target-1");
        assert_eq!(s.as_str(), "target-1");
    }

    #[test]
    fn nodeid_construct() {
        let n = NodeId::new(42);
        assert_eq!(n.0, 42);
    }

    #[test]
    fn page_placeholders_default_construct() {
        let _ = PageFragment::default();
        let _ = PageFragment::new();
        let _ = PageBox::default();
        let _ = PageContext::default();
        let _ = LayoutBuffer::default();
        let _ = TargetRegistry::default();
        let _ = RunningTemplate::default();
        let _ = FormData::default();
    }

    #[test]
    fn resource_policy_is_object_safe() {
        fn _assert<T: ?Sized>() {}
        _assert::<dyn ResourcePolicy>();
    }

    #[test]
    fn network_provider_is_object_safe() {
        fn _assert<T: ?Sized>() {}
        _assert::<dyn NetworkProvider>();
    }

    #[test]
    fn abort_controller_default_and_abort() {
        let c = AbortController::new();
        assert!(!c.signal.is_aborted());
        c.abort();
        assert!(c.signal.is_aborted());
    }

    #[test]
    fn replaced_resolver_is_object_safe() {
        fn _assert<T: ?Sized>() {}
        _assert::<dyn ReplacedResolver>();
    }

    #[test]
    fn resolver_placeholder_types_default_construct() {
        let _ = IntrinsicBox::default();
        let _ = ResolverRequest::default();
    }

    #[test]
    fn render_error_is_error_trait() {
        fn _assert<T: std::error::Error>() {}
        _assert::<RenderError>();
    }

    #[test]
    fn exhaustion_policy_default_is_error() {
        assert_eq!(ExhaustionPolicy::default(), ExhaustionPolicy::Error);
    }

    #[test]
    fn target_slot_id_construct() {
        let id = TargetSlotId {
            page_index: 3,
            sequence: 7,
        };
        assert_eq!(id.page_index, 3);
        assert_eq!(id.sequence, 7);
    }

    #[test]
    fn render_sink_is_object_safe() {
        fn _assert<T: ?Sized>() {}
        _assert::<dyn RenderSink>();
    }

    #[test]
    fn strategy_placeholders_default_construct() {
        let _ = ProbeContext::default();
        let _ = TargetRequest::default();
    }
}
