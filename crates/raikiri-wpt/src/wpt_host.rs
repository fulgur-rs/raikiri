//! [`DocumentHost`] over a prepared WPT document: one screen viewport,
//! style and layout rebuilt lazily after DOM mutations.

use std::path::{Path, PathBuf};

use raikiri_js::runtime::{BoxGeometry, DocumentHost, DomRect, HostError};

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
        let node_id = raikiri_traits::NodeId::new(node as u64);
        let Some(fragment) = scene.fragments.get(&node_id).and_then(|f| f.first()) else {
            return Ok(None);
        };
        let (width, height) = scene
            .drawables
            .block_styles
            .get(&node_id)
            .and_then(|entry| entry.layout_size)
            .unwrap_or((fragment.width, fragment.height));
        let left = f64::from(fragment.x + scene.body_offset_pt.0);
        let top = f64::from(fragment.y + scene.body_offset_pt.1);
        let (width, height) = (f64::from(width), f64::from(height));
        Ok(Some(BoxGeometry {
            border_box: DomRect {
                left,
                top,
                right: left + width,
                bottom: top + height,
                width,
                height,
            },
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

#[cfg(test)]
mod tests;
