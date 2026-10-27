//! Source rewrites for the two places where `usvg` filters differently from
//! the specifications and the browsers.
//!
//! - A zero `divisor` on `feConvolveMatrix` asks for the default (the sum of
//!   the kernel, or 1 when that sum is zero), but `usvg` drops the primitive.
//! - A shape that is not rendered, such as a `rect` of no width or a `path`
//!   without segments, draws nothing, and its filter draws nothing either,
//!   but `usvg` keeps the filter on the empty group it makes, so an `feFlood`
//!   fills its region.
//!
//! Both are mended in the XML before `usvg` reads it: the parsed tree no
//! longer tells an empty `g`, whose filter does draw, from a shape that was
//! left out. The filter of such a shape is turned off in its `style`
//! attribute, so the element keeps its name, its `id` and its place for
//! everything that looks at them, such as a `use`, a `switch` or a selector.

use crate::{MAX_INITIAL_PARSE_XML_NODES, SVG_NAMESPACE, XLINK_NAMESPACE};
use std::borrow::Cow;
use std::ops::Range;
use std::str::FromStr;

#[cfg(test)]
mod tests;

/// What turns the filter of a shape off when it goes first in the `style`
/// attribute the shape has. `usvg` reads that attribute after the stylesheets
/// and keeps the first important declaration of a property, so the filter of
/// the attribute gives way, and so does one of a stylesheet that is not
/// important.
const PREPENDED_NO_FILTER: &str = "filter:none!important;";

/// The same, as the `style` attribute of a shape that has none.
const ADDED_NO_FILTER: &str = " style=\"filter:none!important\"";

/// `source` with the filters of the shapes that are not rendered turned off,
/// and with a zero `divisor` of an `feConvolveMatrix` turned into the default.
///
/// The source is returned as it is when it holds no filter element, or when
/// it is not XML that this reads, which `usvg` then reports.
pub(crate) fn like_browsers(source: &str) -> Cow<'_, str> {
    // Every case needs a filter element, so a document without one is not read.
    if !source.contains("<filter") && !source.contains(":filter") {
        return Cow::Borrowed(source);
    }
    let Ok(xml) = roxmltree::Document::parse_with_options(
        source,
        roxmltree::ParsingOptions {
            nodes_limit: MAX_INITIAL_PARSE_XML_NODES,
            ..Default::default()
        },
    ) else {
        return Cow::Borrowed(source);
    };
    // A style element is found by its name, as `usvg` finds it, and its rules
    // are read as a browser reads them: whatever its text holds counts, even
    // when `usvg` stops at a comment or at an empty `type`.
    let mut sheet = simplecss::StyleSheet::new();
    for node in xml.descendants().filter(|node| node.has_tag_name("style")) {
        if node
            .attribute("type")
            .is_none_or(|kind| kind.is_empty() || kind == "text/css")
        {
            for css in node.children().filter_map(|child| child.text()) {
                sheet.parse_more(css);
            }
        }
    }
    let mut budget = super::SelectorFreezeBudget::new();
    let mut edits = Vec::new();
    for node in xml.descendants().filter(|node| node.is_element()) {
        let name = node.tag_name();
        if name.namespace() != Some(SVG_NAMESPACE) {
            continue;
        }
        if name.name() == "feConvolveMatrix" {
            if let Some(divisor) = node.attribute_node("divisor")
                && svgtypes::Number::from_str(divisor.value())
                    .is_ok_and(|number| is_zero_divisor(number.0))
            {
                // The space before the attribute goes with it.
                let start = source[..divisor.range().start]
                    .trim_end_matches([' ', '\t', '\r', '\n'])
                    .len();
                edits.push((start..divisor.range().end, ""));
            }
        } else if is_not_rendered(node, &[]) {
            // The attributes leave nothing to render, but the declarations
            // that apply to the shape may still give it something to render,
            // or a filter.
            if let Some(declared) = declared_properties(node, &sheet, &mut budget)
                && has_filter(node, &declared)
                && is_not_rendered(node, &declared)
            {
                edits.push(turn_filter_off(source, node));
            }
        }
    }
    if edits.is_empty() {
        Cow::Borrowed(source)
    } else {
        Cow::Owned(apply(source, edits))
    }
}

/// Whether `usvg` takes `number` for a zero: it reads a single precision float
/// and drops the primitive for a divisor within four units in the last place of
/// zero, which holds for a number that underflows.
fn is_zero_divisor(number: f64) -> bool {
    (number as f32).abs() <= 4.0 * f32::from_bits(1)
}

/// The edit that turns the filter of the shape `node` off in its `style`
/// attribute, which it gives the shape if there is none.
fn turn_filter_off<'a>(source: &str, node: roxmltree::Node<'_, '_>) -> (Range<usize>, &'a str) {
    if let Some(style) = node.attribute_node("style") {
        let start = style.range_value().start;
        return (start..start, PREPENDED_NO_FILTER);
    }
    // The attribute goes right after the name of the element.
    let name = node.range().start + 1;
    let rest = &source[name..];
    let end = name
        + rest
            .find(|c: char| c.is_ascii_whitespace() || c == '/' || c == '>')
            .unwrap_or(rest.len());
    (end..end, ADDED_NO_FILTER)
}

/// The names of the properties that the `style` attribute of `node` and the
/// rules of `sheet` that select it declare, or `None` when finding out takes
/// more than the budget allows, which leaves the shape to `usvg`.
fn declared_properties<'a>(
    node: roxmltree::Node<'a, '_>,
    sheet: &simplecss::StyleSheet<'a>,
    budget: &mut super::SelectorFreezeBudget,
) -> Option<Vec<&'a str>> {
    let mut declared: Vec<&str> = node
        .attribute("style")
        .into_iter()
        .flat_map(simplecss::DeclarationTokenizer::from)
        .map(|declaration| declaration.name)
        .collect();
    for rule in &sheet.rules {
        if super::budgeted_selector_matches(&rule.selector, node, budget).ok()? {
            declared.extend(rule.declarations.iter().map(|declaration| declaration.name));
        }
    }
    Some(declared)
}

/// Whether `declared` has the property `name`, whatever the case of the name.
fn declares(declared: &[&str], name: &str) -> bool {
    declared
        .iter()
        .any(|property| property.eq_ignore_ascii_case(name))
}

/// Whether a filter may apply to `node`: it names one itself, or a
/// declaration does.
fn has_filter(node: roxmltree::Node<'_, '_>, declared: &[&str]) -> bool {
    node.attribute("filter")
        .is_some_and(|filter| filter.trim() != "none")
        || declares(declared, "filter")
}

/// A dimension of a shape, as far as its attribute tells.
#[derive(Clone, Copy)]
enum Dimension {
    /// A style declaration may give it another value.
    Styled,
    /// Not given, or not a length.
    Auto,
    Length(f64),
}

impl Dimension {
    /// Whether the dimension leaves something to render: it is above zero, or
    /// a style declaration may make it so.
    fn is_positive(self) -> bool {
        match self {
            Self::Styled => true,
            Self::Auto => false,
            Self::Length(number) => number > 0.0,
        }
    }

    /// `self`, or `other` when `self` is not given.
    fn or(self, other: Self) -> Self {
        match self {
            Self::Auto => other,
            _ => self,
        }
    }
}

/// Whether the shape `node` has nothing to render, by its attributes: a
/// computed zero for a dimension disables the rendering of a `rect`, a
/// `circle`, an `ellipse` and an `image` (SVG 2 §10), a path or a list of
/// points without a point has no shape, and a picture without an address
/// has nothing to draw. Other elements are rendered.
///
/// A path of a moveto only and a list of a single point are shapes with an
/// empty box, which a browser still draws the filter of, so they stay.
///
/// The geometry of a shape can also come from a style declaration, which
/// `usvg` does not read; a dimension that one of the `declared` properties
/// sets is taken as rendered.
fn is_not_rendered(node: roxmltree::Node<'_, '_>, declared: &[&str]) -> bool {
    let styled = |name: &str| declares(declared, name);
    let dimension = |name: &str| {
        if styled(name) {
            return Dimension::Styled;
        }
        node.attribute(name)
            .and_then(|value| svgtypes::Length::from_str(value).ok())
            .map_or(Dimension::Auto, |length| Dimension::Length(length.number))
    };
    match node.tag_name().name() {
        "rect" => !dimension("width").is_positive() || !dimension("height").is_positive(),
        "circle" => !dimension("r").is_positive(),
        "ellipse" => {
            // A radius that is not given takes the other one.
            let (rx, ry) = (dimension("rx"), dimension("ry"));
            !rx.or(ry).is_positive() || !ry.or(rx).is_positive()
        }
        "path" => {
            !styled("d")
                && node.attribute("d").is_none_or(|data| {
                    !svgtypes::SimplifyingPathParser::from(data).any(|segment| segment.is_ok())
                })
        }
        "polygon" | "polyline" => node
            .attribute("points")
            .is_none_or(|points| svgtypes::PointsParser::from(points).next().is_none()),
        // A picture of an unset size takes the size of its pixels.
        "image" => {
            let href = node
                .attribute("href")
                .or_else(|| node.attribute((XLINK_NAMESPACE, "href")));
            href.is_none_or(|href| href.trim().is_empty())
                || ["width", "height"].into_iter().any(
                    |name| matches!(dimension(name), Dimension::Length(number) if number <= 0.0),
                )
        }
        _ => false,
    }
}

/// `source` with each of `edits` put in the place of its range. The ranges
/// do not overlap.
fn apply(source: &str, mut edits: Vec<(Range<usize>, &str)>) -> String {
    edits.sort_by_key(|(range, _)| range.start);
    let mut out = String::with_capacity(source.len());
    let mut at = 0;
    for (range, text) in edits {
        out.push_str(&source[at..range.start]);
        out.push_str(text);
        at = range.end;
    }
    out.push_str(&source[at..]);
    out
}
