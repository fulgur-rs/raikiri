//! Owned layout results and borrowed per-page views for drawing consumers.

mod dom_view;
mod fragment;
mod page;

pub use dom_view::DomView;
pub use fragment::{Fragment, FragmentKind, RepeatKind};
pub use page::{Page, PageGeometry, PageMode};

use crate::render::{PipelineInputs, PipelineOutput, PipelineRun, run_pipeline};
use crate::{ConsumerPropertyRegistration, HtmlDocument, RenderResources};
use raikiri_traits::{
    ConsumerPropertyObserver, PageDefaults, RenderError, RenderWarning, StreamingConfig,
};

/// Result of [`layout`].
#[non_exhaustive]
pub enum LayoutStatus {
    /// Layout finished.
    Completed(DocumentLayout),
    /// The abort signal fired. No partial result is returned.
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
/// after an error or abort.
pub fn layout(
    doc: &HtmlDocument,
    defaults: PageDefaults,
    config: StreamingConfig,
    options: LayoutOptions<'_, '_>,
) -> Result<LayoutStatus, RenderError> {
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
    let out = match run {
        PipelineRun::Completed(out) => out,
        PipelineRun::Aborted => return Ok(LayoutStatus::Aborted),
    };
    if signal.as_ref().is_some_and(|signal| signal.is_aborted()) {
        return Ok(LayoutStatus::Aborted);
    }
    Ok(LayoutStatus::Completed(DocumentLayout { out }))
}

/// An owned, laid-out document.
pub struct DocumentLayout {
    out: PipelineOutput,
}

impl DocumentLayout {
    /// Number of pages.
    pub fn page_count(&self) -> u32 {
        u32::try_from(self.out.pages.len()).unwrap_or(u32::MAX)
    }

    /// Pages in order.
    pub fn pages(&self) -> impl ExactSizeIterator<Item = Page<'_>> + '_ {
        (0..self.out.pages.len()).map(move |i| self.page_at(i))
    }

    /// One page, or `None` when out of range.
    pub fn page(&self, index: u32) -> Option<Page<'_>> {
        let i = index as usize;
        (i < self.out.pages.len()).then(|| self.page_at(i))
    }

    fn page_at(&self, i: usize) -> Page<'_> {
        Page {
            fragment: &self.out.pages[i],
            style: &self.out.page_styles[i],
            document: &self.out.document,
            cascade: &self.out.cascade,
        }
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
