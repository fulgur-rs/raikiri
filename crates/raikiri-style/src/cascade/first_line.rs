//! CSS first-line applicability and alternate inline inheritance.

use crate::property::PropertyKey;

/// Properties supported here that apply to an inline first-line box.
/// CSS Pseudo-Elements 4 §2.1.2 permits font, background, text decoration,
/// inline typesetting/layout, color and opacity. Box geometry and the three
/// excluded writing properties do not apply through the pseudo rule.
pub(crate) fn first_line_property_applies(key: PropertyKey) -> bool {
    use PropertyKey::*;
    matches!(
        key,
        Color
            | Opacity
            | Font
            | FontFamily
            | FontSize
            | FontWeight
            | FontStyle
            | FontVariantCaps
            | FontKerning
            | FontOpticalSizing
            | FontVariantEmoji
            | FontLanguageOverride
            | FontVariantLigatures
            | FontSynthesis
            | FontVariantPosition
            | FontPalette
            | FontVariantNumeric
            | FontVariantEastAsian
            | FontVariationSettings
            | FontFeatureSettings
            | Background
            | BackgroundColor
            | BackgroundImage
            | BackgroundRepeat
            | BackgroundAttachment
            | BackgroundClip
            | BackgroundOrigin
            | BackgroundSize
            | BackgroundPosition
            | LineHeight
            | VerticalAlign
            | RubyPosition
            | TextTransform
            | WordBreak
            | LineBreak
            | OverflowWrap
            | LetterSpacing
            | WordSpacing
            | WhiteSpace
            | WhiteSpaceCollapse
            | TextWrap
            | HangingPunctuation
            | Hyphens
            | HyphenateCharacter
            | HyphenateLimitChars
            | TabSize
            | TextAutospace
            | TextSpacing
            | TextSpacingTrim
            | WordSpaceTransform
            | TextCombineUpright
            | TextDecoration
            | TextDecorationLine
            | TextDecorationColor
            | TextDecorationStyle
            | TextDecorationThickness
            | TextDecorationInset
            | TextDecorationSkipInk
            | TextDecorationSkipSpaces
            | TextShadow
            | TextUnderlineOffset
            | TextUnderlinePosition
            | TextEmphasis
            | TextEmphasisPosition
            | TextEmphasisStyle
            | TextEmphasisColor
    )
}

use super::inherit::{WalkOptions, apply_winners, walk};
use super::limits::{Counter, bytes_of, try_filled};
use super::{CascadeLimits, CascadeResult, finish};
use crate::property::DisplayValue;
use crate::resolve::{ResolveContext, used_line_height_length};
use crate::style_dom::{StyleDom, StyleNode, StyleNodeId, StyleNodeKind};
use crate::{
    CascadeError, CascadeLimitKind, ComputedValues, MediaContext, PageContextQuery, RuleTree,
    SpecifiedValues,
};

/// Normal cascade and resolved first-line inputs on the same immutable DOM.
#[derive(Debug)]
pub struct FirstLineCascade {
    /// Ordinary element styles and pseudo-element declarations.
    pub normal: CascadeResult,
    /// Alternate styles, present only when a rule matches the requested root.
    pub first_line: Option<FirstLineStyles>,
}

/// Node-indexed styles for one block's inline first-line fragments.
#[derive(Debug)]
pub struct FirstLineStyles {
    /// The originating block's real DOM identifier.
    pub root: StyleNodeId,
    /// The root slot contains the pseudo's inline style; descendants contain
    /// full element styles. Slots outside this inline subtree remain `None`.
    /// Layout must obtain the originating block's geometry from normal styles.
    pub computed: Vec<Option<ComputedValues>>,
}

/// Resolve real first-line rules and alternate descendant inheritance.
///
/// This strict entry point supports one block with inline/text descendants.
/// It skips detached and display:none subtrees and rejects nested blocks,
/// atomic inline containers, floats and absolutely/fixed positioned descendants.
/// It does not determine which text fits on a line;
/// the layout consumer selects these styles only for its first accepted line.
/// Custom properties, non-inherited properties explicitly set to inherit, and
/// excluded writing properties retain their non-pseudo inheritance channels.
///
/// The cascade runs within the default [`CascadeLimits`]; the root's
/// subtree's candidates, kept for the first-line styles, count as retained
/// candidates.
///
/// # Errors
/// Returns an error for an absent, detached, unreachable or non-block root,
/// an unsupported visible descendant, or a cascade failure (see
/// [`super::cascade_with_options`]).
pub fn cascade_with_first_line<D: StyleDom>(
    dom: &D,
    rule_tree: &RuleTree,
    media: &MediaContext,
    root: StyleNodeId,
) -> Result<FirstLineCascade, CascadeError> {
    cascade_with_first_line_within(dom, rule_tree, media, root, CascadeLimits::default())
}

/// [`cascade_with_first_line`] within `limits`.
pub(crate) fn cascade_with_first_line_within<D: StyleDom>(
    dom: &D,
    rule_tree: &RuleTree,
    media: &MediaContext,
    root: StyleNodeId,
    limits: CascadeLimits,
) -> Result<FirstLineCascade, CascadeError> {
    let mut outputs = walk(
        dom,
        rule_tree,
        media,
        WalkOptions {
            retain_subtree: Some(root),
            limits,
            ..WalkOptions::default()
        },
    )?;
    // The candidates of the root's subtree, for re-applying its winners over
    // the first-line parent below.
    let candidates = std::mem::take(&mut outputs.retained_subtree);
    // The first-line styles join what the walk counted against the output
    // limit.
    let mut output = Counter::new(limits.max_output_bytes, CascadeLimitKind::OutputBytes);
    output.add(outputs.counts.output_bytes)?;
    let query = PageContextQuery::default();
    let normal = finish(dom, rule_tree, &query, media, outputs);
    let error = |id: StyleNodeId| CascadeError::Internal {
        message: format!(
            "unsupported first-line node {} (one block with inline descendants required)",
            id.0
        ),
    };
    let node = dom
        .node(root)
        .filter(|n| n.is_in_document() && n.kind() == StyleNodeKind::Element)
        .ok_or_else(|| error(root))?;
    let _ = node;
    if normal
        .computed
        .get(root.0 as usize)
        .is_none_or(|cv| cv.display != DisplayValue::Block)
    {
        return Err(error(root));
    }
    // Recover this subtree's document-root rem/rlh bases without resetting them
    // to the originating block's metrics. Separate top-level elements have
    // separate bases, matching the ordinary inheritance traversal.
    let mut search = vec![(dom.root_id(), None::<ResolveContext>)];
    let mut context = None;
    while let Some((id, ctx)) = search.pop() {
        let Some(node) = dom.node(id).filter(|n| n.is_in_document()) else {
            continue;
        };
        let ctx = if ctx.is_none() && node.kind() == StyleNodeKind::Element {
            let cv = &normal.computed[id.0 as usize];
            Some(ResolveContext::with_root_line_height(
                cv.font_size,
                used_line_height_length(cv.line_height, cv.font_size),
            ))
        } else {
            ctx
        };
        if id == root {
            context = ctx;
            break;
        }
        search.extend(dom.child_ids(id).map(|child| (child, ctx)));
    }
    let context = context.ok_or_else(|| error(root))?;
    let Some(pseudo) = normal.pseudo.get(&(root, crate::PseudoElem::FirstLine)) else {
        return Ok(FirstLineCascade {
            normal,
            first_line: None,
        });
    };
    output.add(bytes_of::<Option<ComputedValues>>(dom.node_count()))?;
    let mut computed = try_filled(dom.node_count(), None)?;
    computed[root.0 as usize] = Some(pseudo.clone());
    let mut stack: Vec<_> = dom.child_ids(root).map(|child| (child, root)).collect();
    let mut winners = Vec::new();
    while let Some((id, parent_id)) = stack.pop() {
        let Some(node) = dom.node(id).filter(|n| n.is_in_document()) else {
            continue;
        };
        let ordinary = &normal.computed[id.0 as usize];
        if node.kind() == StyleNodeKind::Element {
            if ordinary.display == DisplayValue::None {
                continue;
            }
            if ordinary.display != DisplayValue::Inline
                || matches!(
                    ordinary.position,
                    crate::property::PositionValue::Absolute
                        | crate::property::PositionValue::Fixed
                )
                || ordinary.float != crate::property::FloatValue::None
            {
                return Err(error(id));
            }
        }
        let inherited = first_line_parent(
            &normal.computed[parent_id.0 as usize],
            computed[parent_id.0 as usize]
                .as_ref()
                .expect("parent visited first"),
        );
        let mut specified = SpecifiedValues::inherit_from(&inherited);
        if let Some(values) = candidates.get(&id) {
            apply_winners(
                values.candidates(),
                &mut winners,
                &mut specified,
                &inherited,
                &ordinary.custom_properties,
                None,
                None,
                None,
            );
        }
        let mut cv = specified.finalize(&inherited, &context);
        cv.custom_properties = ordinary.custom_properties.clone();
        cv.local_custom_properties = ordinary.local_custom_properties.clone();
        computed[id.0 as usize] = Some(cv);
        stack.extend(dom.child_ids(id).map(|child| (child, id)));
    }
    Ok(FirstLineCascade {
        normal,
        first_line: Some(FirstLineStyles { root, computed }),
    })
}

// Mirrors the standard inherited fields consumed by SpecifiedValues::inherit_from.
// All other parent fields and custom-property environments remain ordinary.
pub(super) fn first_line_parent(normal: &ComputedValues, first: &ComputedValues) -> ComputedValues {
    let mut parent = normal.clone();
    parent.border_collapse = first.border_collapse;
    parent.border_spacing = first.border_spacing;
    parent.caption_side = first.caption_side;
    parent.color = first.color;
    parent.empty_cells = first.empty_cells;
    parent.font_family = first.font_family.clone();
    parent.font_kerning = first.font_kerning;
    parent.font_language_override = first.font_language_override.clone();
    parent.font_optical_sizing = first.font_optical_sizing;
    parent.font_palette = first.font_palette.clone();
    parent.font_size = first.font_size;
    parent.font_style = first.font_style;
    parent.font_synthesis = first.font_synthesis;
    parent.font_variant_caps = first.font_variant_caps;
    parent.font_variant_east_asian = first.font_variant_east_asian;
    parent.font_variant_emoji = first.font_variant_emoji;
    parent.font_variant_ligatures = first.font_variant_ligatures;
    parent.font_variant_numeric = first.font_variant_numeric;
    parent.font_variant_position = first.font_variant_position;
    parent.font_variation_settings = first.font_variation_settings.clone();
    parent.font_feature_settings = first.font_feature_settings.clone();
    parent.font_weight = first.font_weight;
    parent.hanging_punctuation = first.hanging_punctuation;
    parent.hyphenate_character = first.hyphenate_character.clone();
    parent.hyphenate_limit_chars = first.hyphenate_limit_chars;
    parent.hyphens = first.hyphens;
    parent.letter_spacing_ch_factor = first.letter_spacing_ch_factor;
    parent.letter_spacing_ch_font = first.letter_spacing_ch_font.clone();
    parent.letter_spacing_ch_offset = first.letter_spacing_ch_offset;
    parent.letter_spacing_computed = first.letter_spacing_computed;
    parent.line_break = first.line_break;
    parent.line_height = first.line_height;
    parent.list_style_image = first.list_style_image.clone();
    parent.list_style_position = first.list_style_position;
    parent.list_style_type = first.list_style_type.clone();
    parent.orphans = first.orphans;
    parent.overflow_wrap = first.overflow_wrap;
    parent.quotes = first.quotes.clone();
    parent.quotes_auto = first.quotes_auto;
    parent.ruby_position = first.ruby_position;
    parent.tab_size = first.tab_size;
    parent.text_align = first.text_align;
    parent.text_align_last = first.text_align_last;
    parent.text_autospace = first.text_autospace;
    parent.text_combine_upright = first.text_combine_upright;
    parent.text_decoration_skip_ink = first.text_decoration_skip_ink;
    parent.text_decoration_skip_spaces = first.text_decoration_skip_spaces;
    parent.text_emphasis_color = first.text_emphasis_color;
    parent.text_emphasis_position = first.text_emphasis_position;
    parent.text_emphasis_style = first.text_emphasis_style.clone();
    parent.text_indent = first.text_indent;
    parent.text_indent_ch_factor = first.text_indent_ch_factor;
    parent.text_indent_ch_font = first.text_indent_ch_font.clone();
    parent.text_indent_ch_inherited = first.text_indent_ch_inherited;
    parent.text_indent_ch_offset = first.text_indent_ch_offset;
    parent.text_indent_each_line = first.text_indent_each_line;
    parent.text_indent_hanging = first.text_indent_hanging;
    parent.text_justify = first.text_justify;
    parent.text_shadow = first.text_shadow.clone();
    parent.text_spacing_trim = first.text_spacing_trim;
    parent.text_transform = first.text_transform;
    parent.text_underline_offset = first.text_underline_offset;
    parent.text_underline_position = first.text_underline_position;
    parent.text_wrap = first.text_wrap;
    parent.text_wrap_style = first.text_wrap_style;
    parent.visibility = first.visibility;
    parent.white_space = first.white_space;
    parent.white_space_collapse = first.white_space_collapse;
    parent.effective_white_space_collapse = first.effective_white_space_collapse;
    parent.effective_text_wrap_mode = first.effective_text_wrap_mode;
    parent.widows = first.widows;
    parent.word_break = first.word_break;
    parent.word_space_transform = first.word_space_transform;
    parent.word_spacing_ch_factor = first.word_spacing_ch_factor;
    parent.word_spacing_ch_font = first.word_spacing_ch_font.clone();
    parent.word_spacing_ch_offset = first.word_spacing_ch_offset;
    parent.word_spacing_computed = first.word_spacing_computed;
    parent
}

#[cfg(test)]
mod tests;
