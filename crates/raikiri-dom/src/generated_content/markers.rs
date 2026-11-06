//! Shared marker content resolution for inline layout and outside painting.

use super::{
    CounterSnapshotLookup, CounterSnapshotView, EMPTY_COUNTER_SNAPSHOT, QuoteDepth,
    SnapshotCollection, author_style_name, format_counter, format_counter_component,
    format_counters_component,
};
use crate::{CompactCounterSnapshots, CounterSnapshots, Document};
use raikiri_style::property::{ContentComponent, CounterStyle, DisplayValue, ListStyleType};
use raikiri_style::{CascadeResult, CounterStyleRegistry, PseudoElem};

impl Document {
    /// Place a standalone image marker from the principal border-box origin.
    /// Outside images have a 7px gap from the border box's inline-start edge.
    /// `computed` is the item's style, not the marker's text direction.
    pub fn standalone_marker_image_rect(
        &self,
        computed: &raikiri_style::ComputedValues,
        origin: (f32, f32),
        border_box_width: f32,
        padding_left: f32,
        size: raikiri_traits::ImageRasterSize,
    ) -> raikiri_traits::PaintRect {
        let offset = match computed.list_style_position {
            raikiri_style::ListStylePosition::Inside => padding_left,
            _ if computed.direction == raikiri_style::property::Direction::Rtl => {
                border_box_width + 7.0
            }
            _ => -size.width - 7.0,
        };
        raikiri_traits::PaintRect::new(origin.0 + offset, origin.1, size.width, size.height)
    }

    /// Shape a standalone marker and resolve its horizontal offset from the
    /// principal box. An outside text marker box ends at the principal box's
    /// start edge, including the widest line's trailing blanks. An outside
    /// `disc`, `circle` or `square` marker starts a font-derived gutter before
    /// that edge instead (see [`outside_symbol_gutter`]). Inside text starts at
    /// the supplied padding.
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
            _ => match outside_symbol_gutter(&shaped, computed) {
                Some(gutter) => -gutter,
                None => {
                    let advance = shaped
                        .lines()
                        .iter()
                        .map(|line| line.inline_size() + line.hang_end())
                        .fold(0.0_f32, f32::max);
                    -advance
                }
            },
        };
        Some((shaped, offset))
    }
}

/// The text of a bullet keyword marker, or `None` for any other
/// `list-style-type`. The bullets cannot be redefined by an author style.
fn bullet_marker_text(list_style_type: &ListStyleType) -> Option<&'static str> {
    match list_style_type {
        ListStyleType::Disc => Some("• "),
        ListStyleType::Named(name) => [("disc", "• "), ("circle", "◦ "), ("square", "▪ ")]
            .into_iter()
            .find(|(bullet, _)| name.eq_ignore_ascii_case(bullet))
            .map(|(_, marker)| marker),
        _ => None,
    }
}

/// The distance from the principal box's start edge back to where an outside
/// `disc`, `circle` or `square` marker starts, or `None` for any other marker.
///
/// Chromium does not lay these three out as text. It paints a bullet of
/// `(ascent * 2 / 3 + 1) / 2` px, starting `ascent * 2 / 3 + 7` px before the
/// principal box, where `ascent` is the marker font's ascent rounded to whole
/// pixels and the divisions truncate. A marker whose `content` the author set
/// is text and keeps the advance rule, as does a string `list-style-type`.
/// The bullet is a glyph here, so only its start is placed: a font whose
/// bullet is wider than the gutter, such as Ahem's em-wide box, reaches into
/// the item where Chromium's narrow vector bullet does not.
fn outside_symbol_gutter(
    shaped: &crate::StandaloneText,
    computed: &raikiri_style::ComputedValues,
) -> Option<f32> {
    if bullet_marker_text(&computed.list_style_type).is_none() || !computed.content.is_empty() {
        return None;
    }
    // The ascent of the font the marker is shaped with, not of a fallback font
    // that happens to supply the bullet glyph: the distance from the baseline
    // to the root font's over edge.
    let metrics = shaped.lines().first()?.metrics();
    // Whole and bounded, so a degenerate metric cannot give an infinite offset,
    // and non-negative, so the floor is the truncating division.
    let ascent = (metrics.text_over - metrics.baseline)
        .abs()
        .round()
        .clamp(0.0, 1.0e6);
    Some((ascent * 2.0 / 3.0).floor() + 7.0)
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

fn custom_marker_text(registry: &CounterStyleRegistry, name: &str, value: i32) -> Option<String> {
    let rule = registry.get(name)?;
    // A value the style cannot show is written by the style its `fallback`
    // names, or by decimal at the end of the chain, but the marker keeps the
    // requested style's own prefix and suffix (CSS Counter Styles 3, the
    // `prefix` and `suffix` descriptors).
    let representation = raikiri_traits::page::format_author_counter(registry, name, value)?;
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
    match list_style_type {
        ListStyleType::None => None,
        ListStyleType::String(value) => Some(value.as_str().to_string()),
        ListStyleType::Named(name) => {
            // The bullets cannot be redefined by an author style, so they
            // need no lookup.
            if let Some(marker) = bullet_marker_text(list_style_type) {
                return Some(marker.to_string());
            }
            // An author `@counter-style` comes first, since it can redefine
            // any other predefined name.
            let author_name = author_style_name(&cascade.counter_styles, name.as_str());
            if let Some(marker) = custom_marker_text(&cascade.counter_styles, &author_name, ordinal)
            {
                return Some(marker);
            }
            // Every other predefined style, and `decimal` for a name that is
            // not one. A value the style cannot show is decimal, with the
            // suffix of the style asked for (CSS Counter Styles 3, the
            // `fallback` and `suffix` descriptors).
            Some(format!(
                "{}{}",
                raikiri_traits::page::format_predefined_counter(
                    ordinal,
                    &CounterStyle::Named(name.clone()),
                ),
                raikiri_traits::page::predefined_counter_suffix(name)
            ))
        }
        // `disc`, which is the initial value of `list-style-type`, and so
        // also the marker for a style this match does not know about.
        _ => Some("• ".to_string()),
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
    let mut depth = QuoteDepth::starting_at(counters.open_quotes());
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
            ContentComponent::Quote(keyword) => {
                text.push_str(depth.apply(*keyword, quotes, quotes_auto));
            }
            _ => {}
        }
    }
    Some(text)
}

/// Resolve marker style and content once for either layout or paint.
#[doc(hidden)]
pub fn marker_render_info_with_snapshots<'a>(
    document: &Document,
    cascade: &'a CascadeResult,
    node_id: usize,
    snapshots: &CounterSnapshots,
) -> Option<(&'a raikiri_style::ComputedValues, String)> {
    marker_render_info_impl(document, cascade, node_id, snapshots)
}

/// Resolve marker content from compact counter snapshots.
#[doc(hidden)]
pub fn marker_render_info_with_compact_snapshots<'a>(
    document: &Document,
    cascade: &'a CascadeResult,
    node_id: usize,
    snapshots: &CompactCounterSnapshots,
) -> Option<(&'a raikiri_style::ComputedValues, String)> {
    marker_render_info_impl(document, cascade, node_id, snapshots)
}

pub(super) fn marker_render_info_impl<'a>(
    document: &Document,
    cascade: &'a CascadeResult,
    node_id: usize,
    snapshots: &impl SnapshotCollection,
) -> Option<(&'a raikiri_style::ComputedValues, String)> {
    // cov:ignore: signature close has no executable mapping
    let computed = cascade.computed.get(node_id)?;
    // The owner's display suppresses its marker; the marker's own display
    // does not apply to its box (CSS Lists 3 section 3.1.1).
    if computed.display == DisplayValue::None {
        return None;
    }
    let marker_computed = cascade
        .pseudo
        .get(&(
            raikiri_style::StyleNodeId::new(node_id as u64),
            raikiri_style::PseudoElem::Marker,
        ))
        .map(|style| style.as_ref());
    let style = marker_computed.unwrap_or(computed);
    let mut counters = snapshots
        .node_view(node_id)
        .unwrap_or_else(|| CounterSnapshotView::new(&EMPTY_COUNTER_SNAPSHOT, None));
    let fallback_snapshots = if counters.stack("list-item").is_empty() {
        document.shared_compact_counter_snapshots(cascade).ok()
    } else {
        None
    };
    counters.list_item_fallback = fallback_snapshots
        .as_ref()
        .and_then(|shared| shared.get(node_id))
        .map(|view| view.stack("list-item"));
    counters.computed = marker_computed;
    let open_quotes = fallback_snapshots.as_ref().map_or_else(
        || snapshots.quote_start(node_id, PseudoElem::Marker),
        |shared| shared.quote_depth(node_id, PseudoElem::Marker),
    );
    let counters = counters.starting_quotes_at(open_quotes);
    // Use the shared counter walk when a caller's snapshot omits the list
    // item counter, keeping repeated marker resolution linear for a list.
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
