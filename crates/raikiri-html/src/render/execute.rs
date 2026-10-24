//! The stages of a layout run, in order, and the output they leave.

use super::continuation::prepare_margin_box_images as prepare_images;
use super::first_page::FirstPage;
use super::page_layout::{LaidOut, PageLayout};
use super::stop::Stop;
use super::{
    ImagePreloads, PageCascader, PipelineInputs, PipelineOutput, Run, page_query_for_slice,
    resolved_consumer_property_events,
};
use raikiri_dom::PageSlice;
use raikiri_traits::{RenderError, RenderWarning};

impl Run<'_, '_> {
    /// Lay out the document, fetch what needs its final page schedule, and
    /// deliver its consumer properties.
    pub(super) fn execute(&self, inputs: PipelineInputs<'_, '_>) -> Result<PipelineOutput, Stop> {
        let FirstPage {
            dom,
            cascade_options,
            first_cascade,
            page_box,
            mut preloads,
        } = self.first_page(inputs.resume)?;
        // Only `page` in a cascade result depends on the page query, so the
        // first-page cascade serves as the element cascade of every later page.
        let page_cascader =
            PageCascader::new(self.tree, &first_cascade, &self.config.media_context);

        let layout = PageLayout {
            run: self,
            first_cascade: &first_cascade,
            page_box,
            page_cascader: &page_cascader,
            cascade_options: &cascade_options,
        };
        let LaidOut {
            mut document,
            slices,
            geometries,
            page_styles,
            mut continuations,
        } = layout.lay_out(dom, inputs.resume)?;

        // Fetch CSS background sources only after the final page schedule is known.
        // Paint remains read-only and consumes the cache through ImagePixelSource.
        if inputs.preload_background_images {
            let urls = self.preload_page_background_images(&page_cascader, &slices, &mut preloads);
            // Margin boxes are laid out later, a page at a time, and need the
            // sizes of the images their content shows.
            if let Some(source) = self.resources.image_pixel_source_ref() {
                document.prepare_margin_box_image_urls(&urls, source, self.base_url);
                prepare_images(&mut continuations, &urls, source, self.base_url);
            }
            // A fetch above may have observed cancellation; do not hand out
            // pages whose backgrounds were left unloaded.
            self.check_aborted()?;
        }

        if let Some(property_observer) = inputs.property_observer {
            // Cancellation can race after layout and before observer delivery.
            self.check_aborted()?;
            for event in resolved_consumer_property_events(
                &document,
                &first_cascade,
                inputs.consumer_properties,
            ) {
                property_observer
                    .observe_event(event)
                    .map_err(RenderError::Observer)?;
            }
        }

        Ok(PipelineOutput {
            document,
            cascade: first_cascade,
            continuations,
            slices,
            geometries,
            page_styles,
            warnings: self.all_warnings(),
            replaced_resources: self.resolver.take_outcomes(),
            base_url: self.base_url.cloned(),
        })
    }

    /// Fetch the CSS background images of the elements of the document and of
    /// the page contexts of `slices`, and the images the margin boxes of those
    /// pages show with `content`. Returns the authored URLs of the latter, each
    /// once.
    fn preload_page_background_images(
        &self,
        cascader: &PageCascader<'_>,
        slices: &[PageSlice],
        preloads: &mut ImagePreloads,
    ) -> Vec<String> {
        // Element backgrounds are page-independent: scan them once, before the
        // page-context backgrounds of the first page. A per-slice rescan would
        // only revisit URLs already recorded in `seen`, or ones skipped before
        // reaching it, so attempt order, deduplication, and limits are unchanged.
        slices.first().into_iter().for_each(|_| {
            self.resources.preload_element_background_images(
                cascader.base,
                self.base_url,
                self.warnings,
                &mut preloads.seen,
                &mut preloads.attempts,
                self.signal,
            );
        });
        let mut content_images = Vec::new();
        let mut listed = std::collections::HashSet::new();
        for slice in slices {
            let page = cascader.page(&page_query_for_slice(slice));
            let urls = self.resources.preload_page_context_background_images(
                &page,
                self.base_url,
                self.warnings,
                &mut preloads.seen,
                &mut preloads.attempts,
                self.signal,
            );
            content_images.extend(urls.into_iter().filter(|url| listed.insert(url.clone())));
        }
        content_images
    }

    /// Every warning of the run: those found parsing the document, then those
    /// reported since.
    fn all_warnings(&self) -> Vec<RenderWarning> {
        let mut all = self.doc.uncascaded.warnings.clone();
        all.extend(
            self.warnings
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .iter()
                .cloned(),
        );
        all
    }
}
