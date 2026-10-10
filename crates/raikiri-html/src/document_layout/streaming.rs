//! Push-style layout: feed HTML bytes as they are produced and receive the
//! laid-out pages in order through a [`PageSink`].
//!
//! [`StreamingLayout`] parses each chunk as soon as it is fed. Pages are
//! handed to the sink once they are final; today that happens when
//! [`StreamingLayout::finish`] runs the whole layout, so every page arrives
//! after the input has ended. The API is shaped for pages that become final
//! earlier, while input is still arriving: a sink must not assume that it
//! sees the first page only after the last byte.

use std::collections::HashMap;
use std::io::Write;

use html5ever::driver::{ParseOpts, Parser, parse_document};
use raikiri_traits::{
    ConsumerPropertyEvent, LayoutConfig, LimitKind, NetworkProvider, NodeId, PageDefaults,
    RenderError, RenderWarning,
};

use super::{
    AnchorIndex, DocumentLayout, LayoutOptions, LayoutStatus, Page, RunningElementLayout, layout,
};
use crate::document_parse::assemble_document;
use crate::input::Utf8Feed;
use crate::parse::finish_document;
use crate::{ConsumerPropertyRegistration, RaikiriTreeSink, RenderResources};

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
}

impl<'a> StreamPage<'a> {
    /// Zero-based page index in the whole document.
    pub fn index(&self) -> u32 {
        self.index
    }

    /// The laid-out page.
    pub fn page(&self) -> Page<'a> {
        self.layout.page_at(self.index as usize)
    }

    /// Lay running element `node` out at `width` CSS px. See
    /// [`DocumentLayout::layout_running_element`].
    pub fn layout_running_element(
        &self,
        node: NodeId,
        width: f32,
    ) -> Result<Option<RunningElementLayout>, RenderError> {
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
/// [`std::io::Write`], then call [`StreamingLayout::finish`]. Input must be
/// UTF-8. The input byte cap and the other parse limits come from the
/// [`RenderResources`]. Only print media is supported.
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
    resources: &'r RenderResources<'a>,
    defaults: PageDefaults,
    config: LayoutConfig,
    consumer_properties: &'r [ConsumerPropertyRegistration],
    preload_background_images: bool,
    sink: S,
    input: Utf8Feed<Parser<RaikiriTreeSink>>,
    input_bytes: u64,
    /// The first input failure. [`StreamingLayout::feed`] returns it to its
    /// caller; a failure inside [`std::io::Write`] is kept here for
    /// [`StreamingLayout::finish`].
    failure: Failure,
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
            resources,
            defaults,
            config,
            consumer_properties: &[],
            preload_background_images: true,
            sink,
            input: Utf8Feed::new(parse_document(tree, ParseOpts::default())),
            input_bytes: 0,
            failure: Failure::None,
        }
    }

    /// Register consumer-owned properties. Their resolved values reach the
    /// sink with the page that holds each element's first fragment.
    pub fn consumer_properties(
        mut self,
        registrations: &'r [ConsumerPropertyRegistration],
    ) -> Self {
        self.consumer_properties = registrations;
        self
    }

    /// Whether to fetch and decode CSS background images during layout.
    /// Defaults to `true`.
    pub fn preload_background_images(mut self, enabled: bool) -> Self {
        self.preload_background_images = enabled;
        self
    }

    /// Parse the next piece of the document.
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
        if let Some(cap) = self.resources.parse_limits().max_input_bytes
            && self.input_bytes > cap
        {
            return Err(RenderError::LimitExceeded {
                kind: LimitKind::InputBytes,
                limit: cap,
                actual: self.input_bytes,
            });
        }
        self.input.feed(bytes).map_err(RenderError::Parse)
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
        let network = self.resources.network_adapter();
        let network_ref = network
            .as_ref()
            .map(|provider| provider as &dyn NetworkProvider);
        let extra_stylesheets = self.resources.extra_stylesheets();
        let options = self
            .resources
            .parse_options(&extra_stylesheets, network_ref);
        let parsed = self.input.finish().map_err(RenderError::Parse)?;
        let uncascaded = finish_document(parsed, &options).map_err(RenderError::Parse)?;
        let document = assemble_document(uncascaded, &options, &self.resources.parse_limits())?;

        let signal = self.config.signal.clone();
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
        let LayoutStatus::Completed(laid_out) =
            layout(&document, self.defaults, self.config, layout_options)?
        else {
            return Ok(StreamStatus::Aborted);
        };

        let (mut by_page, unplaced_events) = events_by_first_page(&laid_out, events);
        let mut sink = self.sink;
        for index in 0..laid_out.page_count() {
            if signal.as_ref().is_some_and(|signal| signal.is_aborted()) {
                return Ok(StreamStatus::Aborted);
            }
            let page = StreamPage {
                layout: &laid_out,
                index,
            };
            let events = std::mem::take(&mut by_page[index as usize]);
            sink.page(page, events).map_err(RenderError::Io)?;
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

#[cfg_attr(
    not(test),
    allow(dead_code, reason = "the progressive driver is not wired in yet")
)]
mod frontier;

#[cfg(test)]
mod tests;
