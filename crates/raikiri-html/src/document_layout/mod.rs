//! Owned layout results and borrowed per-page views for drawing consumers.

mod dom_view;
mod navigation;
mod page;
mod running;
mod streaming;

pub use dom_view::DomView;
pub use navigation::{Anchor, AnchorIndex, Link};
pub use page::PlacedRunningElement;
pub use page::{InlineSvg, Page, PageGeometry, PageMode, RasterImage};
pub use running::RunningElementLayout;
pub use streaming::{
    PageCountRunningElement, PageSink, StreamPage, StreamStatus, StreamSummary, StreamingLayout,
};
// cov:ignore: public type re-exports have no executable mapping; API integration tests verify them.
pub use raikiri_dom::{
    ClipKind, ColumnRule, DecorationKind, DecorationLine, DecorationStyle, DeferredGlyph,
    DeferredSlot, DeferredValue, FontBlob, FontId, FontRef, FontVariation, Fragment, FragmentKind,
    GeneratedBox, GeneratedKind, Glyph, MarginBox, MarginBoxBackgroundImage, MarginBoxBorder,
    MarginBoxRunning, MarginBoxText, OverflowClip, PaintEvent, PositionedGlyphRun, RepeatKind,
    RunSource, Synthesis, Tag, TextLineId, TextShadow,
};

use crate::render::{PipelineInputs, PipelineOutput, PipelineRun, run_pipeline};
use crate::{ConsumerPropertyRegistration, HtmlDocument, RenderResources};
use raikiri_traits::{
    ConsumerPropertyObserver, LayoutConfig, PageDefaults, RenderError, RenderWarning, WarningKind,
};

/// Result of [`layout`].
#[non_exhaustive]
pub enum LayoutStatus {
    /// Layout finished.
    Completed(DocumentLayout),
    /// The abort signal fired. No partial result is returned.
    ///
    /// When the signal fires while consumer property events are being
    /// delivered, the driver still delivers the remaining events of the same
    /// batch before returning. The consumer discards the per-call collection
    /// in that case; one observer call is not a success confirmation.
    Aborted,
}

/// Resources and observers for one [`layout`] call.
pub struct LayoutOptions<'r, 'a> {
    resources: Option<&'r RenderResources<'a>>,
    consumer_properties: &'r [ConsumerPropertyRegistration],
    property_observer: Option<&'r mut dyn ConsumerPropertyObserver>,
    preload_background_images: bool,
}

impl Default for LayoutOptions<'_, '_> {
    fn default() -> Self {
        Self {
            resources: None,
            consumer_properties: &[],
            property_observer: None,
            preload_background_images: true,
        }
    }
}

impl<'r, 'a> LayoutOptions<'r, 'a> {
    /// Options with default resources and no observers.
    pub fn new() -> Self {
        Self::default()
    }

    /// Share this resource configuration with the layout.
    pub fn resources(mut self, resources: &'r RenderResources<'a>) -> Self {
        self.resources = Some(resources);
        self
    }

    /// Register consumer-owned properties and receive their resolved values
    /// before [`layout`] returns.
    ///
    /// Events arrive in deterministic document order after the page count is
    /// fixed. When the abort signal fires during delivery, the remaining events
    /// of the same batch are still delivered and [`layout`] then reports an
    /// aborted status with no partial result. Keep a per-call collection and
    /// discard it unless the status is completed; one observer call is not a
    /// success confirmation.
    pub fn consumer_properties(
        mut self,
        registrations: &'r [ConsumerPropertyRegistration],
        observer: &'r mut dyn ConsumerPropertyObserver,
    ) -> Self {
        self.consumer_properties = registrations;
        self.property_observer = Some(observer);
        self
    }

    /// Whether to fetch and decode CSS background images during layout.
    /// Defaults to `true`.
    pub fn preload_background_images(mut self, enabled: bool) -> Self {
        self.preload_background_images = enabled;
        self
    }
}

/// Lay out `doc` into pages and keep the result for drawing.
///
/// The input document is borrowed and cloned internally, so it stays usable
/// after an error or abort. Only print media is supported; a screen media
/// context returns [`RenderError::Configuration`] before layout begins.
///
/// Consumer property events, when registered, are delivered after the page
/// count is fixed and before this function returns, in deterministic document
/// order. An abort that fires during delivery does not stop the current batch:
/// remaining events are still delivered, then an aborted status is returned
/// with no partial result. Discard the per-call collection unless the status
/// is completed.
///
/// ```
/// use raikiri_html::{
///     LayoutOptions, LayoutStatus, PageDefaults, RenderResources, LayoutConfig,
///     layout, parse_html_with_resources,
/// };
/// let document = parse_html_with_resources(
///     "<p>Hello</p>".as_bytes(), &RenderResources::new(),
/// )?;
/// if let LayoutStatus::Completed(result) = layout(
///     &document, PageDefaults::default(), LayoutConfig::default(), LayoutOptions::new(),
/// )? {
///     for page in result.pages() {
///         for fragment in page.fragments() {
///             let _border_box = fragment.rect();
///         }
///     }
/// }
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn layout(
    doc: &HtmlDocument,
    defaults: PageDefaults,
    config: LayoutConfig,
    options: LayoutOptions<'_, '_>,
) -> Result<LayoutStatus, RenderError> {
    if config.media_context.media_type() != crate::MediaType::Print {
        return Err(RenderError::Configuration(
            "Document layout requires print media".to_owned(),
        ));
    }
    let LayoutOptions {
        resources,
        consumer_properties,
        property_observer,
        preload_background_images,
    } = options;
    let signal = config.signal.clone();
    let run = run_pipeline(
        doc,
        defaults,
        &config,
        PipelineInputs {
            resources,
            consumer_properties,
            property_observer,
            preload_background_images,
        },
    )?;
    let mut out = match run {
        PipelineRun::Completed(out) => out,
        PipelineRun::Aborted => return Ok(LayoutStatus::Aborted),
    };
    // A paragraph or subtree is reported once, however many of the page
    // layouts it appears in.
    let layouts = std::iter::once((&out.document, &out.cascade)).chain(
        out.continuations
            .iter()
            .map(|continuation| (&continuation.document, &continuation.cascade)),
    );
    let mut omitted = Vec::new();
    let mut approximated = Vec::new();
    for (document, cascade) in layouts {
        for entry in document.omitted_text_run_roots(cascade) {
            if !omitted.iter().any(|(node, _)| *node == entry.0) {
                omitted.push(entry);
            }
        }
        for entry in document.paint_order_approximations(cascade) {
            if !approximated.iter().any(|(node, _)| *node == entry.0) {
                approximated.push(entry);
            }
        }
    }
    out.warnings
        .extend(omitted.into_iter().map(|(node, reason)| RenderWarning {
            kind: WarningKind::TextRunsOmitted,
            node_id: Some(node),
            details: format!("text runs are not reported for this paragraph: {reason}"),
        }));
    out.warnings.extend(
        approximated
            .into_iter()
            .map(|(node, reason)| RenderWarning {
                kind: WarningKind::PaintOrderApproximated,
                node_id: Some(node),
                details: format!("paint order is approximated for this subtree: {reason}"),
            }),
    );
    if signal.as_ref().is_some_and(|signal| signal.is_aborted()) {
        return Ok(LayoutStatus::Aborted);
    }
    let rendered = navigation::build_rendered(&out);
    let anchors = navigation::build_anchors(DomView::new(&out.document), &out);
    let running = build_running_index(&out);
    let running_layouts = running::LayoutCache::new(&running);
    let svg_sources = (0..=out.continuations.len())
        .map(|_| page::InlineSvgCache::default())
        .collect();
    if signal.as_ref().is_some_and(|signal| signal.is_aborted()) {
        return Ok(LayoutStatus::Aborted);
    }
    Ok(LayoutStatus::Completed(DocumentLayout {
        out,
        anchors,
        rendered,
        running,
        running_layouts,
        svg_sources,
    }))
}

/// An owned, laid-out document.
pub struct DocumentLayout {
    out: Box<PipelineOutput>,
    anchors: AnchorIndex,
    rendered: std::collections::HashSet<raikiri_traits::NodeId>,
    running: running::RunningIndex,
    running_layouts: running::LayoutCache,
    /// Prepared inline SVG sources, one cache per laid-out document in
    /// [`crate::render::PipelineOutput::layout_slot_for_page`] order.
    svg_sources: Vec<page::InlineSvgCache>,
}

/// Index the running elements by the pages the rendered content around them
/// is on.
fn build_running_index(out: &PipelineOutput) -> running::RunningIndex {
    let has_running = out
        .cascade
        .computed
        .iter()
        .any(|computed| !computed.running_templates.is_empty());
    if !has_running {
        return running::RunningIndex::default();
    }
    let mut rendered_pages = std::collections::HashMap::<usize, (u32, u32)>::new();
    for slice in &out.slices {
        let (document, _) = out.layout_for_page(slice.page_index);
        for fragment in document.page_fragments(slice.page_index) {
            let page = slice.page_index;
            rendered_pages
                .entry(fragment.node().0 as usize)
                .and_modify(|(first, last)| {
                    *first = (*first).min(page);
                    *last = (*last).max(page);
                })
                .or_insert((page, page));
        }
    }
    running::RunningIndex::build(&out.document, &out.cascade, &rendered_pages)
}

impl DocumentLayout {
    /// Number of pages.
    pub fn page_count(&self) -> u32 {
        u32::try_from(self.out.slices.len()).unwrap_or(u32::MAX)
    }

    /// Pages in order.
    pub fn pages(&self) -> impl ExactSizeIterator<Item = Page<'_>> + '_ {
        (0..self.out.slices.len()).map(move |i| self.page_at(i))
    }

    /// One page, or `None` when out of range.
    pub fn page(&self, index: u32) -> Option<Page<'_>> {
        let i = index as usize;
        (i < self.out.slices.len()).then(|| self.page_at(i))
    }

    fn page_at(&self, i: usize) -> Page<'_> {
        let page_index = self.out.slices[i].page_index;
        let (document, cascade) = self.out.layout_for_page(page_index);
        let svg_sources = &self.svg_sources[self.out.layout_slot_for_page(page_index)];
        // A right page pairs with the left page before it; the first page,
        // which has none, pairs with the one after it.
        let paired_style = if self.out.slices[i].page_index % 2 == 1 {
            None
        } else if i > 0 {
            self.out.page_styles.get(i - 1)
        } else {
            self.out.page_styles.get(i + 1)
        };
        Page {
            slice: &self.out.slices[i],
            geometry: &self.out.geometries[i],
            style: &self.out.page_styles[i],
            document,
            cascade,
            page_count: self.page_count(),
            page_count_deferred: false,
            paired_style,
            running: Some(self.running_source()),
            svg_sources,
        }
    }

    /// Lay running element `node` (`position: running(<name>)`) out at
    /// `width` CSS px, the content width of the page margin box that shows
    /// it (CSS GCPM 3 §1.2).
    ///
    /// Use [`Page::running_element`] to find the element a margin box shows
    /// on a page. Returns `None` when `node` is not a running element.
    ///
    /// The layout of an element depends only on the element and the width,
    /// so it is made once and kept: a margin box that shows the same element
    /// on many pages gets the same layout back for each page.
    pub fn layout_running_element(
        &self,
        node: raikiri_traits::NodeId,
        width: f32,
    ) -> Result<Option<&RunningElementLayout>, RenderError> {
        self.running_source().layout(node, width)
    }

    fn running_source(&self) -> running::RunningSource<'_> {
        running::RunningSource {
            index: &self.running,
            layouts: &self.running_layouts,
            document: &self.out.document,
            cascade: &self.out.cascade,
        }
    }

    /// In-document link destinations. Positions are in layout space until
    /// paint-space positions are exposed.
    pub fn anchors(&self) -> &AnchorIndex {
        &self.anchors
    }

    /// Whether `node` has at least one fragment on some page. `false` for an
    /// out-of-range node.
    pub fn is_rendered(&self, node: raikiri_traits::NodeId) -> bool {
        self.rendered.contains(&node)
    }

    /// Parse-time and layout-time warnings.
    pub fn warnings(&self) -> &[RenderWarning] {
        &self.out.warnings
    }

    /// The effective document base URL.
    pub fn base_url(&self) -> Option<&url::Url> {
        self.out.base_url.as_ref()
    }
}

#[cfg(test)]
mod tests;
