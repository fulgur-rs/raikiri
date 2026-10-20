use std::{collections::HashMap, sync::Arc};

use crate::{Document, layout::context::LayoutSession, node::arena::BoxKey};
use raikiri_layout_style::{
    LayoutStyleView, OwnedPropertyOverride, PreparedStyle, StyleMetadata, ViewContext,
};
use raikiri_style::{CascadeResult, ComputedValues};
use raikiri_traits::NodeKind;

mod metadata;
pub(crate) use metadata::neutral_initial_values;
mod overrides;
pub(crate) use overrides::OverrideToken;

pub(crate) struct PreparedBox<'a> {
    prepared: PreparedStyle<'a>,
    pass: Vec<OwnedPropertyOverride>,
    scoped: Vec<OwnedPropertyOverride>,
    scopes: Vec<usize>,
    context: ViewContext,
}

pub(crate) struct StyleViews<'a> {
    cascade: &'a CascadeResult,
    boxes: HashMap<BoxKey, PreparedBox<'a>>,
    pending: HashMap<BoxKey, Vec<OwnedPropertyOverride>>,
    size_scopes: Vec<overrides::SizeScope>,
}

impl<'a> StyleViews<'a> {
    pub(crate) fn new(cascade: &'a CascadeResult) -> Self {
        Self {
            cascade,
            boxes: HashMap::new(),
            pending: HashMap::new(),
            size_scopes: Vec::new(),
        }
    }

    fn prepare(&mut self, doc: &mut Document) {
        use raikiri_style::property::DisplayValue;
        assert!(
            self.size_scopes.is_empty(),
            "prepare runs outside size scopes"
        );
        let mut previous = std::mem::take(&mut self.boxes);
        for idx in doc.nodes.layout_keys() {
            let key = doc.nodes.box_key(idx).expect("prepared layout key exists");
            let node = &doc.nodes[idx];
            let metadata = metadata::prepare_metadata(doc, self.cascade, idx);
            let neutral = !matches!(key, BoxKey::Source(_))
                || node.kind() != NodeKind::Element
                || node.display() == DisplayValue::Contents;
            let prepared = if neutral {
                let owner = if key == BoxKey::InitialContainingBlock {
                    doc.synthetic_style_values
                        .get(&key)
                        .expect("ICB recipe is published with the page dimensions")
                        .clone()
                } else if matches!(key, BoxKey::Source(_)) && node.kind() != NodeKind::Element {
                    metadata::neutral_source_owner()
                } else {
                    Arc::new(metadata::neutral_values(doc, self.cascade, idx))
                };
                doc.synthetic_style_values.insert(key, owner.clone());
                PreparedStyle::prepare_shared(owner, metadata)
            } else {
                PreparedStyle::prepare(&self.cascade.computed[idx], metadata)
            };
            self.boxes.insert(
                key,
                PreparedBox {
                    prepared,
                    pass: self
                        .pending
                        .remove(&key)
                        .or_else(|| previous.remove(&key).map(|prepared| prepared.pass))
                        .unwrap_or_default(),
                    scoped: Vec::new(),
                    scopes: Vec::new(),
                    context: ViewContext::default(),
                },
            );
        }
    }
}

impl LayoutSession<'_> {
    pub(crate) fn propagate_viewport_overflow(&mut self, cascade: &CascadeResult, body: usize) {
        use raikiri_layout_style::Axis;
        use raikiri_style::property::OverflowValue;
        if super::body_overflow::propagate_body_overflow_to_viewport(self.document, cascade, body) {
            for (axis, original) in [
                (Axis::X, cascade.computed[body].overflow.x),
                (Axis::Y, cascade.computed[body].overflow.y),
            ] {
                if original != OverflowValue::Visible {
                    self.record_pass_override(
                        body,
                        OwnedPropertyOverride::overflow(axis, OverflowValue::Visible),
                    );
                }
            }
        }
    }

    pub(crate) fn begin_style_bridge(&mut self) {
        if let Some(views) = &mut self.style_views {
            assert!(
                views.size_scopes.is_empty(),
                "bridge runs outside temporary size scopes"
            );
            views.boxes.clear();
            views.pending.clear();
        }
    }
    pub(crate) fn prepare_style_views(&mut self) {
        if let Some(views) = &mut self.style_views {
            views.prepare(self.document);
        }
    }

    pub(crate) fn style_view(&self, idx: usize) -> LayoutStyleView<'_> {
        let key = self.nodes.box_key(idx).expect("style query has a box key");
        let views = self
            .style_views
            .as_ref()
            .expect("checked session borrows its cascade");
        let prepared = views
            .boxes
            .get(&key)
            .unwrap_or_else(|| panic!("missing prepared style for {key:?}"));
        LayoutStyleView::with_owned_overrides(
            &prepared.prepared,
            prepared.context,
            &prepared.pass,
            &prepared.scoped,
        )
    }

    pub(crate) fn record_pass_override(&mut self, idx: usize, value: OwnedPropertyOverride) {
        let key = self.nodes.box_key(idx).expect("override has a box key");
        if let Some(views) = &mut self.style_views {
            if let Some(prepared) = views.boxes.get_mut(&key) {
                prepared.pass.push(value);
            } else {
                views.pending.entry(key).or_default().push(value);
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod tests;
