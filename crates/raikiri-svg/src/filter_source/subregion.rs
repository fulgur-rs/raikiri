//! The subregion of a filter primitive in `objectBoundingBox` units.
//!
//! A filter whose `primitiveUnits` is `objectBoundingBox` gives the `x`, `y`,
//! `width` and `height` of a primitive as fractions of the bounding box of the
//! element it filters. `usvg` reads them that way for `feFlood` and `feImage`,
//! and for no other primitive: it takes the fractions for a rectangle that it
//! places in the filter region, which clips the result to a rectangle that has
//! nothing to do with the one that was asked for.
//!
//! A primitive that `usvg` gets wrong is followed by a clip that `usvg` gets
//! right: an `feFlood`, which takes the subregion as written, and an
//! `feComposite` that keeps what lies within it. The primitive itself no
//! longer has a subregion of its own, so it fills the filter region and the
//! clip cuts the result down to the subregion, which is what the subregion
//! of a primitive is.
//!
//! A subregion that leaves out some of its four values takes them from the
//! default subregion, which is the filter region unless the primitive reads
//! the result of another one. That is known here for the primitives that
//! read the standard inputs only, in a filter whose region is a fraction of the
//! bounding box too. The others are left as they are. `feFlood` and `feImage`
//! read no input, so the values they leave out are written in: `usvg` would
//! take the whole bounding box for them.

use super::{Edit, after_name, escape, remove_attribute};
use crate::{SVG_NAMESPACE, XLINK_NAMESPACE};
use std::borrow::Cow;
use std::collections::HashSet;
use std::str::FromStr;

#[cfg(test)]
mod tests;

/// The primitives whose subregion `usvg` resolves against the filter region.
const CLIPPED: [&str; 14] = [
    "feBlend",
    "feColorMatrix",
    "feComponentTransfer",
    "feComposite",
    "feConvolveMatrix",
    "feDiffuseLighting",
    "feDisplacementMap",
    "feDropShadow",
    "feGaussianBlur",
    "feMerge",
    "feMorphology",
    "feOffset",
    "feSpecularLighting",
    "feTurbulence",
];

/// The primitives whose subregion `usvg` resolves against the bounding box,
/// but for which it takes the values that are left out for the whole box.
const PLACED: [&str; 2] = ["feFlood", "feImage"];

/// The inputs that are not the result of a primitive.
const STANDARD_INPUTS: [&str; 6] = [
    "SourceGraphic",
    "SourceAlpha",
    "BackgroundImage",
    "BackgroundAlpha",
    "FillPaint",
    "StrokePaint",
];

/// The attributes that make up the subregion of a primitive.
const SUBREGION: [&str; 4] = ["x", "y", "width", "height"];

/// The region of a filter whose units are `objectBoundingBox` when it says
/// nothing: `-10%`, `-10%`, `120%` and `120%`.
const DEFAULT_REGION: [f64; 4] = [-0.1, -0.1, 1.2, 1.2];

/// Whether `source` may hold a filter that needs this. A miss skips the look
/// at the filters, and a false hit only costs that look.
pub(super) fn may_hold(source: &str) -> bool {
    source.contains("primitiveUnits") && source.contains("objectBoundingBox")
}

/// Adds to `edits` the ones that give each primitive of a filter in
/// `objectBoundingBox` units the subregion that a browser gives it.
///
/// Each edit adds a bounded number of bytes to a primitive, and a copy of
/// two of its attributes, so what they add is in proportion to the document,
/// which the limits on its size bound.
pub(super) fn clip_subregions(source: &str, xml: &roxmltree::Document<'_>, edits: &mut Vec<Edit>) {
    let filters: Vec<_> = xml
        .descendants()
        .filter(|node| is_svg(*node, "filter"))
        .collect();
    // A filter that another one takes its attributes or its primitives from
    // may be read in other units than its own, so it is left alone, and so is
    // the one that does the taking.
    let linked: HashSet<&str> = filters
        .iter()
        .filter_map(|filter| href(*filter))
        .map(|target| target.trim().trim_start_matches('#'))
        .collect();
    for filter in filters {
        if filter.attribute("primitiveUnits") != Some("objectBoundingBox")
            || href(filter).is_some()
            || filter.attribute("id").is_some_and(|id| linked.contains(id))
        {
            continue;
        }
        let primitives: Vec<_> = filter
            .children()
            .filter(|child| {
                child.is_element()
                    && child.tag_name().namespace() == Some(SVG_NAMESPACE)
                    && (CLIPPED.contains(&child.tag_name().name())
                        || PLACED.contains(&child.tag_name().name())
                        || child.tag_name().name() == "feTile")
            })
            .collect();
        // A tile repeats the subregion of the primitive it reads, which is
        // the filter region once the clip stands in for it.
        if primitives
            .iter()
            .any(|primitive| primitive.tag_name().name() == "feTile")
        {
            continue;
        }
        clip_filter(source, filter, &primitives, edits);
    }
}

/// Adds to `edits` the ones for the primitives of `filter`.
fn clip_filter(
    source: &str,
    filter: roxmltree::Node<'_, '_>,
    primitives: &[roxmltree::Node<'_, '_>],
    edits: &mut Vec<Edit>,
) {
    let region = region(filter);
    // The names the clips go by are not any that the filter uses.
    let mut used: HashSet<String> = filter
        .descendants()
        .filter(|node| node.is_element())
        .flat_map(|node| {
            ["result", "in", "in2"]
                .into_iter()
                .filter_map(move |name| node.attribute(name))
        })
        .map(str::to_owned)
        .collect();
    let mut next = 0;
    let mut fresh = || loop {
        let name = format!("raikiri-subregion-{next}");
        next += 1;
        if used.insert(name.clone()) {
            return name;
        }
    };
    for (index, primitive) in primitives.iter().enumerate() {
        let name = primitive.tag_name().name();
        let Some(given) = given(*primitive) else {
            continue;
        };
        let count = given.iter().flatten().count();
        if count == 0 {
            continue;
        }
        if PLACED.contains(&name) {
            // The values that are left out are added, to be read as they
            // should be.
            if count < 4
                && let Some(region) = region
            {
                let end = after_name(source, *primitive);
                for ((value, attribute), default) in given.iter().zip(SUBREGION).zip(region) {
                    if value.is_none() {
                        edits.push((
                            end..end,
                            Cow::Owned(format!(" {attribute}=\"{}\"", number(default))),
                        ));
                    }
                }
            }
            continue;
        }
        let Some(rect) = rect(given, region, *primitive, index == 0) else {
            continue;
        };
        // The primitive and the two that follow it are written next to each
        // other, in the scope of the same namespaces.
        let parent = primitive
            .parent()
            .map_or(0, |parent| parent.namespaces().count());
        if primitive.namespaces().count() != parent {
            continue;
        }
        let (inner, mask) = (fresh(), fresh());
        // The name goes first: the attribute that follows the name of the
        // element may be one that is taken out, and an edit that comes at the
        // place of another goes after it.
        match primitive.attribute_node("result") {
            Some(result) => edits.push((result.range_value(), Cow::Owned(inner.clone()))),
            None => {
                let end = after_name(source, *primitive);
                edits.push((end..end, Cow::Owned(format!(" result=\"{inner}\""))));
            }
        }
        edits.extend(SUBREGION.iter().filter_map(|attribute| {
            primitive
                .attribute_node(*attribute)
                .map(|attribute| remove_attribute(source, attribute))
        }));
        let end = primitive.range().end;
        edits.push((
            end..end,
            Cow::Owned(clip(source, *primitive, &inner, &mask, rect)),
        ));
    }
}

/// The text of the two primitives that clip the result `inner` of `primitive`
/// to `rect`, which are the fractions of the bounding box that its subregion
/// is: a flood named `mask` and the composite that takes the place of the
/// primitive for what follows.
///
/// The composite is read as the primitive was, in the same color space, so
/// that nothing is converted between the two.
fn clip(
    source: &str,
    primitive: roxmltree::Node<'_, '_>,
    inner: &str,
    mask: &str,
    rect: [f64; 4],
) -> String {
    let qualified = &source[primitive.range().start + 1..after_name(source, primitive)];
    let prefix = qualified.rfind(':').map_or("", |at| &qualified[..=at]);
    let [x, y, width, height] = rect.map(number);
    // A rule that styles every flood, or every composite, does not reach the
    // clip: the style attribute of the flood goes after the rules.
    let mut text = format!(
        "<{prefix}feFlood x=\"{x}\" y=\"{y}\" width=\"{width}\" height=\"{height}\" \
         style=\"flood-color:black!important;flood-opacity:1!important\" result=\"{mask}\"/>\
         <{prefix}feComposite in=\"{inner}\" in2=\"{mask}\" operator=\"in\""
    );
    if let Some(result) = primitive.attribute("result") {
        text.push_str(&format!(" result=\"{}\"", escape(result)));
    }
    for attribute in ["color-interpolation-filters", "class", "style"] {
        if let Some(value) = primitive.attribute(attribute) {
            text.push_str(&format!(" {attribute}=\"{}\"", escape(value)));
        }
    }
    text.push_str("/>");
    text
}

/// The four values of the subregion of `primitive` as the fractions it writes,
/// or `None` when one of them is not one that a fraction can be made of.
fn given(primitive: roxmltree::Node<'_, '_>) -> Option<[Option<f64>; 4]> {
    let mut given = [None; 4];
    for (at, (value, name)) in given.iter_mut().zip(SUBREGION).enumerate() {
        if let Some(text) = primitive.attribute(name) {
            let fraction = fraction(text)?;
            // A size of nothing or less disables the primitive.
            if at >= 2 && fraction <= 0.0 {
                return None;
            }
            *value = Some(fraction);
        }
    }
    Some(given)
}

/// The subregion of `primitive` as the fractions of the bounding box, or `None`
/// when what it leaves out is not known.
fn rect(
    given: [Option<f64>; 4],
    region: Option<[f64; 4]>,
    primitive: roxmltree::Node<'_, '_>,
    first: bool,
) -> Option<[f64; 4]> {
    if let [Some(x), Some(y), Some(width), Some(height)] = given {
        return Some([x, y, width, height]);
    }
    let region = region?;
    if !standard_inputs(primitive, first) {
        return None;
    }
    let mut rect = region;
    for (value, rect) in given.iter().zip(&mut rect) {
        if let Some(value) = value {
            *rect = *value;
        }
    }
    Some(rect)
}

/// Whether `primitive` reads no result of another primitive: the inputs it
/// names are standard ones, and the ones it leaves out are the source graphic
/// because no primitive comes before it.
fn standard_inputs(primitive: roxmltree::Node<'_, '_>, first: bool) -> bool {
    let inputs: Vec<Option<&str>> = match primitive.tag_name().name() {
        "feBlend" | "feComposite" | "feDisplacementMap" => {
            vec![primitive.attribute("in"), primitive.attribute("in2")]
        }
        "feMerge" => primitive
            .children()
            .filter(|child| is_svg(*child, "feMergeNode"))
            .map(|child| child.attribute("in"))
            .collect(),
        "feTurbulence" => Vec::new(),
        _ => vec![primitive.attribute("in")],
    };
    inputs.into_iter().all(|input| match input {
        Some(name) => STANDARD_INPUTS.contains(&name),
        None => first,
    })
}

/// The region of `filter` as the fractions of the bounding box, or `None` when
/// it is not one.
fn region(filter: roxmltree::Node<'_, '_>) -> Option<[f64; 4]> {
    if filter.attribute("filterUnits") == Some("userSpaceOnUse") {
        return None;
    }
    let mut region = DEFAULT_REGION;
    for (value, name) in region.iter_mut().zip(SUBREGION) {
        if let Some(text) = filter.attribute(name) {
            *value = fraction(text)?;
        }
    }
    (region[2] > 0.0 && region[3] > 0.0).then_some(region)
}

/// What `text` says as a fraction of the bounding box: a number, or a
/// percentage.
fn fraction(text: &str) -> Option<f64> {
    let length = svgtypes::Length::from_str(text).ok()?;
    let fraction = match length.unit {
        svgtypes::LengthUnit::None => length.number,
        svgtypes::LengthUnit::Percent => length.number / 100.0,
        _ => return None,
    };
    // `usvg` reads a single precision number.
    (fraction as f32).is_finite().then_some(fraction)
}

/// A fraction as the text of an attribute, to the precision `usvg` reads it.
fn number(fraction: f64) -> String {
    (fraction as f32).to_string()
}

/// The target of the `href` of `node`.
fn href<'a>(node: roxmltree::Node<'a, '_>) -> Option<&'a str> {
    node.attribute("href")
        .or_else(|| node.attribute((XLINK_NAMESPACE, "href")))
}

/// Whether `node` is the SVG element `name`.
fn is_svg(node: roxmltree::Node<'_, '_>, name: &str) -> bool {
    node.is_element()
        && node.tag_name().namespace() == Some(SVG_NAMESPACE)
        && node.tag_name().name() == name
}
