//! The geometry that CSS gives to a shape.
//!
//! SVG 2 makes `x`, `y`, `width`, `height`, `cx`, `cy`, `r`, `rx`, `ry` and
//! `d` CSS properties of the elements they belong to, and a declaration of one
//! of them takes the place of the attribute of the same name: it is a
//! presentation attribute, so every rule of an author stylesheet and every
//! `style` attribute beats it. `usvg` only reads the presentation attributes
//! that it lists, so the geometry of a CSS declaration is lost.
//!
//! This module reads the declarations of a shape the way the cascade does and
//! tells what each of its geometry attributes has to say. The rewrite that
//! writes that into the source lives in the parent module.

use std::str::FromStr;

/// The geometry properties.
const PROPERTIES: [&str; 10] = [
    "x", "y", "width", "height", "cx", "cy", "r", "rx", "ry", "d",
];

/// The bit of the `filter` property in a [`mask`].
pub(super) const FILTER: u16 = 1 << PROPERTIES.len();

/// The geometry properties of the element `tag`, which are the ones that apply
/// to it as a CSS property. A nested `svg` takes its position and size from
/// attributes only, and so do `foreignObject`, `line` and the other elements.
pub(super) fn properties(tag: &str) -> &'static [&'static str] {
    match tag {
        "rect" => &["x", "y", "width", "height", "rx", "ry"],
        "circle" => &["cx", "cy", "r"],
        "ellipse" => &["cx", "cy", "rx", "ry"],
        "path" => &["d"],
        "image" => &["x", "y", "width", "height"],
        // Its `width` and `height` are attributes only.
        "use" => &["x", "y"],
        _ => &[],
    }
}

/// A set of properties, one bit for each geometry property and one for
/// `filter`, to tell the rules that matter from the ones that do not.
pub(super) fn mask(names: &[&str]) -> u16 {
    names.iter().fold(0, |mask, name| mask | bit(name))
}

/// The bit of the property `name`, whatever the case of its name, or zero.
fn bit(name: &str) -> u16 {
    if name.eq_ignore_ascii_case("filter") {
        return FILTER;
    }
    PROPERTIES
        .iter()
        .position(|property| name.eq_ignore_ascii_case(property))
        .map_or(0, |at| 1 << at)
}

/// The mask of the properties that `declarations` declare.
pub(super) fn declared_mask(declarations: &[simplecss::Declaration<'_>]) -> u16 {
    declarations
        .iter()
        .fold(0, |mask, declaration| mask | bit(declaration.name))
}

/// The bytes of the values that `declarations` give to the geometry properties
/// of `mask`, which is what the cascade reads of them. A value is read again
/// for each shape that it applies to.
pub(super) fn declared_bytes(declarations: &[simplecss::Declaration<'_>], mask: u16) -> usize {
    declarations
        .iter()
        .filter(|declaration| bit(declaration.name) & mask & !FILTER != 0)
        .fold(0, |total, declaration| {
            total.saturating_add(declaration.value.len())
        })
}

/// The geometry properties, which are also the names of attributes that a
/// selector may test and a rewrite would change.
pub(super) fn all() -> &'static [&'static str] {
    &PROPERTIES
}

/// Whether `source` may declare a geometry property: a property name that a
/// colon follows. A cheap test on the source, for the documents that have
/// nothing to rewrite to go without being parsed. A miss leaves the geometry
/// to `usvg` and a false hit only costs a parse.
pub(super) fn may_declare(source: &str) -> bool {
    // `usvg` reads CSS from `style` elements and attributes only.
    if !source.contains("style") {
        return false;
    }
    // A character reference may spell a name or the colon, as in
    // `width&#58;20px`, which the XML parser decodes and this scan does not
    // see. The entities that XML predefines spell none of them, and a document
    // that declares its own is not read at all.
    if source.contains("&#") {
        return true;
    }
    source.match_indices(':').any(|(at, _)| {
        let head = source[..at].trim_end();
        let len = head
            .bytes()
            .rev()
            .take_while(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
            .count();
        let name = &head[head.len() - len..];
        // A comment may stand between the name and the colon, as in
        // `width/**/:20px`, which the declaration is read past. CDATA
        // sections may cut the name up, as in `wid]]><![CDATA[th:`, and the
        // XML parser joins them. What is before such a mark is not looked for:
        // the colon counts.
        let before = &head[..head.len() - len];
        if head.ends_with("*/")
            || ["]]>", "<![CDATA["]
                .iter()
                .any(|mark| before.ends_with(mark))
        {
            return true;
        }
        PROPERTIES
            .iter()
            .any(|property| name.eq_ignore_ascii_case(property))
    })
}

/// What a declaration says about a property.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Declared {
    /// The text that the attribute of the property takes instead of its own.
    Attribute(String),
    /// A value that is valid and wins the cascade, but that this does not
    /// compute: `calc()`, `var()`, `inherit`, a unit that `usvg` does not
    /// know. The attribute is left as it is.
    Opaque,
}

/// How a value reads for a property.
#[derive(Debug, PartialEq)]
enum Parsed {
    Value(Declared),
    /// Not a value of the property: the browser drops the declaration, so it
    /// does not take part in the cascade.
    Invalid,
}

/// What the cascade makes of `declarations`, which are in the order of their
/// priority from the lowest up (the rules by specificity and then by their
/// place in the source, and the `style` attribute after them): for each of
/// `properties` that a valid declaration sets, in the order of `properties`,
/// the winner.
///
/// An important declaration beats every one that is not, and among the
/// declarations of one kind the later one wins.
pub(super) fn cascade<'a>(
    declarations: impl IntoIterator<Item = simplecss::Declaration<'a>>,
    properties: &[&'static str],
) -> Vec<(&'static str, Declared)> {
    // The winner of each property, and whether it is important.
    let mut winners: Vec<Option<(bool, Declared)>> = std::iter::repeat_with(|| None)
        .take(properties.len())
        .collect();
    for declaration in declarations {
        let Some(at) = properties
            .iter()
            .position(|property| declaration.name.eq_ignore_ascii_case(property))
        else {
            continue;
        };
        let Parsed::Value(declared) = parse(properties[at], declaration.value) else {
            continue;
        };
        if !winners[at]
            .as_ref()
            .is_some_and(|(important, _)| *important && !declaration.important)
        {
            winners[at] = Some((declaration.important, declared));
        }
    }
    properties
        .iter()
        .zip(winners)
        .filter_map(|(property, winner)| Some((*property, winner?.1)))
        .collect()
}

/// How the `value` of a declaration of `property` reads.
fn parse(property: &str, value: &str) -> Parsed {
    let value = value.trim();
    // Every property here is reset by `initial`, and so is it by `unset`,
    // because none of them is inherited.
    if value.eq_ignore_ascii_case("initial") || value.eq_ignore_ascii_case("unset") {
        return Parsed::Value(Declared::Attribute(initial(property).to_owned()));
    }
    // The cascade of these keywords reaches into other origins and layers, and
    // a comment or an escape may hide a value that this does not read.
    if ["inherit", "revert", "revert-layer"]
        .iter()
        .any(|keyword| value.eq_ignore_ascii_case(keyword))
        || value.contains("/*")
        || value.contains('\\')
    {
        return Parsed::Value(Declared::Opaque);
    }
    if property == "d" {
        path(value)
    } else {
        length(property, value)
    }
}

/// The value that a property takes when nothing sets it, as an attribute.
fn initial(property: &str) -> &'static str {
    match property {
        // The size of a box and the radius of a rounded corner.
        "width" | "height" | "rx" | "ry" => "auto",
        // No path, which draws nothing.
        "d" => "",
        _ => "0",
    }
}

/// Whether the property takes `auto`, which is a size of no extent for a
/// `rect` and a radius that follows the other one for a rounded corner.
fn takes_auto(property: &str) -> bool {
    matches!(property, "width" | "height" | "rx" | "ry")
}

/// Whether the property refuses a negative length.
fn is_non_negative(property: &str) -> bool {
    matches!(property, "width" | "height" | "r" | "rx" | "ry")
}

/// A `<length-percentage>`, or `auto` where the property takes it.
///
/// A number without a unit is a length here, as it is for an attribute:
/// Chromium reads `width:20` in a style declaration of a `rect` as 20px.
fn length(property: &str, value: &str) -> Parsed {
    if value.eq_ignore_ascii_case("auto") {
        return if takes_auto(property) {
            Parsed::Value(Declared::Attribute("auto".to_owned()))
        } else {
            Parsed::Invalid
        };
    }
    // `usvg` reads the units in lowercase only.
    let text = value.to_ascii_lowercase();
    match svgtypes::Length::from_str(&text) {
        Ok(_) if !is_css_length(&text) => Parsed::Invalid,
        Ok(length) if length.number < 0.0 && is_non_negative(property) => Parsed::Invalid,
        Ok(_) => Parsed::Value(Declared::Attribute(text)),
        Err(_) if is_deferred(value) || has_length_unit(value) => Parsed::Value(Declared::Opaque),
        Err(_) => Parsed::Invalid,
    }
}

/// Whether `value` is, or holds, a function whose result a browser computes
/// later: a math function such as `calc()`, or a substitution such as `var()`.
/// Such a value is valid when it is parsed, whatever it comes to.
fn is_deferred(value: &str) -> bool {
    const MATH: [&str; 22] = [
        "calc",
        "min",
        "max",
        "clamp",
        "round",
        "mod",
        "rem",
        "sin",
        "cos",
        "tan",
        "asin",
        "acos",
        "atan",
        "atan2",
        "pow",
        "sqrt",
        "hypot",
        "log",
        "exp",
        "abs",
        "sign",
        "calc-size",
    ];
    let value = value.to_ascii_lowercase();
    if ["var(", "env(", "attr("]
        .iter()
        .any(|function| value.contains(function))
    {
        return true;
    }
    let value = value
        .strip_prefix("-webkit-")
        .or_else(|| value.strip_prefix("-moz-"))
        .unwrap_or(&value);
    MATH.iter().any(|function| {
        value
            .strip_prefix(function)
            .is_some_and(|rest| rest.starts_with('('))
    })
}

/// The length of the number that starts `text`, in the grammar of CSS. A dot
/// needs a digit after it, so `1.px` is the number `1` and then `.px`, and an
/// exponent needs a digit, or the `e` starts the unit, as in `em`.
fn number_len(text: &str) -> Option<usize> {
    let bytes = text.as_bytes();
    let digits = |at: usize| {
        bytes[at..]
            .iter()
            .take_while(|byte| byte.is_ascii_digit())
            .count()
    };
    let mut at = usize::from(matches!(bytes.first(), Some(b'+' | b'-')));
    let whole = digits(at);
    at += whole;
    let mut fraction = 0;
    if bytes.get(at) == Some(&b'.') {
        fraction = digits(at + 1);
        if fraction > 0 {
            at += 1 + fraction;
        }
    }
    if whole + fraction == 0 {
        return None;
    }
    if matches!(bytes.get(at), Some(b'e' | b'E')) {
        let sign = usize::from(matches!(bytes.get(at + 1), Some(b'+' | b'-')));
        let exponent = digits(at + 1 + sign);
        if exponent > 0 {
            at += 1 + sign + exponent;
        }
    }
    Some(at)
}

/// The units of length that CSS has, in lowercase. A unit that is not one of
/// these makes a declaration invalid, and a browser drops it.
const LENGTH_UNITS: &[&str] = &[
    // Relative to the font.
    "em", "rem", "ex", "rex", "cap", "rcap", "ch", "rch", "ic", "ric", "lh", "rlh",
    // Relative to the viewport, with its small, large and dynamic sizes.
    "vw", "vh", "vi", "vb", "vmin", "vmax", "svw", "svh", "svi", "svb", "svmin", "svmax", "lvw",
    "lvh", "lvi", "lvb", "lvmin", "lvmax", "dvw", "dvh", "dvi", "dvb", "dvmin", "dvmax",
    // Relative to a container.
    "cqw", "cqh", "cqi", "cqb", "cqmin", "cqmax", // Absolute.
    "cm", "mm", "q", "in", "pt", "pc", "px",
];

/// Whether `unit`, which follows a number, is a percent sign or a unit of
/// length.
fn is_unit(unit: &str) -> bool {
    unit == "%"
        || LENGTH_UNITS
            .iter()
            .any(|known| unit.eq_ignore_ascii_case(known))
}

/// Whether `text`, which `svgtypes` has taken for a length, is one in CSS: a
/// number of CSS, and then a unit or nothing. `svgtypes` reads the number of
/// SVG, which allows `1.px`.
fn is_css_length(text: &str) -> bool {
    number_len(text).is_some_and(|at| text[at..].is_empty() || is_unit(&text[at..]))
}

/// Whether `value` is a number with a unit of length, which `svgtypes` failed
/// to read: a unit it does not know, such as `rem` or `vw`, or a number too
/// large for it. A browser reads both. A suffix that is not a unit makes the
/// value invalid.
fn has_length_unit(value: &str) -> bool {
    number_len(value).is_some_and(|at| is_unit(&value[at..]))
}

/// `none`, or a `path()` of a string, which is the data of the attribute.
///
/// A `path()` that starts with a fill rule is not a value of `d`: Chromium
/// drops `d:path(evenodd, '...')` and keeps the attribute. It drops the
/// declaration as well when the data has an error anywhere, where the
/// attribute is drawn up to the error.
fn path(value: &str) -> Parsed {
    if value.eq_ignore_ascii_case("none") {
        return Parsed::Value(Declared::Attribute(String::new()));
    }
    let Some(arguments) = value
        .get(..5)
        .filter(|head| head.eq_ignore_ascii_case("path("))
        .and_then(|_| value[5..].strip_suffix(')'))
    else {
        return if is_deferred(value) {
            Parsed::Value(Declared::Opaque)
        } else {
            Parsed::Invalid
        };
    };
    let arguments = arguments.trim();
    let Some(quote) = arguments.chars().next().filter(|c| matches!(c, '"' | '\'')) else {
        return Parsed::Invalid;
    };
    let rest = &arguments[1..];
    match rest.find(quote) {
        // A line break ends a string that is not closed.
        Some(end)
            if rest[end + 1..].trim().is_empty()
                && !rest[..end].contains(['\n', '\r', '\x0c'])
                && svgtypes::PathParser::from(&rest[..end]).all(|segment| segment.is_ok()) =>
        {
            Parsed::Value(Declared::Attribute(rest[..end].to_owned()))
        }
        _ => Parsed::Invalid,
    }
}

#[cfg(test)]
mod tests;
