//! Resolve typographic pseudo styles after the text's actual parent is known.

use super::CascadeResult;
use super::candidate::{ElementCandidates, OwnedCandidates};
use super::custom_property::resolve_custom_properties;
use super::inherit::apply_winners;
use crate::resolve::ResolveContext;
use crate::{ComputedValues, SpecifiedValues, StyleNodeId};

#[derive(Clone, Debug)]
/// Crate visibility matches the inheritance walk's crate-visible output type.
pub(crate) struct FirstLetterInputs {
    /// The originating element's `::first-letter` candidates, ordinary and
    /// custom-property ones, copied out of the cascade.
    pub(super) candidates: OwnedCandidates,
    pub(super) context: ResolveContext,
}

fn property_applies(key: crate::property::PropertyKey) -> bool {
    use crate::property::PropertyKey::*;
    super::first_line::first_line_property_applies(key)
        || matches!(
            key,
            Margin
                | MarginTop
                | MarginRight
                | MarginBottom
                | MarginLeft
                | MarginInline
                | MarginBlock
                | Padding
                | PaddingTop
                | PaddingRight
                | PaddingBottom
                | PaddingLeft
                | PaddingInline
                | PaddingBlock
                | Border
                | BorderTop
                | BorderRight
                | BorderBottom
                | BorderLeft
                | BorderWidth
                | BorderStyle
                | BorderColor
                | BorderTopWidth
                | BorderRightWidth
                | BorderBottomWidth
                | BorderLeftWidth
                | BorderTopStyle
                | BorderRightStyle
                | BorderBottomStyle
                | BorderLeftStyle
                | BorderTopColor
                | BorderRightColor
                | BorderBottomColor
                | BorderLeftColor
                | BorderImage
                | BorderImageSource
                | BorderImageSlice
                | BorderImageWidth
                | BorderImageOutset
                | BorderImageRepeat
                | BorderRadius
                | BorderRadiusTopLeft
                | BorderRadiusTopRight
                | BorderRadiusBottomLeft
                | BorderRadiusBottomRight
                | BoxShadow
                | Float
        )
}

impl CascadeResult {
    /// Resolve enclosing first-line pseudos in outer-to-inner box-tree order.
    /// The ordinary custom-property inheritance channel remains unchanged.
    pub fn first_letter_parent_with_first_lines<D: crate::StyleDom>(
        &self,
        dom: &D,
        letter_origin: StyleNodeId,
        line_origins: &[StyleNodeId],
        actual_parent: StyleNodeId,
        generated: Option<crate::PseudoElem>,
    ) -> Option<ComputedValues> {
        let inputs = self.first_letter_inputs.get(&letter_origin)?;
        self.first_line_chain(
            dom,
            &inputs.context,
            line_origins,
            None,
            actual_parent,
            generated,
        )
    }

    /// Whether any element has `::first-line` declarations.
    pub fn has_first_line_styles(&self) -> bool {
        !self.first_line_contexts.is_empty()
    }

    /// The computed values `element` has as part of the first formatted line
    /// of the blocks `line_origins`, outermost first (CSS Pseudo 4 section
    /// 2.1.2): each `::first-line` among them is the parent of the inline
    /// content on the line, and every element between them and `element`
    /// inherits through it. With `generated`, the values of `element`'s
    /// `::before` or `::after` instead.
    ///
    /// `element` is `paragraph` or one of its descendants. `paragraph` is the
    /// block that holds the line's inline content. It and
    /// the blocks around it have a first line of their own only through their
    /// `::first-line`, so their other declarations do not reach the line:
    /// text inside takes the inherited values of the `::first-line` it sits
    /// in, even where a block between has set them differently. The elements
    /// below `paragraph` keep their own declarations.
    ///
    /// Only inherited properties pass through a `::first-line`; custom
    /// properties and the rest keep their ordinary inheritance. `None` when
    /// `line_origins` is empty, has no first-line style or does not enclose
    /// `element`.
    pub fn first_line_style<D: crate::StyleDom>(
        &self,
        dom: &D,
        line_origins: &[StyleNodeId],
        paragraph: StyleNodeId,
        element: StyleNodeId,
        generated: Option<crate::PseudoElem>,
    ) -> Option<ComputedValues> {
        let context = self.first_line_contexts.get(line_origins.first()?)?;
        self.first_line_chain(
            dom,
            context,
            line_origins,
            Some(paragraph),
            element,
            generated,
        )
    }

    /// The computed values of a text node `text` directly inside an element
    /// whose first-line values are `parent`: its ordinary values, with the
    /// inherited properties taken from `parent`.
    pub fn first_line_text_style(
        &self,
        text: StyleNodeId,
        parent: &ComputedValues,
    ) -> ComputedValues {
        super::first_line::first_line_parent(&self.computed[text.0 as usize], parent)
    }

    fn first_line_chain<D: crate::StyleDom>(
        &self,
        dom: &D,
        context: &ResolveContext,
        line_origins: &[StyleNodeId],
        paragraph: Option<StyleNodeId>,
        actual_parent: StyleNodeId,
        generated: Option<crate::PseudoElem>,
    ) -> Option<ComputedValues> {
        let &line_origin = line_origins.first()?;
        let mut computed = self
            .pseudo
            .get(&(line_origin, crate::PseudoElem::FirstLine))?
            .as_ref()
            .clone();
        computed.custom_properties = self.computed[line_origin.0 as usize]
            .custom_properties
            .clone();
        computed.local_custom_properties = self.computed[line_origin.0 as usize]
            .local_custom_properties
            .clone();
        let mut chain = Vec::new();
        let mut current = actual_parent;
        // Whether the chain has met `paragraph`: from there outwards the
        // elements are blocks.
        let mut blocks_from = None;
        while current != line_origin {
            if Some(current) == paragraph {
                blocks_from = Some(chain.len());
            }
            chain.push(current);
            current = dom.parent_id(current)?;
        }
        if paragraph == Some(line_origin) {
            blocks_from = Some(chain.len());
        }
        let mut parent_id = line_origin;
        for (position, &id) in chain.iter().enumerate().rev() {
            let inherited = super::first_line::first_line_parent(
                &self.computed[parent_id.0 as usize],
                &computed,
            );
            let pseudo = line_origins
                .contains(&id)
                .then_some(crate::PseudoElem::FirstLine);
            let block = blocks_from.is_some_and(|from| position >= from);
            computed = if block && pseudo.is_none() {
                self.inherit_only(id, &inherited, context)
            } else {
                self.recompute_typographic_child(id, pseudo, &inherited, context)
            };
            parent_id = id;
        }
        if let Some(pseudo) = generated {
            let inherited = super::first_line::first_line_parent(
                &self.computed[actual_parent.0 as usize],
                &computed,
            );
            computed =
                self.recompute_typographic_child(actual_parent, Some(pseudo), &inherited, context);
        }
        Some(computed)
    }

    /// The values of `id` when its declarations do not apply: those it
    /// inherits from `inherited`, with the custom properties of its ordinary
    /// style.
    fn inherit_only(
        &self,
        id: StyleNodeId,
        inherited: &ComputedValues,
        context: &ResolveContext,
    ) -> ComputedValues {
        let ordinary = &self.computed[id.0 as usize];
        let mut computed = SpecifiedValues::inherit_from(inherited).finalize_with(
            inherited,
            context,
            self.normal_line_height(),
        );
        computed.custom_properties = ordinary.custom_properties.clone();
        computed.local_custom_properties = ordinary.local_custom_properties.clone();
        computed
    }

    fn recompute_typographic_child(
        &self,
        id: StyleNodeId,
        pseudo: Option<crate::PseudoElem>,
        inherited: &ComputedValues,
        context: &ResolveContext,
    ) -> ComputedValues {
        let ordinary = pseudo
            .filter(|&pseudo| pseudo != crate::PseudoElem::FirstLine)
            .and_then(|pseudo| self.pseudo.get(&(id, pseudo)))
            .map_or(&self.computed[id.0 as usize], |style| style.as_ref());
        let mut specified = SpecifiedValues::inherit_from(inherited);
        let values = self
            .typographic_inheritance
            .get(&(id, pseudo))
            .map_or(ElementCandidates::EMPTY, OwnedCandidates::candidates);
        let mut spread = Vec::new();
        let mut filtered = Vec::new();
        let values = if pseudo == Some(crate::PseudoElem::FirstLine) {
            values
                .with_all_spread(&mut spread)
                .filtered(&mut filtered, |candidate| {
                    candidate.is_all_rollback()
                        || super::first_line::first_line_property_applies(candidate.key())
                })
        } else {
            values
        };
        // Resolve a pseudo's own declarations with its custom properties;
        // descendant inheritance still uses the ordinary parent's channel.
        let declaration_custom = pseudo
            .and_then(|pseudo| self.pseudo.get(&(id, pseudo)))
            .map_or(&ordinary.custom_properties, |style| {
                &style.custom_properties
            });
        apply_winners(
            values,
            &mut Vec::new(),
            &mut specified,
            inherited,
            declaration_custom,
            None,
            None,
            None,
            None,
        );
        let mut computed = specified.finalize_with(inherited, context, self.normal_line_height());
        computed.custom_properties = ordinary.custom_properties.clone();
        computed.local_custom_properties = ordinary.local_custom_properties.clone();
        computed
    }

    /// Whether any originating element has first-letter declarations.
    pub fn has_first_letter_styles(&self) -> bool {
        !self.first_letter_inputs.is_empty()
    }

    /// Resolve an originating block's first-letter declarations over the
    /// actual inline parent of its typographic text (CSS Pseudo 4 section 2.2.3).
    ///
    /// Layout determines the first letter; this method preserves descendant
    /// inheritance, relative font units, custom properties and cascade rollback.
    pub fn resolve_first_letter_style(
        &self,
        origin: StyleNodeId,
        parent: &ComputedValues,
    ) -> Option<ComputedValues> {
        let inputs = self.first_letter_inputs.get(&origin)?;
        let custom_candidates = inputs.candidates.custom_candidates();
        let custom = resolve_custom_properties(&parent.custom_properties, custom_candidates);
        let mut specified = SpecifiedValues::inherit_from(parent);
        let mut spread = Vec::new();
        let mut filtered = Vec::new();
        let declarations = inputs
            .candidates
            .candidates()
            .with_all_spread(&mut spread)
            .filtered(&mut filtered, |candidate| {
                candidate.is_all_rollback() || property_applies(candidate.key())
            });
        apply_winners(
            declarations,
            &mut Vec::new(),
            &mut specified,
            parent,
            &custom.effective,
            None,
            None,
            None,
            None,
        );
        let mut computed =
            specified.finalize_with(parent, &inputs.context, self.normal_line_height());
        computed.local_custom_properties = custom.local;
        computed.custom_properties = custom.effective;
        Some(computed)
    }
}

#[cfg(test)]
mod tests;
