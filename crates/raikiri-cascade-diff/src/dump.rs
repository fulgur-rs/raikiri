//! Canonical text of cascade results.
//!
//! Every value is printed through [`crate::canon::canonicalize`], and struct
//! values are split one field per line, so a textual diff of two dumps names
//! the node and the field that changed. Run-specific identity (the cascade
//! generation counter) is left out, because it differs between any two runs.

use std::fmt::{Debug, Write as _};

use raikiri_style::{CascadeResult, FirstLineCascade};

use crate::canon::{canonicalize, struct_fields};

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

/// Appends the canonical text of every observable output of `result`.
pub(crate) fn cascade_result(out: &mut String, result: &CascadeResult) {
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
    for ((id, kind), computed) in pseudo {
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
}

/// Appends the canonical text of a `::first-line` cascade.
pub(crate) fn first_line(out: &mut String, cascade: &FirstLineCascade) {
    cascade_result(out, &cascade.normal);
    match &cascade.first_line {
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

/// 64-bit FNV-1a, a stable hash for comparing dumps across processes.
pub(crate) fn fnv1a(text: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in text.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    hash
}
