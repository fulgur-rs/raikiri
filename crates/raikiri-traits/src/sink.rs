//! RenderSink trait: consumer-side receiver for emitted pages.

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

/// Consumer-side render output receiver (Finding #4 completion protocol).
///
/// `accept_page` is called whenever a page is finalized:
/// - Streaming preset: immediately (`ImmediateEmission`)
/// - Batch preset: together after all layout is complete (`DeferredEmission`)
///
/// After all `accept_page` calls finish, `finish_render` sends the final notification.
///
/// Releasing consumer-side resources (such as writing a PDF trailer) is not the
/// responsibility of `finish_render`; the consumer calls `sink.finalize_pdf()` separately.
pub trait RenderSink: Send {
    /// Called as soon as a page is finalized.
    fn accept_page(&mut self, page: PageFragment) -> std::io::Result<()>;

    /// Final notification called by `render()` after all `accept_page` calls complete.
    ///
    /// Deliver the final TargetRegistry state to the consumer via `summary`, giving it
    /// a chance to patch unresolved slots (§4.x completion protocol).
    fn finish_render(&mut self, summary: RenderSummary) -> std::io::Result<()>;
}
