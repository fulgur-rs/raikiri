//! Shared marker content resolution for inline layout and outside painting.

use super::{
    CounterSnapshotLookup, CounterSnapshotView, EMPTY_COUNTER_SNAPSHOT, fallback_quote,
    format_counter, format_counter_component, format_counters_component,
};
use crate::{CounterSnapshot, Document};
use raikiri_style::property::{
    ContentComponent, CounterStyle, DisplayValue, ListStyleType, QuoteKeyword,
};
use raikiri_style::{CascadeResult, CounterStyleRegistry, resolve_custom_counter};

impl Document {
    /// Place a standalone image marker using the built-in painter's origin.
    pub fn standalone_marker_image_rect(
        &self,
        computed: &raikiri_style::ComputedValues,
        origin: (f32, f32),
        padding_left: f32,
        size: raikiri_traits::ImageRasterSize,
    ) -> raikiri_traits::PaintRect {
        let offset = match computed.list_style_position {
            raikiri_style::ListStylePosition::Inside => padding_left,
            _ => padding_left - size.width - 4.0,
        };
        raikiri_traits::PaintRect::new(origin.0 + offset, origin.1, size.width, size.height)
    }

    /// Shape a standalone marker and resolve its horizontal offset from the
    /// principal box, sharing the built-in painter's trailing-space handling.
    #[doc(hidden)]
    pub fn shape_list_marker_text(
        &self,
        content: &str,
        computed: &raikiri_style::ComputedValues,
        padding_left: f32,
    ) -> Option<(crate::StandaloneText, f32)> {
        let size = computed.font_size.px();
        let style = crate::StandaloneStyle {
            families: vec![
                computed
                    .font_family
                    .first()
                    .cloned()
                    .unwrap_or_else(|| raikiri_style::FontFamilyName::generic("serif")),
            ],
            font_size: if size.is_finite() && size > 0.0 {
                size
            } else {
                16.0
            },
            ..crate::StandaloneStyle::default()
        };
        let shaped =
            self.shape_standalone_text_fitted(content, &style, crate::StandaloneAlign::Start)?;
        let width = shaped.width();
        if width <= 0.0 {
            return None;
        }
        let offset = match computed.list_style_position {
            raikiri_style::ListStylePosition::Inside => padding_left,
            _ => padding_left - width - 4.0,
        };
        Some((shaped, offset))
    }
}

fn list_item_counter_value(counters: &impl CounterSnapshotLookup, ordinal: u32) -> i32 {
    counters
        .values_for("list-item")
        .last()
        .unwrap_or(ordinal as i32)
}

fn list_item_ordinal(document: &Document, cascade: &CascadeResult, node_id: usize) -> u32 {
    let Some(parent_id) = document.parent_of(node_id) else {
        return 1;
    };
    let Some(parent) = document.get_node(parent_id) else {
        // cov:ignore: parent indices come only from the document arena
        return 1;
    };
    let mut ordinal = 0_u32;
    for child_id in &parent.children {
        if cascade.computed.get(*child_id).is_some_and(|computed| {
            document.counter_display(*child_id, computed.display) == DisplayValue::ListItem
        }) {
            ordinal = ordinal.saturating_add(1);
            if *child_id == node_id {
                return ordinal;
            }
        }
    }
    1
}

fn alpha_marker(mut value: u32) -> String {
    if value == 0 {
        return String::new();
    }
    let mut result = String::new();
    while value > 0 {
        value -= 1;
        result.insert(0, char::from(b'a' + (value % 26) as u8));
        value /= 26;
    }
    result
}

fn custom_marker_text(registry: &CounterStyleRegistry, name: &str, value: i32) -> Option<String> {
    let rule = registry.get(name)?;
    // A value no style in the fallback chain can render uses `decimal`, but
    // the marker keeps the requested style's own prefix and suffix
    // (CSS Counter Styles 3, the `prefix` and `suffix` descriptors).
    let representation =
        resolve_custom_counter(registry, name, value).unwrap_or_else(|| value.to_string());
    Some(format!(
        "{}{}{}",
        rule.prefix.0.as_str(),
        representation,
        rule.suffix.0.as_str()
    ))
}

#[cfg(test)]
fn list_marker_text(
    document: &Document,
    cascade: &CascadeResult,
    node_id: usize,
    list_style_type: &ListStyleType,
) -> Option<String> {
    let ordinal = list_item_ordinal(document, cascade, node_id);
    list_marker_text_with_ordinal(cascade, list_style_type, ordinal as i32)
}

fn list_marker_text_with_ordinal(
    cascade: &CascadeResult,
    list_style_type: &ListStyleType,
    ordinal: i32,
) -> Option<String> {
    // Ordered marker styles use the CSS default `". "` suffix. The
    // author-supplied string form is handled separately and remains verbatim.
    let suffix = |text: String| format!("{text}. ");
    // CSS Counter Styles 3 §6: the alphabetic and roman styles cover only
    // positive values and fall back to `decimal` for the rest.
    let positive = |format: &dyn Fn(u32) -> String| match u32::try_from(ordinal) {
        Ok(value) if value > 0 => suffix(format(value)),
        _ => suffix(ordinal.to_string()),
    };
    match list_style_type {
        ListStyleType::Disc => Some("• ".to_string()),
        ListStyleType::None => None,
        ListStyleType::String(value) => Some(value.as_str().to_string()),
        ListStyleType::Named(name) => {
            let lower = name.to_ascii_lowercase();
            let marker = match lower.as_str() {
                "disc" => "• ".to_string(),
                "circle" => "◦ ".to_string(),
                "square" => "▪ ".to_string(),
                "decimal" => suffix(ordinal.to_string()),
                // The negative sign does not count toward the two-digit pad
                // (CSS Counter Styles 3 §3.6).
                "decimal-leading-zero" if ordinal < 0 => {
                    suffix(format!("-{:02}", ordinal.unsigned_abs()))
                }
                "decimal-leading-zero" => suffix(format!("{ordinal:02}")),
                "lower-alpha" | "lower-latin" => positive(&alpha_marker),
                "upper-alpha" | "upper-latin" => {
                    positive(&|value| alpha_marker(value).to_uppercase())
                }
                "lower-roman" | "upper-roman" => {
                    let style = CounterStyle::Named(lower.as_str().into());
                    positive(&|value| format_counter(value as i32, &style, &cascade.counter_styles))
                }
                _ => custom_marker_text(&cascade.counter_styles, name.as_str(), ordinal)
                    .unwrap_or_else(|| suffix(ordinal.to_string())),
            };
            Some(marker)
        }
        // cov:ignore: non-exhaustive enum fallback is not constructible here
        _ => Some(suffix(ordinal.to_string())),
    }
}

fn marker_content_text<T: AsRef<str>>(
    components: &[ContentComponent],
    quotes: &[(T, T)],
    quotes_auto: bool,
    ordinal: u32,
    counters: &impl CounterSnapshotLookup,
    registry: &CounterStyleRegistry,
) -> Option<String> {
    if components.is_empty() {
        return None;
    }
    if components
        .iter()
        .any(|component| matches!(component, ContentComponent::None))
    {
        // Explicit `content: none` suppresses a generated marker. `normal`
        // remains the empty-list fallback handled by marker_render_info.
        return Some(String::new());
    }
    let mut text = String::new();
    let mut depth = 0_usize;
    for component in components {
        match component {
            ContentComponent::Literal(value) => text.push_str(value.as_str()),
            ContentComponent::Counter { name, style } if name.as_str() == "list-item" => {
                text.push_str(&format_counter(
                    list_item_counter_value(counters, ordinal),
                    style,
                    registry,
                ));
            }
            ContentComponent::Counter { name, style } => {
                text.push_str(&format_counter_component(
                    counters,
                    name.as_str(),
                    style,
                    registry,
                ));
            }
            ContentComponent::Counters {
                name,
                separator,
                style,
            } if name.as_str() == "list-item" => {
                if counters.values_for(name.as_str()).is_empty() {
                    text.push_str(&format_counter(ordinal as i32, style, registry));
                } else {
                    text.push_str(&format_counters_component(
                        counters,
                        name.as_str(),
                        separator.as_str(),
                        style,
                        registry,
                    ));
                }
            }
            ContentComponent::Counters {
                name,
                separator,
                style,
            } => {
                text.push_str(&format_counters_component(
                    counters,
                    name.as_str(),
                    separator.as_str(),
                    style,
                    registry,
                ));
            }
            ContentComponent::Quote(keyword) => match keyword {
                QuoteKeyword::OpenQuote => {
                    if let Some((open, _)) = quotes.get(depth) {
                        text.push_str(open.as_ref());
                    } else if quotes_auto && quotes.is_empty() {
                        text.push_str(fallback_quote(depth, true));
                    } // cov:ignore: branch-closing line has no executable mapping
                    depth = depth.saturating_add(1);
                }
                QuoteKeyword::CloseQuote => {
                    depth = depth.saturating_sub(1);
                    if let Some((_, close)) = quotes.get(depth) {
                        text.push_str(close.as_ref());
                    } else if quotes_auto && quotes.is_empty() {
                        text.push_str(fallback_quote(depth, false));
                    } // cov:ignore: branch-closing line has no executable mapping
                }
                QuoteKeyword::NoOpenQuote => depth = depth.saturating_add(1),
                QuoteKeyword::NoCloseQuote => depth = depth.saturating_sub(1),
                // cov:ignore: non-exhaustive keyword fallback is not constructible here
                _ => {}
            },
            _ => {}
        }
    }
    Some(text)
}

/// Resolve marker style and content once for either layout or paint.
pub fn marker_render_info_with_snapshots<'a>(
    document: &Document,
    cascade: &'a CascadeResult,
    node_id: usize,
    snapshots: &[CounterSnapshot],
) -> Option<(&'a raikiri_style::ComputedValues, String)> {
    // cov:ignore: signature close has no executable mapping
    let computed = cascade.computed.get(node_id)?;
    let marker_computed = cascade
        .pseudo
        .get(&(
            raikiri_style::StyleNodeId::new(node_id as u64),
            raikiri_style::PseudoElem::Marker,
        ))
        .map(|style| style.as_ref());
    let style = marker_computed.unwrap_or(computed);
    if style.display == DisplayValue::None {
        return None;
    }
    let base = snapshots.get(node_id).unwrap_or(&EMPTY_COUNTER_SNAPSHOT);
    let counters = CounterSnapshotView::new(base, marker_computed);
    // Counter snapshots carry the implicit ordinal for ordinary list items.
    // This fallback remains for callers that supply snapshots without one.
    let ordinal = if counters.values_for("list-item").is_empty() {
        list_item_ordinal(document, cascade, node_id)
    } else {
        0
    };
    let marker_ordinal = list_item_counter_value(&counters, ordinal);
    let content = marker_computed
        .and_then(|marker| {
            marker_content_text(
                &marker.content,
                &marker.quotes,
                marker.quotes_auto,
                ordinal,
                &counters,
                &cascade.counter_styles,
            )
        })
        .or_else(|| {
            list_marker_text_with_ordinal(cascade, &computed.list_style_type, marker_ordinal)
        })?;
    Some((style, content))
}

#[cfg(test)]
mod tests;
