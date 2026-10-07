use super::*;
use crate::page::{PageInheritance, cascade_page_with_media_context};
use crate::property::{PropertyKey, PropertyValue};

fn computed(css: &str, inline: Option<&str>, other: &[(&str, Origin)]) -> ComputedValues {
    let mut doc = TestDoc::new();
    let parent = doc.push_element(
        0,
        "div",
        Some("color:red; background-color:red; font-size:30px"),
    );
    let child = doc.push_element_with_attrs(parent, "p", inline, &[("id", "target")]);
    let mut tree = RuleTree::empty();
    for &(sheet, origin) in other {
        tree.add_stylesheet(sheet, origin);
    }
    tree.add_stylesheet(css, Origin::Author);
    cascade(&doc, &tree).unwrap().computed[child].clone()
}

#[test]
fn css_wide_initial_and_unset_override_weaker_declarations() {
    for keyword in ["initial", "unset", "inherit"] {
        let css = format!(
            "p{{color:blue;background-color:blue;font-size:50px}} #target{{color:{keyword};background-color:{keyword};font-size:{keyword}}}"
        );
        let value = computed(&css, None, &[]);
        let initial = ComputedValues::initial();
        assert_eq!(
            value.color,
            if keyword == "initial" {
                initial.color
            } else {
                RED
            }
        );
        assert_eq!(
            value.background_color,
            if keyword == "inherit" {
                RED
            } else {
                initial.background_color
            }
        );
        assert_eq!(
            value.font_size.0,
            if keyword == "initial" {
                initial.font_size.0
            } else {
                30.0
            }
        );
    }
}

#[test]
fn css_wide_revert_removes_the_entire_author_origin() {
    for importance in ["", "!important"] {
        let css = format!(
            "p{{color:red;background-color:red;font-size:50px}} #target{{color:revert{importance};background-color:revert{importance};font-size:revert{importance}}}"
        );
        let value = computed(
            &css,
            None,
            &[(
                "p{color:blue;background-color:blue;font-size:20px}",
                Origin::User,
            )],
        );
        assert_eq!(value.color, BLUE);
        assert_eq!(value.background_color, BLUE);
        assert_eq!(value.font_size.0, 20.0);
    }
    let value = computed(
        "p{color:red} #target{color:revert!important}",
        None,
        &[
            ("p{color:blue}", Origin::UserAgent),
            ("p{color:red}", Origin::User),
            ("p{color:revert!important}", Origin::User),
        ],
    );
    assert_eq!(value.color, BLUE);
}

#[test]
fn css_wide_revert_layer_removes_same_layer_and_preserves_lower_layers() {
    for (css, expected) in [
        (
            "@layer a,b; @layer a {p{color:blue;background-color:blue;font-size:20px}} @layer b {p{color:red;background-color:red;font-size:50px} #target{color:revert-layer;background-color:revert-layer;font-size:revert-layer}}",
            BLUE,
        ),
        (
            "@layer a,b,c; @layer a {p{color:blue;background-color:blue;font-size:20px}} @layer c {p{color:red!important;background-color:red!important;font-size:50px!important}} @layer b {#target{color:revert-layer!important;background-color:revert-layer!important;font-size:revert-layer!important}}",
            BLUE,
        ),
    ] {
        let value = computed(css, None, &[]);
        assert_eq!(value.color, expected);
        assert_eq!(value.background_color, expected);
        assert_eq!(value.font_size.0, 20.0);
    }
    let value = computed(
        "p{color:blue!important}",
        Some("color:revert-layer!important"),
        &[],
    );
    assert_eq!(value.color, BLUE);
}

#[test]
fn css_wide_rollback_without_fallback_defaults_and_chains() {
    for keyword in ["revert", "revert-layer"] {
        let css = format!(
            "p{{color:blue;background-color:blue;font-size:50px}} #target{{color:{keyword};background-color:{keyword};font-size:{keyword}}}"
        );
        let value = computed(&css, None, &[]);
        assert_eq!(value.color, RED);
        assert_eq!(
            value.background_color,
            ComputedValues::initial().background_color
        );
        assert_eq!(value.font_size.0, 30.0);
    }
    let value = computed(
        "@layer a,b,c; @layer a{p{color:blue}} @layer b{p{color:revert-layer}} @layer c{#target{color:revert-layer}}",
        None,
        &[],
    );
    assert_eq!(value.color, BLUE);
}

#[test]
fn css_wide_substituted_values_use_inheritance_and_rollback() {
    for (source, expected) in [
        ("initial", ComputedValues::initial().color),
        ("unset", RED),
        ("revert", BLUE),
        ("revert-layer", BLUE),
    ] {
        let css = format!(
            "@layer a,b; @layer a{{p{{color:blue}}}} @layer b{{p{{--wide:{source};color:var(--missing, {source})}}}}"
        );
        let value = computed(&css, None, &[("p{color:blue}", Origin::User)]);
        assert_eq!(value.color, expected, "{source}");
    }
}

#[test]
fn css_wide_page_values_resolve_initial_inherit_and_unset() {
    let mut root = ComputedValues::initial();
    root.color = RED;
    root.background_color = RED;
    root.font_size.0 = 30.0;
    for keyword in ["initial", "inherit", "unset"] {
        let mut tree = RuleTree::empty();
        tree.add_stylesheet(
            &format!("@page{{color:{keyword};background-color:{keyword};font-size:{keyword}}}"),
            Origin::Author,
        );
        let result = cascade_page_with_media_context(
            &tree,
            &PageContextQuery::default(),
            PageInheritance::FromRoot(&root),
            &MediaContext::print(),
        );
        let expected_color = if keyword == "initial" {
            ComputedValues::initial().color
        } else {
            RED
        };
        assert_eq!(
            result.declarations().get(&PropertyKey::Color),
            Some(&PropertyValue::Color(expected_color))
        );
        assert_eq!(
            result.declarations().get(&PropertyKey::BackgroundColor),
            Some(&PropertyValue::BackgroundColor(if keyword == "inherit" {
                RED
            } else {
                ComputedValues::initial().background_color
            }))
        );
    }
}

#[test]
fn css_wide_page_rollback_respects_origin_and_layer_after_substitution() {
    for keyword in ["revert", "revert-layer"] {
        for value in [keyword.to_string(), format!("var(--missing, {keyword})")] {
            let mut tree = RuleTree::empty();
            tree.add_stylesheet(
                "@page{color:blue;background-color:blue;font-size:20px}",
                Origin::User,
            );
            tree.add_stylesheet(&format!("@page{{color:red;background-color:red;font-size:50px}} @page{{color:{value};background-color:{value};font-size:{value}}}"), Origin::Author);
            let result = cascade_page_with_media_context(
                &tree,
                &PageContextQuery::default(),
                PageInheritance::LegacyInitialValues,
                &MediaContext::print(),
            );
            assert_eq!(
                result.declarations().get(&PropertyKey::Color),
                Some(&PropertyValue::Color(BLUE)),
                "{value}"
            );
            assert_eq!(
                result.declarations().get(&PropertyKey::BackgroundColor),
                Some(&PropertyValue::BackgroundColor(BLUE)),
                "{value}"
            );
        }
    }
}

#[test]
fn css_wide_keyword_with_importance_obeys_specificity_and_source_order() {
    for css in [
        "p{color:initial!important} #target{color:blue}",
        "#target{color:initial} p{color:blue}",
        "p{color:blue;color:initial}",
        "p{color:blue} p{color:/**/ INITIAL /**/ ! important}",
        r"p{color:blue} p{color:\69 nitial!important}",
    ] {
        assert_eq!(
            computed(css, None, &[]).color,
            ComputedValues::initial().color,
            "{css}"
        );
    }
    assert_eq!(
        computed("p{color:initial;color:blue}", None, &[]).color,
        BLUE
    );
}

#[test]
fn css_wide_layer_rollback_resolves_surviving_deferred_longhands() {
    for (fallback, expected_color, expected_font, expected_background) in [
        ("var(--missing, blue)", BLUE, 20.0, BLUE),
        (
            "var(--missing, initial)",
            ComputedValues::initial().color,
            16.0,
            ComputedValues::initial().background_color,
        ),
        ("var(--missing, inherit)", RED, 30.0, RED),
        (
            "var(--missing)",
            RED,
            30.0,
            ComputedValues::initial().background_color,
        ),
    ] {
        let font = if fallback == "var(--missing, blue)" {
            "var(--missing, 20px)"
        } else {
            fallback
        };
        let css = "@layer a,b; @layer a{p{color:green;font-size:50px;background-color:green;color:FALLBACK;font-size:FONT;background-color:FALLBACK}} @layer b{#target{color:revert-layer;font-size:revert-layer;background-color:revert-layer}}".replace("FALLBACK", fallback).replace("FONT", font);
        let value = computed(&css, None, &[]);
        assert_eq!(value.color, expected_color, "{fallback}");
        assert_eq!(value.font_size.0, expected_font, "{fallback}");
        assert_eq!(value.background_color, expected_background, "{fallback}");
    }
}

#[test]
fn css_wide_page_property_rollback_preserves_other_concrete_winners() {
    let mut tree = RuleTree::empty();
    tree.add_stylesheet("@layer a,b; @layer a{@page{color:blue}} @layer b{@page{color:revert-layer;width:17px;border-right-width:revert}}", Origin::Author);
    let page = cascade_page_with_media_context(
        &tree,
        &PageContextQuery::default(),
        PageInheritance::LegacyInitialValues,
        &MediaContext::print(),
    );
    assert_eq!(
        page.declarations().get(&PropertyKey::Color),
        Some(&PropertyValue::Color(BLUE))
    );
    assert_eq!(
        page.declarations().get(&PropertyKey::Width),
        Some(&PropertyValue::Width(
            crate::property::LengthOrAuto::Length(crate::property::Length::Px(17.0))
        ))
    );
    assert_eq!(
        page.declarations().get(&PropertyKey::BorderRightWidth),
        Some(&PropertyValue::BorderRightWidth(
            crate::property::Length::Px(0.0)
        ))
    );
}

#[test]
fn css_wide_margin_resolution_preserves_deferred_sizing_declarations() {
    let mut tree = RuleTree::empty();
    tree.add_stylesheet(
        "@page{@top-center{content:'x';color:initial;--width:17px;width:var(--width)}}",
        Origin::Author,
    );
    let page = cascade_page_with_media_context(
        &tree,
        &PageContextQuery::default(),
        PageInheritance::LegacyInitialValues,
        &MediaContext::print(),
    );
    let margin = page
        .cascade_margin_box(crate::page::PageMarginBoxSlot::TopCenter)
        .unwrap();
    assert!(margin.declarations.iter().any(|decl| matches!(decl.value(), PropertyValue::Color(color) if *color == crate::property::CssColor::BLACK)));
    let sizing = margin
        .declarations
        .iter()
        .find(|decl| decl.value().key() == PropertyKey::Width)
        .unwrap();
    let PropertyValue::Deferred(deferred) = sizing.value() else {
        panic!("deferred sizing must remain available to its consumer");
    };
    assert_eq!(deferred.value, "var(--width)");
}
