//! CSS-wide keywords on `writing-mode`, `text-orientation` and `direction`
//! (CSS Cascading 4 §7.3 "Explicit Defaulting").

use super::*;
use crate::property::{Direction, TextOrientation, WritingMode};
use crate::resolve::ComputedLengthPercentageOrAuto;

type Flow = (WritingMode, TextOrientation, Direction);

/// A parent whose flow is away from the initial values.
const PARENT: &str = "writing-mode: vertical-rl; text-orientation: upright; direction: rtl";

const INHERITED: Flow = (
    WritingMode::VerticalRl,
    TextOrientation::Upright,
    Direction::Rtl,
);

const INITIAL: Flow = (
    WritingMode::HorizontalTb,
    TextOrientation::Mixed,
    Direction::Ltr,
);

/// The cascade of a parent styled `parent` and of its child `p`
/// (`id="target"`) under `sheets`, followed by `css` as an author sheet, and
/// the child's index.
fn cascade_child(parent: &str, sheets: &[(&str, Origin)], css: &str) -> (CascadeResult, usize) {
    let mut doc = TestDoc::new();
    let parent = doc.push_element(0, "div", Some(parent));
    let child = doc.push_element_with_attrs(parent, "p", None, &[("id", "target")]);
    let mut tree = RuleTree::empty();
    for &(sheet, origin) in sheets {
        tree.add_stylesheet(sheet, origin);
    }
    tree.add_stylesheet(css, Origin::Author);
    (cascade(&doc, &tree).unwrap(), child)
}

fn flow(values: &ComputedValues) -> Flow {
    (
        values.cssom_writing_mode,
        values.text_orientation,
        values.direction,
    )
}

/// The flow of the child of [`PARENT`] under `css`.
fn child(css: &str) -> Flow {
    let (result, child) = cascade_child(PARENT, &[], css);
    flow(&result.computed[child])
}

/// The physical margins of the child of [`PARENT`] under `css`, as
/// `[top, right, bottom, left]`.
fn margins(css: &str) -> [f32; 4] {
    let (result, child) = cascade_child(PARENT, &[], css);
    let margin = result.computed[child].margin;
    [margin.top, margin.right, margin.bottom, margin.left].map(|side| match side {
        ComputedLengthPercentageOrAuto::Px(px) => px,
        other => panic!("expected a length, got {other:?}"),
    })
}

#[test]
fn initial_gives_the_initial_values() {
    assert_eq!(
        child("#target { writing-mode: initial; text-orientation: initial; direction: initial }"),
        INITIAL
    );
    assert_eq!(
        child(
            "#target { writing-mode: vertical-lr; text-orientation: sideways; direction: ltr; \
             writing-mode: initial; text-orientation: initial; direction: initial }"
        ),
        INITIAL
    );
}

#[test]
fn inherit_and_unset_take_the_parents_values() {
    // The three properties are inherited, so `unset` is `inherit`.
    for keyword in ["inherit", "unset"] {
        assert_eq!(
            child(&format!(
                "p {{ writing-mode: vertical-lr; text-orientation: sideways; direction: ltr }} \
                 #target {{ writing-mode: {keyword}; text-orientation: {keyword}; \
                 direction: {keyword} }}"
            )),
            INHERITED,
            "{keyword}"
        );
    }
}

#[test]
fn a_keyword_overrides_by_specificity_importance_and_order() {
    assert_eq!(
        child("#target { writing-mode: vertical-lr; writing-mode: initial }").0,
        WritingMode::HorizontalTb
    );
    assert_eq!(
        child("#target { writing-mode: initial; writing-mode: vertical-lr }").0,
        WritingMode::VerticalLr
    );
    assert_eq!(
        child("p { direction: initial } #target { direction: ltr }").2,
        Direction::Ltr
    );
    assert_eq!(
        child("p { text-orientation: initial !important } #target { text-orientation: sideways }")
            .1,
        TextOrientation::Mixed
    );
}

#[test]
fn revert_rolls_back_the_origin() {
    // The user agent's declarations are what survive.
    let (result, node) = cascade_child(
        PARENT,
        &[(
            "p { writing-mode: vertical-lr; text-orientation: sideways; direction: ltr }",
            Origin::UserAgent,
        )],
        "#target { writing-mode: revert; text-orientation: revert; direction: revert }",
    );
    assert_eq!(
        flow(&result.computed[node]),
        (
            WritingMode::VerticalLr,
            TextOrientation::Sideways,
            Direction::Ltr
        )
    );
    // Nothing underneath: the properties are inherited, so `unset` applies.
    assert_eq!(
        child("#target { writing-mode: revert; text-orientation: revert; direction: revert }"),
        INHERITED
    );
}

#[test]
fn revert_layer_rolls_back_the_layer() {
    let css = "@layer a, b; \
               @layer a { p { writing-mode: vertical-lr; text-orientation: sideways; \
               direction: ltr } } \
               @layer b { p { writing-mode: horizontal-tb; text-orientation: mixed } \
               #target { writing-mode: revert-layer; text-orientation: revert-layer; \
               direction: revert-layer } }";
    assert_eq!(
        child(css),
        (
            WritingMode::VerticalLr,
            TextOrientation::Sideways,
            Direction::Ltr
        )
    );
}

#[test]
fn substituted_keywords_apply() {
    assert_eq!(
        child(
            "#target { --kw: initial; writing-mode: var(--kw); \
             text-orientation: var(--missing, initial); direction: var(--kw) }"
        ),
        INITIAL
    );
    assert_eq!(
        child("#target { direction: ltr; direction: var(--missing, inherit) }").2,
        Direction::Rtl
    );
}

#[test]
fn an_authored_writing_mode_records_the_keyword_it_computes_to() {
    let authored = |css: &str| {
        let (result, child) = cascade_child(PARENT, &[], css);
        result.authored_writing_modes[child]
    };
    assert_eq!(authored(""), None);
    assert_eq!(
        authored("#target { writing-mode: initial }"),
        Some(WritingMode::HorizontalTb)
    );
    assert_eq!(
        authored("#target { writing-mode: inherit }"),
        Some(WritingMode::VerticalRl)
    );
}

#[test]
fn flow_relative_margins_follow_the_keywords() {
    let declared = "margin-block-start: 1px; margin-inline-start: 3px";
    // `horizontal-tb` and `ltr`: the block start is the top, the inline start
    // the left.
    assert_eq!(
        margins(&format!(
            "#target {{ writing-mode: initial; direction: initial; {declared} }}"
        )),
        [1.0, 0.0, 0.0, 3.0]
    );
    // `vertical-rl` with upright text, which is `ltr`: the block start is the
    // right, the inline start the top.
    assert_eq!(
        margins(&format!(
            "#target {{ writing-mode: inherit; text-orientation: inherit; \
             direction: inherit; {declared} }}"
        )),
        [3.0, 1.0, 0.0, 0.0]
    );
    // Mixed text keeps `rtl`: the inline start is the bottom.
    assert_eq!(
        margins(&format!(
            "#target {{ writing-mode: unset; text-orientation: initial; \
             direction: unset; {declared} }}"
        )),
        [0.0, 1.0, 3.0, 0.0]
    );
}

/// The keyword of a flow property that does not resolve is the inherited one.
#[test]
fn an_invalid_substitution_leaves_the_inherited_flow() {
    assert_eq!(
        child(
            "#target { writing-mode: var(--missing); text-orientation: var(--missing); \
             direction: var(--missing) }"
        ),
        INHERITED
    );
}

/// `all: revert-layer` leaves `direction` alone (CSS Cascade 4 §3.1), but the
/// property's own `revert-layer` still rolls it back to the layer below.
#[test]
fn direction_rolls_back_by_its_own_keyword_only() {
    let css = "@layer a, b; \
               @layer a { p { direction: ltr } #target { all: revert-layer } } \
               @layer b { #target { direction: revert-layer } }";
    assert_eq!(child(css).2, Direction::Ltr);
    // Without the own keyword, `all: revert-layer` does not reset it either.
    let css = "@layer a { p { direction: ltr } } \
               @layer b { #target { all: revert-layer } }";
    assert_eq!(child(css).2, Direction::Ltr);
}
