//! The layout of the pages of a run, once its first page is known: the
//! pagination passes that settle the page geometry, the projection of the
//! pages, and the layouts of the pages whose content width differs.

use super::stop::Stop;
use super::{
    Continuation, PageCascader, ResumePoint, Run, continuation, page_geometry_schedule,
    page_query_for_slice, resolve_page_geometries,
};
use raikiri_dom::{
    Document, PageContentInsets, PageMargins, PageSlice, ResolvedPageGeometry,
    layout_pages_with_options,
};
use raikiri_style::{CascadeOptions, CascadeResult, PageCascadeResult, cascade_with_options};
use raikiri_traits::{LayoutError, PageBox, RenderError};

/// The pages of a run, laid out.
pub(super) struct LaidOut {
    pub(super) document: Document,
    pub(super) slices: Vec<PageSlice>,
    pub(super) geometries: Vec<ResolvedPageGeometry>,
    pub(super) page_styles: Vec<PageCascadeResult>,
    pub(super) continuations: Vec<Continuation>,
}

/// What the layout of the pages reads about the first page.
pub(super) struct PageLayout<'r, 'a> {
    pub(super) run: &'r Run<'r, 'a>,
    pub(super) first_cascade: &'r CascadeResult,
    pub(super) page_box: PageBox,
    /// The `@page` cascade of every page after the first.
    pub(super) page_cascader: &'r PageCascader<'r>,
    /// The options the element cascade ran with.
    pub(super) cascade_options: &'r CascadeOptions,
}

impl PageLayout<'_, '_> {
    /// Lay out the pages of `dom`, from the first page or, with `resume`,
    /// from where an earlier layout of the same document stopped.
    pub(super) fn lay_out(
        &self,
        dom: Document,
        resume: Option<&ResumePoint>,
    ) -> Result<LaidOut, Stop> {
        // Pages whose content width differs from the first page's are laid out
        // again from a copy of the document as it is before any layout.
        let pristine = dom.clone();
        match resume {
            Some(resume) => self.lay_out_resumed(dom, pristine, resume),
            None => self.lay_out_fresh(dom, pristine),
        }
    }

    /// A relayout of later pages runs the run's element cascade, on the same
    /// input and with the same options, with the page context of its first
    /// page; the run's own element cascade already succeeded on them.
    fn cascade_for_page(&self, slice: &PageSlice) -> CascadeResult {
        cascade_with_options(
            &self.run.doc.uncascaded.dom,
            self.run.tree,
            &self.run.config.media_context,
            &page_query_for_slice(slice),
            self.cascade_options,
        )
        .expect("cascade は常に Ok のはず")
    }

    /// The geometry of each page of `slices`.
    fn page_geometries(&self, slices: &[PageSlice]) -> Vec<ResolvedPageGeometry> {
        resolve_page_geometries(self.page_cascader, self.run.defaults, slices).0
    }

    /// Give the pages of `slices` their content, in the page areas
    /// `geometries` describe.
    fn project(
        &self,
        document: &mut Document,
        slices: &[PageSlice],
        geometries: &[(PageBox, PageMargins, PageContentInsets)],
    ) -> Result<(), LayoutError> {
        document.project_pages_with_control(
            self.first_cascade,
            self.page_box,
            slices,
            geometries,
            self.run.page_control,
        )
    }

    /// Lay out the pages after the page `resume` names.
    fn lay_out_resumed(
        &self,
        document: Document,
        pristine: Document,
        resume: &ResumePoint,
    ) -> Result<LaidOut, Stop> {
        let run = self.run;
        let cascade_for_page = |slice: &PageSlice| self.cascade_for_page(slice);
        let page_geometries_for = |slices: &[PageSlice]| self.page_geometries(slices);
        let mut slices: Vec<PageSlice> = resume
            .page_names
            .iter()
            .enumerate()
            .map(|(index, name)| PageSlice {
                page_index: index as u32,
                content_origin_y: 0.0,
                page_name: name.clone(),
            })
            .collect();
        let mut page_geometries = self.page_geometries(&slices);
        let resumed = continuation::resume_at(
            &continuation::ContinuationInputs {
                pristine: &pristine,
                cascade_for_page: &cascade_for_page,
                page_geometries: &page_geometries_for,
                resolver: run.resolver,
                base_url: run.base_url,
                max_pages: run.config.limits.max_document_pages,
                abort_check: run.is_aborted,
                skipped: &resume.skipped,
            },
            &document,
            self.first_cascade,
            &mut slices,
            &mut page_geometries,
            resume.first_page as usize,
            resume.start,
            &resume.earlier_fragments,
        );
        drop(pristine);
        let Some(continuations) = resumed? else {
            return Err(Stop::NotResumed);
        };
        let page_styles = resolve_page_geometries(self.page_cascader, run.defaults, &slices).1;
        Ok(LaidOut {
            document,
            slices,
            geometries: page_geometries,
            page_styles,
            continuations,
        })
    }

    /// Lay out the pages from the first page.
    fn lay_out_fresh(&self, mut document: Document, pristine: Document) -> Result<LaidOut, Stop> {
        let run = self.run;
        let cascade_for_page = |slice: &PageSlice| self.cascade_for_page(slice);
        let page_geometries_for = |slices: &[PageSlice]| self.page_geometries(slices);
        // The passes below lay the same content out with the same element
        // cascade at other page sizes: they break the lines of the first
        // pass's paragraphs again instead of shaping them anew.
        document.set_ifc_retain_paragraphs(true);
        let mut slices = layout_pages_with_options(
            &mut document,
            self.first_cascade,
            self.page_box,
            raikiri_dom::PageLayoutOptions {
                images: Some(raikiri_dom::PageImageOptions {
                    resolver: run.resolver,
                    base_url: run.base_url,
                }),
                control: Some(run.discovery_control),
                ..Default::default()
            },
        )?;
        let pagination_truncated = run.discovery_control.page_limit_reached();
        run.check_aborted()?;
        let geometry_converged =
            self.settle_page_geometries(&mut document, &mut slices, pagination_truncated)?;
        document.set_ifc_retain_paragraphs(false);
        require_settled(geometry_converged)?;
        let total_pages = u64::try_from(slices.len()).unwrap_or(u64::MAX);
        require_within_page_limit(run.config.limits.max_document_pages, total_pages)?;

        // Resolve once more after the final bounded schedule pass so metadata and
        // page names always describe the slices that will actually be emitted.
        let (final_geometries, page_styles) =
            resolve_page_geometries(self.page_cascader, run.defaults, &slices);
        let mut page_geometries = final_geometries;

        let geometries: Vec<_> = page_geometries
            .iter()
            .map(|geometry| (geometry.page_box, geometry.margins, geometry.content_insets))
            .collect();
        self.project(&mut document, &slices, &geometries)?;

        let continuations = continuation::continue_at_page_widths(
            &continuation::ContinuationInputs {
                pristine: &pristine,
                cascade_for_page: &cascade_for_page,
                page_geometries: &page_geometries_for,
                resolver: run.resolver,
                base_url: run.base_url,
                max_pages: run.config.limits.max_document_pages,
                abort_check: run.is_aborted,
                skipped: &[],
            },
            &document,
            self.first_cascade,
            &mut slices,
            &mut page_geometries,
        )?;
        drop(pristine);
        let page_styles = if let Some(first) = continuations.first() {
            // The first layout keeps only the pages before the first continuation.
            let own = first.first_page as usize;
            // Root and body go on in the continuations, which number their
            // later fragments.
            document.continue_fragments_after(continuation::root_fragments_after(
                &document,
                &continuations,
                slices.len(),
            ));
            // These pages were projected once already with the same limits.
            self.project(&mut document, &slices[..own], &geometries[..own])?;
            resolve_page_geometries(self.page_cascader, run.defaults, &slices).1
        } else {
            page_styles
        };
        Ok(LaidOut {
            document,
            slices,
            geometries: page_geometries,
            page_styles,
            continuations,
        })
    }

    /// Paginate `document` again at the page sizes its pages resolve to,
    /// until the sizes stop changing, and give back whether they did.
    /// `slices` are the pages of the pass that came before and become those
    /// of the last one; `pagination_truncated` tells whether that pass
    /// stopped at the page limit.
    fn settle_page_geometries(
        &self,
        document: &mut Document,
        slices: &mut Vec<PageSlice>,
        mut pagination_truncated: bool,
    ) -> Result<bool, Stop> {
        let run = self.run;
        let mut page_geometries = self.page_geometries(slices);
        let mut geometry_converged = true;
        let mut schedule_changed = false;
        for pass in 0..MAX_PAGE_GEOMETRY_PASSES {
            let geometry_varies = page_geometries
                .first()
                .is_some_and(|first| page_geometries.iter().any(|geometry| geometry != first));
            let schedule_must_be_applied = std::mem::take(&mut schedule_changed);
            if !geometry_varies && !pagination_truncated && !schedule_must_be_applied {
                break;
            }

            let schedule = page_geometry_schedule(&page_geometries, slices);
            // The first pagination pass establishes page count, names, and source
            // coordinates. If page selectors resolve different used geometry, rerun
            // the existing scheduled paginator with producer-owned page steps/widths.
            // Provisional passes stop at the configured page count and return only
            // the prefix needed to discover page geometry. Once the schedule is
            // stable, a strict pass confirms the final count.
            let scheduled_control = if geometry_varies {
                run.discovery_control
            } else {
                run.page_control
            };
            *slices = layout_pages_with_options(
                document,
                self.first_cascade,
                self.page_box,
                raikiri_dom::PageLayoutOptions {
                    page_steps: &schedule.page_steps,
                    page_widths: &schedule.page_widths,
                    images: Some(raikiri_dom::PageImageOptions {
                        resolver: run.resolver,
                        base_url: run.base_url,
                    }),
                    control: Some(scheduled_control),
                },
            )?;
            run.check_aborted()?;
            pagination_truncated = geometry_varies && run.discovery_control.page_limit_reached();
            // A scheduled pass can change both page count and page selectors. Re-
            // resolve before the next iteration so the following schedule is
            // derived from the slices it will actually replace.
            page_geometries = self.page_geometries(slices);
            let mut refreshed_schedule = page_geometry_schedule(&page_geometries, slices);
            if refreshed_schedule == schedule && pagination_truncated {
                // The soft pass confirmed the geometry schedule only for the
                // configured prefix. Re-run it strictly before treating that
                // prefix as a complete document.
                *slices = layout_pages_with_options(
                    document,
                    self.first_cascade,
                    self.page_box,
                    raikiri_dom::PageLayoutOptions {
                        page_steps: &schedule.page_steps,
                        page_widths: &schedule.page_widths,
                        images: Some(raikiri_dom::PageImageOptions {
                            resolver: run.resolver,
                            base_url: run.base_url,
                        }),
                        control: Some(run.page_control),
                    },
                )?;
                run.check_aborted()?;
                pagination_truncated = false;
                page_geometries = self.page_geometries(slices);
                refreshed_schedule = page_geometry_schedule(&page_geometries, slices);
            }
            if refreshed_schedule == schedule && !pagination_truncated {
                break;
            }
            // A refreshed schedule may differ due to page names or page count even
            // when every page currently has the same geometry. Apply it before converging.
            schedule_changed = refreshed_schedule != schedule;
            // A schedule still changing in the last bounded pass has not settled.
            geometry_converged &= pass + 1 != MAX_PAGE_GEOMETRY_PASSES;
        }
        Ok(geometry_converged)
    }
}

/// How many times the page geometry is settled before the layout gives up.
const MAX_PAGE_GEOMETRY_PASSES: u32 = 3;

/// Fail unless the page geometry settled within the bounded passes, so that
/// no inconsistent pages are emitted.
pub(super) fn require_settled(geometry_converged: bool) -> Result<(), RenderError> {
    if geometry_converged {
        Ok(())
    } else {
        Err(RenderError::PageGeometryDidNotConverge {
            iterations: MAX_PAGE_GEOMETRY_PASSES,
        })
    }
}

/// Fail when a layout of `total_pages` pages exceeds `max_pages`. The strict
/// paginator rejects excess pages before it returns its slices, so this is a
/// last check on what it returned.
pub(super) fn require_within_page_limit(
    max_pages: Option<u32>,
    total_pages: u64,
) -> Result<(), RenderError> {
    match max_pages {
        Some(limit) if total_pages > u64::from(limit) => Err(RenderError::LimitExceeded {
            kind: raikiri_traits::LimitKind::Pages,
            limit: u64::from(limit),
            actual: u64::from(limit) + 1,
        }),
        _ => Ok(()),
    }
}
