//! `plan` / `render_streaming` の M1 stub 実装 (raikiri-spike-m1.11)。
//!
//! この module 内の全 fn は `RenderError::Unimplemented` を返す。
//! **M2+ pagination 実装完了時に、本 module を丸ごと削除して parse.rs / dedicated
//! plan.rs / render_streaming.rs に本実装を配置し直す** (retire scope isolation)。
//!
//! - `plan`: M2+ pagination 完了後に populate、PlanConfig の initial_registry と
//!   併せて DocumentPlan を返す本実装に差し替え
//! - `render_streaming`: M2 pagestream-state-machine で本実装、`RenderSink` に
//!   PageFragment を stream 出力する
//!
//! M1 では両者とも Consumer に「html_to_png (m1.14) を使え」と migration hint
//! を返す。

use raikiri_traits::{
    DocumentPlan, PageDefaults, PlanConfig, RenderError, RenderSink, RenderStatus,
    ReplacedResolver, StreamingConfig,
};

use crate::HtmlDocument;

/// Plan mode (dry-run: parse+cascade+layout planning のみ、PaintedBox 構築なし)。
///
/// **M1 stub**: 常に `Err(RenderError::Unimplemented { feature: "plan", .. })` を
/// 返す。本実装は M2+ pagination 完了後。
///
/// spec §L1075 の signature 準拠。
#[allow(clippy::result_large_err)]
pub fn plan(
    _doc: &HtmlDocument,
    _defaults: PageDefaults,
    _resolver: &dyn ReplacedResolver,
    _config: PlanConfig,
) -> Result<DocumentPlan, RenderError> {
    Err(RenderError::Unimplemented {
        feature: "plan",
        migration_hint: "M1 non-goal; M2+ で pagination 実装後に populate",
    })
}

/// Streaming rendering (1 pass、BoundedLookahead + PlaceholderTargetResolver +
/// ImmediateEmission)。
///
/// **M1 stub**: 常に `Err(RenderError::Unimplemented { feature: "render_streaming", .. })`
/// を返す。本実装は M2 pagestream-state-machine で。
///
/// spec §L1084 の signature 準拠。
#[allow(clippy::result_large_err)]
pub fn render_streaming(
    _doc: &HtmlDocument,
    _defaults: PageDefaults,
    _resolver: &dyn ReplacedResolver,
    _config: StreamingConfig,
    _sink: &mut dyn RenderSink,
) -> Result<RenderStatus, RenderError> {
    Err(RenderError::Unimplemented {
        feature: "render_streaming",
        migration_hint: "M1 では html_to_png (m1.14) 経路のみ動作、render_streaming は M2 pagestream で実装",
    })
}
