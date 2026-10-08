//! HTML presentational hints of legacy table and column attributes.
//!
//! HTML Living Standard §15.3.8 "Tables"
//! (<https://html.spec.whatwg.org/multipage/rendering.html#tables-2>) maps
//! these attributes into the author-level zero-specificity presentational
//! hints part of the cascade:
//!
//! - "The `table` element's `cellspacing` attribute maps to the pixel length
//!   property 'border-spacing' on the element." A pixel length mapping
//!   (§15.2) parses the value with the rules for parsing non-negative
//!   integers and uses the result as a pixel length; a parse error gives no
//!   hint.
//! - `table[rules=none i], table[rules=groups i], table[rules=rows i],
//!   table[rules=cols i], table[rules=all i] { border-style: hidden;
//!   border-collapse: collapse; }`
//! - The `col` element's `width` attribute maps to the width dimension property,
//!   using the same HTML dimension-value algorithm as image attributes.
//! - `cellpadding` maps to all four padding lengths of the table's HTML cells.
//!
//! The UA stylesheet gives every table `border-spacing: 2px`, so honoring
//! `cellspacing="0"` is what lets legacy markup remove that gap.

use crate::property::{
    BorderCollapseValue, BorderSpacingValue, BorderStyle, Length, LengthOrAuto, PropertyValue,
};
use crate::ruletree::Origin;
use crate::style_dom::{StyleDom, StyleElement, StyleNode, StyleNodeId};

use super::collect::{
    CascadedDecl, PRESENTATIONAL_HINT_SOURCE_ORDER, PRESENTATIONAL_HINT_SPECIFICITY,
};

/// Pushes HTML table and column hints, including the containing table's cell padding.
pub(crate) fn push_table_attribute_hints<D: StyleDom>(
    dom: &D,
    elem: &impl StyleElement,
    ancestors: &[StyleNodeId],
    decls: &mut Vec<CascadedDecl>,
) {
    // The mapping belongs to the HTML namespace; `namespace_uri()` is `None`
    // for it.
    if elem.namespace_uri().is_some() {
        return;
    }
    let mut push = |value: PropertyValue| {
        decls.push((
            value,
            false,
            Origin::AuthorPresentationalHint,
            PRESENTATIONAL_HINT_SPECIFICITY,
            PRESENTATIONAL_HINT_SOURCE_ORDER,
            crate::layer::LayerPosition::default(),
        ));
    };
    if elem.tag_name().eq_ignore_ascii_case("td") || elem.tag_name().eq_ignore_ascii_case("th") {
        for ancestor in ancestors.iter().rev() {
            if let Some(node) = dom.node(*ancestor)
                && let Some(table) = node.as_element()
                && table.namespace_uri().is_none()
                && table.tag_name().eq_ignore_ascii_case("table")
            {
                if let Some(padding) = table
                    .attr("cellpadding")
                    .and_then(parse_non_negative_integer)
                {
                    let px = Length::Px(padding as f32);
                    push(PropertyValue::PaddingTop(px));
                    push(PropertyValue::PaddingRight(px));
                    push(PropertyValue::PaddingBottom(px));
                    push(PropertyValue::PaddingLeft(px));
                }
                // A nested table without a valid hint does not borrow the outer table's hint.
                break;
            }
        }
        return;
    }
    if elem.tag_name().eq_ignore_ascii_case("col") {
        if let Some(width) = elem
            .attr("width")
            .and_then(super::html_quirks::parse_html_dimension_value)
        {
            push(PropertyValue::Width(LengthOrAuto::Length(width)));
        }
        return;
    }
    if !elem.tag_name().eq_ignore_ascii_case("table") {
        return;
    }
    if let Some(spacing) = elem
        .attr("cellspacing")
        .and_then(parse_non_negative_integer)
    {
        let px = Length::Px(spacing as f32);
        push(PropertyValue::BorderSpacing(BorderSpacingValue {
            horizontal: px,
            vertical: px,
        }));
    }
    if elem.attr("rules").is_some_and(|rules| {
        ["none", "groups", "rows", "cols", "all"]
            .iter()
            .any(|keyword| rules.eq_ignore_ascii_case(keyword))
    }) {
        push(PropertyValue::BorderCollapse(BorderCollapseValue::Collapse));
        push(PropertyValue::BorderTopStyle(BorderStyle::Hidden));
        push(PropertyValue::BorderRightStyle(BorderStyle::Hidden));
        push(PropertyValue::BorderBottomStyle(BorderStyle::Hidden));
        push(PropertyValue::BorderLeftStyle(BorderStyle::Hidden));
    }
}

/// HTML LS §2.3.4.1 "rules for parsing non-negative integers": the rules
/// for parsing integers (leading ASCII whitespace, an optional sign, then
/// digits; anything after the digits is ignored), failing on a negative
/// result. Values too large for `u32` saturate.
fn parse_non_negative_integer(input: &str) -> Option<u32> {
    let rest = input.trim_start_matches([' ', '\t', '\n', '\x0C', '\r']);
    let (negative, rest) = match rest.as_bytes().first() {
        Some(b'-') => (true, &rest[1..]),
        Some(b'+') => (false, &rest[1..]),
        _ => (false, rest),
    };
    let digits = rest.bytes().take_while(u8::is_ascii_digit);
    let mut value: Option<u32> = None;
    for digit in digits {
        let digit = u32::from(digit - b'0');
        value = Some(value.unwrap_or(0).saturating_mul(10).saturating_add(digit));
    }
    match value {
        Some(0) => Some(0),
        Some(_) if negative => None,
        value => value,
    }
}

#[cfg(test)]
mod tests;
