//! A box is measured in the margin-collapsing mode it is laid out in.
//!
//! Taffy keeps the entries of one mode per box and drops them all when a
//! result of the other mode is stored, so a box that a parent measures in one
//! mode and lays out in the other loses its cache on every probe, and the work
//! on boxes inside boxes grows with every level. The counter that these tests
//! read is per thread and exists in debug builds only; the layout runs on the
//! calling thread.

#![cfg(debug_assertions)]

use crate::Document;
use crate::layout::layout_single_page;
use crate::layout::test_support::{ahem_paragraph_with, page_box_800x600, with_ahem};

/// How many times Taffy dropped the entries of a box on laying out the
/// paragraph that `build` fills in.
fn evictions(build: impl FnOnce(&mut Document, usize)) -> usize {
    let (mut doc, cascade, _) = ahem_paragraph_with("", build);
    let before = taffy::cache_mode_change_evictions();
    layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600()).expect("layout");
    taffy::cache_mode_change_evictions() - before
}

/// `<tag style=css>text` under `parent`.
fn append(doc: &mut Document, parent: usize, tag: &str, css: &str, text: &str) -> usize {
    let element = doc.append_element(Some(parent), tag, Some(css));
    doc.append_text(element, text);
    element
}

#[test]
fn measuring_an_inline_box_does_not_empty_its_cache() {
    // Inline boxes in the other writing modes of the golden spiral in
    // `css/css-inline/dominant-baseline` are atomic inlines.
    const MODES: [&str; 4] = [
        "horizontal-tb",
        "vertical-lr",
        "horizontal-tb;direction:rtl",
        "sideways-rl",
    ];
    assert_eq!(
        evictions(|doc, root| {
            let mut parent = root;
            for mode in MODES.iter().cycle().take(6) {
                parent = append(doc, parent, "span", &format!("writing-mode:{mode}"), "X");
            }
        }),
        0
    );
}

#[test]
fn measuring_a_float_does_not_empty_its_cache() {
    // The inline-block takes its width from its content, floats included.
    assert_eq!(
        evictions(|doc, root| {
            let block = append(doc, root, "div", "display:inline-block", "a");
            let float = append(doc, block, "span", "float:left", "b");
            append(doc, float, "span", "display:inline-block", "c");
        }),
        0
    );
}

#[test]
fn measuring_a_block_does_not_empty_its_cache() {
    // A block that establishes a formatting context, and one that does not, in
    // a horizontal and in a vertical root that take their size from their
    // content.
    for style in ["overflow:hidden", "margin-top:5px"] {
        for root_style in ["", ";writing-mode:vertical-rl"] {
            assert_eq!(
                evictions(|doc, root| {
                    let outer = append(
                        doc,
                        root,
                        "div",
                        &format!("display:inline-block{root_style}"),
                        "x",
                    );
                    let block = append(doc, outer, "div", style, "y");
                    append(doc, block, "span", "display:inline-block", "z");
                }),
                0,
                "{style}{root_style}"
            );
        }
    }
}
