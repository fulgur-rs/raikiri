//! [`DocumentHost`] over a prepared WPT document: one screen viewport. Style
//! and layout are rebuilt from scratch on every `flush` call; it is the
//! runtime that only calls `flush` when a DOM mutation happened since the
//! last one, right before a layout-dependent read.
//!
//! External scripts are read from the WPT checkout on disk, except for two
//! embedded helpers (the crate's own report script and the verbatim upstream
//! reftest-wait helper for the unmaterialized `/common` root); nothing else
//! outside the checkout root is ever read (see
//! [`WptDocumentHost::script_source`]).

use std::any::Any;
use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};

use raikiri_js::runtime::{BoxGeometry, DocumentHost, DomRect, HostError, PositionKind};

use crate::reftest::{
    LiveWptSetup, live_wpt_stylesheets_in_subtree, parse_wpt_inner_html_fragment,
    update_live_wpt_stylesheet_sources,
};

/// The shared `/common/reftest-wait.js` helper, verbatim upstream at the
/// pinned WPT SHA (`scripts/wpt/pinned_sha.txt`). It defines
/// `takeScreenshot()` (removes `reftest-wait` from the document element),
/// `takeScreenshotDelayed(timeout)`, and `failIfNot(condition, msg)`.
/// Served only when the sparse checkout does not materialize the file;
/// the disk copy wins whenever both exist, so a future `/common` checkout
/// change takes effect without code churn.
const REFTEST_WAIT_JS: &str = r#"/**
 * Remove the `reftest-wait` class on the document element.
 * The reftest runner will wait with taking a screenshot while
 * this class is present.
 *
 * See https://web-platform-tests.org/writing-tests/reftests.html#controlling-when-comparison-occurs
 */
function takeScreenshot() {
    document.documentElement.classList.remove("reftest-wait");
}

/**
 * Call `takeScreenshot()` after a delay of at least |timeout| milliseconds.
 * @param {number} timeout - milliseconds
 */
function takeScreenshotDelayed(timeout) {
    setTimeout(function() {
        takeScreenshot();
    }, timeout);
}

/**
 * Ensure that a precondition is met before waiting for a screenshot.
 * @param {bool} condition - Fail the test if this evaluates to false
 * @param {string} msg - Error message to write to the screenshot
 */
function failIfNot(condition, msg) {
  const fail = () => {
    (document.body || document.documentElement).textContent = `Precondition Failed: ${msg}`;
    takeScreenshot();
  };
  if (!condition) {
    if (document.readyState == "interactive") {
      fail();
    } else {
      document.addEventListener("DOMContentLoaded", fail, false);
    }
  }
}
"#;

pub(crate) struct WptDocumentHost {
    setup: LiveWptSetup,
    wpt_root: PathBuf,
    /// `wpt_root` canonicalized once, for the containment check of every
    /// script read; `None` when the root does not exist, which refuses every
    /// script fetch.
    canonical_root: Option<PathBuf>,
    /// The page's own `file:` URL, when the caller knows which file the
    /// document came from; otherwise the document's base URL stands in.
    page_url: Option<raikiri::Url>,
    page_scene: Option<raikiri::PageScene>,
    computed_styles: Option<Vec<raikiri_style::ComputedValues>>,
    /// `<style>` sources connected at the last flush, in tree order.
    style_sources: Vec<raikiri_html::StylesheetSource>,
    /// Descendant content extents per arena index, rebuilt on every `flush`.
    /// Each entry is the bounding box of the descendant border boxes as
    /// painted, in the node's own untransformed coordinates, or `None` when
    /// no descendant has a fragment. The scroll size itself still needs the
    /// padding box, so it is derived per call from this cache without
    /// walking the subtree again.
    scroll_content_extents: Vec<Option<DomRect>>,
    /// Per arena index, the transform the painter draws the node's boxes
    /// through from their layout coordinates, or `None` when a singular
    /// transform hides them. Rebuilt on every `flush`; a node past the end
    /// is drawn where layout put it.
    paint_transforms: Vec<Option<raikiri_traits::PaintTransform>>,
    /// The border boxes of the `::before` and `::after` boxes laid out in
    /// paragraphs, by originating element, in the coordinates of
    /// [`border_box_of`]. Rebuilt on every `flush`.
    generated_boxes: HashMap<usize, Vec<DomRect>>,
    #[cfg(test)]
    pub(crate) flushes: std::rc::Rc<std::cell::Cell<usize>>,
}

/// Expand connected `<style>` text the way the fragment parser does.
///
/// The fragment parser's `stylesheet_sources` already expands `@import` and
/// keeps SVG-namespace `<style>` text. The live host keeps only the fragment
/// DOM, so it expands the reconciled text here with the same document base
/// and checkout network; without this an `innerHTML`-inserted `@import`
/// would stay opaque.
fn expand_live_style_sources(
    raw: Vec<(String, Option<String>)>,
    base_url: Option<&raikiri::Url>,
    wpt_root: &Path,
) -> Vec<raikiri_html::StylesheetSource> {
    if std::fs::canonicalize(wpt_root).is_err() {
        return raw
            .into_iter()
            .map(|(source, media)| raikiri_html::StylesheetSource::new(source, media))
            .collect();
    }
    let network = crate::reftest::local_resources::WptFileNetworkProvider::new(
        std::fs::canonicalize(wpt_root).ok(),
    );
    let (sources, media): (Vec<_>, Vec<_>) = raw.into_iter().unzip();
    raikiri_html::expand_live_stylesheet_imports(
        sources,
        base_url,
        Some(&network as &dyn raikiri_traits::NetworkProvider),
    )
    .into_iter()
    .zip(media)
    .map(|(mut sheet, media)| {
        sheet.media = media;
        sheet
    })
    .collect()
}

impl WptDocumentHost {
    /// Resolve `url` through a checkout-rooted image resolver, returning the
    /// resolver (which caches the decoded source) and the parsed URL.
    fn resolve_image(
        &self,
        url: &str,
    ) -> Result<
        (
            raikiri_net::ImageResolver<crate::reftest::local_resources::WptFileNetworkProvider>,
            raikiri::Url,
        ),
        HostError,
    > {
        let root = self
            .canonical_root
            .clone()
            .ok_or_else(|| HostError(format!("{url}: the WPT root does not exist")))?;
        let parsed = raikiri::Url::parse(url)
            .map_err(|_| HostError(format!("{url}: not an absolute URL")))?;
        let resolver = raikiri_net::ImageResolver::new(
            crate::reftest::local_resources::WptFileNetworkProvider::new(Some(root)),
        );
        raikiri_traits::ReplacedResolver::resolve(
            &resolver,
            raikiri_traits::ResolverRequest::new(&parsed),
        )
        .map_err(|error| HostError(format!("{url}: {error}")))?;
        Ok((resolver, parsed))
    }

    pub(crate) fn new(setup: LiveWptSetup, wpt_root: &Path) -> Self {
        let root = setup.uncascaded.dom.root_index();
        let raw = live_wpt_stylesheets_in_subtree(&setup.uncascaded.dom, root);
        let style_sources =
            expand_live_style_sources(raw, setup.document_base_url.as_ref(), wpt_root);
        Self {
            setup,
            wpt_root: wpt_root.to_path_buf(),
            canonical_root: std::fs::canonicalize(wpt_root).ok(),
            page_url: None,
            page_scene: None,
            computed_styles: None,
            style_sources,
            scroll_content_extents: Vec::new(),
            paint_transforms: Vec::new(),
            generated_boxes: HashMap::new(),
            #[cfg(test)]
            flushes: Default::default(),
        }
    }

    /// Report `url` as the document's URL (`document.URL`, `location`, and
    /// the base that relative `<script src>` values resolve against).
    pub(crate) fn with_page_url(mut self, url: raikiri::Url) -> Self {
        self.page_url = Some(url);
        self
    }

    /// The source text of the external script at `url`, an absolute URL
    /// already resolved against the document URL.
    ///
    /// Only `file:` URLs are fetched. The URL's path is tried twice: as the
    /// real filesystem path it names (a relative `src` merged with the page's
    /// own URL, which already lies inside the checkout), and then re-rooted
    /// at the WPT root (WPT writes support files as root-relative paths such
    /// as `/resources/testharness.js`, which a `file:` base with an empty
    /// host resolves to the filesystem root). The first candidate whose
    /// canonical path lies inside the canonical WPT root is read; `..`
    /// segments, symlinks, and absolute paths that leave the root fail both
    /// candidates and are refused.
    ///
    /// `<wpt_root>/resources/testharnessreport.js` is never read from disk:
    /// it is answered with this crate's own report script, matched on the
    /// candidate path before canonicalization (the file need not exist).
    ///
    /// `<wpt_root>/common/reftest-wait.js` is the shared reftest helper
    /// (verbatim upstream at the pinned WPT SHA, see `REFTEST_WAIT_JS`): the
    /// stable sparse checkout does not materialize `/common`, so the file is
    /// tried from disk first and answered from the embedded copy only when
    /// neither candidate exists. Any other missing support file still fails
    /// closed as a fetch error; this fallback never suppresses one.
    fn script_source(&self, url: &str) -> Result<String, HostError> {
        let refuse = |reason: &str| HostError(format!("{url}: {reason}"));
        let parsed = raikiri::Url::parse(url).map_err(|_| refuse("not an absolute URL"))?;
        if parsed.scheme() != "file" {
            return Err(refuse("only file: URLs are fetched"));
        }
        let path = parsed
            .to_file_path()
            .map_err(|()| refuse("not a local file path"))?;
        let root = self
            .canonical_root
            .as_deref()
            .ok_or_else(|| refuse("the WPT root does not exist"))?;
        let rerooted: PathBuf = path
            .components()
            .filter(|component| matches!(component, Component::Normal(_)))
            .collect();
        let report = root.join("resources").join("testharnessreport.js");
        for candidate in [path.clone(), root.join(&rerooted)] {
            if candidate == report {
                return Ok(crate::testharness_page::REPORT_SCRIPT.to_owned());
            }
            let Ok(canonical) = std::fs::canonicalize(&candidate) else {
                continue;
            };
            if canonical.starts_with(root) {
                return std::fs::read_to_string(&canonical)
                    .map_err(|error| refuse(&format!("read failed: {error}")));
            }
        }
        // The sparse checkout does not materialize `/common`: serve the
        // embedded verbatim helper only for that one path, after both disk
        // candidates failed. Anything else still fails closed below.
        if path.ends_with("common/reftest-wait.js") {
            return Ok(REFTEST_WAIT_JS.to_owned());
        }
        Err(refuse("no such file inside the WPT root"))
    }

    /// Reconcile connected `<style>` sources against the last flush and apply
    /// the change to the setup's author stylesheet list. Parser-loaded
    /// `<link>` sheets in that list are left untouched at the front; inline
    /// sheets follow in tree order.
    fn resync_stylesheets(&mut self) {
        let root = self.setup.uncascaded.dom.root_index();
        let raw = live_wpt_stylesheets_in_subtree(&self.setup.uncascaded.dom, root);
        let current =
            expand_live_style_sources(raw, self.setup.document_base_url.as_ref(), &self.wpt_root);
        if current != self.style_sources {
            let previous = std::mem::replace(&mut self.style_sources, current.clone());
            update_live_wpt_stylesheet_sources(
                &mut self.setup,
                &previous,
                &current,
                &self.wpt_root,
            );
        }
    }
}

impl DocumentHost for WptDocumentHost {
    fn document(&self) -> &raikiri_dom::Document {
        &self.setup.uncascaded.dom
    }

    fn document_mut(&mut self) -> &mut raikiri_dom::Document {
        &mut self.setup.uncascaded.dom
    }

    fn document_url(&self) -> Option<String> {
        self.page_url
            .as_ref()
            .or(self.setup.document_url.as_ref())
            .map(ToString::to_string)
    }

    fn fetch_script(&mut self, url: &str) -> Result<String, HostError> {
        self.script_source(url)
    }

    /// Load and decode the image the way layout will, through the same
    /// checkout-rooted file provider, so `load` fires exactly for the images
    /// the page renders and `error` for the broken ones. Without a checkout
    /// root nothing would confine a `file:` URL, so every load fails, the
    /// same as [`Self::script_source`].
    fn load_image(&mut self, url: &str) -> Result<(), HostError> {
        self.resolve_image(url).map(drop)
    }

    /// Decode the image again through the same checkout-rooted provider as
    /// [`Self::load_image`], rasterized at its concrete object size against
    /// `default_size`.
    fn image_pixels(
        &mut self,
        url: &str,
        default_size: (f64, f64),
    ) -> Option<raikiri_js::runtime::ImagePixels> {
        use raikiri_traits::ImagePixelSource;

        let (resolver, parsed) = self.resolve_image(url).ok()?;
        let natural = resolver.intrinsic_size(&parsed)?;
        let size = raikiri_dom::image_geometry::default_sized_object(natural, default_size);
        resolver
            .get_decoded_at_size(
                &parsed,
                raikiri_traits::ImageRasterSize {
                    width: size.0 as f32,
                    height: size.1 as f32,
                },
                None,
            )
            .map(|image| raikiri_js::runtime::ImagePixels { size, image })
    }

    fn flush(&mut self) -> Result<(), HostError> {
        #[cfg(test)]
        self.flushes.set(self.flushes.get() + 1);
        // A script may have added, changed, or removed `<base href>` since
        // the parse; everything below resolves against the current base.
        let document_url = self.page_url.as_ref().or(self.setup.document_url.as_ref());
        self.setup.document_base_url = self.setup.uncascaded.dom.live_base_url(document_url);
        self.resync_stylesheets();
        self.setup.uncascaded.dom.mark_in_document_flags();
        // Before the cascade, which measures `line-height: normal` in them.
        self.setup
            .uncascaded
            .dom
            .set_font_collection(self.setup.fonts.clone());
        let resolved = crate::reftest::resolved_live_viewport_document(
            &self.setup.uncascaded,
            &self.setup.media_context,
        );
        let mut cascade_options = raikiri::CascadeOptions::default();
        cascade_options.border_width_device_scale = Some(1.0);
        let cascade = raikiri::build_cascaded_with_options(
            &resolved,
            &self.setup.media_context,
            &self.setup.page_query,
            &[],
            &cascade_options,
        )
        .map_err(|error| HostError(error.to_string()))?;
        let image_resolver = raikiri_net::ImageResolver::new(
            crate::reftest::local_resources::WptFileNetworkProvider::new(
                self.canonical_root.clone(),
            ),
        );
        for (element, computed) in cascade.computed.iter().enumerate() {
            if !raikiri_dom::generated_content::inside_marker_in_flow(&cascade, element)
                || cascade
                    .pseudo
                    .get(&(
                        raikiri_style::StyleNodeId::new(element as u64),
                        raikiri_style::PseudoElem::Marker,
                    ))
                    .is_some_and(|marker| !marker.content.is_empty())
            {
                continue;
            }
            let raikiri_style::property::BackgroundImage::Url(raw) = &computed.list_style_image
            else {
                continue;
            };
            if let Some(url) = raikiri::Url::parse(raw).ok().or_else(|| {
                self.setup
                    .document_base_url
                    .as_ref()
                    .and_then(|base| base.join(raw).ok())
            }) {
                // CSS image loading keeps the text marker fallback on failure.
                let _ = raikiri_traits::ReplacedResolver::resolve(
                    &image_resolver,
                    raikiri_traits::ResolverRequest::new(&url),
                );
            }
        }
        crate::http_resources::prefetch_generated_content_images(
            &self.setup.uncascaded.dom,
            &cascade,
            self.setup.document_base_url.as_ref(),
            &image_resolver,
        );
        self.setup.uncascaded.dom.prepare_list_marker_images(
            &cascade,
            &image_resolver,
            self.setup.document_base_url.as_ref(),
        );
        raikiri_dom::layout_single_page_with_resolver_and_base_url(
            &mut self.setup.uncascaded.dom,
            &cascade,
            self.setup.page_box,
            &crate::reftest::local_resources::BrokenImageFallback::new(&image_resolver),
            self.setup.document_base_url.as_ref(),
        )
        .map_err(|error| HostError(format!("live DOM layout: {error:?}")))?;
        // The engine's page scene clips fragments to its page box. CSSOM rect
        // reads must still work for offscreen nodes, so only the geometry
        // projection uses a tall box; layout itself used the configured WPT
        // viewport above.
        let mut projection_box = self.setup.page_box;
        projection_box.height = projection_box.height.max(1_000_000.0);
        let page_name = self
            .setup
            .page_query
            .page_name
            .as_ref()
            .map(ToString::to_string);
        self.page_scene = Some(raikiri::build_page_scene_for_page_named(
            &self.setup.uncascaded.dom,
            &cascade,
            projection_box,
            0,
            0.0,
            page_name.clone(),
        ));
        // Layout leaves no page placements behind; the paint transforms below
        // are read from the same tall page the scene was cut from.
        self.setup
            .uncascaded
            .dom
            .project_pages(
                &cascade,
                projection_box,
                &[raikiri_dom::PageSlice {
                    page_index: 0,
                    content_origin_y: 0.0,
                    page_name,
                }],
                &[],
            )
            .map_err(|error| HostError(format!("live DOM page projection: {error:?}")))?;
        let document = &self.setup.uncascaded.dom;
        let nodes =
            (0..document.node_count()).map(|index| raikiri_traits::NodeId::new(index as u64));
        let paint_transforms = document.page_paint_transforms(&cascade, 0, nodes);
        // The paint order leaves out the root's own transform, which moves
        // the whole canvas (CSS Transforms 1 §6); a singular one hides it.
        let root = raikiri_dom::root_box(document, &cascade, self.setup.page_box, 0.0, 1)
            .and_then(|root| root.transform);
        self.paint_transforms = match root {
            None => paint_transforms,
            Some(root) if inverse(root).is_none() => vec![None; paint_transforms.len()],
            Some(root) => paint_transforms
                .into_iter()
                .map(|transform| transform.map(|transform| root * transform))
                .collect(),
        };
        let scene = self.page_scene.as_ref().expect("scene just built");
        self.generated_boxes = collect_generated_boxes(document, &cascade, scene);
        self.computed_styles = Some(cascade.into_cssom_computed());
        self.scroll_content_extents = compute_scroll_content_extents(
            document,
            scene,
            &self.paint_transforms,
            &self.generated_boxes,
        );
        Ok(())
    }

    fn box_geometry(&mut self, node: usize) -> Result<Option<BoxGeometry>, HostError> {
        let border_box = {
            let scene = self
                .page_scene
                .as_ref()
                .ok_or_else(|| HostError("geometry read before flush".into()))?;
            border_box_of(scene, node)
        };
        let Some(border_box) = border_box else {
            return Ok(None);
        };
        let computed = self
            .computed_styles
            .as_ref()
            .and_then(|styles| styles.get(node));
        let border = |side: fn(&raikiri_style::ComputedValues) -> f32| {
            computed.map_or(0.0, |c| f64::from(side(c)))
        };
        let padding_box = rect(
            border_box.left + border(|c| c.border.left.width().px()),
            border_box.top + border(|c| c.border.top.width().px()),
            border_box.right - border(|c| c.border.right.width().px()),
            border_box.bottom - border(|c| c.border.bottom.width().px()),
        );
        let (content_right, content_bottom) = self
            .scroll_content_extents
            .get(node)
            .copied()
            .flatten()
            .map_or((f64::NEG_INFINITY, f64::NEG_INFINITY), |content| {
                (content.right, content.bottom)
            });
        let padding = self
            .setup
            .uncascaded
            .dom
            .get_node(node)
            .map(|n| n.layout().padding)
            .unwrap_or_default();
        let right = padding_box
            .right
            .max(content_right + f64::from(padding.right));
        let bottom = padding_box
            .bottom
            .max(content_bottom + f64::from(padding.bottom));
        let (scroll_width, scroll_height) = (right - padding_box.left, bottom - padding_box.top);
        Ok(Some(BoxGeometry {
            border_box,
            padding_box,
            scroll_width,
            scroll_height,
            position: computed.map_or(PositionKind::Static, |c| position_kind(&c.position)),
            // An inline-level element that is blockified, as an absolutely
            // positioned one is, generates a block box.
            is_inline: computed.is_some_and(|c| {
                raikiri_dom::generates_inline_box(&self.setup.uncascaded.dom, node, c)
            }),
        }))
    }

    /// Being rendered here means no inclusive ancestor computes to
    /// `display: none`, is an HTML element layout never renders, such as
    /// `script`, whatever its computed display, is hidden by HTML's
    /// rendering rules for `hidden` (which the bundled UA stylesheet does
    /// not carry), or is an SVG never-rendered element such as `defs`. A
    /// box placed off every page still counts, which the page-scene
    /// geometry behind [`Self::box_geometry`] cannot tell apart.
    fn is_being_rendered(&mut self, node: usize) -> Result<bool, HostError> {
        let Some(styles) = self.computed_styles.as_ref() else {
            // cov:ignore: the runtime flushes before asking.
            return Err(HostError("rendering read before flush".into()));
        };
        let document = &self.setup.uncascaded.dom;
        let mut current = Some(node);
        while let Some(index) = current {
            let hidden = styles
                .get(index)
                .is_some_and(|style| style.display == raikiri_style::DisplayValue::None);
            if hidden
                || document
                    .get_node(index)
                    .is_some_and(|node| node.is_non_rendered_html_element())
                || is_never_rendered_svg_element(document, index)
                || is_hidden_html_element(document, index)
            {
                return Ok(false);
            }
            current = document.parent_of(index);
        }
        Ok(true)
    }

    fn is_visible(&mut self, node: usize) -> Result<bool, HostError> {
        let Some(styles) = self.computed_styles.as_ref() else {
            // cov:ignore: the runtime flushes before asking.
            return Err(HostError("rendering read before flush".into()));
        };
        Ok(styles
            .get(node)
            .is_none_or(|style| style.visibility == raikiri_style::property::Visibility::Visible))
    }

    fn viewport_size(&mut self) -> Result<Option<(f64, f64)>, HostError> {
        // The live document lays out one screen page at the viewport size.
        let page = self.setup.page_box;
        Ok(Some((f64::from(page.width), f64::from(page.height))))
    }

    /// The initial containing block (the viewport-sized page box at the
    /// origin) unioned with the margin box of every box laid out in it, as
    /// its transforms draw it. A
    /// fixed-position subtree whose containing block is the viewport stays
    /// with the viewport, so it adds nothing.
    fn viewport_scrolling_area(&mut self) -> Result<Option<DomRect>, HostError> {
        let Some(scene) = self.page_scene.as_ref() else {
            // cov:ignore: the runtime flushes before asking.
            return Err(HostError("scrolling area read before flush".into()));
        };
        let page = self.setup.page_box;
        let mut area = rect(0.0, 0.0, f64::from(page.width), f64::from(page.height));
        let document = &self.setup.uncascaded.dom;
        let styles = self.computed_styles.as_ref();
        // Each entry carries whether an ancestor is the containing block of
        // fixed-position descendants, which then move with that ancestor.
        let mut pending = vec![(document.root_index(), false)];
        while let Some((index, contained)) = pending.pop() {
            let style = styles.and_then(|styles| styles.get(index));
            let fixed =
                style.is_some_and(|style| position_kind(&style.position) == PositionKind::Fixed);
            if fixed && !contained {
                continue;
            }
            let node = document.get_node(index);
            let painted =
                border_box_of(scene, index).zip(paint_transform_of(&self.paint_transforms, index));
            if let Some((border_box, transform)) = painted {
                let margin = node.map(|n| n.layout().margin).unwrap_or_default();
                let margin_box = rect(
                    border_box.left - f64::from(margin.left),
                    border_box.top - f64::from(margin.top),
                    border_box.right + f64::from(margin.right),
                    border_box.bottom + f64::from(margin.bottom),
                );
                area = union(area, transform_rect(transform, margin_box));
            }
            if let Some(transform) = paint_transform_of(&self.paint_transforms, index) {
                for &generated in self.generated_boxes.get(&index).into_iter().flatten() {
                    area = union(area, transform_rect(transform, generated));
                }
            }
            let contained = contained
                || style.is_some_and(|style| {
                    raikiri_dom::establishes_fixed_containing_block(document, index, style)
                });
            let children = node.map(|n| &n.children[..]).unwrap_or_default();
            pending.extend(children.iter().map(|&child| (child, contained)));
        }
        Ok(Some(area))
    }

    /// A fixed-position box stays with the viewport unless an ancestor is
    /// the containing block of its fixed-position descendants (a transform,
    /// filter, containment or matching `will-change`).
    fn is_fixed_to_viewport(&mut self, node: usize) -> Result<bool, HostError> {
        let Some(styles) = self.computed_styles.as_ref() else {
            return Err(HostError("fixed position read before flush".into()));
        };
        let document = &self.setup.uncascaded.dom;
        // Walking up, a box that contains fixed-position descendants ends
        // the viewport attachment of every fixed box below it.
        let mut fixed = false;
        let mut current = Some(node);
        while let Some(index) = current {
            let style = styles.get(index);
            if style.is_some_and(|style| {
                raikiri_dom::establishes_fixed_containing_block(document, index, style)
            }) {
                fixed = false;
            }
            if style.is_some_and(|style| position_kind(&style.position) == PositionKind::Fixed) {
                fixed = true;
            }
            current = document.parent_of(index);
        }
        Ok(fixed)
    }

    fn computed_value(&mut self, node: usize, property: &str) -> Result<Option<String>, HostError> {
        let Some(property) = raikiri_style::ComputedProperty::from_name(property) else {
            return Ok(None);
        };
        let computed = self
            .computed_styles
            .as_ref()
            .and_then(|styles| styles.get(node))
            .ok_or_else(|| HostError(format!("computed style is missing for DOM node {node}")))?;
        Ok(property.serialize(computed))
    }

    fn parse_fragment(
        &mut self,
        context_tag: &str,
        context_ns: &str,
        markup: &str,
    ) -> Result<raikiri_dom::Document, HostError> {
        let fragment = parse_wpt_inner_html_fragment(
            markup,
            context_tag,
            context_ns,
            self.setup.page_box.width as u32,
            self.setup.page_box.height as u32,
            self.setup.document_base_url.as_ref(),
            self.setup.page_resource_base.as_deref(),
            &self.wpt_root,
        )
        .map_err(HostError)?; // cov:ignore: UTF-8 markup is parsed from memory; its reader and parser recover without I/O/encoding errors.
        Ok(fragment.dom)
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
    fn into_any(self: Box<Self>) -> Box<dyn Any> {
        self
    }
}

/// A `DomRect` from its four edges.
fn rect(left: f64, top: f64, right: f64, bottom: f64) -> DomRect {
    DomRect {
        left,
        top,
        right,
        bottom,
        width: right - left,
        height: bottom - top,
    }
}

/// The border box of `node`'s first page-scene fragment, in CSS px relative
/// to the initial containing block, or `None` when it has no fragment.
fn border_box_of(scene: &raikiri::PageScene, node: usize) -> Option<DomRect> {
    let node_id = raikiri_traits::NodeId::new(node as u64);
    let fragment = scene.fragments.get(&node_id)?.first()?;
    let (width, height) = scene
        .drawables
        .block_styles
        .get(&node_id)
        .and_then(|entry| entry.layout_size)
        .unwrap_or((fragment.width, fragment.height));
    let left = f64::from(fragment.x + scene.body_offset_pt.0);
    let top = f64::from(fragment.y + scene.body_offset_pt.1);
    Some(rect(
        left,
        top,
        left + f64::from(width),
        top + f64::from(height),
    ))
}

/// Descendant content extents for every arena index, in one post-order pass.
///
/// Each entry is the bounding box of the descendant border boxes (the
/// content half of the scrolling area in CSSOM View §6 "scrolling area":
/// the union of the padding box with every descendant fragment, extended by
/// the end-side padding after that content per CSS Overflow 3 §3.3
/// "Scrollable Overflow"; this layout only produces in-flow descendants).
/// A descendant counts where its transforms draw it, mapped back into the
/// node's own untransformed coordinates, since transforms affect scrollable
/// overflow (CSS Transforms 1 §3); a rotated or skewed box adds its bounding
/// box. `None` when no descendant has a fragment. Descendants extending
/// above or left of the padding box stay in the union; the caller clamps
/// them out when deriving the scroll size from the padding box origin, since
/// they are not reachable by scrolling.
///
/// The page scene only has fragments for nodes that intersect the page, so
/// descendants laid out entirely off the page are not counted.
fn compute_scroll_content_extents(
    document: &raikiri_dom::Document,
    scene: &raikiri::PageScene,
    transforms: &[Option<raikiri_traits::PaintTransform>],
    generated: &HashMap<usize, Vec<DomRect>>,
) -> Vec<Option<DomRect>> {
    let count = document.node_count();
    let mut extents: Vec<Option<DomRect>> = vec![None; count];
    let mut visited = vec![false; count];
    for start in 0..count {
        if visited[start] {
            continue;
        }
        let mut pending = vec![(start, false)];
        while let Some((index, expanded)) = pending.pop() {
            if index >= count {
                continue;
            }
            if expanded {
                // A node a singular transform hides draws its descendants
                // nowhere; measuring them unmoved keeps its own metrics.
                let own_transform = paint_transform_of(transforms, index);
                let hidden = own_transform.is_none();
                let to_local = own_transform
                    .and_then(inverse)
                    .unwrap_or(raikiri_traits::PaintTransform::IDENTITY);
                // The node's own generated boxes are drawn with it.
                let mut content = generated
                    .get(&index)
                    .into_iter()
                    .flatten()
                    .copied()
                    .reduce(union);
                for &child in document
                    .get_node(index)
                    .map_or(&[][..], |n| &n.children[..])
                {
                    // A child hidden only by its own singular transform adds
                    // nothing; inside a hidden subtree every child is hidden
                    // too, so it is measured unmoved relative to its parent.
                    let drawn = match paint_transform_of(transforms, child) {
                        Some(drawn) => drawn,
                        None if hidden => raikiri_traits::PaintTransform::IDENTITY,
                        None => continue,
                    };
                    let own = border_box_of(scene, child);
                    let below = extents.get(child).copied().flatten();
                    let reach = match (own, below) {
                        (Some(own), Some(below)) => Some(union(own, below)),
                        (reach, None) | (None, reach) => reach,
                    };
                    if let Some(reach) = reach {
                        let placed = transform_rect(to_local * drawn, reach);
                        content = Some(content.map_or(placed, |content| union(content, placed)));
                    }
                }
                extents[index] = content;
                visited[index] = true;
            } else {
                if visited[index] {
                    continue;
                }
                pending.push((index, true));
                if let Some(node) = document.get_node(index) {
                    for &child in &node.children {
                        if child < count && !visited[child] {
                            pending.push((child, false));
                        }
                    }
                }
            }
        }
    }
    extents
}

/// The border boxes of the `::before` and `::after` boxes that paragraphs lay
/// out, by originating element, in the coordinates of [`border_box_of`]. A
/// box split across lines has one rectangle per piece. A box with a
/// transform of its own reports the bounds it is drawn at, and one with a
/// singular transform reports nothing. Floated and
/// absolutely positioned generated boxes are not laid out in a paragraph and
/// have no box to report.
fn collect_generated_boxes(
    document: &raikiri_dom::Document,
    cascade: &raikiri_style::CascadeResult,
    scene: &raikiri::PageScene,
) -> HashMap<usize, Vec<DomRect>> {
    let mut boxes: HashMap<usize, Vec<DomRect>> = HashMap::new();
    for index in 0..document.node_count() {
        let root = document.get_node(index).filter(|node| node.is_ifc_root());
        let Some((node, border_box)) = root.zip(border_box_of(scene, index)) else {
            continue;
        };
        let layout = node.layout();
        let origin = (
            border_box.left + f64::from(layout.border.left + layout.padding.left),
            border_box.top + f64::from(layout.border.top + layout.padding.top),
        );
        let inline = node.ifc_inline_boxes().unwrap_or_default();
        let blocks = node.ifc_generated_block_pieces();
        let pieces = inline
            .iter()
            .map(|piece| (piece.node, piece.border_box))
            .chain(blocks.iter().map(|piece| (piece.node, piece.border_box)));
        for (id, piece) in pieces {
            let Some((owner, raikiri_style::PseudoElem::Before | raikiri_style::PseudoElem::After)) =
                raikiri_dom::generated_content::generated_origin(id)
            else {
                continue;
            };
            let left = origin.0 + f64::from(piece.x);
            let top = origin.1 + f64::from(piece.y);
            let area = rect(
                left,
                top,
                left + f64::from(piece.width),
                top + f64::from(piece.height),
            );
            // A transformable pseudo-element turns about its own box; a
            // singular transform paints nothing of it.
            let paint_rect = raikiri_traits::PaintRect::new(
                area.left as f32,
                area.top as f32,
                area.width as f32,
                area.height as f32,
            );
            let area = match raikiri_dom::paint_rules::pseudo_box_transform(cascade, id, paint_rect)
            {
                None => area,
                Some(None) => continue,
                Some(Some(transform)) => transform_rect(transform, area),
            };
            boxes.entry(owner).or_default().push(area);
        }
    }
    boxes
}

/// The transform the painter draws node `index` through: the identity past
/// the end of `transforms`, `None` when a singular transform hides it.
fn paint_transform_of(
    transforms: &[Option<raikiri_traits::PaintTransform>],
    index: usize,
) -> Option<raikiri_traits::PaintTransform> {
    transforms
        .get(index)
        .copied()
        .unwrap_or(Some(raikiri_traits::PaintTransform::IDENTITY))
}

/// The transform that undoes `transform`, or `None` when it is singular.
fn inverse(transform: raikiri_traits::PaintTransform) -> Option<raikiri_traits::PaintTransform> {
    let raikiri_traits::PaintTransform {
        a, b, c, d, e, f, ..
    } = transform;
    let determinant = a * d - b * c;
    if determinant == 0.0 || !determinant.is_finite() {
        return None;
    }
    Some(raikiri_traits::PaintTransform::new(
        d / determinant,
        -b / determinant,
        -c / determinant,
        a / determinant,
        (c * f - d * e) / determinant,
        (b * e - a * f) / determinant,
    ))
}

/// The bounding box of `area` as `transform` draws it.
fn transform_rect(transform: raikiri_traits::PaintTransform, area: DomRect) -> DomRect {
    if transform == raikiri_traits::PaintTransform::IDENTITY {
        return area;
    }
    let corners = [
        (area.left, area.top),
        (area.right, area.top),
        (area.left, area.bottom),
        (area.right, area.bottom),
    ]
    .map(|(x, y)| {
        let (x, y) = transform.apply((x as f32, y as f32));
        (f64::from(x), f64::from(y))
    });
    let (mut left, mut top, mut right, mut bottom) = (
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    );
    for (x, y) in corners {
        left = left.min(x);
        top = top.min(y);
        right = right.max(x);
        bottom = bottom.max(y);
    }
    rect(left, top, right, bottom)
}

/// The smallest rectangle holding both `first` and `second`.
fn union(first: DomRect, second: DomRect) -> DomRect {
    rect(
        first.left.min(second.left),
        first.top.min(second.top),
        first.right.max(second.right),
        first.bottom.max(second.bottom),
    )
}

/// The runtime's view of computed `position`. `static`, a running element
/// (`position: running(...)`, taken out of the flow into a page margin box),
/// and any value this crate does not know yet (the enum is
/// `#[non_exhaustive]`) all read as `static`.
/// Whether `index` is one of SVG's never-rendered elements, which with
/// their descendants are not rendered even when no style hides them.
///
/// Spec: <https://svgwg.org/svg2-draft/render.html#TermNeverRenderedElement>
fn is_never_rendered_svg_element(document: &raikiri_dom::Document, index: usize) -> bool {
    document.element_namespace_uri(index) == Some("http://www.w3.org/2000/svg")
        && document
            .get_node(index)
            .and_then(|node| node.tag_name())
            .is_some_and(|tag| {
                matches!(
                    tag,
                    "clipPath"
                        | "defs"
                        | "desc"
                        | "linearGradient"
                        | "marker"
                        | "mask"
                        | "metadata"
                        | "pattern"
                        | "radialGradient"
                        | "script"
                        | "style"
                        | "symbol"
                        | "title"
                )
            })
}

/// Whether `index` is an HTML element the HTML rendering rules hide: one
/// with a `hidden` attribute not in the until-found state (except `embed`),
/// or an `input` in the Hidden state.
///
/// Spec: <https://html.spec.whatwg.org/multipage/rendering.html#hidden-elements>
fn is_hidden_html_element(document: &raikiri_dom::Document, index: usize) -> bool {
    if document.element_namespace_uri(index) != Some("http://www.w3.org/1999/xhtml") {
        return false;
    }
    let tag = document.get_node(index).and_then(|node| node.tag_name());
    let attr = |name| document.element_attribute(index, name);
    let hidden = attr("hidden").is_some_and(|value| !value.eq_ignore_ascii_case("until-found"));
    (hidden && tag != Some("embed"))
        || (tag == Some("input")
            && attr("type").is_some_and(|ty| ty.eq_ignore_ascii_case("hidden")))
}

fn position_kind(value: &raikiri_style::property::PositionValue) -> PositionKind {
    use raikiri_style::property::PositionValue as P;
    match value {
        P::Relative => PositionKind::Relative,
        P::Absolute => PositionKind::Absolute,
        P::Fixed => PositionKind::Fixed,
        P::Sticky => PositionKind::Sticky,
        _ => PositionKind::Static,
    }
}

#[cfg(test)]
mod tests;
