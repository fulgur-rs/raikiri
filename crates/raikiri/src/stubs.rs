//! Entry points that are not yet backed by the full planning state machine.
//!
//! `plan` remains an explicit unavailable API. `render_streaming` is the
//! neutral page-output bridge: it uses the merged `raikiri-dom` pagination
//! projection and keeps renderer-specific scene and drawable code out of the
//! sink contract.

use parley::FontContext;
use raikiri_dom::{layout_pages_with_resolver, page_fragments_from_slices};
use raikiri_traits::{
    DocumentPlan, PageBox, PageDefaults, PlanConfig, RenderError, RenderSink, RenderStatus,
    RenderStatus::Aborted, RenderStatus::Completed, RenderSummary, ReplacedResolver,
    StreamingConfig,
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

/// Stream neutral page snapshots to a consumer sink.
///
/// The current driver lays out the document into neutral snapshots before
/// emitting them. This keeps the public contract renderer-neutral while the
/// pagination state machine grows: no Taffy, Parley, style, scene, drawable,
/// or PDF value crosses the sink boundary. The input document is cloned for
/// the mutating layout pass, so the existing shared `&HtmlDocument` API stays
/// source-compatible.
///
/// A configured [`AbortSignal`](raikiri_traits::AbortSignal) is checked before
/// layout, before every page, and before completion. Aborted renders return
/// without calling `RenderSink::finish_render`.
#[allow(clippy::result_large_err)]
pub fn render_streaming(
    doc: &HtmlDocument,
    defaults: PageDefaults,
    resolver: &dyn ReplacedResolver,
    config: StreamingConfig,
    sink: &mut dyn RenderSink,
) -> Result<RenderStatus, RenderError> {
    let signal = config.signal.clone();
    let is_aborted = || signal.as_ref().is_some_and(|signal| signal.is_aborted());
    if is_aborted() {
        return Ok(Aborted { partial_pages: 0 });
    }

    let page_box = doc
        .cascade
        .page
        .size()
        .map(|size| PageBox::from_page_size(Some(size)))
        .unwrap_or(defaults.page_box);
    let mut document = doc.uncascaded.dom.clone();
    let slices = layout_pages_with_resolver(
        &mut document,
        &doc.cascade,
        page_box,
        FontContext::new(),
        resolver,
    )
    .map_err(RenderError::from)?;
    let pages = page_fragments_from_slices(&document, &doc.cascade, page_box, &slices);

    let total_pages = u32::try_from(pages.len()).unwrap_or(u32::MAX);
    if config
        .limits
        .max_document_pages
        .is_some_and(|limit| total_pages > limit)
    {
        return Err(RenderError::LimitExceeded {
            kind: raikiri_traits::LimitKind::Pages,
            limit: config.limits.max_document_pages.unwrap_or(u32::MAX) as u64,
            actual: total_pages as u64,
        });
    }

    let mut emitted_pages = 0_u32;
    for page in pages {
        if is_aborted() {
            return Ok(Aborted {
                partial_pages: emitted_pages,
            });
        }
        sink.accept_page(page).map_err(RenderError::Sink)?;
        emitted_pages = emitted_pages.saturating_add(1);
    }
    if is_aborted() {
        return Ok(Aborted {
            partial_pages: emitted_pages,
        });
    }

    let summary = RenderSummary {
        total_pages: emitted_pages,
        target_registry: config.initial_registry.unwrap_or_default(),
        unresolved_targets: Vec::new(),
        emitted_target_slots: Vec::new(),
        target_discrepancies: Vec::new(),
        warnings: doc.uncascaded.warnings.clone(),
    };
    sink.finish_render(summary.clone())
        .map_err(RenderError::Sink)?;
    Ok(Completed(summary))
}
