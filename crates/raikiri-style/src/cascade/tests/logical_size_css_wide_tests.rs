//! CSS-wide keywords on `inline-size`, `block-size` and `min-block-size`
//! (CSS Cascading 4 §7.3 "Explicit Defaulting"; the sizes map to the physical
//! ones by CSS Logical Properties 1 §4.1).

use super::*;
use crate::resolve::ComputedLengthPercentageOrAuto as Size;

/// A parent whose physical sizes are all away from the initial `auto`.
const PARENT: &str = "width: 100px; height: 40px; min-width: 30px; min-height: 20px";

/// The computed values of a parent styled `parent` and of its child `p`
/// (`id="target"`) under `sheets`, followed by `css` as an author sheet.
fn pair(parent: &str, sheets: &[(&str, Origin)], css: &str) -> (ComputedValues, ComputedValues) {
    let mut doc = TestDoc::new();
    let parent = doc.push_element(0, "div", Some(parent));
    let child = doc.push_element_with_attrs(parent, "p", None, &[("id", "target")]);
    let mut tree = RuleTree::empty();
    for &(sheet, origin) in sheets {
        tree.add_stylesheet(sheet, origin);
    }
    tree.add_stylesheet(css, Origin::Author);
    let result = cascade(&doc, &tree).unwrap();
    (
        result.computed[parent].clone(),
        result.computed[child].clone(),
    )
}

/// The child of [`PARENT`] under `css`.
fn child(css: &str) -> ComputedValues {
    pair(PARENT, &[], css).1
}

fn px(value: f32) -> Size {
    Size::Px(value)
}

/// `(width, height, min-height)`, the sizes a horizontal box maps its
/// `inline-size`, `block-size` and `min-block-size` to.
fn horizontal(values: &ComputedValues) -> (Size, Size, Size) {
    (values.width, values.height, values.min_height)
}

#[test]
fn inherit_takes_the_parents_size_on_the_mapped_axis() {
    let values =
        child("#target { inline-size: inherit; block-size: inherit; min-block-size: inherit }");
    assert_eq!(horizontal(&values), (px(100.0), px(40.0), px(20.0)));
    // `min-inline-size` has no longhand here, and `min-width` is not touched.
    assert_eq!(values.min_width, Size::Auto);
}

#[test]
fn inherit_keeps_the_parents_percentage_and_keyword() {
    let (_, values) = pair(
        "width: 50%; min-height: 25%",
        &[],
        "#target { inline-size: inherit; min-block-size: inherit }",
    );
    assert_eq!(values.width, Size::Percent(50.0));
    assert_eq!(values.min_height, Size::Percent(25.0));
    let (_, values) = pair(
        "width: min-content",
        &[],
        "#target { inline-size: inherit }",
    );
    assert_eq!(values.width, Size::MinContent);
}

#[test]
fn inherit_follows_the_childs_own_writing_mode() {
    // `vertical-rl`: the inline axis is the physical height, the block axis the
    // physical width, and the minimum block size is the minimum width.
    for css in [
        "#target { writing-mode: vertical-rl; inline-size: inherit; block-size: inherit; \
         min-block-size: inherit }",
        // The writing mode may be declared after the sizes.
        "#target { inline-size: inherit; block-size: inherit; min-block-size: inherit; \
         writing-mode: vertical-rl }",
    ] {
        let values = child(css);
        let size = values.vertical_logical_size.expect("a vertical size");
        assert_eq!((size.width, size.height), (px(100.0), px(40.0)), "{css}");
        assert_eq!(values.min_width, px(30.0), "{css}");
        assert_eq!(values.min_height, Size::Auto, "{css}");
    }
}

#[test]
fn inherit_reads_the_parents_physical_sizes_of_a_vertical_parent() {
    // The vertical parent's inline size is its physical height and its block
    // size its physical width; a horizontal child takes those by axis.
    let (_, values) = pair(
        "writing-mode: vertical-rl; inline-size: 30px; block-size: 50px",
        &[],
        "#target { writing-mode: horizontal-tb; inline-size: inherit; block-size: inherit }",
    );
    assert_eq!((values.width, values.height), (px(50.0), px(30.0)));
}

#[test]
fn initial_and_unset_give_auto() {
    // The sizes are not inherited, so `unset` is `initial`.
    for keyword in ["initial", "unset"] {
        let values = child(&format!(
            "#target {{ width: 50px; height: 60px; min-height: 5px; \
             inline-size: {keyword}; block-size: {keyword}; min-block-size: {keyword} }}"
        ));
        assert_eq!(
            horizontal(&values),
            (Size::Auto, Size::Auto, Size::Auto),
            "{keyword}"
        );
    }
}

#[test]
fn a_keyword_competes_with_the_physical_size_by_cascade_order() {
    assert_eq!(
        child("#target { inline-size: inherit; width: 50px }").width,
        px(50.0)
    );
    assert_eq!(
        child("#target { width: 50px; inline-size: inherit }").width,
        px(100.0)
    );
    assert_eq!(
        child("#target { width: 50px; inline-size: initial }").width,
        Size::Auto
    );
    // Specificity and importance decide before the order of appearance.
    assert_eq!(
        child("p { inline-size: inherit } #target { width: 50px }").width,
        px(50.0)
    );
    assert_eq!(
        child("#target { inline-size: inherit } p { width: 50px }").width,
        px(100.0)
    );
    assert_eq!(
        child("p { inline-size: inherit !important } #target { width: 50px }").width,
        px(100.0)
    );
    assert_eq!(
        child("#target { block-size: inherit; height: 10px }").height,
        px(10.0)
    );
    assert_eq!(
        child("#target { min-height: 10px; min-block-size: inherit }").min_height,
        px(20.0)
    );
}

#[test]
fn revert_rolls_back_the_origin() {
    // The user agent's declarations are what survive.
    let (_, values) = pair(
        PARENT,
        &[(
            "p { inline-size: 70px; block-size: 60px; min-block-size: 8px }",
            Origin::UserAgent,
        )],
        "#target { inline-size: revert; block-size: revert; min-block-size: revert }",
    );
    assert_eq!(horizontal(&values), (px(70.0), px(60.0), px(8.0)));
    // An earlier author declaration is rolled back with the rest of the origin.
    let (_, values) = pair(
        PARENT,
        &[("p { inline-size: 70px }", Origin::UserAgent)],
        "#target { inline-size: 5px; inline-size: revert }",
    );
    assert_eq!(values.width, px(70.0));
    // Nothing underneath: the sizes are not inherited, so `unset` applies.
    let values =
        child("#target { inline-size: revert; block-size: revert; min-block-size: revert }");
    assert_eq!(horizontal(&values), (Size::Auto, Size::Auto, Size::Auto));
}

#[test]
fn revert_layer_rolls_back_the_layer() {
    let css = "@layer a, b; \
               @layer a { p { inline-size: 70px; min-block-size: 8px } } \
               @layer b { p { inline-size: 80px; min-block-size: 9px } \
               #target { inline-size: revert-layer; min-block-size: revert-layer } }";
    let values = child(css);
    assert_eq!((values.width, values.min_height), (px(70.0), px(8.0)));
}

#[test]
fn substituted_keywords_apply() {
    let values = child(
        "#target { --kw: inherit; inline-size: var(--kw); \
         block-size: var(--missing, initial); min-block-size: var(--kw) }",
    );
    assert_eq!(horizontal(&values), (px(100.0), Size::Auto, px(20.0)));
}
