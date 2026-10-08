//! Resolve typographic pseudo styles after the text's actual parent is known.

use super::CascadeResult;
use super::collect::{CascadedDecl, CustomCascadedDecl};
use super::custom_property::resolve_custom_properties;
use super::inherit::apply_winners;
use crate::resolve::ResolveContext;
use crate::{ComputedValues, SpecifiedValues, StyleNodeId};

#[derive(Clone, Debug)]
/// Crate visibility matches the inheritance walk's crate-visible output type.
pub(crate) struct FirstLetterInputs {
    pub(super) declarations: Vec<CascadedDecl>,
    pub(super) custom: Vec<CustomCascadedDecl>,
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
    /// Resolve the actual inline parent's first-line inheritance before
    /// applying first-letter rules. Ordinary custom-property environments
    /// remain attached to the real elements, as for first-line fragments.
    pub fn first_letter_parent_with_first_line<D: crate::StyleDom>(
        &self,
        dom: &D,
        letter_origin: StyleNodeId,
        line_origin: StyleNodeId,
        actual_parent: StyleNodeId,
        generated: Option<crate::PseudoElem>,
    ) -> Option<ComputedValues> {
        self.first_letter_parent_with_first_lines(
            dom,
            letter_origin,
            &[line_origin],
            actual_parent,
            generated,
        )
    }

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
        let &line_origin = line_origins.first()?;
        let inputs = self.first_letter_inputs.get(&letter_origin)?;
        let mut computed = self
            .pseudo
            .get(&(line_origin, crate::PseudoElem::FirstLine))?
            .clone();
        computed.custom_properties = self.computed[line_origin.0 as usize]
            .custom_properties
            .clone();
        computed.local_custom_properties = self.computed[line_origin.0 as usize]
            .local_custom_properties
            .clone();
        let mut chain = Vec::new();
        let mut current = actual_parent;
        while current != line_origin {
            chain.push(current);
            current = dom.parent_id(current)?;
        }
        let mut parent_id = line_origin;
        for &id in chain.iter().rev() {
            let inherited = super::first_line::first_line_parent(
                &self.computed[parent_id.0 as usize],
                &computed,
            );
            let pseudo = line_origins
                .contains(&id)
                .then_some(crate::PseudoElem::FirstLine);
            computed = self.recompute_typographic_child(id, pseudo, &inherited, &inputs.context);
            parent_id = id;
        }
        if let Some(pseudo) = generated {
            let inherited = super::first_line::first_line_parent(
                &self.computed[actual_parent.0 as usize],
                &computed,
            );
            computed = self.recompute_typographic_child(
                actual_parent,
                Some(pseudo),
                &inherited,
                &inputs.context,
            );
        }
        Some(computed)
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
            .unwrap_or(&self.computed[id.0 as usize]);
        let mut specified = SpecifiedValues::inherit_from(inherited);
        if let Some(values) = self.typographic_inheritance.get(&(id, pseudo)) {
            let filtered;
            let values = if pseudo == Some(crate::PseudoElem::FirstLine) {
                filtered = values
                    .iter()
                    .filter(|(value, ..)| {
                        matches!(value, crate::property::PropertyValue::AllRevertLayer)
                            || super::first_line::first_line_property_applies(value.key())
                    })
                    .cloned()
                    .collect::<Vec<_>>();
                filtered.as_slice()
            } else {
                values.as_slice()
            };
            apply_winners(
                values,
                &mut Vec::new(),
                &mut specified,
                inherited,
                &ordinary.custom_properties,
                None,
                None,
                None,
            );
        }
        let mut computed = specified.finalize(inherited, context);
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
        let custom = resolve_custom_properties(&parent.custom_properties, &inputs.custom);
        let mut specified = SpecifiedValues::inherit_from(parent);
        let declarations: Vec<_> = inputs
            .declarations
            .iter()
            .filter(|(value, ..)| {
                matches!(value, crate::property::PropertyValue::AllRevertLayer)
                    || property_applies(value.key())
            })
            .cloned()
            .collect();
        apply_winners(
            &declarations,
            &mut Vec::new(),
            &mut specified,
            parent,
            &custom,
            None,
            None,
            None,
        );
        let mut computed = specified.finalize(parent, &inputs.context);
        computed.local_custom_properties = if inputs.custom.is_empty() {
            crate::computed::empty_custom_properties()
        } else {
            std::sync::Arc::clone(&custom)
        };
        computed.custom_properties = custom;
        Some(computed)
    }
}

#[cfg(test)]
mod tests;
