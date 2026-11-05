//! Source rewrites for the places where `usvg` differs from the
//! specifications and the browsers.
//!
//! - A zero `divisor` on `feConvolveMatrix` asks for the default (the sum of
//!   the kernel, or 1 when that sum is zero), but `usvg` drops the primitive.
//! - A shape that is not rendered, such as a `rect` of no width or a `path`
//!   without segments, draws nothing, and its filter draws nothing either,
//!   but `usvg` keeps the filter on the empty group it makes, so an `feFlood`
//!   fills its region.
//! - The geometry of a shape can be a CSS property (`width`, `r`, `d`, and
//!   the like), which takes the place of the attribute of the same name, but
//!   `usvg` only reads the attribute. The declaration that wins the cascade is
//!   written into the attribute.
//!
//! All are mended in the XML before `usvg` reads it: the parsed tree no
//! longer tells an empty `g`, whose filter does draw, from a shape that was
//! left out. The filter of such a shape is turned off in its `style`
//! attribute, so the element keeps its name, its `id` and its place for
//! everything that looks at them, such as a `use`, a `switch` or a selector.

mod geometry;

use crate::{MAX_INITIAL_PARSE_XML_NODES, SVG_NAMESPACE, XLINK_NAMESPACE};
use geometry::Declared;
use std::borrow::Cow;
use std::collections::HashMap;
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

/// A change to the source: the text that takes the place of a range.
type Edit = (Range<usize>, Cow<'static, str>);

/// `source` with the geometry that CSS gives to a shape written into its
/// attributes, with the filters of the shapes that are not rendered turned
/// off, and with a zero `divisor` of an `feConvolveMatrix` turned into the
/// default.
///
/// The source is returned as it is when nothing in it needs one of these, or
/// when it is not XML that this reads, which `usvg` then reports.
pub(crate) fn like_browsers(source: &str) -> Cow<'_, str> {
    // The filter cases need a filter element, and the geometry cases need a
    // declaration of a geometry property, so a document with neither is not
    // read.
    let filters = source.contains("<filter") || source.contains(":filter");
    let styled_geometry = geometry::may_declare(source);
    if !filters && !styled_geometry {
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
    //
    // `simplecss` reads `!important` in lowercase only, so the text that it
    // reads has the keyword spelled that way.
    let styles: Vec<Cow<'_, str>> = xml
        .descendants()
        .filter(|node| {
            node.has_tag_name("style")
                && node
                    .attribute("type")
                    .is_none_or(|kind| kind.is_empty() || kind == "text/css")
        })
        .flat_map(|node| node.children().filter_map(|child| child.text()))
        .map(geometry::with_lowercase_important)
        .collect();
    let mut sheet = simplecss::StyleSheet::new();
    // What a selector tests is left as it is: the edits below change the
    // `style`, `divisor` and geometry attributes, and with them what such a
    // selector matches.
    let (mut tests_style, mut tests_divisor, mut tests_geometry) = (false, false, false);
    for css in &styles {
        sheet.parse_more(css);
        tests_style |= tests_attribute(css, "style");
        tests_divisor |= tests_attribute(css, "divisor");
        tests_geometry |= geometry::all()
            .iter()
            .any(|property| tests_attribute(css, property));
    }
    // The `style` attributes that spell `!important` in another way.
    let restyled: HashMap<roxmltree::NodeId, String> = xml
        .descendants()
        .filter_map(
            |node| match geometry::with_lowercase_important(node.attribute("style")?) {
                Cow::Owned(style) => Some((node.id(), style)),
                Cow::Borrowed(_) => None,
            },
        )
        .collect();
    // The rules that say something about a geometry property or a filter. The
    // others cannot change what is read below, and are not matched.
    let masks: Vec<u16> = sheet
        .rules
        .iter()
        .map(|rule| geometry::declared_mask(&rule.declarations))
        .collect();
    let mut budget = super::SelectorFreezeBudget::new();
    let mut edits: Vec<Edit> = Vec::new();
    // Whether the budget has run out, which leaves the geometry of the shapes
    // that follow to `usvg`.
    let mut spent = false;
    for node in xml.descendants().filter(|node| node.is_element()) {
        let name = node.tag_name();
        if name.namespace() != Some(SVG_NAMESPACE) {
            continue;
        }
        if name.name() == "feConvolveMatrix" {
            if !tests_divisor
                && let Some(divisor) = node.attribute_node("divisor")
                && svgtypes::Number::from_str(divisor.value())
                    .is_ok_and(|number| is_zero_divisor(number.0))
            {
                // The space before the attribute goes with it.
                let start = source[..divisor.range().start]
                    .trim_end_matches([' ', '\t', '\r', '\n'])
                    .len();
                edits.push((start..divisor.range().end, Cow::Borrowed("")));
            }
            continue;
        }
        let properties = geometry::properties(name.name());
        if spent && !properties.is_empty() {
            continue;
        }
        // The filter of a shape is turned off when the shape has nothing to
        // render, which the attributes can say, or a declaration.
        let filtering = filters && !tests_style;
        let empty = filtering && is_not_rendered(node, &[]);
        // What CSS makes of the geometry of the shape is read where a
        // document declares it, and for a shape that the attributes leave
        // with nothing to render, because a declaration may give it
        // something. One look at the rules tells that and the filter.
        let styled = !properties.is_empty() && (styled_geometry || empty);
        let filtered = filtering && (empty || (styled_geometry && !properties.is_empty()));
        let wanted = if styled {
            geometry::mask(properties)
        } else {
            0
        } | if filtered { geometry::FILTER } else { 0 };
        let found = if wanted == 0 {
            Vec::new()
        } else if let Some(found) =
            declarations(node, &sheet, &masks, wanted, &restyled, &mut budget)
        {
            found
        } else {
            // Finding out takes more than the budget allows, which leaves the
            // shape to `usvg`.
            continue;
        };
        let mut declared = if styled {
            geometry::cascade(found.iter().copied(), properties)
        } else {
            Vec::new()
        };
        if tests_geometry {
            // The attributes are not changed, so a declaration is only known
            // to say something about the geometry.
            for (_, value) in &mut declared {
                *value = Declared::Opaque;
            }
        }
        // Reading the declarations and writing the attributes is charged before
        // anything is written: a long value that a rule gives to many shapes
        // is copied for each of them, so the source could grow without bound.
        // A shape that goes over the budget is left to `usvg`, with the ones
        // after it.
        let written = declared
            .iter()
            .filter_map(|(property, value)| match value {
                Declared::Attribute(text) => Some(attribute_bytes(property, text)),
                Declared::Opaque => None,
            })
            .fold(0, usize::saturating_add);
        let read = geometry::declared_bytes(&found, wanted);
        if budget.bytes(read.saturating_add(written)).is_err() {
            spent = true;
            continue;
        }
        for (property, value) in &declared {
            if let Declared::Attribute(text) = value {
                edits.push(set_attribute(source, node, property, text));
            }
        }
        if filtered && is_not_rendered(node, &declared) && has_filter(node, &found) {
            edits.push(turn_filter_off(source, node));
        }
    }
    if edits.is_empty() {
        Cow::Borrowed(source)
    } else {
        Cow::Owned(apply(source, edits))
    }
}

/// Whether a selector in the stylesheet `css` tests the attribute `name`,
/// which is the name that follows a `[`.
fn tests_attribute(css: &str, name: &str) -> bool {
    css.match_indices('[').any(|(at, _)| {
        let rest = css[at + 1..].trim_start();
        rest.get(..name.len())
            .is_some_and(|head| head.eq_ignore_ascii_case(name))
            && !rest[name.len()..]
                .starts_with(|c: char| c.is_alphanumeric() || c == '-' || c == '_')
    })
}

/// Whether `usvg` takes `number` for a zero: it reads a single precision float
/// and drops the primitive for a divisor within four units in the last place of
/// zero, which holds for a number that underflows.
fn is_zero_divisor(number: f64) -> bool {
    (number as f32).abs() <= 4.0 * f32::from_bits(1)
}

/// Where the attributes of the element `node` can be added: right after its
/// name.
fn after_name(source: &str, node: roxmltree::Node<'_, '_>) -> usize {
    let name = node.range().start + 1;
    let rest = &source[name..];
    name + rest
        .find(|c: char| c.is_ascii_whitespace() || c == '/' || c == '>')
        .unwrap_or(rest.len())
}

/// The edit that turns the filter of the shape `node` off in its `style`
/// attribute, which it gives the shape if there is none.
fn turn_filter_off(source: &str, node: roxmltree::Node<'_, '_>) -> Edit {
    if let Some(style) = node.attribute_node("style") {
        let start = style.range_value().start;
        return (start..start, Cow::Borrowed(PREPENDED_NO_FILTER));
    }
    let end = after_name(source, node);
    (end..end, Cow::Borrowed(ADDED_NO_FILTER))
}

/// The text that takes the place of `c` in a quoted attribute, if it has to be
/// escaped.
fn entity(c: char) -> Option<&'static str> {
    match c {
        '&' => Some("&amp;"),
        '<' => Some("&lt;"),
        '"' => Some("&quot;"),
        '\'' => Some("&apos;"),
        _ => None,
    }
}

/// The bytes that `value` takes in a quoted attribute.
fn escaped_len(value: &str) -> usize {
    value
        .chars()
        .map(|c| entity(c).map_or(c.len_utf8(), str::len))
        .fold(0, usize::saturating_add)
}

/// What the edit of [`set_attribute`] holds on to: its text, which the output
/// holds a second time, and the edit itself.
fn attribute_bytes(name: &str, value: &str) -> usize {
    escaped_len(value)
        .saturating_mul(2)
        .saturating_add(name.len() + " =\"\"".len() + std::mem::size_of::<Edit>())
}

/// The edit that gives the attribute `name` of `node` the value `value`, which
/// it adds if the element has none.
fn set_attribute(source: &str, node: roxmltree::Node<'_, '_>, name: &str, value: &str) -> Edit {
    let mut text = String::with_capacity(escaped_len(value));
    for c in value.chars() {
        match entity(c) {
            Some(entity) => text.push_str(entity),
            None => text.push(c),
        }
    }
    if let Some(attribute) = node.attribute_node(name) {
        return (attribute.range_value(), Cow::Owned(text));
    }
    let end = after_name(source, node);
    (end..end, Cow::Owned(format!(" {name}=\"{text}\"")))
}

/// The declarations that apply to `node` and that say something about the
/// properties of `wanted`: those of the `style` attribute and of the rules
/// that select the node, the lowest priority first. The rules are in the
/// order of their specificity and, among the ones that are as specific, of
/// their place in the source.
///
/// `None` when finding out takes more than the budget allows.
fn declarations<'a>(
    node: roxmltree::Node<'a, '_>,
    sheet: &simplecss::StyleSheet<'a>,
    masks: &[u16],
    wanted: u16,
    restyled: &'a HashMap<roxmltree::NodeId, String>,
    budget: &mut super::SelectorFreezeBudget,
) -> Option<Vec<simplecss::Declaration<'a>>> {
    let mut found = Vec::new();
    for (rule, mask) in sheet.rules.iter().zip(masks) {
        if mask & wanted != 0
            && super::budgeted_selector_matches(&rule.selector, node, budget).ok()?
        {
            found.extend(rule.declarations.iter().copied());
        }
    }
    if let Some(style) = restyled
        .get(&node.id())
        .map(String::as_str)
        .or_else(|| node.attribute("style"))
    {
        found.extend(simplecss::DeclarationTokenizer::from(style));
    }
    Some(found)
}

/// Whether a filter may apply to `node`: it names one itself, or a
/// declaration does.
fn has_filter(node: roxmltree::Node<'_, '_>, declarations: &[simplecss::Declaration<'_>]) -> bool {
    node.attribute("filter")
        .is_some_and(|filter| filter.trim() != "none")
        || declarations
            .iter()
            .any(|declaration| declaration.name.eq_ignore_ascii_case("filter"))
}

/// A dimension of a shape, as far as its attribute or declaration tells.
#[derive(Clone, Copy)]
enum Dimension {
    /// A declaration gives it a value that is not computed here, which may be
    /// anything.
    Opaque,
    /// Not given, `auto`, or not a length.
    Auto,
    Length(f64),
}

impl Dimension {
    /// The dimension that `text` is the value of.
    fn of(text: &str) -> Self {
        svgtypes::Length::from_str(text).map_or(Self::Auto, |length| Self::Length(length.number))
    }

    /// Whether the dimension leaves something to render: it is above zero, or
    /// a declaration may make it so.
    fn is_positive(self) -> bool {
        match self {
            Self::Opaque => true,
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

/// Whether the shape `node` has nothing to render: a computed zero for a
/// dimension disables the rendering of a `rect`, a `circle`, an `ellipse` and
/// an `image` (SVG 2 §10), a path or a list of points without a point has no
/// shape, and a picture without an address has nothing to draw. Other
/// elements are rendered.
///
/// A path of a moveto only and a list of a single point are shapes with an
/// empty box, which a browser still draws the filter of, so they stay.
///
/// The geometry of a shape can also come from a declaration, which `declared`
/// holds the winners of, and which takes the place of the attribute.
fn is_not_rendered(node: roxmltree::Node<'_, '_>, declared: &[(&str, Declared)]) -> bool {
    let declaration = |name: &str| {
        declared
            .iter()
            .find(|(property, _)| *property == name)
            .map(|(_, value)| value)
    };
    let dimension = |name: &str| match declaration(name) {
        Some(Declared::Opaque) => Dimension::Opaque,
        Some(Declared::Attribute(text)) => Dimension::of(text),
        None => node.attribute(name).map_or(Dimension::Auto, Dimension::of),
    };
    let has_segment =
        |data: &str| svgtypes::SimplifyingPathParser::from(data).any(|segment| segment.is_ok());
    match node.tag_name().name() {
        "rect" => !dimension("width").is_positive() || !dimension("height").is_positive(),
        "circle" => !dimension("r").is_positive(),
        "ellipse" => {
            // A radius that is not given takes the other one.
            let (rx, ry) = (dimension("rx"), dimension("ry"));
            !rx.or(ry).is_positive() || !ry.or(rx).is_positive()
        }
        "path" => match declaration("d") {
            Some(Declared::Opaque) => false,
            Some(Declared::Attribute(data)) => !has_segment(data),
            None => node.attribute("d").is_none_or(|data| !has_segment(data)),
        },
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
/// do not overlap, and the edits at one place go in the order they come in.
fn apply(source: &str, mut edits: Vec<Edit>) -> String {
    edits.sort_by_key(|(range, _)| range.start);
    let mut out = String::with_capacity(source.len());
    let mut at = 0;
    for (range, text) in edits {
        out.push_str(&source[at..range.start]);
        out.push_str(&text);
        at = range.end;
    }
    out.push_str(&source[at..]);
    out
}
