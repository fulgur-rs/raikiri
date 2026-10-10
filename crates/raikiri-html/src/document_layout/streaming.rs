//! Push-style layout: feed HTML bytes as they are produced and receive the
//! laid-out pages in order through a [`PageSink`].
//!
//! [`StreamingLayout`] parses each chunk as soon as it is fed. After every
//! [`CHECKPOINT_BYTES`] of input it lays out the document parsed so far and
//! hands the sink every page that no later input can change (see the
//! `frontier` module for what makes a page final). The remaining pages
//! arrive from [`StreamingLayout::finish`]. Content that holds the whole
//! document back, such as an open table or a selector like `:last-child`,
//! delays delivery but never changes the pages already delivered.
//!
//! The pages are those of a batch layout of the whole input, except where
//! later content would have to change pages already delivered: that content
//! is left out and reported, as [`StreamingLayout`] lists.

use std::collections::HashMap;
use std::io::Write;

use html5ever::driver::{ParseOpts, Parser, parse_document};
use raikiri_style::property::{DisplayValue, PositionValue};
use raikiri_traits::{
    ConsumerPropertyEvent, FetchOutcome, FetchedResource, LayoutConfig, LimitKind, Method,
    NetworkError, NetworkProvider, NodeId, NodeKind, PageDefaults, RenderError, RenderWarning,
    Request, WarningKind,
};

use super::{
    AnchorIndex, DocumentLayout, LayoutOptions, LayoutStatus, Page, RunningElementLayout, layout,
};
use crate::document_parse::assemble_document;
use crate::input::Utf8Feed;
use crate::parse::finish_document;
use crate::sink::{collect_body_inline_stylesheet_ids, traced_handles};
use crate::types::UncascadedDocument;
use crate::{
    ConsumerPropertyRegistration, HtmlDocument, MarginBox, RaikiriTreeSink, RenderResources,
    build_rule_tree,
};
use frontier::{Frontier, final_page_count, stable_frontier};

/// Input bytes between two layouts of the document parsed so far.
const CHECKPOINT_BYTES: usize = 64 * 1024;

/// Receiver of the pages of a [`StreamingLayout`].
pub trait PageSink {
    /// Value returned by [`PageSink::finish`] and, through it, by
    /// [`StreamingLayout::finish`].
    type Output;

    /// Receive one final page. Pages arrive once each, in page order.
    ///
    /// `events` holds the consumer property events of the elements whose
    /// first fragment is on this page, in document order. The page view is
    /// borrowed for this call only; draw it before returning. An error stops
    /// the layout, and [`StreamingLayout::finish`] returns it as
    /// [`RenderError::Io`].
    fn page(
        &mut self,
        page: StreamPage<'_>,
        events: Vec<ConsumerPropertyEvent>,
    ) -> std::io::Result<()>;

    /// Called once after the last page, with values known only at the end.
    ///
    /// Not called when the layout fails or is aborted.
    fn finish(self, summary: StreamSummary) -> std::io::Result<Self::Output>;
}

/// One final page handed to [`PageSink::page`].
#[derive(Clone, Copy)]
pub struct StreamPage<'a> {
    layout: &'a DocumentLayout,
    index: u32,
    /// Stands for the number of pages while it is not known yet.
    page_count_placeholder: Option<u32>,
}

impl<'a> StreamPage<'a> {
    /// Zero-based page index in the whole document.
    pub fn index(&self) -> u32 {
        self.index
    }

    /// The laid-out page.
    ///
    /// A page delivered before the input ended does not know the number of
    /// pages: its [`Page::margin_boxes`] show a placeholder for
    /// `counter(pages)` and list it in [`crate::MarginBox::deferred`]. The
    /// placeholder has as many digits as the page limit
    /// ([`raikiri_traits::RenderLimits::max_document_pages`]) allows, and is
    /// shown in decimal where the requested counter style would make it far
    /// longer, as `symbolic` styles do. Write
    /// [`crate::DeferredSlot::text`] of [`StreamSummary::page_count`] in its
    /// place once [`PageSink::finish`] runs, or draw the page's margin boxes
    /// from [`StreamSummary::page_count_margin_boxes`] instead. Pages
    /// delivered by [`StreamingLayout::finish`] show the real number.
    pub fn page(&self) -> Page<'a> {
        let page = self.layout.page_at(self.index as usize);
        match self.page_count_placeholder {
            Some(placeholder) => page.with_deferred_page_count(placeholder),
            None => page,
        }
    }

    /// The document's base URL, for resolving the targets of
    /// [`Page::links`]. See [`DocumentLayout::base_url`].
    pub fn base_url(&self) -> Option<&'a url::Url> {
        self.layout.base_url()
    }

    /// Lay running element `node` out at `width` CSS px. See
    /// [`DocumentLayout::layout_running_element`].
    pub fn layout_running_element(
        &self,
        node: NodeId,
        width: f32,
    ) -> Result<Option<&'a RunningElementLayout>, RenderError> {
        self.layout.layout_running_element(node, width)
    }
}

/// Values known only after the last page, passed to [`PageSink::finish`].
#[derive(Debug, Default)]
#[non_exhaustive]
pub struct StreamSummary {
    /// Number of pages, the value of `counter(pages)`.
    pub page_count: u32,
    /// In-document link destinations of the whole document.
    pub anchors: AnchorIndex,
    /// Parse-time and layout-time warnings.
    pub warnings: Vec<RenderWarning>,
    /// Consumer property events of elements without a fragment on any page,
    /// in document order.
    pub unplaced_events: Vec<ConsumerPropertyEvent>,
    /// The margin boxes, laid out with the real page count, of every page
    /// that was delivered with a page count placeholder in its margin boxes,
    /// in page order. The page count can change the size of a margin box and
    /// of its neighbours, so these replace all margin boxes of the page.
    pub page_count_margin_boxes: Vec<(u32, Vec<MarginBox>)>,
}

/// Result of [`StreamingLayout::finish`].
#[derive(Debug)]
#[non_exhaustive]
pub enum StreamStatus<T> {
    /// Every page was delivered and the sink finished with this value.
    Completed(T),
    /// The abort signal fired. Pages delivered before the signal stay
    /// delivered; the sink is not finished.
    Aborted,
}

/// Lays out HTML that arrives in pieces and hands each page to a
/// [`PageSink`].
///
/// Feed the document with [`StreamingLayout::feed`] or through
/// [`std::io::Write`], then call [`StreamingLayout::finish`]. Every 64 KiB
/// of input, the document parsed so far is laid out and the pages no later
/// input can change are delivered; each of those layouts covers the whole
/// prefix, so a checkpoint costs as much as a layout of everything fed so
/// far. Input must be UTF-8. The input byte cap and the other parse limits
/// come from the [`RenderResources`]. Only print media is supported.
///
/// Delivered pages never change, so content that would change them is left
/// out, each case reported in [`StreamSummary::warnings`] as
/// [`WarningKind::StreamingContentIgnored`]:
///
/// - `<style>` elements inside `<body>` are not applied; put style sheets in
///   `<head>`;
/// - a `position: fixed` element created after the first page was delivered
///   is not laid out;
/// - attributes added by a repeated `<html>` or `<body>` tag after the first
///   page was delivered are ignored.
///
/// ```
/// use raikiri_html::{
///     LayoutConfig, PageDefaults, PageSink, RenderResources, StreamPage, StreamStatus,
///     StreamSummary, StreamingLayout,
/// };
/// use std::io::Write;
///
/// struct CountPages(u32);
/// impl PageSink for CountPages {
///     type Output = u32;
///     fn page(
///         &mut self,
///         _page: StreamPage<'_>,
///         _events: Vec<raikiri_html::ConsumerPropertyEvent>,
///     ) -> std::io::Result<()> {
///         self.0 += 1;
///         Ok(())
///     }
///     fn finish(self, summary: StreamSummary) -> std::io::Result<u32> {
///         assert_eq!(self.0, summary.page_count);
///         Ok(self.0)
///     }
/// }
///
/// let resources = RenderResources::new();
/// let mut stream = StreamingLayout::new(
///     &resources, PageDefaults::default(), LayoutConfig::default(), CountPages(0),
/// );
/// write!(stream, "<p>Hello</p>")?;
/// let StreamStatus::Completed(pages) = stream.finish()? else { unreachable!() };
/// assert_eq!(pages, 1);
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub struct StreamingLayout<'r, 'a, S> {
    settings: Settings<'r, 'a>,
    sink: S,
    input: Utf8Feed<Parser<RaikiriTreeSink>>,
    input_bytes: u64,
    /// Input bytes fed since the last checkpoint.
    unchecked_bytes: usize,
    checkpoint_bytes: usize,
    /// Pages already handed to the sink.
    delivered: u32,
    /// Node count of the document whose pages were delivered first. Fixed
    /// positioned elements created after that are not laid out: they would
    /// repeat on pages already delivered.
    late_nodes_from: Option<usize>,
    /// The abort signal fired during a checkpoint.
    aborted: bool,
    /// The first input failure. [`StreamingLayout::feed`] returns it to its
    /// caller; a failure inside [`std::io::Write`] is kept here for
    /// [`StreamingLayout::finish`].
    failure: Failure,
}

/// What every layout of one stream shares.
struct Settings<'r, 'a> {
    resources: &'r RenderResources<'a>,
    defaults: PageDefaults,
    config: LayoutConfig,
    consumer_properties: &'r [ConsumerPropertyRegistration],
    preload_background_images: bool,
    /// The network every cascade fetches style sheets through.
    network: Option<StreamNetwork<'r>>,
}

/// A network provider that fetches each style sheet once per stream.
///
/// Every checkpoint finishes the document parsed so far again, which loads
/// its external style sheets and imports. Without the cache each of those
/// loads would be fetched again and charged again to the resources' byte
/// budget. Failed fetches are not kept, so a later checkpoint retries them.
struct StreamNetwork<'r> {
    inner: crate::resources::ResourceNetworkProvider<'r>,
    bodies: std::sync::Mutex<HashMap<(url::Url, raikiri_traits::ResourceKind), FetchedResource>>,
}

impl<'r> StreamNetwork<'r> {
    fn new(inner: crate::resources::ResourceNetworkProvider<'r>) -> Self {
        Self {
            inner,
            bodies: std::sync::Mutex::new(HashMap::new()),
        }
    }
}

impl NetworkProvider for StreamNetwork<'_> {
    fn fetch_one_hop(&self, request: Request) -> Result<FetchOutcome, NetworkError> {
        let cacheable =
            request.method == Method::Get && matches!(request.body, raikiri_traits::Body::Empty);
        let key = (request.url.clone(), request.kind);
        if cacheable
            && let Some(body) = self
                .bodies
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .get(&key)
        {
            return Ok(FetchOutcome::Body(body.clone()));
        }
        let outcome = self.inner.fetch_one_hop(request)?;
        if cacheable && let FetchOutcome::Body(body) = &outcome {
            self.bodies
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .insert(key, body.clone());
        }
        Ok(outcome)
    }

    fn max_import_depth(&self) -> Option<u32> {
        self.inner.max_import_depth()
    }
}

/// A cascaded document and what [`Settings::cascade`] found on the way.
struct Cascaded {
    document: HtmlDocument,
    forward_dependent: bool,
    /// Warnings for the content left out.
    ignored: Vec<RenderWarning>,
}

/// Fixed positioned elements with node id `from` or more.
fn late_fixed_elements(document: &HtmlDocument, from: usize) -> Vec<usize> {
    let dom = &document.uncascaded.dom;
    (from..dom.node_count())
        .filter(|&id| {
            dom.get_node(id)
                .is_some_and(|node| node.kind() == NodeKind::Element && node.is_in_document())
                && document.cascade.computed.get(id).is_some_and(|values| {
                    values.position == PositionValue::Fixed && values.display != DisplayValue::None
                })
        })
        .collect()
}

/// Keep element `id` from generating boxes, through its inline style so
/// that every cascade of the document sees it.
///
/// The author's inline declarations are replaced rather than extended: an
/// element without boxes needs none of them, and appending to them could
/// land the declaration inside an unterminated comment or block.
fn hide_element(dom: &mut raikiri_dom::Document, id: usize) {
    dom.set_element_inline_style(id, Some("display: none !important".into()));
}

impl Settings<'_, '_> {
    /// Replace the parse-time cascade, computed for the default media
    /// context, with the one layout uses, so that the frontier and the late
    /// fixed elements are found with the styles the pages are laid out with.
    fn recascade_for_layout(
        &self,
        document: &mut HtmlDocument,
        limits: &raikiri_traits::RenderLimits,
    ) -> Result<(), RenderError> {
        let media_context = &self.config.media_context;
        if *media_context == raikiri_style::MediaContext::default() {
            return Ok(());
        }
        let mut first_page = raikiri_style::PageContextQuery::default();
        first_page.is_first = true;
        first_page.is_right = true;
        let mut options = raikiri_style::CascadeOptions::default();
        options.limits = limits.cascade_limits();
        document.cascade = raikiri_style::cascade_with_options(
            &document.uncascaded.dom,
            &build_rule_tree(&document.uncascaded),
            media_context,
            &first_page,
            &options,
        )?;
        Ok(())
    }

    /// Finish and cascade a parsed document, leaving out what a streaming
    /// layout does not apply: `<style>` elements inside `<body>`, and fixed
    /// positioned elements whose node id is `late_nodes_from` or more. With
    /// `forward_dependent`, also report whether the style sheets hold
    /// selectors that depend on later content.
    fn cascade(
        &self,
        mut parsed: UncascadedDocument,
        forward_dependent: bool,
        late_nodes_from: Option<usize>,
    ) -> Result<Cascaded, RenderError> {
        let mut ignored = Vec::new();
        let body_sheets = collect_body_inline_stylesheet_ids(&parsed.dom);
        let kept = parsed.stylesheet_sources.len() - body_sheets.len();
        parsed.stylesheet_sources.truncate(kept);
        ignored.extend(body_sheets.into_iter().map(|id| RenderWarning {
            kind: WarningKind::StreamingContentIgnored,
            node_id: Some(id),
            details: "a <style> element inside <body> is not applied while streaming".to_owned(),
        }));

        let network_ref = self
            .network
            .as_ref()
            .map(|provider| provider as &dyn NetworkProvider);
        let extra_stylesheets = self.resources.extra_stylesheets();
        let options = self
            .resources
            .parse_options(&extra_stylesheets, network_ref);
        let limits = self.resources.parse_limits();
        let uncascaded = finish_document(parsed, &options).map_err(RenderError::Parse)?;
        let forward_dependent =
            forward_dependent && build_rule_tree(&uncascaded).has_forward_dependent_selectors();
        let mut document = assemble_document(uncascaded, &options, &limits)?;
        self.recascade_for_layout(&mut document, &limits)?;

        let late =
            late_nodes_from.map_or_else(Vec::new, |from| late_fixed_elements(&document, from));
        if !late.is_empty() {
            let mut uncascaded = document.uncascaded;
            for &id in &late {
                hide_element(&mut uncascaded.dom, id);
                ignored.push(RenderWarning {
                    kind: WarningKind::StreamingContentIgnored,
                    node_id: Some(NodeId(id as u64)),
                    details: "a position: fixed element arrived after pages were delivered"
                        .to_owned(),
                });
            }
            document = assemble_document(uncascaded, &options, &limits)?;
            self.recascade_for_layout(&mut document, &limits)?;
        }
        Ok(Cascaded {
            document,
            forward_dependent,
            ignored,
        })
    }

    /// Lay `document` out, collecting its consumer property events.
    /// Returns `None` when the abort signal fires.
    fn run_layout(
        &self,
        document: &HtmlDocument,
    ) -> Result<Option<(DocumentLayout, Vec<ConsumerPropertyEvent>)>, RenderError> {
        let mut events = Vec::new();
        let mut collect = |event| {
            events.push(event);
            Ok(())
        };
        let mut layout_options = LayoutOptions::new()
            .resources(self.resources)
            .preload_background_images(self.preload_background_images);
        if !self.consumer_properties.is_empty() {
            layout_options =
                layout_options.consumer_properties(self.consumer_properties, &mut collect);
        }
        let status = layout(
            document,
            self.defaults.clone(),
            self.config.clone(),
            layout_options,
        )?;
        Ok(match status {
            LayoutStatus::Completed(laid_out) => Some((laid_out, events)),
            LayoutStatus::Aborted => None,
        })
    }
}

enum Failure {
    None,
    Reported,
    Pending(RenderError),
}

impl<'r, 'a, S: PageSink> StreamingLayout<'r, 'a, S> {
    /// Start a layout that uses `resources` for parsing and layout and
    /// delivers pages to `sink`.
    pub fn new(
        resources: &'r RenderResources<'a>,
        defaults: PageDefaults,
        config: LayoutConfig,
        sink: S,
    ) -> Self {
        let tree = RaikiriTreeSink::new(resources.parse_limits().max_parse_warnings);
        Self {
            settings: Settings {
                resources,
                defaults,
                config,
                consumer_properties: &[],
                preload_background_images: true,
                network: resources.network_adapter().map(StreamNetwork::new),
            },
            sink,
            input: Utf8Feed::new(parse_document(tree, ParseOpts::default())),
            input_bytes: 0,
            unchecked_bytes: 0,
            checkpoint_bytes: CHECKPOINT_BYTES,
            delivered: 0,
            late_nodes_from: None,
            aborted: false,
            failure: Failure::None,
        }
    }

    #[cfg(test)]
    fn checkpoint_bytes(mut self, bytes: usize) -> Self {
        self.checkpoint_bytes = bytes;
        self
    }

    /// Register consumer-owned properties. Their resolved values reach the
    /// sink with the page that holds each element's first fragment.
    pub fn consumer_properties(
        mut self,
        registrations: &'r [ConsumerPropertyRegistration],
    ) -> Self {
        self.settings.consumer_properties = registrations;
        self
    }

    /// Whether to fetch and decode CSS background images during layout.
    /// Defaults to `true`.
    pub fn preload_background_images(mut self, enabled: bool) -> Self {
        self.settings.preload_background_images = enabled;
        self
    }

    /// Parse the next piece of the document, and deliver the pages that
    /// became final when a checkpoint is due.
    ///
    /// A character may be split across calls. After an error the layout
    /// cannot continue: later calls and [`StreamingLayout::finish`] return
    /// [`RenderError::Configuration`].
    ///
    /// # Errors
    ///
    /// - `RenderError::Parse(ParseError::Encoding)`: the input is not UTF-8.
    /// - `RenderError::LimitExceeded { kind: LimitKind::InputBytes, .. }`:
    ///   more input than the configured input byte cap.
    /// - The cascade and layout errors of [`layout`] at a checkpoint, and
    ///   [`RenderError::Io`] when the sink fails.
    pub fn feed(&mut self, bytes: &[u8]) -> Result<(), RenderError> {
        if !matches!(self.failure, Failure::None) {
            return Err(failed_earlier());
        }
        let result = self.feed_unchecked(bytes);
        if result.is_err() {
            self.failure = Failure::Reported;
        }
        result
    }

    fn feed_unchecked(&mut self, bytes: &[u8]) -> Result<(), RenderError> {
        self.input_bytes = self.input_bytes.saturating_add(bytes.len() as u64);
        if let Some(cap) = self.settings.resources.parse_limits().max_input_bytes
            && self.input_bytes > cap
        {
            return Err(RenderError::LimitExceeded {
                kind: LimitKind::InputBytes,
                limit: cap,
                actual: self.input_bytes,
            });
        }
        // Split the input at checkpoint boundaries, so that checkpoints follow
        // the input however the caller chunks it.
        let mut rest = bytes;
        while !rest.is_empty() {
            if self.aborted {
                return self.input.feed(rest).map_err(RenderError::Parse);
            }
            let room = self
                .checkpoint_bytes
                .saturating_sub(self.unchecked_bytes)
                .max(1);
            let (piece, tail) = rest.split_at(room.min(rest.len()));
            rest = tail;
            self.input.feed(piece).map_err(RenderError::Parse)?;
            self.unchecked_bytes += piece.len();
            if self.unchecked_bytes >= self.checkpoint_bytes {
                self.unchecked_bytes = 0;
                self.checkpoint()?;
            }
        }
        Ok(())
    }

    /// Lay out the document parsed so far and deliver its final pages.
    fn checkpoint(&mut self) -> Result<(), RenderError> {
        let builder = &self.input.parser().tokenizer.sink;
        let traced = traced_handles(builder);
        let parsed = builder.sink.snapshot();
        let Cascaded {
            document,
            forward_dependent,
            ..
        } = self.settings.cascade(parsed, true, self.late_nodes_from)?;
        let frontier = stable_frontier(
            &document.uncascaded.dom,
            &document.cascade,
            &traced,
            forward_dependent,
        );
        if frontier == Frontier::At(document.uncascaded.dom.root_index()) {
            return Ok(());
        }
        let Some((laid_out, events)) = self.settings.run_layout(&document)? else {
            self.aborted = true;
            return Ok(());
        };
        let final_pages = final_page_count(
            &document.uncascaded.dom,
            &document.cascade,
            frontier,
            &laid_out,
        );
        let signal = self.settings.config.signal.clone();
        let (mut by_page, _) = events_by_first_page(&laid_out, events);
        let delivery = deliver(
            &mut self.sink,
            &laid_out,
            &mut by_page,
            &mut self.delivered,
            final_pages,
            Some(page_count_placeholder(&self.settings.config)),
            signal.as_ref(),
        )?;
        if delivery == Delivery::Aborted {
            self.aborted = true;
        }
        if self.delivered > 0 && self.late_nodes_from.is_none() {
            self.late_nodes_from = Some(document.uncascaded.dom.node_count());
            self.input
                .parser()
                .tokenizer
                .sink
                .sink
                .freeze_root_attributes();
        }
        Ok(())
    }

    /// End the input, lay the document out, and deliver the remaining pages
    /// and the summary to the sink.
    ///
    /// # Errors
    ///
    /// Returns the parse, limit, cascade and layout errors of
    /// [`crate::parse_html_with_resources`] and [`layout`], an input error
    /// kept from [`std::io::Write`], and [`RenderError::Io`] when the sink
    /// fails.
    pub fn finish(self) -> Result<StreamStatus<S::Output>, RenderError> {
        match self.failure {
            Failure::None => {}
            Failure::Reported => return Err(failed_earlier()),
            Failure::Pending(error) => return Err(error),
        }
        if self.aborted {
            return Ok(StreamStatus::Aborted);
        }
        let Self {
            settings,
            input,
            mut sink,
            mut delivered,
            late_nodes_from,
            ..
        } = self;
        let parsed = input.finish().map_err(RenderError::Parse)?;
        let Cascaded {
            mut document,
            ignored,
            ..
        } = settings.cascade(parsed, false, late_nodes_from)?;
        document.uncascaded.warnings.extend(ignored);
        let Some((laid_out, events)) = settings.run_layout(&document)? else {
            return Ok(StreamStatus::Aborted);
        };
        let signal = settings.config.signal.clone();
        let (mut by_page, unplaced_events) = events_by_first_page(&laid_out, events);
        let Some(page_count_margin_boxes) = page_count_margin_boxes(
            &laid_out,
            delivered,
            page_count_placeholder(&settings.config),
            signal.as_ref(),
        ) else {
            return Ok(StreamStatus::Aborted);
        };
        let delivery = deliver(
            &mut sink,
            &laid_out,
            &mut by_page,
            &mut delivered,
            laid_out.page_count(),
            None,
            signal.as_ref(),
        )?;
        // The sink can fire the signal while handling the last page.
        if delivery == Delivery::Aborted || signal.is_some_and(|signal| signal.is_aborted()) {
            return Ok(StreamStatus::Aborted);
        }
        let page_count = laid_out.page_count();
        let DocumentLayout {
            mut out, anchors, ..
        } = laid_out;
        let summary = StreamSummary {
            page_count,
            anchors,
            warnings: std::mem::take(&mut out.warnings),
            unplaced_events,
            page_count_margin_boxes,
        };
        sink.finish(summary)
            .map(StreamStatus::Completed)
            .map_err(RenderError::Io)
    }
}

impl<S: PageSink> Write for StreamingLayout<'_, '_, S> {
    /// Feed `buf`. An input error is reported as an
    /// [`std::io::ErrorKind::Other`] error here and returned unchanged by
    /// [`StreamingLayout::finish`].
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if !matches!(self.failure, Failure::None) {
            return Err(std::io::Error::other(failed_earlier().to_string()));
        }
        match self.feed_unchecked(buf) {
            Ok(()) => Ok(buf.len()),
            Err(error) => {
                let io = std::io::Error::other(error.to_string());
                self.failure = Failure::Pending(error);
                Err(io)
            }
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Delivery {
    Done,
    Aborted,
}

/// Hand pages `*delivered..until` of `laid_out` to `sink`, with the events
/// bucketed for them in `by_page`, advancing `*delivered` past each page.
fn deliver<S: PageSink>(
    sink: &mut S,
    laid_out: &DocumentLayout,
    by_page: &mut [Vec<ConsumerPropertyEvent>],
    delivered: &mut u32,
    until: u32,
    page_count_placeholder: Option<u32>,
    signal: Option<&raikiri_traits::AbortSignal>,
) -> Result<Delivery, RenderError> {
    while *delivered < until {
        if signal.is_some_and(|signal| signal.is_aborted()) {
            return Ok(Delivery::Aborted);
        }
        let index = *delivered;
        let page = StreamPage {
            layout: laid_out,
            index,
            page_count_placeholder,
        };
        let events = std::mem::take(&mut by_page[index as usize]);
        sink.page(page, events).map_err(RenderError::Io)?;
        *delivered += 1;
    }
    Ok(Delivery::Done)
}

/// The number standing for the page count on pages delivered before it is
/// known: all nines, with as many digits as the page limit has.
fn page_count_placeholder(config: &LayoutConfig) -> u32 {
    let digits = config
        .limits
        .max_document_pages
        .map_or(u32::MAX.ilog10() + 1, |limit| limit.max(1).ilog10() + 1)
        .min(9);
    10_u32.pow(digits) - 1
}

/// The margin boxes of the first `delivered` pages, with the real page count,
/// for the pages whose margin boxes showed `placeholder` when delivered.
/// Returns `None` when the abort signal fires.
fn page_count_margin_boxes(
    laid_out: &DocumentLayout,
    delivered: u32,
    placeholder: u32,
    signal: Option<&raikiri_traits::AbortSignal>,
) -> Option<Vec<(u32, Vec<MarginBox>)>> {
    let mut boxes = Vec::new();
    for index in 0..delivered.min(laid_out.page_count()) {
        if signal.is_some_and(|signal| signal.is_aborted()) {
            return None;
        }
        let page = laid_out.page_at(index as usize);
        let deferred = page
            .with_deferred_page_count(placeholder)
            .margin_boxes()
            .iter()
            .any(|margin_box| !margin_box.deferred.is_empty());
        if deferred {
            boxes.push((index, page.margin_boxes()));
        }
    }
    Some(boxes)
}

fn failed_earlier() -> RenderError {
    RenderError::Configuration("streaming layout input already failed".to_owned())
}

/// Bucket `events` by the page of each element's first fragment. Events of
/// elements without fragments come back separately. Document order is kept
/// within each bucket.
fn events_by_first_page(
    laid_out: &DocumentLayout,
    events: Vec<ConsumerPropertyEvent>,
) -> (Vec<Vec<ConsumerPropertyEvent>>, Vec<ConsumerPropertyEvent>) {
    let page_count = laid_out.page_count() as usize;
    let mut by_page = vec![Vec::new(); page_count];
    let mut unplaced = Vec::new();
    if events.is_empty() {
        return (by_page, unplaced);
    }
    let mut first_page = HashMap::<NodeId, usize>::new();
    for index in 0..page_count {
        for fragment in laid_out.page_at(index).fragments() {
            first_page.entry(fragment.node()).or_insert(index);
        }
    }
    for event in events {
        match first_page.get(&event.node_id) {
            Some(&index) => by_page[index].push(event),
            None => unplaced.push(event),
        }
    }
    (by_page, unplaced)
}

mod frontier;

#[cfg(test)]
mod tests;
