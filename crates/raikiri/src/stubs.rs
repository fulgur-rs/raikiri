//! `plan` / `render_streaming` の 未実装 API。
//!
//! この module 内の全 fn は `RenderError::Unimplemented` を返す。
//! **pagination 実装完了時に、本 module を丸ごと削除して parse.rs / dedicated
//! plan.rs / render_streaming.rs に本実装を配置し直す** (retire scope isolation)。
//!
//! - `plan`: pagination 完了後に populate、PlanConfig の initial_registry と
//!   併せて DocumentPlan を返す本実装に差し替え
//! - `render_streaming`: pagestream state machine 実装後に本実装、`RenderSink` に
//!   PageFragment を stream 出力する
//!
//! 現状は両者とも Consumer に migration hint を返す (単一ページ用の
//! `html_to_png` は実装済み、multi-page streaming は pagestream state
//! machine の実装完了後)。

use raikiri_traits::{
    DocumentPlan, PageDefaults, PlanConfig, RenderError, RenderSink, RenderStatus,
    ReplacedResolver, StreamingConfig,
};

use crate::HtmlDocument;

/// Plan mode (dry-run: parse+cascade+layout planning のみ、PaintedBox 構築なし)。
///
/// **Unavailable implementation**: 常に `Err(RenderError::Unimplemented { feature: "plan", .. })` を
/// 返す。本実装は pagination 完了後。
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
        migration_hint: "non-goal for now; populated once pagination is implemented",
    })
}

/// Streaming rendering (1 pass、BoundedLookahead + PlaceholderTargetResolver +
/// ImmediateEmission)。
///
/// **Unavailable implementation**: 常に `Err(RenderError::Unimplemented { feature: "render_streaming", .. })`
/// を返す。本実装は pagestream state machine 実装後。
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
        migration_hint: "render_streaming is a non-goal for now. Single-page raster is implemented via `html_to_png`; multi-page streaming lands once the pagestream state machine is implemented",
    })
}
