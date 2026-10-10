//! Canonical text of cascade results.
//!
//! Every value is printed through [`crate::canon::canonicalize`], and struct
//! values are split one field per line, so a textual diff of two dumps names
//! the node and the field that changed. Outputs that are only reachable
//! through methods — SVG paint properties, first-letter styles, contextual
//! highlight backgrounds, inherited custom properties, and the `@page` cascade
//! for other page queries — are queried with fixed arguments, the way layout
//! and paint query them. Each group of queries runs as its own section, so a
//! panic in one is recorded on a line of its own and the rest of the case is
//! still compared. The output left out is run-specific identity (the cascade
//! generation counter), which differs between any two runs, and the
//! custom-property environments, which print how bindings are shared between
//! nodes rather than what they are; the custom properties a case names are
//! printed by value instead.

use std::collections::BTreeSet;
use std::fmt::{Debug, Write as _};
use std::panic::{AssertUnwindSafe, catch_unwind};

use raikiri_style::{
    Atom, CascadeResult, ComputedValues, CssColor, FirstLineStyles, MediaContext,
    PageCascadeResult, PageContextQuery, PageInheritance, PageMarginBoxSlot, PseudoElem, RuleTree,
    StyleDom, StyleNode, StyleNodeId, StyleNodeKind, cascade_page_with_media_context,
};

use crate::canon::{canonicalize, struct_fields};

/// What a cascade result was computed from, for the outputs queried through
/// methods.
pub(crate) struct Inputs<'a, D> {
    pub(crate) dom: &'a D,
    pub(crate) tree: &'a RuleTree,
    pub(crate) media: &'a MediaContext,
    /// Custom property names whose effective value is printed for every node.
    pub(crate) custom_names: &'a [&'a str],
}

/// The struct whose literals print how custom-property bindings are shared
/// between nodes rather than what they are. Fields holding one are left out
/// of every value (see [`push_custom_properties`]).
const CUSTOM_PROPERTY_ENVIRONMENT: &str = "CustomPropertyEnvironment";

/// Appends `value` as `label: Name` followed by one indented line per field,
/// or as a single `label: value` line when it is not a struct.
fn push_value(out: &mut String, label: &str, value: &dyn Debug) {
    let canon = canonicalize(&format!("{value:?}"), Some(CUSTOM_PROPERTY_ENVIRONMENT));
    match struct_fields(&canon) {
        Some((name, fields)) => {
            let _ = writeln!(out, "{label}: {name}");
            for field in fields {
                let _ = writeln!(out, "  {field}");
            }
        }
        None => {
            let _ = writeln!(out, "{label}: {canon}");
        }
    }
}

/// Appends computed values that are not a node's own, such as a
/// first-letter style: [`push_value`], and then the custom properties
/// `names` resolve on them, labeled `label custom`.
fn push_derived(out: &mut String, label: &str, computed: &ComputedValues, names: &[&str]) {
    push_value(out, label, computed);
    push_custom_properties(out, &format!("{label} custom"), computed, names);
}

/// Appends a page result, and then each margin box one of its rules fills
/// as layout resolves it, with the page's custom properties substituted, so
/// those are compared by value.
fn push_page(out: &mut String, label: &str, page: &PageCascadeResult) {
    push_value(out, label, page);
    let mut slots: Vec<PageMarginBoxSlot> =
        page.margin_boxes().iter().map(|rule| rule.slot).collect();
    slots.sort_by_key(|slot| format!("{slot:?}"));
    slots.dedup();
    for slot in slots {
        push_value(
            out,
            &format!("{label} margin[{slot:?}]"),
            &page.cascade_margin_box(slot),
        );
    }
}

fn node_id(index: usize) -> StyleNodeId {
    StyleNodeId::new(index as u64)
}

/// Appends the canonical text of every observable output of `result`.
pub(crate) fn cascade_result<D: StyleDom>(
    out: &mut String,
    inputs: &Inputs<'_, D>,
    result: &CascadeResult,
) {
    let root = result.root_element_computed();
    let root_index = result
        .computed
        .iter()
        .position(|computed| std::ptr::eq(computed, root));
    let _ = writeln!(out, "root_element_index: {root_index:?}");
    for (index, computed) in result.computed.iter().enumerate() {
        push_value(out, &format!("computed[{index}]"), computed);
    }
    let mut pseudo: Vec<_> = result.pseudo.iter().collect();
    pseudo.sort_by_key(|((id, kind), _)| (id.0, format!("{kind:?}")));
    for ((id, kind), computed) in &pseudo {
        push_value(out, &format!("pseudo[{}, {kind:?}]", id.0), computed);
    }
    push_value(out, "opacity_specified", &result.opacity_specified);
    push_value(
        out,
        "background_color_specified",
        &result.background_color_specified,
    );
    push_value(
        out,
        "authored_writing_modes",
        &result.authored_writing_modes,
    );
    push_value(out, "page_values", &result.page_values);
    push_page(out, "page", &result.page);
    push_value(out, "counter_styles", &result.counter_styles);
    push_value(
        out,
        "custom_highlight_styles",
        &result.custom_highlight_styles,
    );
    section(out, "custom properties", |out| {
        for (index, computed) in result.computed.iter().enumerate() {
            let label = format!("custom[{index}]");
            push_custom_properties(out, &label, computed, inputs.custom_names);
        }
        for ((id, kind), computed) in &pseudo {
            let label = format!("custom[{}, {kind:?}]", id.0);
            push_custom_properties(out, &label, computed, inputs.custom_names);
        }
    });
    section(out, "svg properties", |out| {
        for index in 0..result.computed.len() {
            let properties = result.svg_style_properties(node_id(index));
            if !properties.is_empty() {
                push_value(out, &format!("svg[{index}]"), &properties);
            }
        }
    });
    section(out, "first letters", |out| {
        first_letters(out, inputs, result)
    });
    section(out, "highlights", |out| highlight_backgrounds(out, result));
    section(out, "page queries", |out| page_queries(out, inputs, result));
}

/// Runs one section of a dump. A panic in it is recorded as a `PANIC in`
/// line after whatever the section printed, so a panic both builds share in
/// one query does not hide the comparison of the rest of the case.
pub(crate) fn section(out: &mut String, name: &str, write: impl FnOnce(&mut String)) {
    let mut text = String::new();
    let outcome = catch_unwind(AssertUnwindSafe(|| write(&mut text)));
    out.push_str(&text);
    if let Err(payload) = outcome {
        let _ = writeln!(out, "PANIC in {name}: {}", crate::panic_message(&*payload));
    }
}

/// Appends the effective value of every name in `names` that `computed`
/// resolves, walking inherited bindings the way `var()` does, and then the
/// values declared on the node itself.
fn push_custom_properties(
    out: &mut String,
    label: &str,
    computed: &ComputedValues,
    names: &[&str],
) {
    let effective = names.iter().filter_map(|name| {
        let value = computed.resolved_custom_property(name)?;
        Some(format!("{name}={value:?}"))
    });
    let local = names.iter().filter_map(|name| {
        let value = computed.local_resolved_custom_property(name)?;
        Some(format!("{name}={value:?}"))
    });
    for (suffix, values) in [
        ("", effective.collect::<Vec<_>>()),
        (" local", local.collect()),
    ] {
        if !values.is_empty() {
            let _ = writeln!(out, "{label}{suffix}: {}", values.join(", "));
        }
    }
}

/// First-letter styles of every originating element: over its own computed
/// values, and — for the element and its first element descendants, the
/// usual parents of the letter's text — through the `::first-line`
/// pseudo-elements that enclose it, as layout resolves them once it knows the
/// letter's actual parent.
fn first_letters<D: StyleDom>(out: &mut String, inputs: &Inputs<'_, D>, result: &CascadeResult) {
    let (dom, names) = (inputs.dom, inputs.custom_names);
    let _ = writeln!(
        out,
        "has_first_letter_styles: {}",
        result.has_first_letter_styles()
    );
    for (index, own) in result.computed.iter().enumerate() {
        let origin = node_id(index);
        let Some(style) = result.resolve_first_letter_style(origin, own) else {
            continue;
        };
        push_derived(out, &format!("first_letter[{index}]"), &style, names);
        for parent in first_descendants(dom, origin) {
            let lines = enclosing_first_lines(dom, result, parent);
            if lines.is_empty() {
                continue;
            }
            // Layout passes the generated pseudo-element whose text holds the
            // letter, when it is one.
            for generated in [
                None,
                Some(PseudoElem::Before),
                Some(PseudoElem::After),
                Some(PseudoElem::Marker),
            ] {
                if generated.is_some_and(|kind| !result.pseudo.contains_key(&(parent, kind))) {
                    continue;
                }
                let Some(inherited) = result
                    .first_letter_parent_with_first_lines(dom, origin, &lines, parent, generated)
                else {
                    continue; // cov:ignore: unreachable — `origin` has first-letter inputs, `lines` is non-empty with a `::first-line` style first, and `parent` descends from it
                };
                let label = format!("first_letter[{index}] at [{}, {generated:?}]", parent.0);
                push_derived(out, &format!("{label} parent"), &inherited, names);
                if let Some(style) = result.resolve_first_letter_style(origin, &inherited) {
                    push_derived(out, &label, &style, names);
                }
            }
        }
    }
}

/// `origin` and up to two levels of first in-document element children below
/// it.
fn first_descendants<D: StyleDom>(dom: &D, origin: StyleNodeId) -> Vec<StyleNodeId> {
    let mut chain = vec![origin];
    while chain.len() < 3 {
        let last = chain[chain.len() - 1];
        let child = dom.child_ids(last).find(|&id| {
            dom.node(id)
                .is_some_and(|node| node.kind() == StyleNodeKind::Element && node.is_in_document())
        });
        match child {
            Some(child) => chain.push(child),
            None => break,
        }
    }
    chain
}

/// The ancestors-or-self of `node` that have a `::first-line` style,
/// outermost first.
fn enclosing_first_lines<D: StyleDom>(
    dom: &D,
    result: &CascadeResult,
    node: StyleNodeId,
) -> Vec<StyleNodeId> {
    let mut lines = Vec::new();
    let mut current = Some(node);
    while let Some(id) = current {
        if result.pseudo.contains_key(&(id, PseudoElem::FirstLine)) {
            lines.push(id);
        }
        current = dom.parent_id(id);
    }
    lines.reverse();
    lines
}

/// Highlight backgrounds resolved against two foregrounds, which is where a
/// contextual color differs from its inspection value.
fn highlight_backgrounds(out: &mut String, result: &CascadeResult) {
    const FOREGROUNDS: [CssColor; 2] = [
        CssColor::BLACK,
        CssColor {
            r: 10,
            g: 20,
            b: 30,
            a: 128,
        },
    ];
    let names: BTreeSet<&String> = result.custom_highlight_styles.keys().collect();
    for name in names {
        for foreground in FOREGROUNDS {
            let background = result.custom_highlight_background(name, foreground);
            let _ = writeln!(out, "highlight[{name:?}, {foreground:?}]: {background:?}");
        }
    }
}

fn page_query(set: impl FnOnce(&mut PageContextQuery)) -> PageContextQuery {
    let mut query = PageContextQuery::default();
    set(&mut query);
    query
}

/// The `@page` cascade for the page queries the result's own (default) query
/// does not cover.
fn page_queries<D>(out: &mut String, inputs: &Inputs<'_, D>, result: &CascadeResult) {
    let queries = [
        (
            "first",
            page_query(|query| {
                query.is_first = true;
                query.is_right = true;
            }),
        ),
        ("left", page_query(|query| query.is_left = true)),
        (
            "named",
            page_query(|query| {
                query.page_name = Some(Atom::from("named"));
                query.is_right = true;
            }),
        ),
        (
            "named first",
            page_query(|query| {
                query.page_name = Some(Atom::from("named"));
                query.is_first = true;
                query.is_right = true;
            }),
        ),
        (
            "blank",
            page_query(|query| {
                query.is_blank = true;
                query.is_left = true;
            }),
        ),
    ];
    for (label, query) in queries {
        let page = cascade_page_with_media_context(
            inputs.tree,
            &query,
            PageInheritance::FromRoot(result.root_element_computed()),
            inputs.media,
        );
        push_page(out, &format!("page[{label}]"), &page);
    }
}

/// Appends the canonical text of the styles the `::first-line` entry point
/// returned for its block, with the custom properties `names` resolve on
/// them.
pub(crate) fn first_line_styles(
    out: &mut String,
    styles: Option<&FirstLineStyles>,
    names: &[&str],
) {
    match styles {
        None => {
            let _ = writeln!(out, "first_line: None");
        }
        Some(styles) => {
            let _ = writeln!(out, "first_line.root: {}", styles.root.0);
            for (index, computed) in styles.computed.iter().enumerate() {
                if let Some(computed) = computed {
                    push_derived(out, &format!("first_line[{index}]"), computed, names);
                }
            }
        }
    }
}

/// Every name in `sources` that could be a custom property, sorted: each
/// run of name code points and escapes that decodes to `--` and more. CSS
/// tokenizes an identifier this way wherever it is nested, so an escaped
/// name is found by its value at any depth. A name no node defines prints
/// nothing.
pub(crate) fn custom_property_names<'a>(sources: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    let is_name = |c: char| c.is_ascii_alphanumeric() || c == '-' || c == '_' || !c.is_ascii();
    let mut names = BTreeSet::new();
    for source in sources {
        // CSS drops a leading byte order mark and reads a NUL in its input as
        // U+FFFD before it tokenizes, escaped or not.
        let chars: Vec<char> = source
            .strip_prefix('\u{feff}')
            .unwrap_or(source)
            .chars()
            .map(|c| {
                if c == '\0' {
                    char::REPLACEMENT_CHARACTER
                } else {
                    c
                }
            })
            .collect();
        let mut position = 0;
        while position < chars.len() {
            let mut name = String::new();
            while let Some(&c) = chars.get(position) {
                if is_name(c) {
                    name.push(c);
                    position += 1;
                } else if c == '\\'
                    && chars
                        .get(position + 1)
                        .is_some_and(|&next| !is_newline(next))
                {
                    position += 1;
                    name.push(escaped_code_point(&chars, &mut position));
                } else {
                    break;
                }
            }
            if name.is_empty() {
                position += 1;
            } else if name.len() > 2 && name.starts_with("--") {
                // `--` alone is reserved, not a custom property name.
                names.insert(name);
            }
        }
    }
    names.into_iter().collect()
}

fn is_newline(c: char) -> bool {
    matches!(c, '\n' | '\r' | '\x0c')
}

/// Decodes the escape whose backslash precedes `chars[*position]`, as CSS
/// Syntax 3 consumes an escaped code point: up to six hex digits and one
/// whitespace after them, or else the code point itself.
fn escaped_code_point(chars: &[char], position: &mut usize) -> char {
    let start = *position;
    let mut value = 0;
    while let Some(digit) = chars.get(*position).and_then(|c| c.to_digit(16)) {
        if *position - start == 6 {
            break;
        }
        value = value * 16 + digit;
        *position += 1;
    }
    if *position == start {
        *position += 1;
        return chars[start];
    }
    match chars.get(*position) {
        Some('\r') => {
            *position += 1;
            if chars.get(*position) == Some(&'\n') {
                *position += 1;
            }
        }
        Some(' ' | '\t' | '\n' | '\x0c') => *position += 1,
        _ => {}
    }
    char::from_u32(value)
        .filter(|&c| c != '\0')
        .unwrap_or(char::REPLACEMENT_CHARACTER)
}

/// 64-bit FNV-1a, a stable hash for comparing dumps across processes.
pub(crate) fn fnv1a(text: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in text.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    hash
}

#[cfg(test)]
mod tests;
