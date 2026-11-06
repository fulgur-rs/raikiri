//! Source eligibility for the running index, independent of geometry storage.

use std::collections::HashSet;

use raikiri_style::{
    CascadeResult, PseudoElem,
    property::{ContentComponent, DisplayValue, FloatValue, PositionValue},
};

use crate::{Document, generated_content::generated_origin, node::NodeData};

pub(super) fn excluded_contents(document: &Document, cascade: &CascadeResult) -> HashSet<usize> {
    let mut excluded: HashSet<_> = document
        .nodes
        .iter()
        .enumerate()
        .filter(|(id, node)| {
            matches!(node.data, NodeData::Element(_))
                && cascade.computed[*id].display == DisplayValue::Contents
        })
        .map(|(id, _)| id)
        .collect();
    if !cascade
        .pseudo
        .keys()
        .any(|(owner, _)| excluded.contains(&(owner.0 as usize)))
    {
        return excluded;
    }

    // Existing owner entries can stand in for generated content. Keep them
    // conservatively when retained layout data contains a before/after source.
    // This does not add missing projected entries or infer their page range.
    let mut generated = HashSet::new();
    for node in &document.nodes {
        let Some(root) = node.ifc.as_ref() else {
            continue;
        };
        let sources = root
            .lines
            .iter()
            .flat_map(|lines| lines.lines.iter())
            .flat_map(crate::layout::ifc::text_lines::glyph_owners)
            .chain(root.generated_atomics.iter().map(|(id, _)| id.0 as usize))
            .chain(root.generated_boxes.iter().map(|item| item.id as usize))
            .chain(root.generated_blocks.iter().map(|item| item.start as usize))
            .chain(
                node.ifc_inline_boxes()
                    .into_iter()
                    .flatten()
                    .map(|item| item.node),
            );
        for source in sources {
            if let Some((owner, PseudoElem::Before | PseudoElem::After)) = generated_origin(source)
                && excluded.contains(&owner)
            {
                generated.insert(owner);
            }
        }
    }
    // Float and positioned pseudos are painted as overlays, outside the IFC
    // inventory. Preserve their existing owner entry when content can generate
    // an overlay; absence and display:none must not protect an empty wrapper.
    for ((owner, pseudo), style) in &cascade.pseudo {
        let owner = owner.0 as usize;
        if excluded.contains(&owner)
            && matches!(pseudo, PseudoElem::Before | PseudoElem::After)
            && style.display != DisplayValue::None
            && (style.float != FloatValue::None
                || matches!(
                    style.position,
                    PositionValue::Absolute | PositionValue::Fixed
                ))
            && !style.content.is_empty()
            && !style
                .content
                .iter()
                .any(|part| matches!(part, ContentComponent::None))
        {
            generated.insert(owner);
        }
    }
    excluded.retain(|owner| !generated.contains(owner));
    excluded
}
