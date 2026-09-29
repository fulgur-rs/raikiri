//! [`DocumentHost`] over a prepared WPT document: one screen viewport. Style
//! and layout are rebuilt from scratch on every `flush` call; it is the
//! runtime that only calls `flush` when a DOM mutation happened since the
//! last one, right before a layout-dependent read.
//!
//! External scripts are read from the WPT checkout on disk; nothing outside
//! the checkout root is ever read (see [`WptDocumentHost::script_source`]).

use std::path::{Component, Path, PathBuf};

use raikiri_js::runtime::{BoxGeometry, DocumentHost, DomRect, HostError, PositionKind};
use raikiri_style::property::DisplayValue;

use crate::reftest::{
    LiveWptSetup, live_wpt_stylesheet_sources_in_subtree, parse_wpt_inner_html_fragment,
    update_live_wpt_stylesheet_sources,
};

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
    style_sources: Vec<String>,
    /// Descendant content extents per arena index, rebuilt on every `flush`.
    /// Each entry is the union of descendant border-box right/bottom edges,
    /// or negative infinity when no descendant has a fragment. The scroll
    /// size itself still needs the padding box, so it is derived per call
    /// from this cache without walking the subtree again.
    scroll_content_extents: Vec<(f64, f64)>,
    #[cfg(test)]
    pub(crate) flushes: std::rc::Rc<std::cell::Cell<usize>>,
}

impl WptDocumentHost {
    pub(crate) fn new(setup: LiveWptSetup, wpt_root: &Path) -> Self {
        let root = setup.uncascaded.dom.root_index();
        let style_sources = live_wpt_stylesheet_sources_in_subtree(&setup.uncascaded.dom, root);
        Self {
            setup,
            wpt_root: wpt_root.to_path_buf(),
            canonical_root: std::fs::canonicalize(wpt_root).ok(),
            page_url: None,
            page_scene: None,
            computed_styles: None,
            style_sources,
            scroll_content_extents: Vec::new(),
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
        for candidate in [path.clone(), root.join(rerooted)] {
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
        Err(refuse("no such file inside the WPT root"))
    }

    /// Diff the connected `<style>` sources against the last flush and apply
    /// the change to the setup's author stylesheet list. Parser-loaded
    /// `<link>` sheets in that list are left untouched.
    fn resync_stylesheets(&mut self) {
        let root = self.setup.uncascaded.dom.root_index();
        let current = live_wpt_stylesheet_sources_in_subtree(&self.setup.uncascaded.dom, root);
        let mut removed = self.style_sources.clone();
        let mut added = Vec::new();
        for source in &current {
            if let Some(i) = removed.iter().position(|s| s == source) {
                removed.remove(i);
            } else {
                added.push(source.clone());
            }
        }
        if !removed.is_empty() || !added.is_empty() {
            update_live_wpt_stylesheet_sources(&mut self.setup, removed, added, &self.wpt_root);
        }
        self.style_sources = current;
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
            .or(self.setup.document_base_url.as_ref())
            .map(ToString::to_string)
    }

    fn fetch_script(&mut self, url: &str) -> Result<String, HostError> {
        self.script_source(url)
    }

    fn flush(&mut self) -> Result<(), HostError> {
        #[cfg(test)]
        self.flushes.set(self.flushes.get() + 1);
        self.resync_stylesheets();
        self.setup.uncascaded.dom.mark_in_document_flags();
        let mut cascade = raikiri::build_cascaded_with_media_context_for_page(
            &self.setup.uncascaded,
            &self.setup.media_context,
            &self.setup.page_query,
        );
        let mut font_context = self.setup.font_context.clone();
        raikiri_dom::expand_font_face_aliases(
            &mut cascade.computed,
            self.setup.font_face_tree.font_faces(),
            &mut font_context,
        );
        let image_resolver = raikiri_net::ImageResolver::new(raikiri_net::FileNetworkProvider);
        raikiri_dom::layout_single_page_with_resolver_and_base_url(
            &mut self.setup.uncascaded.dom,
            &cascade,
            self.setup.page_box,
            font_context,
            &image_resolver,
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
            page_name,
        ));
        self.computed_styles = Some(cascade.computed);
        let scene = self.page_scene.as_ref().expect("scene just built");
        self.scroll_content_extents =
            compute_scroll_content_extents(&self.setup.uncascaded.dom, scene);
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
            .unwrap_or((f64::NEG_INFINITY, f64::NEG_INFINITY));
        let padding = self
            .setup
            .uncascaded
            .dom
            .get_node(node)
            .map(|n| n.unrounded_layout.padding)
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
            is_inline: computed.is_some_and(|c| c.display == DisplayValue::Inline),
        }))
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
        let mut font_context = None;
        let mut ch_advance = |font: &raikiri_style::ChFontKey| {
            let font_context = font_context.get_or_insert_with(|| self.setup.font_context.clone());
            raikiri_dom::layout::measure_ch_advance_for_font_key(font_context, font)
        };
        Ok(property.serialize(computed, &mut ch_advance))
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
/// Each entry is the union of descendant border-box right/bottom edges (the
/// content half of the scrolling area in CSSOM View §6 "scrolling area":
/// the union of the padding box with every descendant fragment, extended by
/// the end-side padding after that content per CSS Overflow 3 §3.3
/// "Scrollable Overflow"; this layout only produces in-flow descendants).
/// Negative infinity when no descendant has a fragment. Descendants
/// extending above or left of the padding box stay in the union; the caller
/// clamps them out when deriving the scroll size from the padding box
/// origin, since they are not reachable by scrolling.
///
/// The page scene only has fragments for nodes that intersect the page, so
/// descendants laid out entirely off the page are not counted.
fn compute_scroll_content_extents(
    document: &raikiri_dom::Document,
    scene: &raikiri::PageScene,
) -> Vec<(f64, f64)> {
    let count = document.node_count();
    let mut extents = vec![(f64::NEG_INFINITY, f64::NEG_INFINITY); count];
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
                let mut content_right = f64::NEG_INFINITY;
                let mut content_bottom = f64::NEG_INFINITY;
                if let Some(node) = document.get_node(index) {
                    for &child in &node.children {
                        if let Some(descendant) = border_box_of(scene, child) {
                            content_right = content_right.max(descendant.right);
                            content_bottom = content_bottom.max(descendant.bottom);
                        }
                        if child < count {
                            content_right = content_right.max(extents[child].0);
                            content_bottom = content_bottom.max(extents[child].1);
                        }
                    }
                }
                extents[index] = (content_right, content_bottom);
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

/// The runtime's view of computed `position`. `static`, a running element
/// (`position: running(...)`, taken out of the flow into a page margin box),
/// and any value this crate does not know yet (the enum is
/// `#[non_exhaustive]`) all read as `static`.
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
