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
//! still compared. The only output left out is run-specific identity (the
//! cascade generation counter), which differs between any two runs.

use std::collections::BTreeSet;
use std::fmt::{Debug, Write as _};
use std::panic::{AssertUnwindSafe, catch_unwind};

use raikiri_style::{
    Atom, CascadeResult, ComputedValues, CssColor, FirstLineStyles, MediaContext, PageContextQuery,
    PageInheritance, PseudoElem, RuleTree, StyleDom, StyleNode, StyleNodeId, StyleNodeKind,
    cascade_page_with_media_context,
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

/// Appends `value` as `label: Name` followed by one indented line per field,
/// or as a single `label: value` line when it is not a struct.
fn push_value(out: &mut String, label: &str, value: &dyn Debug) {
    let canon = canonicalize(&format!("{value:?}"));
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
    push_value(out, "page", &result.page);
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
        first_letters(out, inputs.dom, result)
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
/// resolves, walking inherited bindings the way `var()` does.
fn push_custom_properties(
    out: &mut String,
    label: &str,
    computed: &ComputedValues,
    names: &[&str],
) {
    let values: Vec<String> = names
        .iter()
        .filter_map(|name| {
            let value = computed.resolved_custom_property(name)?;
            Some(format!("{name}={value:?}"))
        })
        .collect();
    if !values.is_empty() {
        let _ = writeln!(out, "{label}: {}", values.join(", "));
    }
}

/// First-letter styles of every originating element: over its own computed
/// values, and — for the element and its first element descendants, the
/// usual parents of the letter's text — through the `::first-line`
/// pseudo-elements that enclose it, as layout resolves them once it knows the
/// letter's actual parent.
fn first_letters<D: StyleDom>(out: &mut String, dom: &D, result: &CascadeResult) {
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
        push_value(out, &format!("first_letter[{index}]"), &style);
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
                push_value(out, &format!("{label} parent"), &inherited);
                if let Some(style) = result.resolve_first_letter_style(origin, &inherited) {
                    push_value(out, &label, &style);
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
        push_value(out, &format!("page[{label}]"), &page);
    }
}

/// Appends the canonical text of the styles the `::first-line` entry point
/// returned for its block.
pub(crate) fn first_line_styles(out: &mut String, styles: Option<&FirstLineStyles>) {
    match styles {
        None => {
            let _ = writeln!(out, "first_line: None");
        }
        Some(styles) => {
            let _ = writeln!(out, "first_line.root: {}", styles.root.0);
            for (index, computed) in styles.computed.iter().enumerate() {
                if let Some(computed) = computed {
                    push_value(out, &format!("first_line[{index}]"), computed);
                }
            }
        }
    }
}

/// Every name in `source` that could be a custom property: `--` followed by
/// name code points, sorted. A name no node defines prints nothing.
pub(crate) fn custom_property_names(source: &str) -> Vec<&str> {
    let is_name = |c: char| c.is_ascii_alphanumeric() || c == '-' || c == '_' || !c.is_ascii();
    let mut names = BTreeSet::new();
    let mut position = 0;
    while let Some(offset) = source[position..].find("--") {
        let start = position + offset;
        let tail = &source[start + 2..];
        let length = tail.find(|c: char| !is_name(c)).unwrap_or(tail.len());
        if length > 0 {
            names.insert(&source[start..start + 2 + length]);
        }
        position = start + 2 + length;
    }
    names.into_iter().collect()
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
