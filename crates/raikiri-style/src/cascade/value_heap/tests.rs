use super::*;
use crate::cascade::cascade;
use crate::ruletree::build_rule_tree;
use crate::test_dom::TestDoc;

/// The own heap of a `div` with inline style `style` under `body`, and of a
/// `span` inside it, which declares nothing.
fn heap_of(style: &str) -> (u64, u64) {
    let mut doc = TestDoc::new();
    let body = doc.push_element(0, "body", None);
    let div = doc.push_element(body, "div", Some(style));
    let span = doc.push_element(div, "span", None);
    let tree = build_rule_tree(&doc);
    let result = cascade(&doc, &tree).expect("cascade Ok");
    (
        own_heap_bytes(&result.computed[div], &result.computed[body]),
        own_heap_bytes(&result.computed[span], &result.computed[div]),
    )
}

const LONG: &str = "a-name-long-enough-to-live-on-the-heap";

#[test]
fn values_shared_with_the_parent_hold_nothing_of_their_own() {
    let values = ComputedValues::initial();
    assert_eq!(own_heap_bytes(&values, &values.clone()), 0);
    assert_eq!(heap_of("color: red"), (0, 0));
}

#[test]
fn inherited_values_count_on_the_element_that_declares_them() {
    for style in [
        format!("font-family: {LONG}, serif"),
        format!("list-style-type: {LONG}"),
        format!("list-style-type: '{LONG}'"),
        format!("list-style-image: url({LONG}.png)"),
        format!("quotes: '{LONG}' '{LONG}'"),
        "text-shadow: 1px 1px red, 2px 2px blue".to_owned(),
        format!("text-emphasis-style: '{LONG}'"),
        format!("font-palette: --{LONG}"),
        r#"font-feature-settings: "liga" 1, "kern" 0"#.to_owned(),
        r#"font-variation-settings: "wght" 700"#.to_owned(),
        format!("hyphenate-character: '{LONG}'"),
        format!("--custom: {LONG}"),
    ] {
        let (own, inherited) = heap_of(&style);
        assert!(own > 0, "{style}: nothing counted");
        assert_eq!(inherited, 0, "{style}: the child copied it");
    }
}

#[test]
fn values_that_are_not_inherited_count_on_each_element() {
    for style in [
        format!("counter-reset: {LONG} 1"),
        format!("counter-increment: {LONG}"),
        format!("counter-set: {LONG} 2"),
        format!("content: '{LONG}'"),
        format!("content: counter({LONG})"),
        format!("content: counters({LONG}, '{LONG}')"),
        format!("content: attr({LONG})"),
        format!("content: string({LONG})"),
        format!("content: url({LONG}.png)"),
        format!("content: target-counter(url(#{LONG}), {LONG})"),
        format!("content: target-counters(url(#{LONG}), {LONG}, '{LONG}')"),
        format!("content: target-text(url(#{LONG}))"),
        format!("string-set: {LONG} '{LONG}'"),
        format!("position: running({LONG})"),
        "box-shadow: 1px 1px red".to_owned(),
        "transform: translate(1px, 2px) rotate(3deg)".to_owned(),
        format!("filter: url(#{LONG}) blur(1px)"),
        format!("grid-template-columns: [{LONG}] 1fr repeat(2, [{LONG}] 10px)"),
        format!("grid-template-areas: '{LONG}'"),
        "grid-auto-columns: 10px 20px".to_owned(),
        format!("grid-row-start: {LONG}"),
        format!("grid-column-end: span {LONG}"),
        format!("background-image: url({LONG}.png)"),
        "background-image: radial-gradient(red, blue)".to_owned(),
        "mask-image: conic-gradient(red, blue)".to_owned(),
        format!("clip-path: url(#{LONG})"),
        "clip-path: polygon(0 0, 10px 0, 0 10px)".to_owned(),
        "clip-path: path('M 0 0 L 10 10')".to_owned(),
        // A shape is boxed, so even one without a list holds heap.
        "clip-path: circle(10px)".to_owned(),
    ] {
        let (own, _) = heap_of(&style);
        assert!(own > 0, "{style}: nothing counted");
    }
}

#[test]
fn short_strings_and_shapes_without_lists_hold_no_heap() {
    for style in [
        "list-style-type: square",
        "clip-path: border-box",
        "font-language-override: 'TRK'",
        "background-image: none",
    ] {
        assert_eq!(heap_of(style).0, 0, "{style}");
    }
}

#[test]
fn a_custom_property_environment_counts_once() {
    // The element's local environment is also its effective one.
    let (own, _) = heap_of(&format!("--a: {LONG}"));
    let (two, _) = heap_of(&format!("--a: {LONG}; --b: {LONG}"));
    assert!(two > own);
    assert!(own < 4 * LONG.len() as u64 + 1024, "{own}: counted twice");
}

#[test]
fn values_held_with_the_parent_are_free_however_they_were_built() {
    // An element that holds its own gradient, track list and areas, measured
    // against a copy of itself, shares every one of them.
    let mut doc = TestDoc::new();
    let body = doc.push_element(0, "body", None);
    let div = doc.push_element(
        body,
        "div",
        Some(&format!(
            "background-image: linear-gradient(red, blue); \
             grid-template-columns: [{LONG}] 1fr; grid-template-areas: '{LONG}'"
        )),
    );
    let tree = build_rule_tree(&doc);
    let result = cascade(&doc, &tree).expect("cascade Ok");
    let values = &result.computed[div];
    assert!(own_heap_bytes(values, &result.computed[body]) > 0);
    assert_eq!(own_heap_bytes(values, &values.clone()), 0);
}

#[test]
fn strings_inside_every_kind_of_value_count() {
    use crate::property::{GridLineValue, StringFetchMode};
    let parent = ComputedValues::initial();
    let name = SmolStr::new(LONG);
    let mut values = parent.clone();
    values.position = PositionValue::Running(name.clone());
    assert_eq!(own_heap_bytes(&values, &parent), LONG.len() as u64);
    let mut values = parent.clone();
    values.grid_column_end = GridLineValue::SpanNamed(name.clone(), 1);
    values.grid_row_start = GridLineValue::NamedLine(name.clone(), 2);
    assert_eq!(own_heap_bytes(&values, &parent), 2 * LONG.len() as u64);
    let mut values = parent.clone();
    values.content = Arc::new(vec![
        ContentComponent::Element {
            name: name.clone(),
            fetch: StringFetchMode::First,
        },
        ContentComponent::AttrFallback {
            name: name.clone(),
            fallback: Some(name.clone()),
        },
        ContentComponent::Contents,
    ]);
    assert_eq!(
        own_heap_bytes(&values, &parent),
        3 * size_of::<ContentComponent>() as u64 + 3 * LONG.len() as u64
    );
}

#[test]
fn family_names_counter_styles_and_spare_capacity_count() {
    use crate::Atom;
    use crate::computed::RunningTemplate;
    use crate::property::{CounterStyle, FontFamilyKind, FontFamilyName};
    let parent = ComputedValues::initial();
    let name = SmolStr::new(LONG);
    let mut values = parent.clone();
    values.font_family = Arc::new(vec![FontFamilyName(
        Atom(name.clone()),
        FontFamilyKind::Named,
    )]);
    assert_eq!(
        own_heap_bytes(&values, &parent),
        values.font_family.capacity() as u64 * size_of::<FontFamilyName>() as u64
            + LONG.len() as u64
    );
    let mut values = parent.clone();
    values.content = Arc::new(vec![ContentComponent::Counter {
        name: SmolStr::new("c"),
        style: CounterStyle::Named(name.clone()),
    }]);
    assert_eq!(
        own_heap_bytes(&values, &parent),
        size_of::<ContentComponent>() as u64 + LONG.len() as u64
    );
    // A vector's whole buffer counts, not only the entries in use.
    let mut values = parent.clone();
    let mut templates = Vec::with_capacity(8);
    templates.push(RunningTemplate::new(SmolStr::new("r")));
    values.running_templates = templates;
    assert_eq!(
        own_heap_bytes(&values, &parent),
        8 * size_of::<RunningTemplate>() as u64
    );
}
