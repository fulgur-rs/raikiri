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
//! M1 では両者とも Consumer に migration hint を返す (m1.14 hello-world-vrt で
//! 単一ページ用の `html_to_png` が実装される予定、multi-page streaming は M2
//! pagestream-state-machine 完了後)。

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
        migration_hint: "M1 では render_streaming は non-goal。単一ページ raster は m1.14 で `html_to_png` として実装予定、multi-page streaming は M2 pagestream-state-machine で実装",
    })
}
