//! Later pages whose content width differs from the pages before them.
//!
//! The paginator lays the document out once, at the content width of its
//! first page, and slices that flow into pages. When `@page` rules give a
//! later page another content width (for example `@page :first` with wider
//! margins), the pages from that one on are taken from a copy of the
//! document laid out at their own width. The copy resumes at the source
//! position where the page began in the earlier layout: a block box, or a
//! paragraph line that a forced break in the copy also starts.
//!
//! In a document whose root lays its blocks out on vertical axes the pages
//! follow the x axis and lines run down the page, so the page area's height
//! takes the place of its width throughout.

use raikiri_dom::{
    CrossingBox, Document, PageLayoutControl, PageSlice, PageStart, PageStartToken,
    layout_pages_with_options,
};
use raikiri_style::CascadeResult;
use raikiri_style::property::PageValue;
use raikiri_traits::{ImagePixelSource, LayoutError, ReplacedResolver};

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

/// Record in the copies of the document the sizes of the images that the
/// margin boxes show with `content`. The copies were made before the images
/// were fetched, and the pages laid out in them are drawn from them.
pub(super) fn prepare_margin_box_images(
    continuations: &mut [Continuation],
    urls: &[String],
    source: &dyn ImagePixelSource,
    base_url: Option<&url::Url>,
) {
    for continuation in continuations {
        continuation
            .document
            .prepare_margin_box_image_urls(urls, source, base_url);
    }
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

/// The axis the pages of a layout follow. The paginator lays lines out at
/// the page area's extent across it and slices the flow along it.
#[derive(Clone, Copy)]
enum PageAxis {
    /// The flow runs down the page: lines are as long as the page area is
    /// wide.
    Y,
    /// The flow of a root whose blocks stack on vertical axes runs across
    /// the page: lines are as long as the page area is tall.
    X,
}

impl PageAxis {
    fn of(document: &Document, cascade: &CascadeResult) -> Self {
        match raikiri_dom::pages_follow_x_axis(document, cascade) {
            Some(_) => Self::X,
            None => Self::Y,
        }
    }

    /// The extent of the page area that lines are laid out across.
    fn inline_size(self, geometry: ResolvedPageGeometry) -> f32 {
        match self {
            Self::Y => geometry.content_box.width,
            Self::X => geometry.content_box.height,
        }
    }

    /// The extent of the page area along the flow.
    fn block_size(self, geometry: ResolvedPageGeometry) -> f32 {
        match self {
            Self::Y => geometry.content_box.height,
            Self::X => geometry.content_box.width,
        }
    }

    /// The page steps and widths arguments of the paginator for pages whose
    /// extents along the flow are `steps` and across it `inline_sizes`. On
    /// x the paginator takes the extents along the flow as page widths and
    /// lays every page out at the inline size of the page box it is given.
    fn layout_schedule<'a>(
        self,
        steps: &'a [f32],
        inline_sizes: &'a [f32],
    ) -> (&'a [f32], &'a [f32]) {
        match self {
            Self::Y => (steps, inline_sizes),
            Self::X => (&[], steps),
        }
    }

    /// The page box that measuring pages `extent` long along the flow are
    /// projected onto, and the pages they are projected as.
    ///
    /// On y the measuring page box is grown by `extent`. On x the page area's
    /// width is the containing block of the root and the edge that flow
    /// positions count from, so it stays as laid out; a page ending `extent`
    /// after the last measuring page gives that page its extent instead.
    fn measuring_pages(
        self,
        page_box: raikiri_traits::PageBox,
        measured: &[PageSlice],
        extent: f32,
    ) -> (raikiri_traits::PageBox, Vec<PageSlice>) {
        let mut pages = measured.to_vec();
        match self {
            Self::Y => {
                let mut page_box = page_box;
                page_box.height += extent;
                (page_box, pages)
            }
            Self::X => {
                if let Some(last) = measured.last() {
                    pages.push(PageSlice {
                        page_index: last.page_index + 1,
                        content_origin_y: last.content_origin_y + extent,
                        page_name: None,
                    });
                }
                (page_box, pages)
            }
        }
    }
}

/// Content widths closer than this are the same width.
const WIDTH_TOLERANCE: f32 = 0.01;
/// Where a relayout's page start may land from where the earlier layout had it.
const POSITION_TOLERANCE: f32 = 0.5;
/// The room the page the relayout is first measured on leaves beyond where
/// the earlier layout placed the page start.
const MEASURE_HEIGHT: f32 = 1.0e6;
/// Relayouts of one continuation before its page schedule must be stable.
const MAX_SCHEDULE_PASSES: usize = 3;
/// Measurements of one continuation before it gives up finding its page
/// start within the document's page limit.
const MAX_MEASURE_PASSES: usize = 8;

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
    let Some((continuation, pages)) = relayout_from(
        inputs,
        PageAxis::of(document, cascade),
        slices,
        geometries,
        RelayoutStart {
            page,
            start,
            earlier: Some(earlier),
            content_before: None,
        },
    )?
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
        let segment = match continuations.last() {
            Some(continuation) => Segment {
                document: &continuation.document,
                cascade: &continuation.cascade,
            },
            None => Segment { document, cascade },
        };
        let axis = PageAxis::of(segment.document, segment.cascade);
        let run_width = axis.inline_size(geometries[run_start]);
        let other_widths = (run_start + 1..slices.len())
            .filter(|&page| {
                (axis.inline_size(geometries[page]) - run_width).abs() > WIDTH_TOLERANCE
            })
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
        // Root and body retain their fragments across the resumed page.
        // Other boxes crossing the start begin anew in the copy.
        let roots: Vec<_> = root_boxes(segment.document).collect();
        let crossing: Vec<CrossingBox> = segment
            .document
            .crossing_boxes(page as u32)
            .into_iter()
            .filter(|crossing| roots.contains(&(crossing.node.0 as usize)))
            .collect();
        let Some((continuation, pages)) = relayout_from(
            inputs,
            axis,
            slices,
            geometries,
            RelayoutStart {
                page,
                start,
                earlier: Some(&crossing),
                content_before: Some(&before),
            },
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

/// Source root boxes whose fragment sequences span continuation layouts.
fn root_boxes(document: &Document) -> impl Iterator<Item = usize> {
    let root = document.document_element_index();
    let body =
        raikiri_dom::paint_rules::find_paint_root(document).filter(|body| Some(*body) != root);
    [root, body].into_iter().flatten()
}

/// Root and body fragments on the pages that `continuations` lay out, out of
/// `page_count` pages, as [`Document::continue_fragments_after`] takes them
/// for the layout before the first continuation.
#[doc(hidden)]
pub fn root_fragments_after(
    document: &Document,
    continuations: &[Continuation],
    page_count: usize,
) -> Vec<(raikiri_traits::NodeId, u32)> {
    root_boxes(document)
        .map(|root| {
            let node = raikiri_traits::NodeId::new(root as u64);
            let count = continuations
                .iter()
                .enumerate()
                .flat_map(|(index, continuation)| {
                    let end = continuations
                        .get(index + 1)
                        .map_or(page_count, |next| next.first_page as usize);
                    (continuation.first_page as usize..end)
                        .map(move |page| (continuation, page as u32))
                })
                .filter(|(continuation, page)| {
                    continuation
                        .document
                        .page_fragments(*page)
                        .any(|fragment| fragment.node() == node)
                })
                .count();
            (node, count as u32)
        })
        .collect()
}

/// Where a relayout resumes the document, and what the earlier layout has
/// there.
struct RelayoutStart<'a> {
    /// The page the relayout begins at.
    page: usize,
    /// Where the earlier layout began that page.
    start: PageStart,
    /// The boxes crossing `start` in the earlier layout, which the relayout
    /// continues.
    earlier: Option<&'a [CrossingBox]>,
    /// The content the earlier layout has before the token of `start` on its
    /// page. The relayout does not reproduce the page start when it puts
    /// other content there.
    content_before: Option<&'a [usize]>,
}

/// The heights of the pages that a relayout lays out: a lead page first when
/// content comes before the page start, then each page from the resumed one
/// on.
struct Schedule {
    /// The height of the page that ends where the resumed page begins.
    /// Without content before that point the copy needs no such page.
    lead: Option<f32>,
}

impl Schedule {
    /// The schedule for a copy whose page start lies `token_y` from its
    /// first measuring page, when the earlier layout had it `offset` down its
    /// page.
    fn new(token_y: f32, offset: f32) -> Self {
        let lead = token_y - offset;
        Self {
            lead: (lead > POSITION_TOLERANCE).then_some(lead),
        }
    }

    /// The number of pages before the resumed one.
    fn skipped(&self) -> usize {
        usize::from(self.lead.is_some())
    }

    /// The page steps for the lead page and then pages with `geometries`.
    fn steps(&self, axis: PageAxis, geometries: &[ResolvedPageGeometry]) -> Vec<f32> {
        self.lead
            .into_iter()
            .chain(geometries.iter().map(|&geometry| axis.block_size(geometry)))
            .collect()
    }

    /// Whether the resumed page of `laid_out` does not begin where the lead
    /// page ends.
    fn misses_the_lead(&self, laid_out: &[PageSlice]) -> bool {
        laid_out
            .get(self.skipped())
            .map(|slice| slice.content_origin_y)
            .is_none_or(|origin| (origin - self.lead.unwrap_or(0.0)).abs() > POSITION_TOLERANCE)
    }
}

/// Whether `next`, the schedule that the laid out pages give, has the heights
/// of `used`, the one they were laid out with. Pages of another width are
/// relaid out by the next continuation, so only heights have to match.
fn schedule_is_stable(next: &[f32], used: &[f32]) -> bool {
    next.len() <= used.len()
        && next
            .iter()
            .zip(used)
            .all(|(next, used)| (next - used).abs() <= WIDTH_TOLERANCE)
}

/// A copy of the document that is laid out again at the content width of a
/// page.
struct Relayout<'a> {
    inputs: &'a ContinuationInputs<'a>,
    axis: PageAxis,
    /// The pages of the earlier layout.
    slices: &'a [PageSlice],
    /// The geometry of each of those pages.
    geometries: &'a [ResolvedPageGeometry],
    /// The geometry of the page the copy resumes at.
    geometry: ResolvedPageGeometry,
    /// Where the earlier layout placed that page in the document.
    origin_y: f32,
    resume: RelayoutStart<'a>,
    document: Document,
    cascade: CascadeResult,
}

/// Lay a copy of the document out at the width of the page that `resume`
/// names and resume it there. Returns the copy and its pages, numbered from
/// that page, or `None` when the copy does not reproduce the page start, or
/// puts content before the token on that page that `resume.content_before`,
/// the earlier layout's, does not have.
fn relayout_from<'a>(
    inputs: &'a ContinuationInputs<'a>,
    axis: PageAxis,
    slices: &'a [PageSlice],
    geometries: &'a [ResolvedPageGeometry],
    resume: RelayoutStart<'a>,
) -> Result<Option<(Continuation, Vec<PageSlice>)>, LayoutError> {
    let mut relayout = Relayout::new(inputs, axis, slices, geometries, resume);
    // The token was found on the earlier layout's page, and the copy lays out
    // the same source before it, so one of the measuring pages holds it.
    let Some(token_y) = relayout.measure_token_y()? else {
        return Ok(None); // cov:ignore: defensive; the copy lays out the same source before the token
    };
    relayout.resume_at_token(token_y)
}

impl<'a> Relayout<'a> {
    /// The copy of the document for `resume`, with the element cascade of
    /// the page it resumes at.
    fn new(
        inputs: &'a ContinuationInputs<'a>,
        axis: PageAxis,
        slices: &'a [PageSlice],
        geometries: &'a [ResolvedPageGeometry],
        resume: RelayoutStart<'a>,
    ) -> Self {
        // The run's element cascade already succeeded on the same input with the
        // same options, and the cascade's limits count only its input.
        let mut cascade = (inputs.cascade_for_page)(&slices[resume.page]);
        let mut document = inputs.pristine.clone();
        document.skip_layout(&mut cascade, inputs.skipped);
        if let PageStartToken::Line { text, offset } = resume.start.token {
            document.set_continuation_break(Some((text, offset)));
        }
        document.prepare_continuation_cascade(&mut cascade, resume.start.token);
        Self {
            inputs,
            axis,
            slices,
            geometries,
            geometry: geometries[resume.page],
            origin_y: slices[resume.page].content_origin_y,
            resume,
            document,
            cascade,
        }
    }

    /// Measure where the copy places the page start, on one page tall enough
    /// to hold everything before it. Page names would force a break before
    /// the token, such as the change to its own named page, and move it off
    /// the measuring page, so the measurement ignores them for breaking. The
    /// page the measurement ends then starts at that break, which the later
    /// layout, with the names, places there again.
    fn measure_token_y(&mut self) -> Result<Option<f32>, LayoutError> {
        // The changes of named page still give the boxes after them clearance,
        // as in the later layout: preparing the cascade had the document keep
        // them.
        let page_values = std::mem::replace(
            &mut self.cascade.page_values,
            vec![PageValue::Auto; self.document.node_count()],
        );
        // The measuring page holds everything the earlier layout placed before
        // the token, with room to spare. Content that grows at this width can
        // still run past it, onto more measuring pages than the document's page
        // limit allows; the measurement then stops before the token and is
        // repeated on a page that holds everything it laid out, twice over.
        let mut measure_height = self.origin_y + self.resume.start.offset + MEASURE_HEIGHT;
        let control = PageLayoutControl::for_geometry_discovery(self.inputs.max_pages)
            .with_abort_check(self.inputs.abort_check);
        let mut token_y = None;
        for _ in 0..MAX_MEASURE_PASSES {
            let measured = self.lay_out_measuring_pages(measure_height, &control)?;
            // Content taller than one measuring page runs onto the next, so the
            // token's offset counts from the first measuring page.
            let first_origin = measured.first().map_or(0.0, |slice| slice.content_origin_y);
            token_y = measured.iter().find_map(|slice| {
                self.document
                    .page_token_offset(slice.page_index, self.resume.start.token)
                    .map(|offset| slice.content_origin_y - first_origin + offset)
            });
            if token_y.is_some() || !control.page_limit_reached() {
                break;
            }
            let last_origin = measured
                .last()
                .map_or(first_origin, |slice| slice.content_origin_y);
            measure_height = 2.0 * (last_origin - first_origin + measure_height);
        }
        self.cascade.page_values = page_values;
        Ok(token_y)
    }

    /// Lay the copy out on one measuring page `height` tall and project it
    /// onto the measuring pages that results in.
    fn lay_out_measuring_pages(
        &mut self,
        height: f32,
        control: &PageLayoutControl,
    ) -> Result<Vec<PageSlice>, LayoutError> {
        let (steps, widths) = self
            .axis
            .layout_schedule(std::slice::from_ref(&height), &[]);
        // Only a cancellation landing in this pass fails it; the first
        // layout and projection already met the same work limits.
        let measured = layout_pages_with_options(
            &mut self.document,
            &self.cascade,
            self.geometry.page_box,
            raikiri_dom::PageLayoutOptions {
                page_steps: steps,
                page_widths: widths,
                images: Some(raikiri_dom::PageImageOptions {
                    resolver: self.inputs.resolver,
                    base_url: self.inputs.base_url,
                }),
                control: Some(control),
            },
        )?;
        let (measure_box, projected) =
            self.axis
                .measuring_pages(self.geometry.page_box, &measured, height);
        let measure_geometries: Vec<_> = projected
            .iter()
            .map(|_| {
                (
                    measure_box,
                    self.geometry.margins,
                    self.geometry.content_insets,
                )
            })
            .collect();
        self.document
            .project_pages_with_control(
                &self.cascade,
                measure_box,
                &projected,
                &measure_geometries,
                control,
            )
            .map(|()| measured)
    }

    /// Lay the copy out from the page start, which lies `token_y` from its
    /// first measuring page, until the pages it lays out give the schedule
    /// that it was laid out with. `None` when the copy does not reproduce the
    /// page start.
    fn resume_at_token(
        mut self,
        token_y: f32,
    ) -> Result<Option<(Continuation, Vec<PageSlice>)>, LayoutError> {
        let page = self.resume.page;
        let schedule = Schedule::new(token_y, self.resume.start.offset);
        let skipped = schedule.skipped();
        if let Some(earlier) = self.resume.earlier {
            self.document.continue_fragments(earlier.iter().copied());
        }
        let mut steps = schedule.steps(self.axis, &self.geometries[page..]);
        let control = PageLayoutControl::new(self.inputs.max_pages.map(|limit| {
            limit
                .saturating_sub(page as u32)
                .saturating_add(skipped as u32)
        }))
        .with_abort_check(self.inputs.abort_check);
        for _ in 0..MAX_SCHEDULE_PASSES {
            let laid_out = self.lay_out_steps(&steps, skipped, &control)?;
            if schedule.misses_the_lead(&laid_out) {
                return Ok(None); // cov:ignore: defensive; the lead page ends at a line or box edge, so the copy breaks there
            }
            let pages = self.resumed_pages(&laid_out[skipped..]);
            let resumed = self.resumed_geometries(&pages);
            let next_steps = schedule.steps(self.axis, &resumed);
            if !schedule_is_stable(&next_steps, &steps) {
                steps = next_steps;
                continue;
            }
            let projection: Vec<_> = resumed
                .iter()
                .map(|geometry| (geometry.page_box, geometry.margins, geometry.content_insets))
                .collect();
            self.document.project_pages_with_control(
                &self.cascade,
                self.geometry.page_box,
                &pages,
                &projection,
                &control,
            )?; // cov:ignore: only a cancellation landing in this pass fails it; the first projection already met the same work limits
            return Ok(self.into_continuation(pages));
        }
        // cov:ignore: no fixture keeps a page schedule changing through every pass
        Ok(None)
    }

    /// Lay the copy out on pages with the page `steps`, all at the width of
    /// the resumed page, `skipped` of which come before it.
    fn lay_out_steps(
        &mut self,
        steps: &[f32],
        skipped: usize,
        control: &PageLayoutControl,
    ) -> Result<Vec<PageSlice>, LayoutError> {
        let widths = vec![self.axis.inline_size(self.geometry); steps.len()];
        let (page_steps, page_widths) = self.axis.layout_schedule(steps, &widths);
        layout_pages_with_options(
            &mut self.document,
            &self.cascade,
            self.geometry.page_box,
            raikiri_dom::PageLayoutOptions {
                page_steps,
                page_widths,
                images: Some(raikiri_dom::PageImageOptions {
                    resolver: self.inputs.resolver,
                    base_url: self.inputs.base_url,
                }),
                control: Some(control),
            },
        )
        .map_err(|error| self.in_document_pages(error, skipped))
    }

    /// `error` with a page count of the whole document. The copy counts its
    /// pages from its own first page; a page limit is reported against the
    /// pages of the whole document.
    fn in_document_pages(&self, error: LayoutError, skipped: usize) -> LayoutError {
        match (error, self.inputs.max_pages) {
            (LayoutError::PageLimitExceeded { actual, .. }, Some(limit)) => {
                LayoutError::PageLimitExceeded {
                    limit,
                    actual: actual - skipped as u64 + self.resume.page as u64,
                }
            }
            (error, _) => error, // cov:ignore: other layout errors already name no page count
        }
    }

    /// The pages of the copy from the resumed one on, numbered from it.
    fn resumed_pages(&self, laid_out: &[PageSlice]) -> Vec<PageSlice> {
        laid_out
            .iter()
            .enumerate()
            .map(|(index, slice)| PageSlice {
                page_index: self.resume.page as u32 + index as u32,
                content_origin_y: slice.content_origin_y,
                page_name: slice.page_name.clone(),
            })
            .collect()
    }

    /// The geometry of each of the resumed `pages`, among the pages of the
    /// earlier layout that come before them.
    fn resumed_geometries(&self, pages: &[PageSlice]) -> Vec<ResolvedPageGeometry> {
        let mut all_pages = self.slices[..self.resume.page].to_vec();
        all_pages.extend(pages.iter().cloned());
        (self.inputs.page_geometries)(&all_pages).split_off(self.resume.page)
    }

    /// The copy as a continuation with its `pages`, unless the resumed page
    /// does not begin as the earlier layout's did.
    fn into_continuation(self, pages: Vec<PageSlice>) -> Option<(Continuation, Vec<PageSlice>)> {
        (self.starts_at_the_token() && self.has_the_content_before_it()).then(|| {
            (
                Continuation {
                    first_page: self.resume.page as u32,
                    document: self.document,
                    cascade: self.cascade,
                },
                pages,
            )
        })
    }

    /// Whether the resumed page holds the token at the offset that the
    /// earlier layout had it at.
    fn starts_at_the_token(&self) -> bool {
        let resumed_offset = self
            .document
            .page_token_offset(self.resume.page as u32, self.resume.start.token);
        !resumed_offset
            .is_none_or(|offset| (offset - self.resume.start.offset).abs() > POSITION_TOLERANCE)
    }

    /// Whether the resumed page has the content before the token that the
    /// earlier layout had. The copy drops the forced breaks before the token,
    /// so content that the earlier layout kept on an earlier page, such as a
    /// float below the end of its block, can run onto the resumed page.
    /// Content can also move up onto the dropped lead page and be painted
    /// nowhere. Either way the page no longer starts with what it did.
    fn has_the_content_before_it(&self) -> bool {
        self.resume.content_before.is_none_or(|earlier| {
            let earlier: std::collections::HashSet<usize> = earlier.iter().copied().collect();
            let copy: std::collections::HashSet<usize> = self
                .document
                .page_content_before(
                    &self.cascade,
                    self.resume.page as u32,
                    self.resume.start.token,
                )
                .into_iter()
                .collect();
            copy == earlier
        })
    }
}
