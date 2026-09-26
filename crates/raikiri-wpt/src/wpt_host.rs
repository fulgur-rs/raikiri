//! [`DocumentHost`] over a prepared WPT document: one screen viewport. Style
//! and layout are rebuilt from scratch on every `flush` call; it is the
//! runtime that only calls `flush` when a DOM mutation happened since the
//! last one, right before a layout-dependent read.

use std::path::{Path, PathBuf};

use raikiri_js::runtime::{BoxGeometry, DocumentHost, DomRect, HostError, PositionKind};
use raikiri_style::property::DisplayValue;

use crate::reftest::{
    LiveWptSetup, live_wpt_stylesheet_sources_in_subtree, parse_wpt_inner_html_fragment,
    update_live_wpt_stylesheet_sources,
};

pub(crate) struct WptDocumentHost {
    setup: LiveWptSetup,
    wpt_root: PathBuf,
    page_scene: Option<raikiri::PageScene>,
    computed_styles: Option<Vec<raikiri_style::ComputedValues>>,
    /// `<style>` sources connected at the last flush, in tree order.
    style_sources: Vec<String>,
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
            page_scene: None,
            computed_styles: None,
            style_sources,
            #[cfg(test)]
            flushes: Default::default(),
        }
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
        Ok(())
    }

    fn box_geometry(&mut self, node: usize) -> Result<Option<BoxGeometry>, HostError> {
        let scene = self
            .page_scene
            .as_ref()
            .ok_or_else(|| HostError("geometry read before flush".into()))?;
        let Some(border_box) = border_box_of(scene, node) else {
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
        let (scroll_width, scroll_height) =
            scroll_extent(scene, &self.setup.uncascaded.dom, node, &padding_box);
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

/// The scrolling area size of `node` (CSSOM View §6 "scrolling area"),
/// approximated as the union of its padding box with the border boxes of
/// every descendant that has a fragment, extended by the box's own
/// end-side padding after that content (CSS Overflow 3 §3.3 "Scrollable
/// Overflow"; this layout only produces in-flow descendants), measured from
/// the padding box origin. It is never smaller than the padding box.
/// Descendants extending above or left of the padding box do not add to it
/// (they are not reachable by scrolling).
///
/// The page scene only has fragments for nodes that intersect the page, so
/// descendants laid out entirely off the page are not counted.
///
/// The end-side padding is the used value from the node's layout (so a
/// percentage is already resolved against its containing block).
fn scroll_extent(
    scene: &raikiri::PageScene,
    document: &raikiri_dom::Document,
    node: usize,
    padding_box: &DomRect,
) -> (f64, f64) {
    let (mut content_right, mut content_bottom) = (f64::NEG_INFINITY, f64::NEG_INFINITY);
    let (mut pending, padding) = document
        .get_node(node)
        .map(|n| (n.children.to_vec(), n.unrounded_layout.padding))
        .unwrap_or_default();
    while let Some(index) = pending.pop() {
        if let Some(descendant) = border_box_of(scene, index) {
            content_right = content_right.max(descendant.right);
            content_bottom = content_bottom.max(descendant.bottom);
        }
        if let Some(n) = document.get_node(index) {
            pending.extend(n.children.iter().copied());
        }
    }
    let right = padding_box
        .right
        .max(content_right + f64::from(padding.right));
    let bottom = padding_box
        .bottom
        .max(content_bottom + f64::from(padding.bottom));
    (right - padding_box.left, bottom - padding_box.top)
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
