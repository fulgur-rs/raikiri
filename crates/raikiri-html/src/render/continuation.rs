//! Later pages whose content width differs from the pages before them.
//!
//! The paginator lays the document out once, at the content width of its
//! first page, and slices that flow into pages. When `@page` rules give a
//! later page another content width (for example `@page :first` with wider
//! margins), the pages from that one on are taken from a copy of the
//! document laid out at their own width. The copy resumes at the source
//! position where the page began in the earlier layout: a block box, or a
//! paragraph line that a forced break in the copy also starts.

use raikiri_dom::{
    Document, PageLayoutControl, PageSlice, PageStart, PageStartToken,
    layout_pages_with_page_geometry_and_resolver_and_base_url_and_control,
};
use raikiri_style::{CascadeOptions, CascadeResult, MediaContext, RuleTree, cascade_with_options};
use raikiri_traits::{LayoutError, ReplacedResolver};

use super::{PageCascader, ResolvedPageGeometry, content_width_for_geometry, page_query_for_slice};

/// Pages laid out again at their own content width.
pub(crate) struct Continuation {
    /// Index of the first page taken from this layout.
    pub(crate) first_page: u32,
    pub(crate) document: Document,
    pub(crate) cascade: CascadeResult,
}

/// Inputs shared by every relayout of one pipeline run.
pub(super) struct ContinuationInputs<'a> {
    /// The document as it was before its first layout.
    pub(super) pristine: &'a Document,
    pub(super) source: &'a Document,
    pub(super) tree: &'a RuleTree,
    pub(super) media_context: &'a MediaContext,
    /// The options of the run's element cascade.
    pub(super) cascade_options: &'a CascadeOptions,
    pub(super) cascader: &'a PageCascader<'a>,
    pub(super) defaults: &'a raikiri_traits::PageDefaults,
    pub(super) resolver: &'a dyn ReplacedResolver,
    pub(super) base_url: Option<&'a url::Url>,
    pub(super) max_pages: Option<u32>,
    pub(super) abort_check: &'a dyn Fn() -> bool,
}

/// Content widths closer than this are the same width.
const WIDTH_TOLERANCE: f32 = 0.01;
/// Where a relayout's page start may land from where the earlier layout had it.
const POSITION_TOLERANCE: f32 = 0.5;
/// The height of the single page the relayout is first measured on.
const MEASURE_HEIGHT: f32 = 1.0e6;
/// Relayouts of one continuation before its page schedule must be stable.
const MAX_SCHEDULE_PASSES: usize = 3;

/// The pages of one layout, with its document and cascade.
struct Segment<'d> {
    document: &'d Document,
    cascade: &'d CascadeResult,
}

/// Relay out every page run whose content width differs from the run before it.
///
/// `slices` and `geometries` describe the pages of `document`, which must
/// already be projected onto them. On return they describe the final pages;
/// pages from each continuation's `first_page` on come from that
/// continuation. A page start that cannot be resumed ends the search, and
/// the remaining pages keep the width of the layout before them.
pub(super) fn continue_at_page_widths(
    inputs: &ContinuationInputs<'_>,
    document: &Document,
    cascade: &CascadeResult,
    slices: &mut Vec<PageSlice>,
    geometries: &mut Vec<ResolvedPageGeometry>,
) -> Result<Vec<Continuation>, LayoutError> {
    let mut continuations: Vec<Continuation> = Vec::new();
    let mut run_start = 0_usize;
    loop {
        let run_width = content_width_for_geometry(geometries[run_start]);
        let Some(page) = (run_start + 1..slices.len()).find(|&page| {
            (content_width_for_geometry(geometries[page]) - run_width).abs() > WIDTH_TOLERANCE
        }) else {
            break;
        };
        let segment = match continuations.last() {
            Some(continuation) => Segment {
                document: &continuation.document,
                cascade: &continuation.cascade,
            },
            None => Segment { document, cascade },
        };
        let Some(start) = segment.document.page_start(segment.cascade, page as u32) else {
            break;
        };
        let Some((continuation, pages)) = relayout_from(inputs, slices, geometries, page, start)?
        else {
            break;
        };
        slices.truncate(page);
        slices.extend(pages);
        *geometries = super::resolve_page_geometries(inputs.cascader, inputs.defaults, slices).0;
        continuations.push(continuation);
        run_start = page;
    }
    Ok(continuations)
}

/// Lay a copy of the document out at the width of page `page` and resume it
/// at `start`. Returns the copy and its pages, numbered from `page`, or
/// `None` when the copy does not reproduce the page start.
fn relayout_from(
    inputs: &ContinuationInputs<'_>,
    slices: &[PageSlice],
    geometries: &[ResolvedPageGeometry],
    page: usize,
    start: PageStart,
) -> Result<Option<(Continuation, Vec<PageSlice>)>, LayoutError> {
    let geometry = geometries[page];
    // The run's element cascade already succeeded on the same input with the
    // same options, and the cascade's limits count only its input.
    let mut cascade = cascade_with_options(
        inputs.source,
        inputs.tree,
        inputs.media_context,
        &page_query_for_slice(&slices[page]),
        inputs.cascade_options,
    )
    .expect("cascade は常に Ok のはず");
    let mut document = inputs.pristine.clone();
    if let PageStartToken::Line { text, offset } = start.token {
        document.set_continuation_break(Some((text, offset)));
    }
    document.prepare_continuation_cascade(&mut cascade, start.token);

    // Measure where the copy places the page start, on one page tall
    // enough to hold everything before it.
    let measure_control = PageLayoutControl::for_geometry_discovery(inputs.max_pages)
        .with_abort_check(inputs.abort_check);
    let measured = layout_pages_with_page_geometry_and_resolver_and_base_url_and_control(
        &mut document,
        &cascade,
        geometry.page_box,
        &[MEASURE_HEIGHT],
        &[],
        inputs.resolver,
        inputs.base_url,
        &measure_control,
    )?; // cov:ignore: only a cancellation landing in this pass fails it; the first layout already met the same work limits
    let mut measure_box = geometry.page_box;
    measure_box.height = MEASURE_HEIGHT + geometry.page_box.height;
    let measure_geometries: Vec<_> = measured
        .iter()
        .map(|_| (measure_box, geometry.margins, geometry.content_insets))
        .collect();
    document.project_pages_with_control(
        &cascade,
        measure_box,
        &measured,
        &measure_geometries,
        &measure_control,
    )?; // cov:ignore: only a cancellation landing in this pass fails it; the first projection already met the same work limits
    // The token was found on the earlier layout's page, and the copy breaks
    // no line or box before it, so the measuring page holds it too.
    let Some(token_y) = document.page_token_offset(0, start.token) else {
        return Ok(None); // cov:ignore: defensive; the copy lays out the same source before the token
    };
    // The copy's first page ends where the resumed page begins, so that the
    // resumed page holds the token at the same offset as before. Without
    // content before that point the copy needs no such page.
    let lead = token_y - start.offset;
    let lead = (lead > POSITION_TOLERANCE).then_some(lead);
    let skipped = usize::from(lead.is_some());

    let width = content_width_for_geometry(geometry);
    let schedule = |heights: &[ResolvedPageGeometry]| -> Vec<f32> {
        lead.into_iter()
            .chain(heights.iter().map(|geometry| geometry.content_box.height))
            .collect()
    };
    let mut steps = schedule(&geometries[page..]);
    let control = PageLayoutControl::new(inputs.max_pages.map(|limit| {
        limit
            .saturating_sub(page as u32)
            .saturating_add(skipped as u32)
    }))
    .with_abort_check(inputs.abort_check);
    // The copy counts its pages from its own first page; a page limit is
    // reported against the pages of the whole document.
    let document_pages = |error| match (error, inputs.max_pages) {
        (LayoutError::PageLimitExceeded { actual, .. }, Some(limit)) => {
            LayoutError::PageLimitExceeded {
                limit,
                actual: actual - skipped as u64 + page as u64,
            }
        }
        (error, _) => error, // cov:ignore: other layout errors already name no page count
    };
    for _ in 0..MAX_SCHEDULE_PASSES {
        let widths = vec![width; steps.len()];
        let laid_out = layout_pages_with_page_geometry_and_resolver_and_base_url_and_control(
            &mut document,
            &cascade,
            geometry.page_box,
            &steps,
            &widths,
            inputs.resolver,
            inputs.base_url,
            &control,
        )
        .map_err(document_pages)?;
        let resumed_origin = laid_out.get(skipped).map(|slice| slice.content_origin_y);
        if resumed_origin
            .is_none_or(|origin| (origin - lead.unwrap_or(0.0)).abs() > POSITION_TOLERANCE)
        {
            return Ok(None); // cov:ignore: defensive; the lead page ends at a line or box edge, so the copy breaks there
        }
        let pages: Vec<PageSlice> = laid_out[skipped..]
            .iter()
            .enumerate()
            .map(|(index, slice)| PageSlice {
                page_index: page as u32 + index as u32,
                content_origin_y: slice.content_origin_y,
                page_name: slice.page_name.clone(),
            })
            .collect();
        let mut all_pages = slices[..page].to_vec();
        all_pages.extend(pages.iter().cloned());
        let all_geometries =
            super::resolve_page_geometries(inputs.cascader, inputs.defaults, &all_pages).0;
        let resumed = &all_geometries[page..];
        let next_steps = schedule(resumed);
        // Pages of another width are relaid out by the next continuation, so
        // only heights have to match the schedule this layout used.
        let stable = next_steps.len() <= steps.len()
            && next_steps
                .iter()
                .zip(&steps)
                .all(|(next, used)| (next - used).abs() <= WIDTH_TOLERANCE);
        if !stable {
            steps = next_steps;
            continue;
        }
        let projection: Vec<_> = resumed
            .iter()
            .map(|geometry| (geometry.page_box, geometry.margins, geometry.content_insets))
            .collect();
        document.project_pages_with_control(
            &cascade,
            geometry.page_box,
            &pages,
            &projection,
            &control,
        )?; // cov:ignore: only a cancellation landing in this pass fails it; the first projection already met the same work limits
        let resumed_offset = document.page_token_offset(page as u32, start.token);
        if resumed_offset.is_none_or(|offset| (offset - start.offset).abs() > POSITION_TOLERANCE) {
            return Ok(None); // cov:ignore: defensive; the resumed page starts at the token's measured position
        }
        return Ok(Some((
            Continuation {
                first_page: page as u32,
                document,
                cascade,
            },
            pages,
        )));
    }
    // cov:ignore: no fixture keeps a page schedule changing through every pass
    Ok(None)
}
