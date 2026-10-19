//! The first page of a layout run: its element cascade and its page
//! context, which size the page area that everything else is laid out in.

use super::{
    ImagePreloads, ResumePoint, Run, cascade_page_for_query, enable_inline_engine,
    map_initial_page_context_error, page_area_size, page_box_for_page,
};
use crate::cascade::first_page_query;
use crate::resources::push_resource_warning;
use raikiri_dom::{InitialPageProbeResources, first_page_name, resolve_initial_page_context};
use raikiri_style::{
    Atom, CascadeOptions, CascadeResult, PageContextQuery, PageInheritance,
    cascade_page_with_media_context_and_border_width_device_scale, cascade_with_options,
};
use raikiri_traits::{PageBox, RenderError, RenderWarning, ResourceKind, WarningKind};

/// What the first page leaves for the layout that follows it.
pub(super) struct FirstPage {
    /// The document, with its fonts, before any layout.
    pub(super) dom: raikiri_dom::Document,
    /// The options the element cascade ran with.
    pub(super) cascade_options: CascadeOptions,
    /// The element cascade, with the page context of the first page.
    pub(super) first_cascade: CascadeResult,
    pub(super) page_box: PageBox,
    /// The resources fetched for the first page, which later fetches count
    /// with.
    pub(super) preloads: ImagePreloads,
}

impl Run<'_, '_> {
    /// Cascade the first page and resolve its page context.
    ///
    /// A resumed layout takes the first page's name from `resume`.
    pub(super) fn first_page(
        &self,
        resume: Option<&ResumePoint>,
    ) -> Result<FirstPage, RenderError> {
        let media_context = &self.config.media_context;
        let base_url = self.base_url;
        // Resolve the first page context before layout so `:first` and the first
        // resolved `@page size` participate in the initial fragmentainer.
        let mut first_query = first_page_query();
        // The fonts are given to the document before the element cascade, which
        // measures `line-height: normal` in the document's fonts for the `lh`
        // unit, and before
        // the first-page probe, which lays out a clone of this document, so the
        // probe and the layout below use the same fonts.
        let mut dom = self.doc.uncascaded.dom.clone();
        // Paint resolves relative `url()` values against the base the images
        // were fetched with, which the resources' fallback base can change.
        if let Some(base_url) = base_url {
            dom.set_base_url(Some(base_url.clone()));
        }
        let font_report =
            enable_inline_engine(&mut dom, self.resources, self.font_faces, self.font_loader);
        let cascade_options = self.element_cascade_options(&dom, &first_query);
        let mut first_cascade = self.element_cascade(&cascade_options, &first_query)?;
        // The first class-A box can select a named page. Resolve that name before
        // the initial layout so a named `:first` page is not flattened to the
        // anonymous page geometry. Only the page context depends on the name.
        // A resumed layout takes the first page's name from the earlier layout.
        let first_name = match resume {
            Some(resume) => resume.page_names.first().cloned().flatten(),
            None => first_page_name(&self.doc.uncascaded.dom, &first_cascade),
        };
        if let Some(name) = first_name {
            first_query.page_name = Some(Atom::from(name.as_str()));
            let page =
                cascade_page_for_query(self.tree, &first_cascade, media_context, &first_query);
            first_cascade.replace_page(page);
        }
        let page_box = page_box_for_page(&first_cascade.page, self.defaults);

        let mut preloads = ImagePreloads::default();
        self.prepare_marker_images(&mut dom, &first_cascade, &mut preloads);
        // A resumed layout already knows the first page's name, which the probe
        // would look for in content the layout skips.
        let (first_cascade, page_box) = if resume.is_some() {
            (first_cascade, page_box)
        } else {
            self.probe_initial_page_context(&mut dom, &mut first_query, first_cascade, page_box)?
        };
        let mut first = FirstPage {
            dom,
            cascade_options,
            first_cascade,
            page_box,
            preloads,
        };
        self.cascade_in_page_area(&mut first, &first_query)?;
        self.warn_about_skipped_fonts(font_report.skipped);
        Ok(first)
    }

    /// Probe the document for the first page's name and page context, and
    /// give `dom` the paragraphs the probe laid out.
    ///
    /// A resumed layout does not probe: it already knows the first page's name
    /// and the probe would look for it in content the layout skips.
    fn probe_initial_page_context(
        &self,
        dom: &mut raikiri_dom::Document,
        first_query: &mut PageContextQuery,
        first_cascade: CascadeResult,
        page_box: PageBox,
    ) -> Result<(CascadeResult, PageBox), RenderError> {
        let media_context = &self.config.media_context;
        let mut resolved_initial_context = resolve_initial_page_context(
            dom,
            first_query.page_name.as_ref().map(ToString::to_string),
            first_cascade,
            page_box,
            InitialPageProbeResources::new(Some(self.resolver), self.base_url),
            |page_name, cascade| {
                first_query.page_name = page_name.map(Atom::from);
                let page = cascade_page_for_query(self.tree, cascade, media_context, first_query);
                let page_box = page_box_for_page(&page, self.defaults);
                (page, page_box)
            },
        )
        .map_err(map_initial_page_context_error)?;
        resolved_initial_context.give_paragraphs_to(dom);
        Ok((
            resolved_initial_context.cascade,
            resolved_initial_context.page_box,
        ))
    }

    /// Run the element cascade again when the first page's page area is not
    /// the viewport it ran in.
    ///
    /// A named first page, or page lengths relative to the root's font, can
    /// give the first page another page area than the provisional one. The
    /// element cascade is then run again in that viewport; the page context
    /// it already resolved is kept.
    fn cascade_in_page_area(
        &self,
        first: &mut FirstPage,
        first_query: &PageContextQuery,
    ) -> Result<(), RenderError> {
        let page_area = page_area_size(&first.first_cascade.page, self.defaults);
        if first.cascade_options.viewport != Some(page_area) {
            first.cascade_options.viewport = Some(page_area);
            let page = first.first_cascade.page.clone();
            first.first_cascade = self.element_cascade(&first.cascade_options, first_query)?;
            first.first_cascade.replace_page(page);
            // Marker images without full intrinsic dimensions are sized from
            // the marker's font, which can be viewport-relative.
            self.prepare_marker_images(&mut first.dom, &first.first_cascade, &mut first.preloads);
        }
        Ok(())
    }

    /// Warn about each `@font-face` family in `skipped`, which had no usable
    /// source.
    fn warn_about_skipped_fonts(&self, skipped: Vec<String>) {
        for family in skipped {
            push_resource_warning(
                self.warnings,
                RenderWarning {
                    kind: WarningKind::ResourceFallback {
                        kind: ResourceKind::Font,
                        url: None,
                    },
                    node_id: None,
                    details: format!(
                        "@font-face family {family:?} had no usable source; fallback fonts will be used"
                    ),
                },
            );
        }
    }

    /// The options the element cascade of the run starts with.
    ///
    /// The viewport-percentage lengths need the first page's page area before
    /// the element cascade, whose root the page context inherits from. Take it
    /// from the page context inheriting initial values; the root's styles only
    /// matter to font-relative page lengths, which [`Self::cascade_in_page_area`] checks
    /// again once the page context is resolved.
    fn element_cascade_options(
        &self,
        dom: &raikiri_dom::Document,
        first_query: &PageContextQuery,
    ) -> CascadeOptions {
        let mut cascade_options = CascadeOptions::default();
        cascade_options.limits = self.config.limits.cascade_limits();
        cascade_options.border_width_device_scale =
            self.resources.configured_border_width_device_scale();
        // The element cascade below runs on the source document, which may have
        // no fonts, so the measurer comes from `dom`, which has them.
        cascade_options.normal_line_height = Some(dom.normal_line_height());
        let provisional_page = cascade_page_with_media_context_and_border_width_device_scale(
            self.tree,
            first_query,
            PageInheritance::LegacyInitialValues,
            &self.config.media_context,
            cascade_options.border_width_device_scale,
        );
        cascade_options.viewport = Some(page_area_size(&provisional_page, self.defaults));
        cascade_options
    }

    /// The element cascade of the source document with the page context of
    /// `query`.
    fn element_cascade(
        &self,
        cascade_options: &CascadeOptions,
        query: &PageContextQuery,
    ) -> Result<CascadeResult, RenderError> {
        Ok(cascade_with_options(
            &self.doc.uncascaded.dom,
            self.tree,
            &self.config.media_context,
            query,
            cascade_options,
        )?)
    }

    /// Fetch the images of the list markers of `cascade` and hand them to
    /// `dom`.
    fn prepare_marker_images(
        &self,
        dom: &mut raikiri_dom::Document,
        cascade: &CascadeResult,
        preloads: &mut ImagePreloads,
    ) {
        self.resources.preload_list_marker_images(
            cascade,
            self.base_url,
            self.warnings,
            &mut preloads.seen,
            &mut preloads.attempts,
            self.signal,
        );
        if let Some(source) = self.resources.image_pixel_source_ref() {
            dom.prepare_list_marker_images(cascade, source, self.base_url);
        }
    }
}
