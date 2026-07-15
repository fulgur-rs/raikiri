//! raikiri-traits — foundation traits and neutral model types.
//!
//! 設計仕様書 §4 に定義された全 trait / 中立モデル型を集約する。実装は持たず、
//! raikiri-html / raikiri-style / raikiri-dom / raikiri-paint / raikiri-net が
//! 参照する共通型層。
//!
//! ## Module tour
//!
//! - [`dom`]      — DOM abstraction trait + identifier newtypes (Symbol, NodeId)
//! - [`page`]     — Page-related opaque model types (PageFragment, PageBox, ...)
//! - [`policy`]   — ResourcePolicy trait + violation types
//! - [`net`]      — NetworkProvider trait + Request / FetchedResource types
//! - [`resolver`] — ReplacedResolver trait + intrinsic size types
//! - [`error`]    — RenderError taxonomy + status / summary types
//! - [`sink`]     — RenderSink trait
//! - [`strategy`] — Strategy traits (LookaheadPolicy, TargetResolver, EmissionPolicy, ReflowPolicy)
//! - [`config`]   — Entry point configs (RenderLimits, LookaheadConfig, ...)
//! - [`plan`]     — `plan()` output types (DocumentPlan, PageSummary)
//!
//! ## Spec authority
//!
//! 型定義の authoritative source は
//! `docs/superpowers/specs/2026-07-13-raikiri-rebuild-design.md` §4 (§13.0 spec
//! drift protocol 準拠)。narrative §5-§12 は補助資料。

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

// 主要型 crate-root re-export (Consumer が `use raikiri_traits::*` で足りる shape)
pub use config::{
    BatchConfig, BatchConfigBuilder, LookaheadConfig, LookaheadConfigBuilder, PlanConfig,
    PlanConfigBuilder, RenderLimits, RenderLimitsBuilder, StreamingConfig, StreamingConfigBuilder,
};
pub use dom::{Dom, Element, Node, NodeId, NodeKind, QuirksMode, Symbol};
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

    // ── DOM foundations ─────────────────────────────────────────

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

    // ── Page placeholders ───────────────────────────────────────

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

    // ── Trait object safety ─────────────────────────────────────

    #[test]
    fn dyn_traits_are_object_safe() {
        fn _assert<T: ?Sized>() {}
        _assert::<dyn RenderSink>();
        _assert::<dyn ReplacedResolver>();
        _assert::<dyn NetworkProvider>();
        _assert::<dyn ResourcePolicy>();
        // Strategy traits (LookaheadPolicy / TargetResolver / EmissionPolicy /
        // ReflowPolicy) は generic param 経由で受ける (§4 `render_with<L,T,E,R>`
        // 設計) ため object-safety は要件外。M1.5+ で dyn 化が必要なら判断。
        //
        // Dom / Element / Node は M1.5 dom-model で associated type / GAT を
        // 追加する予定で、その段階で non-object-safe になる可能性が高いため
        // M1.1 では assert しない。
    }

    // ── AbortController semantic ────────────────────────────────

    #[test]
    fn abort_controller_default_and_abort() {
        let c = AbortController::new();
        assert!(!c.signal.is_aborted());
        c.abort();
        assert!(c.signal.is_aborted());
    }

    // ── Resolver placeholders ───────────────────────────────────

    #[test]
    fn resolver_placeholder_types_default_construct() {
        let _ = IntrinsicBox::default();
        let _ = ResolverRequest::default();
    }

    // ── Error taxonomy ──────────────────────────────────────────

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

    // ── Strategy placeholders ───────────────────────────────────

    #[test]
    fn strategy_placeholders_default_construct() {
        let _ = ProbeContext::default();
        let _ = TargetRequest::default();
    }

    // ── Config defaults ─────────────────────────────────────────

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

    // ── Config builders ─────────────────────────────────────────

    #[test]
    fn lookahead_config_builder_roundtrip() {
        let cfg = LookaheadConfig::builder()
            .widow_line_buffer(5)
            .max_container_probe_pages(None)
            .build();
        assert_eq!(cfg.widow_line_buffer, 5);
        assert_eq!(cfg.max_container_probe_pages, None);
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

    #[test]
    fn batch_config_builder_roundtrip() {
        let limits = RenderLimits::builder().max_document_pages(Some(30)).build();
        let cfg = BatchConfig::builder().limits(limits).build();
        assert_eq!(cfg.limits.max_document_pages, Some(30));
    }

    #[test]
    fn plan_config_builder_roundtrip() {
        let lookahead = LookaheadConfig::builder().widow_line_buffer(3).build();
        let cfg = PlanConfig::builder().lookahead(lookahead).build();
        assert_eq!(cfg.lookahead.widow_line_buffer, 3);
    }

    // ── Plan placeholders ───────────────────────────────────────

    #[test]
    fn plan_placeholder_default_construct() {
        let _ = TargetDefinition::default();
    }

    // ── Sub-error trait bounds (M1.2) ───────────────────────────

    #[test]
    fn parse_error_is_error_and_display() {
        fn _assert_error<T: std::error::Error>() {}
        fn _assert_display<T: std::fmt::Display>() {}
        _assert_error::<ParseError>();
        _assert_display::<ParseError>();
    }

    #[test]
    fn parse_error_io_source_chain() {
        let io_err = std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "eof");
        let pe: ParseError = io_err.into();
        use std::error::Error as _;
        let src = pe.source();
        assert!(src.is_some(), "ParseError::Io should expose inner io::Error via source()");
    }

    #[test]
    fn cascade_error_is_error_and_display() {
        fn _assert_error<T: std::error::Error>() {}
        fn _assert_display<T: std::fmt::Display>() {}
        _assert_error::<CascadeError>();
        _assert_display::<CascadeError>();
    }

    #[test]
    fn layout_error_is_error_and_display() {
        fn _assert_error<T: std::error::Error>() {}
        fn _assert_display<T: std::fmt::Display>() {}
        _assert_error::<LayoutError>();
        _assert_display::<LayoutError>();
    }

    #[test]
    fn render_error_parse_source_chain() {
        use std::error::Error as _;
        let io_err = std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "eof");
        let re: RenderError = RenderError::Parse(ParseError::Io(io_err));
        let src = re.source();
        assert!(src.is_some(), "RenderError::Parse should expose inner ParseError via source()");
    }

    #[test]
    fn question_mark_propagates_io_through_parse_to_render() {
        // RenderError is ≥144B (Network / Policy variants); see net.rs:33 rationale.
        #[allow(clippy::result_large_err)]
        fn producer() -> Result<(), RenderError> {
            let io_err = std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "eof");
            let pe: ParseError = io_err.into();
            Err(pe)?;
            Ok(())
        }
        let e = producer().expect_err("expected RenderError");
        match e {
            RenderError::Parse(ParseError::Io(_)) => (),
            other => panic!("expected RenderError::Parse(ParseError::Io(_)), got {other:?}"),
        }
    }

    // ── RenderStatus::Aborted contract (M1.2、実 semantic は M6c) ─

    /// Type-level contract test。`RenderStatus::Aborted` の `partial_pages`
    /// field が Consumer から観測可能で、round-trip することを固定する。
    /// 実 render pipeline 経由での partial_pages 追跡は M6c
    /// (`abort-signal-integration` / `renderstatus-aborted-impl`) で verify。
    #[test]
    fn render_status_aborted_carries_partial_pages() {
        let s = RenderStatus::Aborted { partial_pages: 7 };
        match s {
            RenderStatus::Aborted { partial_pages } => assert_eq!(partial_pages, 7),
            RenderStatus::Completed(_) => panic!("expected Aborted"),
        }
    }

    // ── Element trait extension (M1.4) ──────────────────────────

    #[test]
    fn element_default_inline_style_source_is_none() {
        use crate::Element;

        struct BareElement;
        impl<'a> Element<'a> for BareElement {
            fn tag_name(&self) -> &str {
                "p"
            }
            // inline_style_source は default impl (None) を利用
        }

        let e = BareElement;
        assert_eq!(e.inline_style_source(), None);
    }

    // ── WarningKind extension (M1.3) ────────────────────────────

    #[test]
    fn warning_kind_html_parse_error_is_constructable() {
        // #[non_exhaustive] 契約下で raikiri-html crate 相当の external consumer が
        // 該 variant を build できることを regression 防止する。
        let w = crate::WarningKind::HtmlParseError {
            message: String::from("unexpected end tag"),
        };
        match w {
            crate::WarningKind::HtmlParseError { message } => {
                assert_eq!(message, "unexpected end tag");
            }
            _ => panic!("expected HtmlParseError"),
        }
    }
}
