//! raikiri-traits — foundation traits and neutral model types.
//!
//! 設計仕様書 §4 に定義された全 trait / 中立モデル型を集約する。実装は持たず、
//! raikiri-html / raikiri-style / raikiri-dom / raikiri-paint / raikiri-net が
//! 参照する共通型層。詳細は `docs/superpowers/specs/2026-07-13-raikiri-rebuild-design.md`
//! §4 を参照。

pub mod config;
pub mod dom;
pub mod error;
pub mod net;
pub mod page;
pub mod plan;
pub mod policy;
pub mod resolver;
pub mod sink;
pub mod strategy;

pub use config::{
    BatchConfig, BatchConfigBuilder, LookaheadConfig, LookaheadConfigBuilder, PlanConfig,
    PlanConfigBuilder, RenderLimits, RenderLimitsBuilder, StreamingConfig, StreamingConfigBuilder,
};
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
pub use plan::{BreakReason, DocumentPlan, PageSummary, TargetDefinition};
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

    #[test]
    fn plan_placeholder_default_construct() {
        let _ = TargetDefinition::default();
    }

    #[test]
    fn all_configs_default_construct() {
        let _ = LookaheadConfig::new();
        let _ = LookaheadConfig::default();
        let _ = RenderLimits::default();
        let _ = RenderLimits::new();
        let _ = StreamingConfig::default();
        let _ = BatchConfig::default();
        let _ = PlanConfig::default();
    }

    #[test]
    fn lookahead_config_defaults() {
        let d = LookaheadConfig::default();
        assert_eq!(d.widow_line_buffer, 2);
        assert_eq!(d.orphan_line_buffer, 2);
        assert_eq!(d.break_avoid_max_subtree_blocks, 20);
        assert_eq!(d.max_container_probe_pages, Some(4));
        assert!(!d.allow_cross_size_lookahead);
    }

    #[test]
    fn render_limits_defaults() {
        let d = RenderLimits::default();
        assert_eq!(d.max_document_pages, Some(10_000));
        assert_eq!(d.max_dom_nodes, Some(1_000_000));
        assert_eq!(d.max_target_slots, Some(100_000));
        assert_eq!(d.max_layout_buffer_entries, Some(10_000));
        assert_eq!(d.max_aggregate_bytes, Some(1_073_741_824));
    }

    #[test]
    fn lookahead_config_builder_roundtrip() {
        let cfg = LookaheadConfig::builder()
            .widow_line_buffer(5)
            .max_container_probe_pages(None)
            .build();
        assert_eq!(cfg.widow_line_buffer, 5);
        assert_eq!(cfg.max_container_probe_pages, None);
        // 未設定 field は default 値
        assert_eq!(
            cfg.orphan_line_buffer,
            LookaheadConfig::default().orphan_line_buffer
        );
    }

    #[test]
    fn render_limits_builder_roundtrip() {
        let cfg = RenderLimits::builder()
            .max_document_pages(Some(100))
            .max_dom_nodes(None)
            .build();
        assert_eq!(cfg.max_document_pages, Some(100));
        assert_eq!(cfg.max_dom_nodes, None);
        // 未設定 field は default 値
        assert_eq!(
            cfg.max_target_slots,
            RenderLimits::default().max_target_slots
        );
    }

    #[test]
    fn streaming_config_builder_roundtrip() {
        let limits = RenderLimits::builder().max_document_pages(Some(50)).build();
        let cfg = StreamingConfig::builder().limits(limits).build();
        assert_eq!(cfg.limits.max_document_pages, Some(50));
    }
}
