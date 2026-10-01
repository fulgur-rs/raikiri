//! The text of generated content (`::before`, `::after`, markers): the
//! resolution of `content` with counters, quotes and attributes, shared by
//! the inline engine, which lays the text of in-flow pseudo-elements out in
//! their paragraphs, and the painter, which draws the rest as overlays.

use crate::Document;
use crate::target::CounterSnapshot;
use raikiri_style::property::{
    ContentComponent, CounterStyle, DisplayValue, FloatValue, PositionValue, QuoteKeyword,
};
use raikiri_style::{
    CascadeResult, ComputedValues, CounterStyleRegistry, PseudoElem, StyleNodeId,
    resolve_custom_counter,
};

/// Marks a shodo node id that stands for a pseudo-element: the inline engine
/// lays the text of `::before` and `::after` out under ids of their own,
/// outside the range of document node ids.
const GENERATED_ID_BIT: usize = 1 << 62;

/// The node id the inline engine gives the `pseudo` of `element`.
pub fn generated_node_id(element: usize, pseudo: PseudoElem) -> usize {
    GENERATED_ID_BIT | (element << 1) | usize::from(pseudo == PseudoElem::After)
}

/// The element and pseudo-element an id of [`generated_node_id`] stands for;
/// `None` for a document node id.
pub fn generated_origin(id: usize) -> Option<(usize, PseudoElem)> {
    if id & GENERATED_ID_BIT == 0 {
        return None;
    }
    let raw = id & !GENERATED_ID_BIT;
    let pseudo = if raw & 1 == 1 {
        PseudoElem::After
    } else {
        PseudoElem::Before
    };
    Some((raw >> 1, pseudo))
}

/// The computed values of a document node, or of the pseudo-element an id of
/// [`generated_node_id`] stands for.
pub fn computed_for_id(cascade: &CascadeResult, id: usize) -> Option<&ComputedValues> {
    match generated_origin(id) {
        Some((element, pseudo)) => cascade
            .pseudo
            .get(&(StyleNodeId::new(element as u64), pseudo)),
        None => cascade.computed.get(id),
    }
}

/// Whether the `pseudo` of `element` is a rendered, in-flow box whose content
/// can produce text: its `display` is not `none`, it is neither floated nor
/// out of flow, and its `content` is neither `none` nor `normal`. The inline
/// engine lays such content out in the paragraph of its element; the painter
/// draws every other generated content as an overlay.
pub fn is_in_flow_generated_text(
    cascade: &CascadeResult,
    element: usize,
    pseudo: PseudoElem,
) -> bool {
    let Some(cv) = cascade
        .pseudo
        .get(&(StyleNodeId::new(element as u64), pseudo))
    else {
        return false;
    };
    cv.display != DisplayValue::None
        && cv.float == FloatValue::None
        && matches!(cv.position, PositionValue::Static | PositionValue::Relative)
        && !cv.content.is_empty()
        && !cv
            .content
            .iter()
            .any(|part| matches!(part, ContentComponent::None))
        && cv.content.iter().any(|part| {
            matches!(
                part,
                ContentComponent::Literal(_)
                    | ContentComponent::Attr { .. }
                    | ContentComponent::AttrFallback { .. }
                    | ContentComponent::Counter { .. }
                    | ContentComponent::Counters { .. }
                    | ContentComponent::Quote(_)
            )
        })
}

/// The computed values and the text of the `pseudo` of `element`, with the
/// counters in effect at the element (`snapshots`, from
/// [`crate::counter_snapshots`]) updated by the pseudo-element's own
/// directives. Components that are not text (images) contribute nothing.
pub fn generated_text<'a>(
    document: &Document,
    cascade: &'a CascadeResult,
    element: usize,
    pseudo: PseudoElem,
    snapshots: &[CounterSnapshot],
) -> Option<(&'a ComputedValues, String)> {
    let computed = cascade
        .pseudo
        .get(&(StyleNodeId::new(element as u64), pseudo))?;
    let mut counters = snapshots.get(element).cloned().unwrap_or_default();
    apply_counter_directives_to_snapshot(&mut counters, computed);
    let content = content_components_to_text_with_quotes(
        document,
        element,
        &computed.content,
        &computed.quotes,
        computed.quotes_auto,
        &counters,
        &cascade.counter_styles,
    )?;
    Some((computed, content))
}

/// Apply the `counter-reset`, `counter-increment` and `counter-set` of
/// `computed` (a pseudo-element) to a copy of the counters in effect at its
/// element.
pub fn apply_counter_directives_to_snapshot(
    snapshot: &mut CounterSnapshot,
    computed: &raikiri_style::ComputedValues,
) {
    // Apply pseudo directives to the local content snapshot. `::before`
    // scope propagation to descendants is handled by
    // `raikiri_dom::counter_snapshots`; this clone resolves the pseudo's own
    // generated content before the scope is used by later real children.
    let mut reset_values = std::collections::HashMap::new();
    for (name, value) in computed.counter_reset.iter() {
        reset_values.insert(name.as_str(), *value);
    }
    for (name, value) in reset_values {
        snapshot
            .entry(raikiri_traits::Symbol::new(name))
            .or_default()
            .push(value);
    }
    for (name, delta) in computed.counter_increment.iter() {
        let stack = snapshot
            .entry(raikiri_traits::Symbol::new(name.as_str()))
            .or_default();
        if let Some(top) = stack.last_mut() {
            *top = top.saturating_add(*delta);
        } else {
            stack.push(*delta);
        }
    }
    for (name, value) in computed.counter_set.iter() {
        let stack = snapshot
            .entry(raikiri_traits::Symbol::new(name.as_str()))
            .or_default();
        if let Some(top) = stack.last_mut() {
            *top = *value;
        } else {
            stack.push(*value);
        }
    }
}

/// The innermost value of the counter `name` in `style`, `0` when the
/// counter does not exist (`counter()`).
pub fn format_counter_component(
    snapshot: &CounterSnapshot,
    name: &str,
    style: &CounterStyle,
    registry: &CounterStyleRegistry,
) -> String {
    let value = snapshot
        .get(&raikiri_traits::Symbol::new(name))
        .and_then(|values| values.last())
        .copied()
        .unwrap_or(0);
    format_counter(value, style, registry)
}

/// Every value of the counter `name`, outermost first, in `style` and joined
/// by `separator` (`counters()`).
pub fn format_counters_component(
    snapshot: &CounterSnapshot,
    name: &str,
    separator: &str,
    style: &CounterStyle,
    registry: &CounterStyleRegistry,
) -> String {
    snapshot
        .get(&raikiri_traits::Symbol::new(name))
        .map(|values| {
            values
                .iter()
                .map(|value| format_counter(*value, style, registry))
                .collect::<Vec<_>>()
                .join(separator)
        })
        .unwrap_or_default()
}

/// The text of a `content` value: literals, attributes of `node_id`,
/// counters and quotes; other components (images) contribute nothing.
/// `None` for an empty value.
pub fn content_components_to_text_with_quotes<T: AsRef<str>>(
    document: &Document,
    node_id: usize,
    components: &[ContentComponent],
    quotes: &[(T, T)],
    quotes_auto: bool,
    counters: &CounterSnapshot,
    registry: &CounterStyleRegistry,
) -> Option<String> {
    if components.is_empty() {
        return None;
    }
    let mut text = String::new();
    let mut depth = 0_usize;
    for component in components {
        match component {
            ContentComponent::Literal(value) => text.push_str(value.as_str()),
            ContentComponent::Attr { name } => {
                if let Some(node) = document.get_node(node_id) {
                    text.push_str(node.attribute(name.as_str()).unwrap_or_default());
                }
            }
            ContentComponent::AttrFallback { name, fallback } => {
                let value = document
                    .get_node(node_id)
                    .and_then(|node| node.attribute(name.as_str()))
                    .map(str::to_owned)
                    .or_else(|| fallback.as_ref().map(|value| value.to_string()))
                    .unwrap_or_default();
                text.push_str(&value);
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
                        text.push_str(match depth {
                            0 => "“",
                            _ => "‘",
                        });
                    }
                    depth = depth.saturating_add(1);
                }
                QuoteKeyword::CloseQuote => {
                    depth = depth.saturating_sub(1);
                    if let Some((_, close)) = quotes.get(depth) {
                        text.push_str(close.as_ref());
                    } else if quotes_auto && quotes.is_empty() {
                        text.push_str(match depth {
                            0 => "”",
                            _ => "’",
                        });
                    }
                }
                QuoteKeyword::NoOpenQuote => depth = depth.saturating_add(1),
                QuoteKeyword::NoCloseQuote => depth = depth.saturating_sub(1),
                _ => {}
            },
            _ => {}
        }
    }
    Some(text)
}

/// `value` in the counter style `style`: the roman styles, a custom
/// `@counter-style` of `registry`, else decimal.
pub fn format_counter(value: i32, style: &CounterStyle, registry: &CounterStyleRegistry) -> String {
    match style {
        CounterStyle::Named(name) if name.as_str().eq_ignore_ascii_case("lower-roman") => {
            if value <= 0 {
                return value.to_string();
            }
            let mut n = value;
            let mut result = String::new();
            for (unit, glyph) in [
                (1000, "m"),
                (900, "cm"),
                (500, "d"),
                (400, "cd"),
                (100, "c"),
                (90, "xc"),
                (50, "l"),
                (40, "xl"),
                (10, "x"),
                (9, "ix"),
                (5, "v"),
                (4, "iv"),
                (1, "i"),
            ] {
                while n >= unit {
                    result.push_str(glyph);
                    n -= unit;
                }
            }
            result
        }
        CounterStyle::Named(name) if name.as_str().eq_ignore_ascii_case("upper-roman") => {
            format_counter(value, &CounterStyle::Named("lower-roman".into()), registry)
                .to_uppercase()
        }
        CounterStyle::Named(name) => resolve_custom_counter(registry, name.as_str(), value)
            .unwrap_or_else(|| value.to_string()),
        _ => value.to_string(),
    }
}
