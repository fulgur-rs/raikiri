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
    CrossingBox, Document, PageLayoutControl, PageSlice, PageStart, PageStartToken,
    layout_pages_with_page_geometry_and_resolver_and_base_url_and_control,
};
use raikiri_style::CascadeResult;
use raikiri_style::property::PageValue;
use raikiri_traits::{LayoutError, ReplacedResolver};

use raikiri_dom::ResolvedPageGeometry;

/// Pages laid out again at their own content width.
#[doc(hidden)]
pub struct Continuation {
    /// Index of the first page taken from this layout.
    pub first_page: u32,
    /// The copy of the document these pages are laid out in.
    pub document: Document,
    /// The element cascade of `document`, with the page context of
    /// `first_page`.
    pub cascade: CascadeResult,
}

/// Inputs shared by every relayout of one layout run.
#[doc(hidden)]
pub struct ContinuationInputs<'a> {
    /// The document as it was before its first layout.
    pub pristine: &'a Document,
    /// The element cascade of the document with the page context of a page.
    /// Every relayout runs it on the same input that the run's own element
    /// cascade succeeded on.
    pub cascade_for_page: &'a dyn Fn(&PageSlice) -> CascadeResult,
    /// The geometry of each page of a page list.
    pub page_geometries: &'a dyn Fn(&[PageSlice]) -> Vec<ResolvedPageGeometry>,
    pub resolver: &'a dyn ReplacedResolver,
    pub base_url: Option<&'a url::Url>,
    pub max_pages: Option<u32>,
    pub abort_check: &'a dyn Fn() -> bool,
    /// Elements that every relayout lays out as `display: none`: content
    /// wholly before the pages being laid out (see
    /// [`raikiri_dom::Document::skip_layout`]).
    pub skipped: &'a [usize],
}

fn content_width(geometry: ResolvedPageGeometry) -> f32 {
    geometry.content_box.width
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
/// continuation. A page whose start cannot be named, such as one that only
/// continues a tall box or starts inside columns, keeps the width of the
/// layout before it, and the next page of another width whose start can be
/// named is resumed instead. A start that cannot be resumed ends the search,
/// and the remaining pages keep the width of the layout before them.
#[doc(hidden)]
pub fn continue_at_page_widths(
    inputs: &ContinuationInputs<'_>,
    document: &Document,
    cascade: &CascadeResult,
    slices: &mut Vec<PageSlice>,
    geometries: &mut Vec<ResolvedPageGeometry>,
) -> Result<Vec<Continuation>, LayoutError> {
    continue_from(inputs, document, cascade, slices, geometries, Vec::new(), 0)
}

/// Lay the document out from page `page`, which begins at `start`, and
/// relay out every later page run of another content width, as
/// [`continue_at_page_widths`] does.
///
/// `slices` and `geometries` describe pages `0..=page`; the pages before
/// `page` are left as they are, without content. On success they describe
/// every page and the continuations hold the content of the pages from
/// `page` on. `earlier` holds the boxes crossing `start` in the earlier
/// layout, which the relayout continues. Returns `None` when the relayout does not reproduce the
/// page start.
#[allow(clippy::too_many_arguments)]
pub(super) fn resume_at(
    inputs: &ContinuationInputs<'_>,
    document: &Document,
    cascade: &CascadeResult,
    slices: &mut Vec<PageSlice>,
    geometries: &mut Vec<ResolvedPageGeometry>,
    page: usize,
    start: PageStart,
    earlier: &[CrossingBox],
) -> Result<Option<Vec<Continuation>>, LayoutError> {
    let Some((continuation, pages)) =
        relayout_from(inputs, slices, geometries, page, start, Some(earlier), None)?
    else {
        return Ok(None);
    };
    slices.truncate(page);
    slices.extend(pages);
    *geometries = (inputs.page_geometries)(slices);
    continue_from(
        inputs,
        document,
        cascade,
        slices,
        geometries,
        vec![continuation],
        page,
    )
    .map(Some)
}

/// [`continue_at_page_widths`] from the page run starting at `run_start`,
/// whose pages come from the last of `continuations`, or from `document`
/// when there is none.
fn continue_from(
    inputs: &ContinuationInputs<'_>,
    document: &Document,
    cascade: &CascadeResult,
    slices: &mut Vec<PageSlice>,
    geometries: &mut Vec<ResolvedPageGeometry>,
    mut continuations: Vec<Continuation>,
    mut run_start: usize,
) -> Result<Vec<Continuation>, LayoutError> {
    loop {
        let run_width = content_width(geometries[run_start]);
        let segment = match continuations.last() {
            Some(continuation) => Segment {
                document: &continuation.document,
                cascade: &continuation.cascade,
            },
            None => Segment { document, cascade },
        };
        let other_widths = (run_start + 1..slices.len())
            .filter(|&page| (content_width(geometries[page]) - run_width).abs() > WIDTH_TOLERANCE)
            .map(|page| page as u32);
        let Some((page, start)) = segment
            .document
            .first_page_start(segment.cascade, other_widths)
        else {
            break;
        };
        let page = page as usize;
        let before =
            segment
                .document
                .page_content_before(segment.cascade, page as u32, start.token);
        // Continue only the body from the earlier layout: it spans the
        // paginated content, so its fragments go on across the resumed page.
        // Other boxes crossing the start begin anew in the copy.
        let body = raikiri_dom::paint_rules::find_paint_root(segment.document);
        let crossing: Vec<CrossingBox> = segment
            .document
            .crossing_boxes(page as u32)
            .into_iter()
            .filter(|crossing| Some(crossing.node.0 as usize) == body)
            .collect();
        let Some((continuation, pages)) = relayout_from(
            inputs,
            slices,
            geometries,
            page,
            start,
            Some(&crossing),
            Some(&before),
        )?
        else {
            break;
        };
        slices.truncate(page);
        slices.extend(pages);
        *geometries = (inputs.page_geometries)(slices);
        continuations.push(continuation);
        run_start = page;
    }
    Ok(continuations)
}

/// The body's fragments on the pages that `continuations` lay out, out of
/// `page_count` pages, as [`Document::continue_fragments_after`] takes them
/// for the layout before the first continuation.
#[doc(hidden)]
pub fn body_fragments_after(
    document: &Document,
    continuations: &[Continuation],
    page_count: usize,
) -> Option<(raikiri_traits::NodeId, u32)> {
    let body = raikiri_dom::paint_rules::find_paint_root(document)?;
    let body = raikiri_traits::NodeId::new(body as u64);
    let count = continuations
        .iter()
        .enumerate()
        .flat_map(|(index, continuation)| {
            let end = continuations
                .get(index + 1)
                .map_or(page_count, |next| next.first_page as usize);
            (continuation.first_page as usize..end).map(move |page| (continuation, page as u32))
        })
        .filter(|(continuation, page)| {
            continuation
                .document
                .page_fragments(*page)
                .any(|fragment| fragment.node() == body)
        })
        .count();
    Some((body, count as u32))
}

/// Lay a copy of the document out at the width of page `page` and resume it
/// at `start`. Returns the copy and its pages, numbered from `page`, or
/// `None` when the copy does not reproduce the page start, or puts content
/// before the token on that page that `content_before`, the earlier
/// layout's, does not have.
#[allow(clippy::too_many_arguments)]
fn relayout_from(
    inputs: &ContinuationInputs<'_>,
    slices: &[PageSlice],
    geometries: &[ResolvedPageGeometry],
    page: usize,
    start: PageStart,
    earlier: Option<&[CrossingBox]>,
    content_before: Option<&[usize]>,
) -> Result<Option<(Continuation, Vec<PageSlice>)>, LayoutError> {
    let geometry = geometries[page];
    // The run's element cascade already succeeded on the same input with the
    // same options, and the cascade's limits count only its input.
    let mut cascade = (inputs.cascade_for_page)(&slices[page]);
    let mut document = inputs.pristine.clone();
    document.skip_layout(&mut cascade, inputs.skipped);
    if let PageStartToken::Line { text, offset } = start.token {
        document.set_continuation_break(Some((text, offset)));
    }
    document.prepare_continuation_cascade(&mut cascade, start.token);

    // Measure where the copy places the page start, on one page tall
    // enough to hold everything before it. Page names would force a break
    // before the token, such as the change to its own named page, and move
    // it off the measuring page, so the measurement ignores them for
    // breaking. The page the measurement ends then starts at that break,
    // which the later layout, with the names, places there again.
    let page_values = std::mem::replace(
        &mut cascade.page_values,
        vec![PageValue::Auto; document.node_count()],
    );
    // The changes of named page still give the boxes after them clearance,
    // as in the later layout.
    document.set_clearance_page_values(Some(page_values.clone()));
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
    document.set_clearance_page_values(None);
    cascade.page_values = page_values;
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
    if let Some(earlier) = earlier {
        document.continue_fragments(earlier.iter().copied());
    }

    let width = content_width(geometry);
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
        let all_geometries = (inputs.page_geometries)(&all_pages);
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
        // The copy drops the forced breaks before the token, so content that
        // the earlier layout kept on an earlier page, such as a float below
        // the end of its block, can run onto the resumed page. Content can
        // also move up onto the dropped lead page and be painted nowhere.
        // Either way the page no longer starts with what it did.
        if content_before.is_some_and(|earlier| {
            let earlier: std::collections::HashSet<usize> = earlier.iter().copied().collect();
            let copy: std::collections::HashSet<usize> = document
                .page_content_before(&cascade, page as u32, start.token)
                .into_iter()
                .collect();
            copy != earlier
        }) {
            return Ok(None);
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
