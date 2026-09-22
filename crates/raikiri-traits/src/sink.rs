//! RenderSink trait — Consumer 側の page emission receiver。

use crate::error::RenderSummary;
use crate::page::{PageFragment, PageFragmentEvent}; // cov:ignore: type-only import
use crate::paint::PagePaintPayload; // cov:ignore: type-only import

/// Optional receiver for neutral page-local link events.
///
/// The event path is separate from [`RenderSink`]'s page emission path so
/// existing sinks remain source-compatible. A caller that does not provide an
/// observer receives the same page-only behavior as before.
// cov:ignore: observer trait declaration has no executable body
pub trait PageEventObserver: Send {
    /// Receive one deterministic page-local event.
    fn observe_event(&mut self, event: PageFragmentEvent) -> std::io::Result<()>; // cov:ignore: trait signature has no executable body
}

/// Optional receiver for the additive neutral paint payload prototype.
///
/// This trait is intentionally separate from [`RenderSink`]: existing geometry
/// consumers do not need to accept paint, and a producer may add paint without
/// changing the geometry/event contracts. `accept_paint` takes ownership of an
/// entire page payload, including its shared resource bundle, so the consumer
/// may retain or serialize it after the callback returns. A future producer
/// calls [`Self::finish_paint`] exactly once after all successful payload
/// callbacks; it skips completion after an acceptance error or abort. The
/// current `render_streaming` API does not call this trait.
// cov:ignore: paint sink trait declaration has no executable body
pub trait PagePaintSink: Send {
    /// Receive one owned, page-local paint payload.
    fn accept_paint(&mut self, payload: PagePaintPayload) -> std::io::Result<()>; // cov:ignore: trait signature has no executable body

    /// Complete a successful paint stream.
    fn finish_paint(&mut self) -> std::io::Result<()>; // cov:ignore: trait signature has no executable body
}

/// Consumer 側 render output receiver (Finding #4 completion protocol)。
///
/// 1 ページ確定ごとに `accept_page` が呼ばれる:
/// - Streaming preset: 逐次 (`ImmediateEmission`)
/// - Batch preset: 全 layout 完了後まとめて (`DeferredEmission`)
///
/// 全 `accept_page` 呼び出し完了後、`finish_render` が最終通知として呼ばれる。
///
/// Consumer 側 resource 解放 (PDF trailer 書出 等) は `finish_render` の責務外で、
/// Consumer が別途 `sink.finalize_pdf()` などを呼び出す。
pub trait RenderSink: Send {
    /// 1 ページ確定次第呼ばれる。
    fn accept_page(&mut self, page: PageFragment) -> std::io::Result<()>;

    /// 全 `accept_page` 完了後、`render()` が呼ぶ最終通知。
    ///
    /// `summary` で TargetRegistry の最終状態を Consumer に届け、Consumer は
    /// 未解決 slot を patch する機会を得る (§4.x completion protocol)。
    fn finish_render(&mut self, summary: RenderSummary) -> std::io::Result<()>;
}
