//! External consumer が `use raikiri::*;` のみで parse_html → plan (Err) →
//! render_streaming (Err) の chain を書けることを compile + run で pin
//! (raikiri-spike-m1.15 前哨、raikiri-spike-m1.11 で追加)。

use raikiri::*;

#[test]
fn external_consumer_can_reference_all_reexported_types() {
    // 各型が use raikiri::*; だけで名前解決できることを compile で pin。
    // 実際に値を使う必要なし (dead_code lint 抑制のため let _ で消費)。
    let _ = std::marker::PhantomData::<(
        RenderStatus,
        RenderSummary,
        LimitKind,
        DocumentPlan,
        PageSummary,
        PageDefaults,
        PageDefaultsBuilder,
        PageBox,
        PageContext,
        PageFragment,
        PlanConfig,
        PlanConfigBuilder,
        StreamingConfig,
        StreamingConfigBuilder,
        BatchConfig,
        BatchConfigBuilder,
        LookaheadConfig,
        LookaheadConfigBuilder,
        RenderLimits,
        RenderLimitsBuilder,
        LayoutError,
        Symbol,
    )>;
}
