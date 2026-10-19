//! Host-cascaded styles of an inline SVG, shared by the page's vector export
//! and the built-in painter so that both draw the same document.
//!
//! The SVG source only holds authored attributes and `style` attributes. The
//! declarations a page stylesheet gives to its descendants, and the host's
//! root font, reach the SVG renderer through these helpers.

use crate::Document;
use raikiri_style::cascade::SvgStyleProperty;
use raikiri_style::property::{FontFamilyKind, PropertyKey};
use raikiri_style::{CascadeResult, ComputedValues, StyleNodeId};
use raikiri_traits::NodeKind;

/// Upper bound on the declaration text the descendants of one SVG may add.
const DECLARATION_BYTE_LIMIT: usize = 32 * 1024 * 1024;

/// Resolved declarations for one element of an inline SVG.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InlineSvgElementDeclarations {
    /// Zero-based element preorder index within the SVG subtree, counting the
    /// root as index zero. The root is styled separately, so it is listed only
    /// for the painting properties (`fill`, `stroke`, the stroke shape and dash
    /// properties, their opacities and the fill and clip rules) that its root
    /// style does not carry.
    pub element_index: usize,
    /// CSS declarations, each marked `!important`.
    pub declarations: String,
}

/// The declarations the cascade resolves for the elements of the inline SVG
/// rooted at `root`, in document order.
///
/// Only elements whose cascade holds an explicit SVG property are listed, and
/// the root only by the painting properties among them.
/// Fails when the declarations would exceed the size bound, so that a few
/// rules matching a very large SVG cannot allocate without limit.
pub fn inline_svg_element_declarations(
    document: &Document,
    cascade: &CascadeResult,
    root: usize,
) -> Result<Vec<InlineSvgElementDeclarations>, String> {
    let Some(root_node) = document.get_node(root) else {
        return Ok(Vec::new());
    };
    let mut elements = Vec::new();
    let mut remaining_bytes = DECLARATION_BYTE_LIMIT;
    let mut stack = vec![(root, root_node)];
    let mut element_index = 0;
    while let Some((node_id, node)) = stack.pop() {
        if node.kind() == NodeKind::Element {
            let style_node = StyleNodeId(node_id as u64);
            let properties = cascade.svg_style_properties(style_node);
            let relative_attributes = cascade.svg_relative_presentation_attributes(style_node);
            // The root's color, font, visibility and opacity reach the renderer
            // through the root style, so only its painting properties are
            // listed. Descendants inherit them from the root there.
            let root_painting: Vec<SvgStyleProperty>;
            let properties = if element_index == 0 {
                root_painting = properties
                    .iter()
                    .filter(|property| property.property.is_svg_presentation())
                    .cloned()
                    .collect();
                &root_painting[..]
            } else {
                properties
            };
            if (!properties.is_empty() || !relative_attributes.is_empty())
                && let Some(style) = cascade.computed.get(node_id)
            {
                elements.push(InlineSvgElementDeclarations {
                    element_index,
                    declarations: node_declarations(
                        style,
                        properties,
                        relative_attributes,
                        &mut remaining_bytes,
                    )?,
                });
            }
            element_index += 1;
        }
        stack.extend(
            node.children
                .iter()
                .rev()
                .filter_map(|&child| document.get_node(child).map(|node| (child, node))),
        );
    }
    Ok(elements)
}

/// The CSS `font-family` list of `style`, with every non-generic name quoted.
pub fn svg_font_family(style: &ComputedValues) -> String {
    style
        .font_family
        .iter()
        .map(|family| {
            if family.1 == FontFamilyKind::Generic {
                family.as_str().to_owned()
            } else {
                // Writing to a `String` cannot fail.
                let mut quoted = String::new();
                let _ = cssparser::serialize_string(family.as_str(), &mut quoted);
                quoted
            }
        })
        .collect::<Vec<_>>()
        .join(",")
}

/// The CSS name of an SVG painting property.
fn svg_presentation_name(key: PropertyKey) -> &'static str {
    key.svg_presentation_name().unwrap_or_default()
}

fn node_declarations(
    style: &ComputedValues,
    properties: &[SvgStyleProperty],
    relative_attributes: &[SvgStyleProperty],
    remaining_bytes: &mut usize,
) -> Result<String, String> {
    let mut css = String::new();
    // Bound metadata before allocating repeated declarations from the host CSS.
    let mut consume = |bytes: usize| {
        *remaining_bytes = remaining_bytes
            .checked_sub(bytes)
            .ok_or_else(|| "SVG descendant style source limit exceeded".to_owned())?;
        Ok::<(), String>(())
    };
    consume(128)?;
    for property in properties {
        let size = if property.property == PropertyKey::FontFamily && !property.inherited {
            style.font_family.iter().fold(128usize, |size, name| {
                size.saturating_add(name.as_str().len().saturating_mul(8))
            })
        } else if property.property.is_svg_presentation() && !property.inherited {
            // The declared text is cloned and then copied into the
            // declaration, so a long `url(#...)` paint reaching many elements
            // counts twice per element.
            property.expression.as_ref().map_or(128, |text| {
                128usize.saturating_add(text.len().saturating_mul(2))
            })
        } else {
            128
        };
        consume(size)?;
        let (name, value) = if property.inherited {
            let name = match property.property {
                PropertyKey::Color => "color",
                PropertyKey::Display => "display",
                PropertyKey::Opacity => "opacity",
                PropertyKey::Visibility => "visibility",
                PropertyKey::FontSize => "font-size",
                PropertyKey::FontFamily => "font-family",
                PropertyKey::FontStyle => "font-style",
                PropertyKey::FontWeight => "font-weight",
                key if key.is_svg_presentation() => svg_presentation_name(key),
                _ => continue,
            };
            // SVG renderers such as usvg copy an ancestor's raw `font-size`
            // for `inherit` and apply it again, compounding a relative size.
            // `100%` has the same computed value without that copy.
            let value = if property.property == PropertyKey::FontSize {
                "100%"
            } else {
                "inherit"
            };
            (name, value.to_owned())
        } else {
            match property.property {
                PropertyKey::Color => {
                    let color = style.color;
                    (
                        "color",
                        format!(
                            "rgba({},{},{},{:.6})",
                            color.r,
                            color.g,
                            color.b,
                            f32::from(color.a) / 255.0
                        ),
                    )
                }
                PropertyKey::Display => ("display", style.display.as_css_str().to_owned()),
                PropertyKey::Opacity => ("opacity", style.opacity.to_string()),
                PropertyKey::Visibility => ("visibility", style.visibility.as_css_str().to_owned()),
                PropertyKey::FontSize => ("font-size", format!("{}px", style.font_size.0)),
                PropertyKey::FontFamily => ("font-family", svg_font_family(style)),
                PropertyKey::FontStyle => ("font-style", style.font_style.as_css_str().to_owned()),
                PropertyKey::FontWeight => ("font-weight", style.font_weight.to_string()),
                // Held by the declaration alone: no computed field carries them.
                key if key.is_svg_presentation() => {
                    let Some(value) = &property.expression else {
                        continue;
                    };
                    (svg_presentation_name(key), value.clone())
                }
                _ => continue,
            }
        };
        let value = property
            .expression
            .as_ref()
            .filter(|_| !property.inherited)
            .unwrap_or(&value);
        css.push_str(name);
        css.push(':');
        css.push_str(value);
        css.push_str("!important;");
    }
    for attribute in relative_attributes {
        let Some(expression) = &attribute.expression else {
            continue;
        };
        let Some(value) = resolve_relative_svg_presentation_value(
            attribute.property,
            expression,
            style.font_size.0,
        ) else {
            continue;
        };
        consume(
            128usize
                .saturating_add(expression.len())
                .saturating_add(value.len().saturating_mul(2)),
        )?;
        css.push_str(svg_presentation_name(attribute.property));
        css.push(':');
        css.push_str(&value);
        css.push_str("!important;");
    }
    Ok(css)
}

fn resolve_relative_svg_presentation_value(
    property: PropertyKey,
    expression: &str,
    font_size: f32,
) -> Option<String> {
    if !font_size.is_finite() {
        return None;
    }
    match property {
        PropertyKey::StrokeWidth | PropertyKey::StrokeDashoffset => {
            let (value, changed) = resolve_relative_svg_length(expression, font_size)?;
            changed.then_some(value)
        }
        PropertyKey::StrokeDasharray => {
            let mut changed = false;
            let values = expression
                .split_ascii_whitespace()
                .map(|item| {
                    let (value, item_changed) = resolve_relative_svg_length(item, font_size)?;
                    changed |= item_changed;
                    Some(value)
                })
                .collect::<Option<Vec<_>>>()?;
            changed.then(|| values.join(" "))
        }
        _ => None,
    }
}

fn resolve_relative_svg_length(value: &str, font_size: f32) -> Option<(String, bool)> {
    let (number, multiplier) = if let Some(number) = value.strip_suffix("em") {
        (number, 1.0)
    } else if let Some(number) = value.strip_suffix("ex") {
        (number, 0.5)
    } else {
        return Some((value.to_owned(), false));
    };
    let number = number.parse::<f32>().ok()?;
    let pixels = number * font_size * multiplier;
    pixels.is_finite().then(|| (format!("{pixels}px"), true))
}

#[cfg(test)]
mod tests;
