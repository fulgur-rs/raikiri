//! Entry points that are not yet backed by the full planning state machine.
//!
//! `plan` remains an explicit unavailable API. `render_streaming` is the
//! neutral page-output bridge: it uses the merged `raikiri-dom` pagination
//! projection and keeps renderer-specific scene and drawable code out of the
//! sink contract.

use parley::FontContext;
use raikiri_dom::{
    PageSlice, first_page_name, layout_pages_with_page_geometry_and_resolver,
    layout_pages_with_resolver, page_fragment_events_from_pages,
    page_fragments_from_slices_with_page_geometry, resolve_page_fragment_geometry,
};
use raikiri_traits::{
    DocumentPlan, PageBox, PageDefaults, PageEventObserver, PageFragmentPageGeometry, PlanConfig,
    RenderError, RenderSink, RenderStatus, RenderStatus::Aborted, RenderStatus::Completed,
    RenderSummary, ReplacedResolver, StreamingConfig,
};
use std::collections::BTreeMap;

use crate::{
    Atom, HtmlDocument, MediaContext, PageContextQuery, build_cascaded_with_media_context_for_page,
};

/// Build the page-context query used by the neutral page stream.
///
/// The first page is treated as recto (`:right`) by default. Blank-page state
/// is not inferred here because the current `PageSlice` contract does not
/// expose blank-page insertion; that remains an explicit pagination follow-up.
fn page_query_for_slice(slice: &PageSlice) -> PageContextQuery {
    let mut query = PageContextQuery::default();
    query.page_name = slice.page_name.as_deref().map(Atom::from);
    query.is_first = slice.page_index == 0;
    query.is_left = slice.page_index % 2 == 1;
    query.is_right = !query.is_left;
    query
}

fn page_box_for_cascade(
    cascade: &raikiri_style::CascadeResult,
    defaults: &PageDefaults,
) -> PageBox {
    cascade
        .page
        .size()
        .map(|size| PageBox::from_page_size(Some(size)))
        .unwrap_or(defaults.page_box)
}

fn resolve_page_geometries(
    doc: &HtmlDocument,
    defaults: &PageDefaults,
    slices: &[PageSlice],
) -> Vec<PageFragmentPageGeometry> {
    slices
        .iter()
        .map(|slice| {
            let query = page_query_for_slice(slice);
            let cascade = build_cascaded_with_media_context_for_page(
                &doc.uncascaded,
                &MediaContext::default(),
                &query,
            );
            let page_box = page_box_for_cascade(&cascade, defaults);
            resolve_page_fragment_geometry(&cascade, page_box, slice.page_index)
        })
        .collect()
}

fn content_width_for_geometry(geometry: PageFragmentPageGeometry) -> f32 {
    (geometry.page_box.width - geometry.margins.left - geometry.margins.right).max(0.0)
}

#[derive(Debug, Clone, PartialEq)]
struct PageGeometrySchedule {
    page_steps: Vec<f32>,
    page_widths: Vec<f32>,
    page_names: Vec<Option<String>>,
}

fn page_geometry_schedule(
    page_geometries: &[PageFragmentPageGeometry],
    slices: &[PageSlice],
) -> PageGeometrySchedule {
    PageGeometrySchedule {
        page_steps: page_geometries
            .iter()
            .map(|geometry| geometry.content_box.height)
            .collect(),
        page_widths: page_geometries
            .iter()
            .map(|geometry| content_width_for_geometry(*geometry))
            .collect(),
        page_names: slices.iter().map(|slice| slice.page_name.clone()).collect(),
    }
}

fn geometry_differs(left: PageFragmentPageGeometry, right: PageFragmentPageGeometry) -> bool {
    left.page_box != right.page_box
        || left.margins != right.margins
        || left.content_insets != right.content_insets
        || left.content_box != right.content_box
        || left.orientation != right.orientation
}

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
/// without calling `RenderSink::finish_render`. If the bounded page-geometry
/// schedule does not converge, [`RenderError::PageGeometryDidNotConverge`]
/// is returned before any page is emitted.
#[allow(clippy::result_large_err)]
pub fn render_streaming(
    doc: &HtmlDocument,
    defaults: PageDefaults,
    resolver: &dyn ReplacedResolver,
    config: StreamingConfig,
    sink: &mut dyn RenderSink,
) -> Result<RenderStatus, RenderError> {
    render_streaming_inner(doc, defaults, resolver, config, sink, None)
}

/// Stream neutral page snapshots and page-local link/annotation events.
///
/// This is the opt-in observer variant of [`render_streaming`]. The page
/// stream remains renderer-neutral, and the observer receives only opaque
/// [`NodeId`](raikiri_traits::NodeId), page indices, CSS-pixel rectangles, and
/// link values. Events are delivered after the corresponding
/// [`RenderSink::accept_page`] call. Observer I/O failures are returned as
/// [`RenderError::Sink`]; the page may already have been accepted and
/// `finish_render` is skipped. Successful renders still call
/// [`RenderSink::finish_render`] exactly once.
#[allow(clippy::result_large_err)]
pub fn render_streaming_with_observer(
    doc: &HtmlDocument,
    defaults: PageDefaults,
    resolver: &dyn ReplacedResolver,
    config: StreamingConfig,
    sink: &mut dyn RenderSink,
    observer: &mut dyn PageEventObserver,
) -> Result<RenderStatus, RenderError> {
    render_streaming_inner(doc, defaults, resolver, config, sink, Some(observer))
}

#[allow(clippy::result_large_err)]
fn render_streaming_inner(
    doc: &HtmlDocument,
    defaults: PageDefaults,
    resolver: &dyn ReplacedResolver,
    config: StreamingConfig,
    sink: &mut dyn RenderSink,
    mut observer: Option<&mut dyn PageEventObserver>,
) -> Result<RenderStatus, RenderError> {
    let signal = config.signal.clone();
    let is_aborted = || signal.as_ref().is_some_and(|signal| signal.is_aborted());
    if is_aborted() {
        return Ok(Aborted { partial_pages: 0 });
    }

    // Resolve the first page context before layout so `:first` and the first
    // resolved `@page size` participate in the initial fragmentainer.
    let mut first_query = PageContextQuery::default();
    first_query.is_first = true;
    first_query.is_right = true;
    let mut first_cascade = build_cascaded_with_media_context_for_page(
        &doc.uncascaded,
        &MediaContext::default(),
        &first_query,
    );
    // The first class-A box can select a named page. Resolve that name before
    // the initial layout so a named `:first` page is not flattened to the
    // anonymous page geometry.
    if let Some(name) = first_page_name(&doc.uncascaded.dom, &first_cascade) {
        first_query.page_name = Some(Atom::from(name.as_str()));
        first_cascade = build_cascaded_with_media_context_for_page(
            &doc.uncascaded,
            &MediaContext::default(),
            &first_query,
        );
    }
    let page_box = page_box_for_cascade(&first_cascade, &defaults);
    let mut document = doc.uncascaded.dom.clone();
    let mut slices = layout_pages_with_resolver(
        &mut document,
        &first_cascade,
        page_box,
        FontContext::new(),
        resolver,
    )
    .map_err(RenderError::from)?;
    const MAX_PAGE_GEOMETRY_PASSES: u32 = 3;
    let mut page_geometries = resolve_page_geometries(doc, &defaults, &slices);
    let mut geometry_converged = true;
    for pass in 0..MAX_PAGE_GEOMETRY_PASSES {
        let Some(first_geometry) = page_geometries.first().copied() else {
            break; // cov:ignore: a successful document with a body always emits a page slice.
        };
        let geometry_varies = page_geometries
            .iter()
            .any(|geometry| geometry_differs(*geometry, first_geometry));
        if !geometry_varies {
            break;
        }

        let schedule = page_geometry_schedule(&page_geometries, &slices);
        // The first pagination pass establishes page count, names, and source
        // coordinates. If page selectors resolve different used geometry, rerun
        // the existing scheduled paginator with producer-owned page steps/widths.
        // This remains a bounded batch layout pass; a changed page count or page
        // name can trigger another schedule pass.
        slices = layout_pages_with_page_geometry_and_resolver(
            &mut document,
            &first_cascade,
            page_box,
            FontContext::new(),
            &schedule.page_steps,
            &schedule.page_widths,
            resolver,
        )
        .map_err(RenderError::from)?;
        // A scheduled pass can change both page count and page selectors. Re-
        // resolve before the next iteration so the following schedule is
        // derived from the slices it will actually replace.
        page_geometries = resolve_page_geometries(doc, &defaults, &slices);
        let refreshed_schedule = page_geometry_schedule(&page_geometries, &slices);
        if refreshed_schedule == schedule {
            break;
        }
        // cov:ignore: no current public fixture can keep a static page-rule schedule
        // changing through all three bounded passes; the terminal status has a
        // direct contract test in raikiri-traits.
        if pass + 1 == MAX_PAGE_GEOMETRY_PASSES {
            geometry_converged = false;
        }
    }
    // cov:ignore: see the bounded non-convergence branch above; no inconsistent
    // pages are emitted, and the structured error variant is contract-tested.
    if !geometry_converged {
        return Err(RenderError::PageGeometryDidNotConverge {
            iterations: MAX_PAGE_GEOMETRY_PASSES,
        });
    }

    // Resolve once more after the final bounded schedule pass so metadata and
    // page names always describe the slices that will actually be emitted.
    page_geometries = resolve_page_geometries(doc, &defaults, &slices);

    let pages = page_fragments_from_slices_with_page_geometry(
        &document,
        &first_cascade,
        page_box,
        &slices,
        &page_geometries,
    );

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

    let mut events_by_page = BTreeMap::new();
    if observer.is_some() {
        for event in page_fragment_events_from_pages(&document, &pages) {
            let page_index = match &event {
                raikiri_traits::PageFragmentEvent::Link(event) => event.page_index,
                _ => continue, // cov:ignore: future non-exhaustive event variant cannot be constructed here
            };
            events_by_page
                .entry(page_index)
                .or_insert_with(Vec::new)
                .push(event);
        }
    }

    let mut emitted_pages = 0_u32;
    for page in pages {
        if is_aborted() {
            return Ok(Aborted {
                partial_pages: emitted_pages,
            });
        }
        let page_index = page.page_index;
        sink.accept_page(page).map_err(RenderError::Sink)?;
        if let (Some(observer), Some(events)) =
            (observer.as_deref_mut(), events_by_page.remove(&page_index))
        {
            for event in events {
                observer.observe_event(event).map_err(RenderError::Sink)?;
            }
        }
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
