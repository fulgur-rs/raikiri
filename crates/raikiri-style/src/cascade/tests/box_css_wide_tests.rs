//! CSS-wide keywords on margin, padding, offset and size properties
//! (CSS Cascading 4 §7.3 "Explicit Defaulting").

use super::*;
use crate::page::{PageInheritance, cascade_page_with_media_context};
use crate::property::{Length, LengthOrAuto, PropertyKey, PropertyValue};
use crate::resolve::{ComputedLengthPercentageOrAuto, ComputedLengthPercentageOrCalc};

type Computed = ComputedLengthPercentageOrAuto;

/// A parent whose box values are all away from their initial values.
const PARENT: &str = "margin: 10px 20px 30px 40px; padding: 1px 2px 3px 4px; \
    width: 200px; height: 50%; min-width: 5px; min-height: 6px; \
    max-width: 70px; max-height: 8%; top: 9px; right: 10%; bottom: 11px; left: auto";

/// A rule that gives every box property a value `#target` then overrides.
const WEAKER: &str = "p { margin: 7px; padding: 7px; width: 7px; height: 7px; \
    min-width: 7px; min-height: 7px; max-width: 7px; max-height: 7px; \
    top: 7px; right: 7px; bottom: 7px; left: 7px }";

/// The computed values of a parent styled `parent` and of its child `p`
/// (`id="target"`) under `sheets`, followed by `css` as an author sheet.
fn cascade_pair(
    parent: &str,
    sheets: &[(&str, Origin)],
    css: &str,
) -> (ComputedValues, ComputedValues) {
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

fn child(css: &str) -> ComputedValues {
    cascade_pair(PARENT, &[], css).1
}

fn px(value: f32) -> Computed {
    Computed::Px(value)
}

fn padding_px(value: f32) -> ComputedLengthPercentageOrCalc {
    ComputedLengthPercentageOrCalc::Px(value)
}

/// The margins of `values` as `[top, right, bottom, left]`.
fn margins(values: &ComputedValues) -> [Computed; 4] {
    let m = values.margin;
    [m.top, m.right, m.bottom, m.left]
}

/// The paddings of `values` as `[top, right, bottom, left]`.
fn paddings(values: &ComputedValues) -> [ComputedLengthPercentageOrCalc; 4] {
    let p = values.padding;
    [p.top, p.right, p.bottom, p.left]
}

/// The offsets of `values` as `[top, right, bottom, left]`.
fn offsets(values: &ComputedValues) -> [Computed; 4] {
    [values.top, values.right, values.bottom, values.left]
}

/// The preferred, minimum and maximum sizes of `values`.
fn sizes(values: &ComputedValues) -> [Computed; 6] {
    [
        values.width,
        values.height,
        values.min_width,
        values.min_height,
        values.max_width,
        values.max_height,
    ]
}

const ALL_LONGHANDS: &str = "margin-top: KW; margin-right: KW; margin-bottom: KW; \
    margin-left: KW; padding-top: KW; padding-right: KW; padding-bottom: KW; padding-left: KW; \
    width: KW; height: KW; min-width: KW; min-height: KW; max-width: KW; max-height: KW; \
    top: KW; right: KW; bottom: KW; left: KW";

#[test]
fn inherit_takes_the_parents_computed_values() {
    let (parent, child) = cascade_pair(
        PARENT,
        &[],
        &format!(
            "{WEAKER} #target {{ {} }}",
            ALL_LONGHANDS.replace("KW", "inherit")
        ),
    );
    assert_eq!(margins(&child), [px(10.0), px(20.0), px(30.0), px(40.0)]);
    assert_eq!(margins(&child), margins(&parent));
    assert_eq!(
        paddings(&child),
        [
            padding_px(1.0),
            padding_px(2.0),
            padding_px(3.0),
            padding_px(4.0)
        ]
    );
    assert_eq!(paddings(&child), paddings(&parent));
    // A percentage stays a percentage, and `auto` stays `auto`.
    assert_eq!(
        sizes(&child),
        [
            px(200.0),
            Computed::Percent(50.0),
            px(5.0),
            px(6.0),
            px(70.0),
            Computed::Percent(8.0),
        ]
    );
    assert_eq!(sizes(&child), sizes(&parent));
    assert_eq!(
        offsets(&child),
        [px(9.0), Computed::Percent(10.0), px(11.0), Computed::Auto]
    );
    assert_eq!(offsets(&child), offsets(&parent));
}

#[test]
fn inherit_keeps_the_parents_sizing_keywords() {
    for (parent_width, expected) in [
        ("max-content", Computed::MaxContent),
        ("min-content", Computed::MinContent),
        ("fit-content", Computed::FitContent),
        ("auto", Computed::Auto),
    ] {
        let (parent, child) = cascade_pair(
            &format!("width: {parent_width}"),
            &[],
            "#target { width: inherit }",
        );
        assert_eq!(parent.width, expected, "{parent_width}");
        assert_eq!(child.width, expected, "{parent_width}");
    }
    // The limit of `fit-content()` stays a percentage until layout.
    let (parent, child) =
        cascade_pair("width: fit-content(25%)", &[], "#target { width: inherit }");
    assert!(matches!(parent.width, Computed::FitContentOf(_)));
    assert_eq!(child.width, parent.width);
}

#[test]
fn inherit_keeps_the_parents_calc() {
    let (parent, child) = cascade_pair(
        "width: calc(50% + 10px); margin-left: calc(10% - 3px); padding-top: calc(5% + 2px)",
        &[],
        "#target { width: inherit; margin-left: inherit; padding-top: inherit }",
    );
    assert!(matches!(parent.width, Computed::Calc(_)));
    assert_eq!(child.width, parent.width);
    assert_eq!(child.margin.left, parent.margin.left);
    assert_eq!(child.padding.top, parent.padding.top);
}

#[test]
fn initial_and_unset_give_the_initial_values() {
    // None of these properties is inherited, so `unset` is `initial`.
    for keyword in ["initial", "unset"] {
        let child = child(&format!(
            "{WEAKER} #target {{ {} }}",
            ALL_LONGHANDS.replace("KW", keyword)
        ));
        assert_eq!(margins(&child), [px(0.0); 4], "{keyword}");
        assert_eq!(paddings(&child), [padding_px(0.0); 4], "{keyword}");
        assert_eq!(sizes(&child), [Computed::Auto; 6], "{keyword}");
        assert_eq!(offsets(&child), [Computed::Auto; 4], "{keyword}");
    }
}

#[test]
fn a_keyword_overrides_by_specificity_importance_and_order() {
    for (css, expected) in [
        ("p { margin-top: inherit } #target { margin-top: 3px }", 3.0),
        (
            "p { margin-top: 3px !important } #target { margin-top: inherit }",
            3.0,
        ),
        (
            "p { margin-top: inherit !important } #target { margin-top: 3px }",
            10.0,
        ),
        ("p { margin-top: 3px; margin-top: inherit }", 10.0),
        ("p { margin-top: inherit; margin-top: 3px }", 3.0),
        // A keyword next to another component is invalid and dropped.
        ("p { margin-top: 3px } #target { margin: inherit 1px }", 3.0),
        (
            "p { margin-top: 3px } #target { margin-top: initial inherit }",
            3.0,
        ),
        (
            "#target { margin-top: /**/ INHERIT /**/ ! important } p { margin-top: 3px !important }",
            10.0,
        ),
    ] {
        assert_eq!(child(css).margin.top, px(expected), "{css}");
    }
}

#[test]
fn shorthands_set_every_longhand_in_order_of_appearance() {
    let (parent, inherit) = cascade_pair(
        PARENT,
        &[],
        &format!("{WEAKER} #target {{ margin: inherit; padding: inherit; inset: inherit }}"),
    );
    assert_eq!(margins(&inherit), margins(&parent));
    assert_eq!(paddings(&inherit), paddings(&parent));
    assert_eq!(offsets(&inherit), offsets(&parent));
    // Width and height are not part of those shorthands.
    assert_eq!(sizes(&inherit)[..2], [px(7.0), px(7.0)]);

    let initial = child(&format!(
        "{WEAKER} #target {{ margin: initial; padding: unset; inset: initial }}"
    ));
    assert_eq!(margins(&initial), [px(0.0); 4]);
    assert_eq!(paddings(&initial), [padding_px(0.0); 4]);
    assert_eq!(offsets(&initial), [Computed::Auto; 4]);

    // A longhand after the shorthand wins its side, and the shorthand wins
    // over a longhand before it.
    let after =
        child("#target { margin: inherit; margin-left: 3px; padding: initial; padding-top: 9px }");
    assert_eq!(margins(&after), [px(10.0), px(20.0), px(30.0), px(3.0)]);
    assert_eq!(
        paddings(&after),
        [
            padding_px(9.0),
            padding_px(0.0),
            padding_px(0.0),
            padding_px(0.0)
        ]
    );
    let before = child("#target { margin-left: 3px; margin: inherit; top: 3px; inset: initial }");
    assert_eq!(margins(&before), [px(10.0), px(20.0), px(30.0), px(40.0)]);
    assert_eq!(offsets(&before), [Computed::Auto; 4]);
}

#[test]
fn flow_relative_shorthands_set_their_two_sides() {
    let values = child(&format!(
        "{WEAKER} #target {{ margin-inline: inherit; margin-block: initial; \
         padding-inline: initial; padding-block: inherit; \
         inset-inline: initial; inset-block: inherit }}"
    ));
    assert_eq!(margins(&values), [px(0.0), px(20.0), px(0.0), px(40.0)]);
    assert_eq!(
        paddings(&values),
        [
            padding_px(1.0),
            padding_px(0.0),
            padding_px(3.0),
            padding_px(0.0)
        ]
    );
    assert_eq!(
        offsets(&values),
        [px(9.0), Computed::Auto, px(11.0), Computed::Auto]
    );
}

#[test]
fn flow_relative_longhands_resolve_on_their_own_side() {
    // The parent and the child share the writing mode and direction, so the
    // physical side is the parent's inline-start side as well.
    for (style, [top, right, bottom, left]) in [
        ("", [10.0, 20.0, 30.0, 40.0]),
        ("direction: rtl", [10.0, 20.0, 30.0, 40.0]),
        ("writing-mode: vertical-rl", [10.0, 20.0, 30.0, 40.0]),
    ] {
        let parent = format!("{style}; margin: 10px 20px 30px 40px");
        let (_, child) = cascade_pair(
            &parent,
            &[],
            "#target { margin-inline-start: inherit; margin-block-start: 3px; \
             margin-inline-end: initial; margin-block-end: inherit }",
        );
        let m = margins(&child);
        // Inline-start inherits and block-start is a length, and so on, on
        // the side each maps to under `style`.
        let inherited = [top, right, bottom, left];
        let count = m.iter().filter(|side| **side == px(3.0)).count();
        assert_eq!(count, 1, "{style}");
        assert_eq!(
            m.iter().filter(|side| **side == px(0.0)).count(),
            1,
            "{style}"
        );
        for (index, side) in m.iter().enumerate() {
            if *side != px(3.0) && *side != px(0.0) {
                assert_eq!(*side, px(inherited[index]), "{style}");
            }
        }
    }
}

#[test]
fn revert_rolls_back_the_origin() {
    let (_, values) = cascade_pair(
        PARENT,
        &[
            (
                "p { margin: 5px; padding-left: 6px; width: 50px; top: 4px }",
                Origin::User,
            ),
            ("p { margin: 3px; width: 2px }", Origin::UserAgent),
        ],
        "p { margin: 7px; padding-left: 7px; width: 7px; top: 7px } \
         #target { margin: revert; padding-left: revert; width: revert; top: revert }",
    );
    assert_eq!(margins(&values), [px(5.0); 4]);
    assert_eq!(values.padding.left, padding_px(6.0));
    assert_eq!(values.width, px(50.0));
    assert_eq!(values.top, px(4.0));
    // Nothing underneath: the initial value, even though the parent has
    // another.
    let (_, values) = cascade_pair(
        PARENT,
        &[],
        "#target { margin-top: 7px; margin: revert; left: revert; height: revert }",
    );
    assert_eq!(margins(&values), [px(0.0); 4]);
    assert_eq!(values.left, Computed::Auto);
    assert_eq!(values.height, Computed::Auto);
}

#[test]
fn revert_layer_rolls_back_the_layer() {
    let values = child(
        "@layer a, b; \
         @layer a { p { margin: 5px; padding-top: 6px; height: 40px; bottom: 4px } } \
         @layer b { p { margin: 7px; padding-top: 7px; height: 7px; bottom: 7px } \
                    #target { margin: revert-layer; padding-top: revert-layer; \
                              height: revert-layer; bottom: revert-layer } }",
    );
    assert_eq!(margins(&values), [px(5.0); 4]);
    assert_eq!(values.padding.top, padding_px(6.0));
    assert_eq!(values.height, px(40.0));
    assert_eq!(values.bottom, px(4.0));
}

#[test]
fn a_rolled_back_flow_relative_declaration_moves_to_its_own_side() {
    // Under `rtl`, `margin-inline-start` is the right side. The low layer's
    // flow-relative declaration is the one that survives for it.
    for fallback in ["9px", "var(--m)"] {
        let values = child(&format!(
            "@layer low, high; \
             @layer low {{ p {{ --m: 9px; direction: rtl; margin: 0; margin-inline-start: {fallback} }} }} \
             @layer high {{ #target {{ margin-right: revert-layer }} }}"
        ));
        assert_eq!(
            margins(&values),
            [px(0.0), px(9.0), px(0.0), px(0.0)],
            "{fallback}"
        );
    }
    // The other way round: a flow-relative keyword rolls back to a physical
    // declaration for the side it maps to.
    let values = child(
        "@layer low, high; \
         @layer low { p { direction: rtl; margin: 0; margin-right: 9px } } \
         @layer high { #target { margin-inline-start: revert-layer } }",
    );
    assert_eq!(margins(&values), [px(0.0), px(9.0), px(0.0), px(0.0)]);
}

#[test]
fn substituted_keywords_apply_to_longhands_of_a_shorthand() {
    let values = child(&format!(
        "{WEAKER} #target {{ margin: var(--missing, inherit); \
         padding: var(--missing, initial); inset-inline: var(--missing, inherit); \
         width: var(--missing, inherit); height: var(--missing, initial) }}"
    ));
    assert_eq!(margins(&values), [px(10.0), px(20.0), px(30.0), px(40.0)]);
    assert_eq!(paddings(&values), [padding_px(0.0); 4]);
    assert_eq!(values.left, Computed::Auto);
    assert_eq!(values.right, Computed::Percent(10.0));
    assert_eq!(values.top, px(7.0));
    assert_eq!(values.width, px(200.0));
    assert_eq!(values.height, Computed::Auto);
}

#[test]
fn a_computed_value_of_the_parent_follows_its_own_em() {
    // `inherit` copies the computed (absolute) length, not the `em` term.
    let (_, values) = cascade_pair(
        "font-size: 10px; margin-left: 2em; width: 3em",
        &[],
        "#target { font-size: 40px; margin-left: inherit; width: inherit }",
    );
    assert_eq!(values.margin.left, px(20.0));
    assert_eq!(values.width, px(30.0));
}

#[test]
fn inherit_takes_the_physical_sizes_of_a_vertical_parent() {
    // `inline-size` is the parent's physical height in `vertical-rl`, although
    // layout folds it into the parent's `width`.
    let (parent, child) = cascade_pair(
        "writing-mode: vertical-rl; inline-size: 100px; block-size: 40px",
        &[],
        "#target { width: inherit; height: inherit }",
    );
    assert_eq!((parent.width, parent.height), (px(100.0), px(40.0)));
    assert_eq!((child.width, child.height), (px(40.0), px(100.0)));
}

fn page_declarations(
    css: &str,
    root: &ComputedValues,
) -> std::collections::HashMap<PropertyKey, PropertyValue> {
    let mut tree = RuleTree::empty();
    tree.add_stylesheet(css, Origin::Author);
    let result = cascade_page_with_media_context(
        &tree,
        &PageContextQuery::default(),
        PageInheritance::FromRoot(root),
        &MediaContext::print(),
    );
    result.declarations().clone()
}

#[test]
fn the_page_context_takes_the_roots_values_for_inherit() {
    let (root, _) = cascade_pair(PARENT, &[], "");
    let declarations = page_declarations(
        "@page { margin: inherit; padding-left: inherit; width: inherit; \
         min-height: inherit; top: initial }",
        &root,
    );
    let get = |key| declarations.get(&key);
    let length = |px: f32| LengthOrAuto::Length(Length::Px(px));
    assert_eq!(
        get(PropertyKey::MarginTop),
        Some(&PropertyValue::MarginTop(length(10.0)))
    );
    assert_eq!(
        get(PropertyKey::MarginLeft),
        Some(&PropertyValue::MarginLeft(length(40.0)))
    );
    assert_eq!(
        get(PropertyKey::Width),
        Some(&PropertyValue::Width(length(200.0)))
    );
    assert_eq!(
        get(PropertyKey::MinHeight),
        Some(&PropertyValue::MinHeight(length(6.0)))
    );
    assert!(matches!(
        get(PropertyKey::PaddingLeft),
        Some(PropertyValue::PaddingLeft(_))
    ));
    assert_eq!(
        get(PropertyKey::Top),
        Some(&PropertyValue::Top(LengthOrAuto::Auto))
    );
}

#[test]
fn the_page_context_resets_to_the_initial_values() {
    let (root, _) = cascade_pair(PARENT, &[], "");
    for keyword in ["initial", "unset"] {
        let declarations = page_declarations(
            &format!("@page {{ margin-top: 1px; margin: {keyword}; width: {keyword} }}"),
            &root,
        );
        let get = |key| declarations.get(&key);
        assert_eq!(
            get(PropertyKey::MarginTop),
            Some(&PropertyValue::MarginTop(LengthOrAuto::Length(Length::Px(
                0.0
            )))),
            "{keyword}"
        );
        assert_eq!(
            get(PropertyKey::Width),
            Some(&PropertyValue::Width(LengthOrAuto::Auto)),
            "{keyword}"
        );
    }
}
