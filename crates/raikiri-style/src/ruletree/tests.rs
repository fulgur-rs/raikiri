use super::*;
use crate::test_dom::TestDoc;

#[test]
fn unicode_supports_conditions_preserve_fallback_rules() {
    for condition in [
        "ééé()",
        "日本語()",
        "🦀🦀()",
        "ééé() or (display: invalid)",
        "(display: invalid) or ééé()",
    ] {
        let mut tree = RuleTree::empty();
        tree.add_stylesheet(
            &format!("@supports {condition} {{ p {{ color: red }} }} p {{ color: blue }}"),
            Origin::Author,
        );
        assert_eq!(tree.style_rules.len(), 1, "{condition}");
        assert_eq!(tree.opaque_at_rules().len(), 1, "{condition}");
    }
}

#[test]
fn custom_highlight_rules_capture_named_background_colors() {
    let mut tree = RuleTree::empty();
    tree.add_stylesheet(
        "::highlight(open) { background-color: orange; } \
         ::highlight(close) { background-color: springgreen; }",
        Origin::Author,
    );
    assert!(tree.style_rules().is_empty());
    assert_eq!(
        tree.custom_highlight_styles().get("open"),
        Some(&crate::CssColor {
            r: 255,
            g: 165,
            b: 0,
            a: 255,
        })
    );
    assert_eq!(
        tree.custom_highlight_styles().get("close"),
        Some(&crate::CssColor {
            r: 0,
            g: 255,
            b: 127,
            a: 255,
        })
    );
}

#[test]
fn unicode_at_rules_preserve_opaque_content_and_following_style() {
    for name in ["ééé", "日本語", "🦀🦀", "x日本", "é"] {
        for body in [";", "{ p { color: red } }"] {
            let mut tree = RuleTree::empty();
            tree.add_stylesheet(
                &format!("@{name}{body} p {{ color: blue }}"),
                Origin::Author,
            );
            assert_eq!(tree.style_rules.len(), 1, "{name}{body}");
            assert_eq!(tree.opaque_at_rules().len(), 1, "{name}{body}");
            assert_eq!(tree.opaque_at_rules()[0].name, name);
        }
    }
}

#[test]
fn unicode_supports_values_and_ascii_layer_controls_remain_supported() {
    for condition in [
        "(font-family: \"日本語\")",
        "(display: block) and (color: red)",
        "(display: invalid) or (display: block)",
        "not (display: invalid)",
    ] {
        let mut tree = RuleTree::empty();
        tree.add_stylesheet(
            &format!("@supports {condition} {{ p {{ color: red }} }}"),
            Origin::Author,
        );
        assert_eq!(tree.style_rules.len(), 1, "{condition}");
    }
    let mut tree = RuleTree::empty();
    tree.add_stylesheet(
        "@LaYeR base { p { color: red } } p { color: blue }",
        Origin::Author,
    );
    assert_eq!(tree.style_rules.len(), 2);
}

fn dom_with_style(css: &str) -> TestDoc {
    let mut doc = TestDoc::new();
    let style = doc.push_element(0, "style", None);
    doc.push_text(style, css);
    doc
}

#[test]
fn empty_dom_returns_empty_ruletree() {
    let doc = TestDoc::new();
    assert!(build_rule_tree(&doc).style_rules.is_empty());
}

#[test]
fn dom_without_style_returns_empty_ruletree() {
    let mut doc = TestDoc::new();
    doc.push_element(0, "p", None);
    assert!(build_rule_tree(&doc).style_rules.is_empty());
}

#[test]
fn single_style_type_selector_captured() {
    let doc = dom_with_style("p { color: red }");
    let tree = build_rule_tree(&doc);
    assert_eq!(tree.style_rules.len(), 1);
    assert_eq!(tree.style_rules[0].source_order, 0);
    assert_eq!(tree.style_rules[0].declarations.len(), 1);
}

#[test]
fn nested_qualified_rules_flatten_to_descendant_selectors() {
    let doc = dom_with_style(".test { color: red; span { margin-left: 5px; margin-right: 5px; } }");
    let tree = build_rule_tree(&doc);
    assert_eq!(tree.style_rules.len(), 2);
    assert_eq!(tree.style_rules[0].declarations.len(), 1);
    assert_eq!(tree.style_rules[1].declarations.len(), 2);
}

#[test]
fn nested_ampersand_rules_flatten_to_combined_selectors() {
    use cssparser::ToCss;

    let doc = dom_with_style(".test, .other { & > span { color: red } }");
    let tree = build_rule_tree(&doc);
    assert_eq!(tree.style_rules.len(), 1);
    assert_eq!(tree.style_rules[0].declarations.len(), 1);
    let mut selector = String::new();
    tree.style_rules[0]
        .selectors
        .to_css(&mut selector)
        .expect("selector serialization");
    assert_eq!(selector, ":is(.test, .other) > span");
}

#[test]
fn declarations_after_a_nested_rule_keep_their_cascade_position() {
    // `color: blue` follows the nested rule, so it must come after it in
    // source order (and win over it for `.a` itself on equal footing).
    let doc = dom_with_style(".a { color: red; & { color: green } color: blue; }");
    let tree = build_rule_tree(&doc);
    let colors: Vec<_> = tree
        .style_rules
        .iter()
        .map(|rule| rule.declarations.len())
        .collect();
    assert_eq!(colors, vec![1, 1, 1]);
    let order: Vec<_> = tree
        .style_rules
        .iter()
        .map(|rule| rule.source_order)
        .collect();
    assert!(order.windows(2).all(|pair| pair[0] < pair[1]), "{order:?}");
    let color = |r, g, b| {
        crate::property::PropertyValue::Color(crate::property::CssColor { r, g, b, a: 255 })
    };
    assert_eq!(tree.style_rules[0].declarations[0].value, color(255, 0, 0));
    assert_eq!(tree.style_rules[1].declarations[0].value, color(0, 128, 0));
    assert_eq!(tree.style_rules[2].declarations[0].value, color(0, 0, 255));
}

#[test]
fn class_selector_is_captured() {
    // Class selectors are no longer dropped; both rules remain.
    let doc = dom_with_style(".foo { color: red } p { color: blue }");
    let tree = build_rule_tree(&doc);
    assert_eq!(tree.style_rules.len(), 2);
    assert_eq!(tree.style_rules[0].source_order, 0);
    assert_eq!(tree.style_rules[1].source_order, 1);
}

#[test]
fn id_selector_is_captured() {
    let doc = dom_with_style("#header { color: red }");
    let tree = build_rule_tree(&doc);
    assert_eq!(tree.style_rules.len(), 1);
}

#[test]
fn attribute_exists_selector_is_captured() {
    let doc = dom_with_style("[data-foo] { color: red }");
    let tree = build_rule_tree(&doc);
    assert_eq!(tree.style_rules.len(), 1);
}

#[test]
fn attribute_exists_selector_with_mixed_case_local_name_is_captured() {
    // `[Data-Foo]` (no value, no namespace) — unlike the value-bearing
    // form (`non_lowercase_attribute_name_with_value_selector_still_dropped`
    // below), the selectors crate parser routes this to
    // `Component::AttributeInNoNamespaceExists` regardless of the local
    // name's case (only `namespace.is_some()` sends the exists-only form
    // to `AttributeOther` — see `is_supported_selector_list`'s doc for
    // the parser trace). Must therefore be captured, not dropped.
    let doc = dom_with_style("[Data-Foo] { color: red }");
    let tree = build_rule_tree(&doc);
    assert_eq!(tree.style_rules.len(), 1);
}

#[test]
fn attribute_value_selector_is_captured() {
    let doc = dom_with_style("[data-foo=\"bar\"] { color: red }");
    let tree = build_rule_tree(&doc);
    assert_eq!(tree.style_rules.len(), 1);
}

#[test]
fn descendant_combinator_selector_is_captured() {
    // The descendant combinator (`div p`) is no longer dropped — both
    // rules are kept (was
    // `combinator_selector_still_dropped` when combinators
    // were entirely out of scope and this asserted `len() == 1`).
    let doc = dom_with_style("div p { color: red } p { color: blue }");
    let tree = build_rule_tree(&doc);
    assert_eq!(tree.style_rules.len(), 2);
    assert_eq!(tree.style_rules[0].source_order, 0);
    assert_eq!(tree.style_rules[1].source_order, 1);
}

#[test]
fn child_combinator_selector_is_captured() {
    // child combinator acceptance: `ol > li` must be captured.
    let doc = dom_with_style("ol > li { color: red }");
    let tree = build_rule_tree(&doc);
    assert_eq!(tree.style_rules.len(), 1);
}

#[test]
fn structural_pseudo_class_selectors_are_captured() {
    // structural pseudo-class acceptance: `:root`/`:empty`/
    // `:first-child`/`:nth-child()`/`-of-type` counterparts must no
    // longer be dropped by `is_supported_selector_list` — pairs with
    // `pseudo_class_selector_still_dropped` (which pins that
    // `:hover`/`:active`, true `NonTSPseudoClass` components, remain
    // dropped; these are architecturally different `Component`
    // variants the `selectors` crate parses directly, see
    // `is_supported_selector_list`'s doc).
    let doc = dom_with_style(
        ":root { color: red } \
         p:empty { color: red } \
         li:first-child { color: red } \
         li:last-child { color: red } \
         li:only-child { color: red } \
         li:nth-child(2n+1) { color: red } \
         li:nth-last-child(1) { color: red } \
         h2:first-of-type { color: red } \
         h2:last-of-type { color: red } \
         h2:only-of-type { color: red } \
         h2:nth-of-type(2) { color: red } \
         h2:nth-last-of-type(1) { color: red }",
    );
    let tree = build_rule_tree(&doc);
    assert_eq!(
        tree.style_rules.len(),
        12,
        "all 12 structural pseudo-class rules must be kept"
    );
}

#[test]
fn logical_and_relational_pseudo_class_selectors_are_captured() {
    let doc = dom_with_style(
        "div:is(.featured, .selected) { color: red } \
         div:where(.featured, .selected) { color: blue } \
         div:has(> .featured) { color: green }",
    );
    let tree = build_rule_tree(&doc);
    assert_eq!(tree.style_rules.len(), 3);
}

#[test]
fn unsupported_pseudo_class_inside_logical_selector_is_dropped() {
    // `:hover` is syntactically parseable but not a supported matching
    // state. The support gate must not let `:is()`/`:has()` turn its
    // fail-closed matcher result into a false positive.
    let doc = dom_with_style(
        "div:is(:hover, .featured) { color: red } \
         div:has(:hover) { color: blue } \
         div { color: green }",
    );
    let tree = build_rule_tree(&doc);
    assert_eq!(tree.style_rules.len(), 1);
    assert_eq!(tree.style_rules[0].source_order, 0);
}

#[test]
fn non_forgiving_has_rejects_invalid_and_nested_branches() {
    // Unlike :is()/:where(), :has() uses a non-forgiving relative
    // selector list. A malformed list or a directly nested :has() drops
    // the complete style rule.
    let doc = dom_with_style(
        "div:has(.featured, 123) { color: red } \
         div:has(.featured:has(.nested)) { color: blue } \
         div { color: green }",
    );
    let tree = build_rule_tree(&doc);
    assert_eq!(tree.style_rules.len(), 1);
    assert_eq!(tree.style_rules[0].source_order, 0);
}

#[test]
fn forgiving_logical_selector_keeps_valid_branches() {
    // `selectors` represents a syntactically invalid branch in a
    // forgiving `:is()`/`:where()` list as `Component::Invalid`. That
    // branch must not make the whole stylesheet rule disappear; the
    // cascade matcher will ignore it and still try the valid branch.
    let doc = dom_with_style(
        "div:is(.featured, :unknown-pseudo) { color: red } \
         div:where(.selected, 123) { color: blue }",
    );
    let tree = build_rule_tree(&doc);
    assert_eq!(tree.style_rules.len(), 2);
}

#[test]
fn nth_child_of_extended_syntax_selector_is_captured() {
    let doc = dom_with_style(
        "p:nth-child(2n+1 of .foo) { color: red } \
         p:nth-last-child(1 of [data-kind=selected]) { color: blue }",
    );
    let tree = build_rule_tree(&doc);
    assert_eq!(tree.style_rules.len(), 2);
    assert_eq!(tree.style_rules[0].source_order, 0);
    assert_eq!(tree.style_rules[1].source_order, 1);
}

#[test]
fn nested_nth_child_of_selector_is_dropped() {
    let doc = dom_with_style("li:nth-child(2 of li:nth-child(2 of .featured)) { color: red }");
    let tree = build_rule_tree(&doc);
    assert_eq!(tree.style_rules.len(), 0);
}

#[test]
fn chained_combinator_selector_is_captured() {
    // CSS Selectors L4 child-combinators
    // (<https://www.w3.org/TR/selectors-4/#child-combinators>) example
    // selector `div ol>li p`, verbatim from the spec — mixes descendant
    // and child combinators in one complex selector. Must be captured
    // whole (not partially, `is_supported_selector_list` walks every
    // component in the selector regardless of which combinator
    // separates it from its neighbours).
    let doc = dom_with_style("div ol>li p { color: red }");
    let tree = build_rule_tree(&doc);
    assert_eq!(tree.style_rules.len(), 1);
}

#[test]
fn sibling_combinator_selector_is_captured() {
    // next-sibling (`+`) / subsequent-sibling
    // (`~`) combinators are no longer dropped — both rules kept (was
    // `sibling_combinator_selector_still_dropped`, asserting
    // `len() == 1` / only the second `p` rule surviving).
    let doc = dom_with_style("p + p { color: red } p { color: blue }");
    let tree = build_rule_tree(&doc);
    assert_eq!(tree.style_rules.len(), 2);
    assert_eq!(tree.style_rules[0].source_order, 0);
    assert_eq!(tree.style_rules[1].source_order, 1);

    let doc = dom_with_style("p ~ p { color: red } p { color: blue }");
    let tree = build_rule_tree(&doc);
    assert_eq!(tree.style_rules.len(), 2);
    assert_eq!(tree.style_rules[0].source_order, 0);
    assert_eq!(tree.style_rules[1].source_order, 1);
}

#[test]
fn mixed_ancestor_and_sibling_combinator_selector_is_captured() {
    // mixing ancestor-chain (`>`/space) and
    // sibling-chain (`+`/`~`) combinators within one complex selector is
    // fully supported in both compositional orders (see
    // `is_supported_selector_list`'s "mixing four combinators" doc note for
    // why no extra tracking state is needed either way) — must be
    // captured whole, not dropped.
    let doc = dom_with_style(".x > .y ~ .z { color: red }");
    let tree = build_rule_tree(&doc);
    assert_eq!(tree.style_rules.len(), 1);

    let doc = dom_with_style(".x ~ .y > .z { color: red }");
    let tree = build_rule_tree(&doc);
    assert_eq!(tree.style_rules.len(), 1);
}

#[test]
fn pseudo_class_selector_still_dropped() {
    // `:hover` (NonTSPseudoClass) remains out of scope and is still
    // dropped (safety-net regression test).
    let doc = dom_with_style("p:hover { color: red } p { color: blue }");
    let tree = build_rule_tree(&doc);
    assert_eq!(tree.style_rules.len(), 1);
    assert_eq!(tree.style_rules[0].source_order, 0);
}

#[test]
fn lang_and_dir_pseudo_class_selectors_are_captured() {
    // acceptance counterpart to
    // `descendant_combinator_selector_is_captured` /
    // `child_combinator_selector_is_captured` above — `:lang()`/`:dir()`
    // are the first `Component::NonTSPseudoClass` variants admitted by
    // `is_supported_selector_list` (siblings `:hover`/`:active` remain
    // dropped, pinned by `pseudo_class_selector_still_dropped` above).
    let doc = dom_with_style(":lang(ja) { color: red } :dir(ltr) { color: blue }");
    let tree = build_rule_tree(&doc);
    assert_eq!(tree.style_rules.len(), 2);
    assert_eq!(tree.style_rules[0].source_order, 0);
    assert_eq!(tree.style_rules[1].source_order, 1);
}

#[test]
fn non_lowercase_attribute_name_with_value_selector_still_dropped() {
    // `[Data-Foo="bar"]` — a value-bearing attribute selector with a
    // local name that is not ASCII-lowercase becomes
    // `Component::AttributeOther` (unsupported by
    // `is_supported_selector_list`) in the selectors crate parser.
    // This gate applies only to value-bearing attributes: without a
    // value, `[Data-Foo]` is accepted as
    // `AttributeInNoNamespaceExists` even with a mixed-case local name
    // when no namespace is specified (see
    // `attribute_exists_selector_with_mixed_case_local_name_is_captured`).
    // Only the value-bearing form remains dropped (safety-net regression).
    let doc = dom_with_style("[Data-Foo=\"bar\"] { color: red } p { color: blue }");
    let tree = build_rule_tree(&doc);
    assert_eq!(tree.style_rules.len(), 1);
    assert_eq!(tree.style_rules[0].source_order, 0);
}

#[test]
fn universal_selector_captured() {
    let doc = dom_with_style("* { color: red }");
    let tree = build_rule_tree(&doc);
    assert_eq!(tree.style_rules.len(), 1);
}

#[test]
fn multiple_style_elements_source_order() {
    let mut doc = TestDoc::new();
    let s1 = doc.push_element(0, "style", None);
    doc.push_text(s1, "p { color: red }");
    let s2 = doc.push_element(0, "style", None);
    doc.push_text(s2, "div { color: blue }");
    let tree = build_rule_tree(&doc);
    assert_eq!(tree.style_rules.len(), 2);
    assert_eq!(tree.style_rules[0].source_order, 0);
    assert_eq!(tree.style_rules[1].source_order, 1);
}

#[test]
fn invalid_rule_silently_dropped() {
    // @nope; is retained as opaque data; malformed selector syntax is
    // still dropped from the compatibility style view.
    let doc = dom_with_style("@nope; p { color: red } ;;garbage;;");
    let tree = build_rule_tree(&doc);
    assert_eq!(tree.style_rules.len(), 1);
}

#[test]
fn important_flag_captured() {
    let doc = dom_with_style("p { color: red !important }");
    let tree = build_rule_tree(&doc);
    assert_eq!(tree.style_rules.len(), 1);
    assert!(tree.style_rules[0].declarations[0].important);
}

/// `walk_and_collect` was recursive DFS —
/// a deeply nested DOM (e.g. approaching `max_dom_nodes = 1M`) could
/// stack-overflow the process. 5000-level linear chain with `<style>`
/// at the deepest level (forcing the walk all the way down before
/// finding rule text) must complete without overflow and still find
/// the rule.
#[test]
fn deep_nesting_5000_build_rule_tree_no_overflow() {
    let mut doc = TestDoc::new();
    let mut parent = 0usize;
    for _ in 0..5000 {
        parent = doc.push_element(parent, "div", None);
    }
    let style = doc.push_element(parent, "style", None);
    doc.push_text(style, "div { color: red }");

    let tree = build_rule_tree(&doc);
    assert_eq!(tree.style_rules.len(), 1);
    assert_eq!(tree.style_rules[0].declarations.len(), 1);
}

// ── Origin + add_stylesheet ──

#[test]
fn origin_is_copy_eq() {
    fn assert_copy<T: Copy + PartialEq + Eq>() {}
    assert_copy::<Origin>();
    assert_ne!(Origin::UserAgent, Origin::Author);
    // 3rd variant (CSS Cascading L5 §6.5 "author
    // presentational hint origin") is pairwise distinct from both.
    assert_ne!(Origin::UserAgent, Origin::AuthorPresentationalHint);
    assert_ne!(Origin::AuthorPresentationalHint, Origin::Author);
}

#[test]
fn add_stylesheet_ua_and_author_populate_rule_tree() {
    let mut tree = RuleTree::empty();
    tree.add_stylesheet("p { color: red }", Origin::UserAgent);
    tree.add_stylesheet("p { color: blue }", Origin::Author);

    assert_eq!(tree.style_rules.len(), 2);
    assert_eq!(tree.style_rules[0].origin, Origin::UserAgent);
    assert_eq!(tree.style_rules[0].source_order, 0);
    assert_eq!(tree.style_rules[1].origin, Origin::Author);
    assert_eq!(tree.style_rules[1].source_order, 1);
}

#[test]
fn add_stylesheet_source_order_monotonic_across_calls() {
    let mut tree = RuleTree::empty();
    tree.add_stylesheet("p { color: red }", Origin::UserAgent);
    tree.add_stylesheet("div { color: green }", Origin::UserAgent);
    tree.add_stylesheet("span { color: blue }", Origin::Author);
    let orders: Vec<u32> = tree.style_rules.iter().map(|r| r.source_order).collect();
    assert_eq!(orders, vec![0, 1, 2]);
}

#[test]
fn add_stylesheet_dropped_selectors_do_not_consume_source_order() {
    // `div:hover` (pseudo-class, `NonTSPseudoClass`) is still unsupported
    // — dropped, `p` survives (was `div + p`: the
    // next-sibling combinator it used is now accepted, so this fixture
    // moved to a selector that remains genuinely unsupported — regression
    // intent unchanged: a dropped rule must not consume the
    // `source_order` counter).
    let mut tree = RuleTree::empty();
    tree.add_stylesheet("div:hover { color: red } p { color: blue }", Origin::Author);
    assert_eq!(tree.style_rules.len(), 1);
    assert_eq!(tree.style_rules[0].source_order, 0);
}

#[test]
fn build_rule_tree_produces_author_origin_for_dom_style_elements() {
    let doc = dom_with_style("p { color: red }");
    let tree = build_rule_tree(&doc);
    assert_eq!(tree.style_rules.len(), 1);
    assert_eq!(tree.style_rules[0].origin, Origin::Author);
}

#[test]
fn walk_style_elements_pub_visits_all_style_texts_in_document_order() {
    // Collect two sibling <style> elements in call order.
    let mut doc = TestDoc::new();
    let s1 = doc.push_element(0, "style", None);
    doc.push_text(s1, "p { color: red }");
    let s2 = doc.push_element(0, "style", None);
    doc.push_text(s2, "div { color: blue }");

    let mut collected: Vec<String> = Vec::new();
    super::walk_style_elements(&doc, |css| collected.push(css.to_string()));

    assert_eq!(collected.len(), 2);
    assert_eq!(collected[0], "p { color: red }");
    assert_eq!(collected[1], "div { color: blue }");
}

/// Style elements must be visited in document order even when an earlier
/// element is deeper than a later sibling. This order feeds the CSS
/// order-of-appearance cascade tie-break.
#[test]
fn walk_and_collect_preserves_document_order_across_mixed_sibling_descendant_depths() {
    use crate::property::PropertyValue;

    let mut doc = TestDoc::new();
    let section = doc.push_element(0, "section", None);
    let mid = doc.push_element(section, "mid", None);
    let style_a = doc.push_element(mid, "style", None);
    // A uses `color` — distinguishable from B's `background-color` below
    // so the build_rule_tree assertions can check *which* rule landed at
    // which source_order, not just that 2 rules exist.
    doc.push_text(style_a, "p { color: red }");

    let aside = doc.push_element(0, "aside", None); // later sibling of `section`
    let style_b = doc.push_element(aside, "style", None);
    doc.push_text(style_b, "div { background-color: blue }");

    // Entry point 1: raw text collection order via `walk_style_elements`.
    let mut collected: Vec<String> = Vec::new();
    super::walk_style_elements(&doc, |css| collected.push(css.to_string()));
    assert_eq!(collected.len(), 2);
    assert_eq!(collected[0], "p { color: red }");
    assert_eq!(collected[1], "div { background-color: blue }");

    // Entry point 2: `source_order` assigned via `build_rule_tree`, which
    // is the value that actually feeds the cascade tie-break — check it
    // too so a regression here is caught even if a future change routes
    // rule extraction through `build_rule_tree` without going through
    // the raw-text collection path in the same way. Distinguish A vs B
    // by declaration kind (Color vs BackgroundColor) rather than just
    // counting, so a swap is actually detected.
    let tree = build_rule_tree(&doc);
    assert_eq!(tree.style_rules.len(), 2);
    assert_eq!(tree.style_rules[0].source_order, 0);
    assert_eq!(tree.style_rules[1].source_order, 1);
    assert!(matches!(
        tree.style_rules[0].declarations()[0].value(),
        PropertyValue::Color(_)
    ));
    assert!(matches!(
        tree.style_rules[1].declarations()[0].value(),
        PropertyValue::BackgroundColor(_)
    ));
}

#[test]
fn style_inside_template_is_skipped_per_html_spec_inertness() {
    // <template> is inert under the HTML spec; its <style> must not
    // reach the cascade. Consistent with the invariant in
    // raikiri-html/src/sink.rs:315-318.
    let mut doc = TestDoc::new();
    let template = doc.push_element(0, "template", None);
    let style_in_template = doc.push_element(template, "style", None);
    doc.push_text(style_in_template, "p { color: red }");

    // The <style> outside <template> must still be collected (baseline).
    let style_outer = doc.push_element(0, "style", None);
    doc.push_text(style_outer, "div { color: blue }");

    let tree = build_rule_tree(&doc);
    // Only the outer <style>'s one rule (div{...}); skip the template's rule.
    assert_eq!(tree.style_rules.len(), 1);
    assert_eq!(tree.style_rules[0].source_order, 0);
}

// ── @page at-rule scaffolding ──
//
// Spec: CSS Paged Media Level 3, §4.3 "@page rule grammar"
// <https://www.w3.org/TR/css-page-3/#syntax-page-selector>
//
// Choose test bodies with properties already supported in property.rs
// (color / font-*). `crate::page::parse_page_declaration_block` accepts
// the `size` / `marks` / `bleed` descriptors using dedicated grammars
// (see the page_size_* / page_marks_* / page_bleed_* test groups below).
// Those tests check each descriptor's grammar boundaries — accepted and
// rejected values — not the general drop mechanism described next.
// For other unsupported properties, `parse_value` returns `None`, and
// the declaration is silently dropped (guarded below by
// page_body_unknown_property_is_dropped_declaration_survives).
// This branch in `PageDeclParser::parse_value` parallels the qualified-rule
// `crate::rule::DeclParser`: `.ok_or_else` converts `None` to `Err`, and
// cssparser's error recovery skips only that declaration. The code is a
// separate copy, however; the qualified-rule analogues
// (`crate::rule::tests::drops_invalid_property_and_value` /
// `crate::property::tests::unknown_property_returns_none`) do not test
// this `@page` branch.
//
// Margin-box at-rules (such as `@top-left { … }`, L3 §5.1) follow another
// path: they are not declarations, so they never reach `parse_value`.
// `PageDeclParser`'s `AtRuleParser::parse_prelude` compares the identifier
// against the sixteen-slot table. On a match, it parses the body as an
// ordinary declaration list and stores it in `PageRule::margin_box_rules`
// (see the "margin-box at-rules" test group below). An unknown nested
// at-rule name still produces an `Err` in `parse_prelude`, causing
// cssparser's error recovery to skip the whole block (guarded by
// page_unknown_nested_at_rule_body_is_skipped_declaration_survives below).
//
// NB: `margin` is now a supported author-scope property (expanded to four
// longhands at the exit of `parse_page_declaration_block`). Margin-box
// at-rule bodies undergo the same expansion because they reuse
// `parse_declaration_block`. The **geometric placement of margin-box
// slots** (the sixteen-slot layout of L3 §5) remains unimplemented;
// this group tests only parsing and declaration retention.

use crate::page::{
    PageBleed, PageMarginBoxSlot, PageMarks, PageOrientation, PagePseudo, PageSelector,
    PageSelectorEntry, PageSize, PageSizeKeyword,
};
use crate::{Atom, Length, PageRule};

/// Test helper — build a `PageSelector` with a single compound entry
/// containing exactly the given ident and pseudo-page list. Reduces the
/// verbosity of `PageSelector { entries: vec![PageSelectorEntry { ident,
/// pseudos, .. }] }` at every assertion site.
fn ps_single(ident: Option<Atom>, pseudos: Vec<PagePseudo>) -> PageSelector {
    PageSelector {
        entries: vec![PageSelectorEntry {
            ident,
            pseudos,
            ..PageSelectorEntry::default()
        }],
    }
}

fn page_rules(source: &str) -> Vec<PageRule> {
    let mut tree = RuleTree::empty();
    tree.add_stylesheet(source, Origin::Author);
    tree.page_rules
}

#[test]
fn page_default_selector_no_prelude() {
    // `@page { color: red }` → empty prelude represented as one default
    // entry (PageSelector is a Vec<Entry> shape, uniform for future
    // cascade iteration). There is one declaration; the
    // `page_rules(...)` helper hardcodes the Author origin.
    let rules = page_rules("@page { color: red }");
    assert_eq!(rules.len(), 1);
    assert_eq!(rules[0].selector, ps_single(None, vec![]));
    assert_eq!(rules[0].declarations.len(), 1);
    assert_eq!(rules[0].source_order, 0);
    assert_eq!(rules[0].origin, Origin::Author);
}

#[test]
fn page_pseudo_first() {
    let rules = page_rules("@page :first { color: red }");
    assert_eq!(rules.len(), 1);
    assert_eq!(rules[0].selector, ps_single(None, vec![PagePseudo::First]));
}

#[test]
fn page_pseudo_left_right_blank() {
    let rules = page_rules(
        "@page :left { color: red } \
         @page :right { color: red } \
         @page :blank { color: red }",
    );
    assert_eq!(rules.len(), 3);
    assert_eq!(rules[0].selector, ps_single(None, vec![PagePseudo::Left]));
    assert_eq!(rules[1].selector, ps_single(None, vec![PagePseudo::Right]));
    assert_eq!(rules[2].selector, ps_single(None, vec![PagePseudo::Blank]));
    // page_order is a counter independent of other rule kinds.
    assert_eq!(rules[0].source_order, 0);
    assert_eq!(rules[1].source_order, 1);
    assert_eq!(rules[2].source_order, 2);
}

#[test]
fn page_named_selector() {
    let rules = page_rules("@page my-cover { color: red }");
    assert_eq!(rules.len(), 1);
    assert_eq!(
        rules[0].selector,
        ps_single(Some(Atom::from("my-cover")), vec![])
    );
}

#[test]
fn page_multi_pseudo_is_accepted() {
    // L3 `<page-selector>` = `[ <ident-token>?
    // <pseudo-page>* ]!` permits any number of adjacent pseudo-pages.
    // Compound rule ("No whitespace is allowed between the productions
    // in `<page-selector>` or `<pseudo-page>`") — the input must be
    // written without whitespace between the two pseudos, hence
    // `:first:left`, not `:first :left`. The spaced form is pinned by
    // `page_multi_pseudo_with_whitespace_between_is_dropped` below.
    let rules = page_rules("@page :first:left { color: red }");
    assert_eq!(rules.len(), 1);
    assert_eq!(
        rules[0].selector,
        ps_single(None, vec![PagePseudo::First, PagePseudo::Left])
    );
}

#[test]
fn page_functional_pseudo_is_dropped() {
    // Neither CSS Paged Media L3 (anchor `#syntax-page-selector`) nor
    // the L4 Editor's Draft defines a functional pseudo such as
    // `:nth-page(...)`. raikiri-style therefore treats it as an unknown
    // pseudo and drops the entire rule. If it becomes a raikiri-local
    // extension, add a variant explicitly, reverse this guard test, and
    // record the scope in the human ledger.
    let rules = page_rules("@page :nth-page(2n+1) { color: red }");
    assert!(rules.is_empty());
}

#[test]
fn page_ident_plus_pseudo_is_accepted() {
    // L3 `<page-selector>` allows an ident followed
    // by pseudo-pages (`named:first`). No whitespace between them per
    // the compound rule — the spaced form (`named :first`) is dropped
    // by `page_named_with_whitespace_before_pseudo_is_dropped` below.
    let rules = page_rules("@page named:first { color: red }");
    assert_eq!(rules.len(), 1);
    assert_eq!(
        rules[0].selector,
        ps_single(Some(Atom::from("named")), vec![PagePseudo::First])
    );
}

#[test]
fn page_selector_list_with_comma_is_accepted() {
    // L3 `<page-selector-list>` = `<page-selector>#`
    // — a comma-separated list of compound page-selectors. Whitespace
    // around the `,` is spec-permitted (the list is not itself a
    // compound). Expected: one rule with two entries.
    let rules = page_rules("@page :first, :left { color: red }");
    assert_eq!(rules.len(), 1);
    assert_eq!(
        rules[0].selector,
        PageSelector {
            entries: vec![
                PageSelectorEntry {
                    ident: None,
                    pseudos: vec![PagePseudo::First],
                    ..PageSelectorEntry::default()
                },
                PageSelectorEntry {
                    ident: None,
                    pseudos: vec![PagePseudo::Left],
                    ..PageSelectorEntry::default()
                },
            ],
        }
    );
}

#[test]
fn page_unknown_pseudo_is_dropped() {
    // Drop `:cover`: it does not exist in the L3 grammar (spec anchor `#syntax-page-selector`).
    let rules = page_rules("@page :cover { color: red }");
    assert!(rules.is_empty());
}

#[test]
fn page_pseudo_is_case_insensitive() {
    // CSS keywords are ASCII case-insensitive (via `match_ignore_ascii_case!`).
    let rules = page_rules("@page :FIRST { color: red } @page :Left { color: red }");
    assert_eq!(rules.len(), 2);
    assert_eq!(rules[0].selector, ps_single(None, vec![PagePseudo::First]));
    assert_eq!(rules[1].selector, ps_single(None, vec![PagePseudo::Left]));
}

// ── Compound whitespace tightening ──
//
// CSS Paged Media L3 (anchor `#syntax-page-selector`) states: "No
// whitespace is allowed between the productions in `<page-selector>` or
// `<pseudo-page>` (similar to the rule for `<compound-selector>`)".
// Whitespace *around* the `,` separator of `<page-selector-list>` is
// allowed (the list is not a compound); that variant is exercised by
// `page_comma_list_with_whitespace_around_comma_is_accepted` below.

#[test]
fn page_whitespace_between_colon_and_pseudo_is_dropped() {
    // `@page : left` — whitespace between `:` and pseudo ident violates
    // the `<pseudo-page>` compound rule. Whole rule dropped.
    let rules = page_rules("@page : left { color: red }");
    assert!(rules.is_empty());
}

#[test]
fn page_multi_pseudo_with_whitespace_between_is_dropped() {
    // `@page :first :left` — whitespace between two pseudo-pages of the
    // same compound violates the `<page-selector>` compound rule.
    let rules = page_rules("@page :first :left { color: red }");
    assert!(rules.is_empty());
}

#[test]
fn page_named_with_whitespace_before_pseudo_is_dropped() {
    // `@page my-cover :first` — whitespace between ident and pseudo of
    // the same compound violates the `<page-selector>` compound rule.
    let rules = page_rules("@page my-cover :first { color: red }");
    assert!(rules.is_empty());
}

#[test]
fn page_comma_list_with_whitespace_around_comma_is_accepted() {
    // Whitespace around the `,` of `<page-selector-list>` is spec-
    // permitted (the list is not a compound). Both `,` and ` , ` and
    // `, ` MUST all yield the same two-entry result.
    for src in [
        "@page :first, :left { color: red }",
        "@page :first , :left { color: red }",
        "@page :first ,:left { color: red }",
    ] {
        let rules = page_rules(src);
        assert_eq!(rules.len(), 1, "source: {src:?}");
        assert_eq!(rules[0].selector.entries.len(), 2, "source: {src:?}");
    }
}

#[test]
fn page_ident_with_multi_pseudo_is_accepted() {
    // Ident + multiple pseudo-pages, all adjacent — the fullest shape
    // the L3 compound grammar permits.
    let rules = page_rules("@page named:first:left { color: red }");
    assert_eq!(rules.len(), 1);
    assert_eq!(
        rules[0].selector,
        ps_single(
            Some(Atom::from("named")),
            vec![PagePseudo::First, PagePseudo::Left]
        )
    );
}

#[test]
fn page_margin_box_at_rule_is_parsed_declaration_survives_alongside_it() {
    // A margin-box at-rule (`@top-left { … }`, CSS Paged Media Level 3
    // §5.1) is now recognized and its body stored on
    // `PageRule::margin_box_rules`, separate from the surrounding
    // `@page` block's own `declarations`. This pins both halves at
    // once: the ordinary `color: red` declaration lands in
    // `declarations` unaffected, and the nested `@top-left` block lands
    // in `margin_box_rules` with its own `content:` declaration parsed.
    use crate::property::PropertyValue;

    let rules = page_rules("@page :first { @top-left { content: 'x' } color: red }");
    assert_eq!(rules.len(), 1);
    assert_eq!(rules[0].declarations.len(), 1);
    assert_eq!(rules[0].margin_box_rules.len(), 1);
    assert_eq!(
        rules[0].margin_box_rules[0].slot,
        PageMarginBoxSlot::TopLeft
    );
    assert_eq!(rules[0].margin_box_rules[0].declarations.len(), 1);
    assert!(matches!(
        rules[0].margin_box_rules[0].declarations[0].value(),
        PropertyValue::Content(_)
    ));
}

// ── margin-box at-rules ──
//
// Spec: CSS Paged Media Level 3, §5.1 "At-rules for page-margin boxes"
// <https://www.w3.org/TR/css-page-3/#margin-at-rules>. Sixteen at-rules,
// each naming one page-margin box, no prelude, body = ordinary
// declaration list (§5.2 "Populating page-margin boxes" calls out
// `content:` specifically; §5.1 additionally restricts the body to
// "page-margin properties" — a restriction this parser does not
// enforce, see `PageMarginBoxRule`'s doc "Not filtered against the
// applicable-property list" section).

#[test]
fn page_margin_box_all_sixteen_slots_recognized() {
    // Every one of the sixteen margin-box idents (CSS Paged Media Level
    // 3 §5.1) round-trips to its matching `PageMarginBoxSlot` variant.
    let cases: &[(&str, PageMarginBoxSlot)] = &[
        ("top-left-corner", PageMarginBoxSlot::TopLeftCorner),
        ("top-left", PageMarginBoxSlot::TopLeft),
        ("top-center", PageMarginBoxSlot::TopCenter),
        ("top-right", PageMarginBoxSlot::TopRight),
        ("top-right-corner", PageMarginBoxSlot::TopRightCorner),
        ("right-top", PageMarginBoxSlot::RightTop),
        ("right-middle", PageMarginBoxSlot::RightMiddle),
        ("right-bottom", PageMarginBoxSlot::RightBottom),
        ("bottom-right-corner", PageMarginBoxSlot::BottomRightCorner),
        ("bottom-right", PageMarginBoxSlot::BottomRight),
        ("bottom-center", PageMarginBoxSlot::BottomCenter),
        ("bottom-left", PageMarginBoxSlot::BottomLeft),
        ("bottom-left-corner", PageMarginBoxSlot::BottomLeftCorner),
        ("left-bottom", PageMarginBoxSlot::LeftBottom),
        ("left-middle", PageMarginBoxSlot::LeftMiddle),
        ("left-top", PageMarginBoxSlot::LeftTop),
    ];
    for (ident, expected_slot) in cases {
        let source = format!("@page {{ @{ident} {{ content: 'x' }} }}");
        let rules = page_rules(&source);
        assert_eq!(rules.len(), 1, "source: {source}");
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            rules[0].margin_box_rules.len(),
            1,
            "ident {ident} was not recognized as a margin-box at-rule"
        );
        assert_eq!(rules[0].margin_box_rules[0].slot, *expected_slot);
    }
}

#[test]
fn page_margin_box_ident_is_case_insensitive() {
    // Matches every other at-rule/keyword production in this module —
    // `match_ignore_ascii_case!` in `PageDeclParser`'s `AtRuleParser`
    // impl.
    let rules = page_rules("@page { @TOP-LEFT { content: 'x' } }");
    assert_eq!(rules[0].margin_box_rules.len(), 1);
    assert_eq!(
        rules[0].margin_box_rules[0].slot,
        PageMarginBoxSlot::TopLeft
    );

    let rules = page_rules("@page { @Bottom-Right-Corner { content: 'x' } }");
    assert_eq!(rules[0].margin_box_rules.len(), 1);
    assert_eq!(
        rules[0].margin_box_rules[0].slot,
        PageMarginBoxSlot::BottomRightCorner
    );
}

#[test]
fn page_unknown_nested_at_rule_body_is_skipped_declaration_survives() {
    // regression guard: a nested at-rule name that is *not* one of the
    // sixteen margin-box idents (here `@foo`, standing in for e.g. a
    // stray `@media`) still falls through `PageDeclParser`'s
    // `AtRuleParser::parse_prelude` to `Err`, and cssparser's
    // error-recovery skips just that nested block. The sibling
    // `color: blue` declaration survives, and no `PageMarginBoxRule` is
    // produced.
    let rules = page_rules("@page { @foo { color: red } color: blue }");
    assert_eq!(rules.len(), 1);
    assert_eq!(rules[0].declarations.len(), 1);
    assert!(rules[0].margin_box_rules.is_empty());
}

#[test]
fn page_margin_box_non_slot_hyphenated_ident_is_dropped() {
    // `@top-middle` is not one of the sixteen spec idents (the top edge
    // has left/center/right, not "middle" — that name is reserved for
    // the *right*/*left* edges' vertical slots). Confirms the ident
    // match is exact, not a prefix/substring match.
    let rules = page_rules("@page { @top-middle { content: 'x' } color: red }");
    assert_eq!(rules[0].declarations.len(), 1);
    assert!(rules[0].margin_box_rules.is_empty());
}

#[test]
fn page_margin_box_non_empty_prelude_is_rejected() {
    // Margin-box at-rules take no prelude (`@top-left { <declaration-list> }`,
    // nothing between the ident and `{`). A stray token there —
    // `@top-left foo { … }` — is spec-invalid and the whole nested
    // block is dropped, same as an unrecognized ident. The sibling
    // declaration survives.
    let rules = page_rules("@page { @top-left foo { content: 'x' } color: red }");
    assert_eq!(rules[0].declarations.len(), 1);
    assert!(rules[0].margin_box_rules.is_empty());
}

#[test]
fn page_margin_box_statement_form_without_block_is_rejected() {
    // `@top-left;` (no `{ … }` block, terminated by `;` instead) is
    // spec-invalid — the margin-box grammar is `@top-left { <declaration-list> }`,
    // always with a block. `PageDeclParser`'s `AtRuleParser` impl does
    // not override `rule_without_block`, so cssparser's default (`Err`)
    // applies and the whole statement is dropped, same as any other
    // malformed nested at-rule. The sibling declaration survives.
    let rules = page_rules("@page { @top-left; color: red }");
    assert_eq!(rules[0].declarations.len(), 1);
    assert!(rules[0].margin_box_rules.is_empty());
}

#[test]
fn page_margin_box_content_counter_page_and_pages() {
    // `counter(page)` / `counter(pages)` — the automatic `page` counter
    // CSS Paged Media Level 3 §6.1 "Page-based counters"
    // (<https://www.w3.org/TR/css-page-3/#page-based-counters>) defines
    // ("A counter named page is automatically created and incremented
    // by 1 on every page of the document"); `pages` is its
    // document-total counterpart, defined further in the same section —
    // canonical use case a margin-box `content:` declaration exists for
    // (page-number headers/footers) — parses inside a margin-box body
    // via the same `ContentComponent` grammar
    // `crate::property::parse_content` already implements for ordinary
    // style rules.
    use crate::property::{ContentComponent, CounterStyle, PropertyValue};
    use smol_str::SmolStr;

    let rules =
        page_rules("@page { @bottom-center { content: counter(page) \" / \" counter(pages) } }");
    assert_eq!(rules[0].margin_box_rules.len(), 1);
    let decl = &rules[0].margin_box_rules[0].declarations[0];
    let items = match decl.value() {
        PropertyValue::Content(items) => items,
        // cov:ignore: defensive-only arm — `content: counter(page) " / "
        // counter(pages)` always parses to `PropertyValue::Content`, so
        // this panic is unreachable while the test passes.
        other => panic!("expected PropertyValue::Content, got {other:?}"),
    };
    assert_eq!(
        (**items).clone(),
        vec![
            ContentComponent::Counter {
                name: SmolStr::new("page"),
                style: CounterStyle::Decimal,
            },
            ContentComponent::Literal(SmolStr::new(" / ")),
            ContentComponent::Counter {
                name: SmolStr::new("pages"),
                style: CounterStyle::Decimal,
            },
        ]
    );
}

#[test]
fn page_margin_box_body_is_not_filtered_at_parse_time() {
    // CSS Paged Media Level 3 §5.1 states "The margin at-rules can only
    // contain page-margin properties" — but this parser does not
    // enforce that restriction (see `PageMarginBoxRule`'s doc "Not filtered
    // against the applicable-property list" section), so a property
    // with no obvious margin-box meaning (`color` here) still parses
    // and is kept, same as `content:`. This also confirms the body
    // reuses `crate::rule::parse_declaration_block`'s shorthand
    // expansion (`margin:` here expands to 4 longhands, matching
    // `page_declaration_block_never_emits_shorthand_keys`'s guarantee
    // for the outer `@page` body), and that none of it leaks into the
    // surrounding `@page` block's own `declarations`.
    use crate::property::PropertyValue;

    let rules =
        page_rules("@page { @top-left { content: 'x'; color: red; margin: 1px 2px 3px 4px } }");
    assert_eq!(rules[0].margin_box_rules.len(), 1);
    assert!(rules[0].declarations.is_empty());
    let decls = &rules[0].margin_box_rules[0].declarations;
    assert_eq!(decls.len(), 6, "content + color + 4 margin longhands");
    for decl in decls {
        assert!(!matches!(decl.value(), PropertyValue::Margin(_)));
    }
}

#[test]
fn page_margin_box_duplicate_slot_kept_as_separate_entries() {
    // Two `@top-left` blocks in one `@page` rule are both kept, in
    // source order — cascade winner selection across duplicate slots is
    // future scope (`PageMarginBoxRule`'s doc, "Scope" section), mirroring
    // `PageRule::size_declarations`'s own "kept in source order"
    // treatment of duplicate `size:` declarations.
    let rules = page_rules("@page { @top-left { content: 'a' } @top-left { content: 'b' } }");
    assert_eq!(rules[0].margin_box_rules.len(), 2);
    assert_eq!(
        rules[0].margin_box_rules[0].slot,
        PageMarginBoxSlot::TopLeft
    );
    assert_eq!(
        rules[0].margin_box_rules[1].slot,
        PageMarginBoxSlot::TopLeft
    );
}

#[test]
fn page_margin_box_rules_preserve_source_order_interleaved_with_declarations() {
    // Margin-box at-rules interleaved with ordinary declarations and
    // `size`/`marks`/`bleed` descriptors all land in their own field,
    // each in its own source order — none of the four lists disturbs
    // another's ordering or count.
    let rules = page_rules(
        "@page { \
            @top-left { content: counter(page) } \
            color: red; \
            size: A4; \
            @bottom-right { content: 'end' } \
            marks: crop; \
         }",
    );
    assert_eq!(rules.len(), 1);
    assert_eq!(rules[0].declarations.len(), 1);
    assert_eq!(rules[0].size_declarations.len(), 1);
    assert_eq!(rules[0].marks_declarations.len(), 1);
    assert_eq!(rules[0].margin_box_rules.len(), 2);
    assert_eq!(
        rules[0].margin_box_rules[0].slot,
        PageMarginBoxSlot::TopLeft
    );
    assert_eq!(
        rules[0].margin_box_rules[1].slot,
        PageMarginBoxSlot::BottomRight
    );
}

#[test]
fn page_source_order_independent_from_style_rules() {
    // page_rules use a source_order counter independent of style_rules.
    // Also check that all rules inherit the origin (Author) of the same
    // add_stylesheet call (`PageRule.origin`, groundwork for cascading).
    let mut tree = RuleTree::empty();
    tree.add_stylesheet(
        "p { color: red } \
         @page :first { color: red } \
         div { color: blue } \
         @page :left { color: red }",
        Origin::Author,
    );
    assert_eq!(tree.style_rules.len(), 2);
    assert_eq!(tree.style_rules[0].source_order, 0);
    assert_eq!(tree.style_rules[1].source_order, 1);
    assert_eq!(tree.page_rules.len(), 2);
    assert_eq!(tree.page_rules[0].source_order, 0);
    assert_eq!(tree.page_rules[0].origin, Origin::Author);
    assert_eq!(tree.page_rules[1].source_order, 1);
    assert_eq!(tree.page_rules[1].origin, Origin::Author);
    assert_eq!(tree.rules().len(), 4);
    assert_eq!(tree.rules()[0].source_order, 0);
    assert_eq!(tree.rules()[0].kind, CssRuleKind::Style { index: 0 });
    assert_eq!(tree.rules()[1].source_order, 1);
    assert_eq!(tree.rules()[1].kind, CssRuleKind::Page { index: 0 });
    assert_eq!(tree.rules()[2].source_order, 2);
    assert_eq!(tree.rules()[2].kind, CssRuleKind::Style { index: 1 });
    assert_eq!(tree.rules()[3].source_order, 3);
    assert_eq!(tree.rules()[3].kind, CssRuleKind::Page { index: 1 });
}

#[test]
fn cross_kind_rule_order_preserves_stylesheet_order() {
    let mut tree = RuleTree::empty();
    tree.add_stylesheet(
        "p { color: red } @media print { p { color: blue } } \
         @page :first { color: green } div { color: black }",
        Origin::Author,
    );
    assert_eq!(
        tree.rules()
            .iter()
            .map(|rule| rule.kind.clone())
            .collect::<Vec<_>>(),
        vec![
            CssRuleKind::Style { index: 0 },
            CssRuleKind::AtRule { index: 0 },
            CssRuleKind::Page { index: 0 },
            CssRuleKind::Style { index: 1 },
        ]
    );
    assert_eq!(tree.opaque_at_rules()[0].source_order, 1);
}

#[test]
fn cross_kind_rule_order_continues_across_stylesheets() {
    let mut tree = RuleTree::empty();
    tree.add_stylesheet("p { color: red } @future { x: y }", Origin::Author);
    tree.add_stylesheet(
        "@page :first { color: blue } div { color: green }",
        Origin::User,
    );

    assert_eq!(
        tree.rules()
            .iter()
            .map(|rule| rule.source_order)
            .collect::<Vec<_>>(),
        vec![0, 1, 2, 3]
    );
    assert_eq!(tree.rules()[2].origin, Origin::User);
    assert_eq!(tree.opaque_at_rules()[0].source_order, 1);
}

#[test]
fn page_source_order_monotonic_across_add_stylesheet_calls() {
    // page_order continues across multiple add_stylesheet calls.
    // Check that each rule's origin matches its add_stylesheet argument:
    // `PageRule.origin` retains the per-call origin so a per-origin
    // cascade can be built without re-indexing (groundwork for CSS
    // Cascading L4
    // <https://www.w3.org/TR/css-cascade-4/#cascade-origin>).
    let mut tree = RuleTree::empty();
    tree.add_stylesheet("@page :first { color: red }", Origin::UserAgent);
    tree.add_stylesheet("@page :left { color: blue }", Origin::Author);
    assert_eq!(tree.page_rules.len(), 2);
    assert_eq!(tree.page_rules[0].source_order, 0);
    assert_eq!(tree.page_rules[0].origin, Origin::UserAgent);
    assert_eq!(tree.page_rules[1].source_order, 1);
    assert_eq!(tree.page_rules[1].origin, Origin::Author);
}

#[test]
fn page_body_size_marks_bleed_all_parsed_together() {
    // `size` / `marks` / `bleed` each have a dedicated grammar
    // (`crate::page::parse_page_size_value` /
    // `parse_page_marks_value` / `parse_page_bleed_value`) — this test
    // pins that all three parse independently and land in their own
    // `*_declarations` field when they appear together in one `@page`
    // block (the `page_size_*` / `page_marks_*` / `page_bleed_*` test
    // groups elsewhere in this module each exercise only one descriptor
    // in isolation).
    //
    // The fixture also throws in a margin-box at-rule (`@top-left { … }`,
    // CSS Paged Media Level 3 §5.1) to confirm it lands in its own
    // `margin_box_rules` field alongside the three descriptors, rather
    // than in `declarations` or disturbing their counts — the
    // "margin-box at-rules" test group below covers that field's
    // grammar/shape in depth; this is the "all four coexist in one
    // block" cross-check.
    let rules =
        page_rules("@page { size: A4; marks: crop; bleed: 6pt; @top-left { content: 'x' } }");
    assert_eq!(rules.len(), 1);
    assert_eq!(rules[0].selector, ps_single(None, vec![]));
    assert!(rules[0].declarations.is_empty());
    assert_eq!(rules[0].margin_box_rules.len(), 1);
    assert_eq!(
        rules[0].margin_box_rules[0].slot,
        PageMarginBoxSlot::TopLeft
    );
    assert_eq!(rules[0].size_declarations.len(), 1);
    assert_eq!(
        rules[0].size_declarations[0].value,
        PageSize::Named {
            keyword: Some(PageSizeKeyword::A4),
            orientation: None,
        }
    );
    assert!(!rules[0].size_declarations[0].important);
    assert_eq!(rules[0].marks_declarations.len(), 1);
    assert_eq!(
        rules[0].marks_declarations[0].value,
        PageMarks::Marks {
            crop: true,
            cross: false,
        }
    );
    assert!(!rules[0].marks_declarations[0].important);
    assert_eq!(rules[0].bleed_declarations.len(), 1);
    assert_eq!(
        rules[0].bleed_declarations[0].value,
        PageBleed::Length(Length::Pt(6.0))
    );
    assert!(!rules[0].bleed_declarations[0].important);
}

#[test]
fn page_declaration_block_never_emits_shorthand_keys() {
    // Page declaration parsing expands the supported shorthands so that
    // consumers do not observe raw shorthand values.
    use crate::property::PropertyValue;

    let rules = page_rules("@page { margin: 1cm 2cm 3cm 4cm; outline: auto 2px red }");
    assert_eq!(rules.len(), 1);
    assert_eq!(rules[0].declarations.len(), 7);
    for decl in &rules[0].declarations {
        assert!(!matches!(
            decl.value(),
            PropertyValue::Margin(_) | PropertyValue::Outline(_)
        ));
    }
}

// ── `size` descriptor grammar ──
//
// Spec: CSS Paged Media Level 3, §7.1 "Page size: the size property"
// <https://www.w3.org/TR/css-page-3/#page-size-prop>. Grammar:
// `<length>{1,2} | auto | [ <page-size> || [ portrait | landscape ] ]`.

#[test]
fn page_size_auto() {
    let rules = page_rules("@page { size: auto }");
    assert_eq!(rules[0].size_declarations.len(), 1);
    assert_eq!(rules[0].size_declarations[0].value, PageSize::Auto);
}

#[test]
fn page_size_two_lengths() {
    let rules = page_rules("@page { size: 210mm 297mm }");
    assert_eq!(
        rules[0].size_declarations[0].value,
        PageSize::Lengths {
            width: Length::Mm(210.0),
            height: Length::Mm(297.0),
        }
    );
}

#[test]
fn page_size_single_length_sets_both_width_and_height() {
    // "If only one length value is specified, it sets both the width
    // and height of the page box (i.e., the box is a square)."
    let rules = page_rules("@page { size: 10em }");
    assert_eq!(
        rules[0].size_declarations[0].value,
        PageSize::Lengths {
            width: Length::Em(10.0),
            height: Length::Em(10.0),
        }
    );
}

#[test]
fn page_size_unitless_zero() {
    // CSS Values 3 §5 unitless-zero clause — reaches `size` the same
    // way it reaches every other `<length>` consumer in this crate.
    let rules = page_rules("@page { size: 0 }");
    assert_eq!(
        rules[0].size_declarations[0].value,
        PageSize::Lengths {
            width: Length::Px(0.0),
            height: Length::Px(0.0)
        }
    );
}

#[test]
fn page_size_percentage_rejected() {
    // Grammar alternative 1 is `<length>`, not `<length-percentage>` —
    // `%` is outside the `size` descriptor grammar entirely.
    let rules = page_rules("@page { size: 50% }");
    assert!(rules[0].size_declarations.is_empty());
}

#[test]
fn page_size_negative_length_rejected() {
    // "Negative lengths are illegal."
    let rules = page_rules("@page { size: -10px }");
    assert!(rules[0].size_declarations.is_empty());
}

#[test]
fn page_size_three_lengths_rejected() {
    // Grammar caps at `{1,2}` — a third length is trailing garbage that
    // must drop the whole declaration, not silently truncate to the
    // first two.
    let rules = page_rules("@page { size: 10px 20px 30px }");
    assert!(rules[0].size_declarations.is_empty());
}

#[test]
fn page_size_second_length_negative_drops_whole_declaration() {
    // Non-obvious path: the second length fails
    // `parse_non_negative_length` and `try_parse` rewinds, so
    // `parse_page_size_value` falls through to the "only one length was
    // authored" arm (`Lengths { width: 10px, height: 10px }`) — but the
    // leftover `-5px` tokens are still unconsumed, so
    // `PageDeclParser::parse_value`'s `expect_exhausted` call must
    // reject the whole declaration rather than silently accepting that
    // truncated-to-one-length reading.
    let rules = page_rules("@page { size: 10px -5px }");
    assert!(rules[0].size_declarations.is_empty());
}

#[test]
fn page_size_named_keyword_alone() {
    let rules = page_rules("@page { size: A4 }");
    assert_eq!(
        rules[0].size_declarations[0].value,
        PageSize::Named {
            keyword: Some(PageSizeKeyword::A4),
            orientation: None,
        }
    );
}

#[test]
fn page_size_orientation_alone() {
    // `||` combinator: the `<page-size>` half is optional as long as
    // `portrait | landscape` is present.
    let rules = page_rules("@page { size: landscape }");
    assert_eq!(
        rules[0].size_declarations[0].value,
        PageSize::Named {
            keyword: None,
            orientation: Some(PageOrientation::Landscape),
        }
    );
}

#[test]
fn page_size_keyword_and_orientation_either_order() {
    // `||` combinator: both sub-components may appear, in either source
    // order (spec examples this with `A4 landscape`).
    let forward = page_rules("@page { size: A4 landscape }");
    let backward = page_rules("@page { size: landscape A4 }");
    let expected = PageSize::Named {
        keyword: Some(PageSizeKeyword::A4),
        orientation: Some(PageOrientation::Landscape),
    };
    assert_eq!(forward[0].size_declarations[0].value, expected);
    assert_eq!(backward[0].size_declarations[0].value, expected);
}

#[test]
fn page_size_duplicate_keyword_rejected() {
    // Each of `<page-size>` / `portrait|landscape` may appear at most
    // once under `||` — a second page-size keyword is not a second
    // valid alternative, it's trailing garbage.
    let rules = page_rules("@page { size: A4 A3 }");
    assert!(rules[0].size_declarations.is_empty());
}

#[test]
fn page_size_duplicate_orientation_rejected() {
    let rules = page_rules("@page { size: A4 landscape portrait }");
    assert!(rules[0].size_declarations.is_empty());
}

#[test]
fn page_size_important() {
    let rules = page_rules("@page { size: A4 !important }");
    assert_eq!(rules[0].size_declarations.len(), 1);
    assert!(rules[0].size_declarations[0].important);
}

// Runs the `value()` accessor body: doctests aren't covered by this
// repo's coverage toolchain, so the compile-fail check on
// `PageSizeDeclaration`'s struct doc doesn't exercise it. Every other
// test above reaches into the same-crate `pub(crate)` field directly,
// which never calls the accessor at all. Asserted against a concrete
// expected value (not `decl.value`) so this can't degrade into a
// tautological "the accessor returns the field" check.
#[test]
fn page_size_declaration_value_accessor_matches_the_field() {
    let rules = page_rules("@page { size: A4 landscape }");
    let decl = rules[0].size_declarations[0];
    assert_eq!(
        decl.value(),
        PageSize::Named {
            keyword: Some(PageSizeKeyword::A4),
            orientation: Some(PageOrientation::Landscape),
        }
    );
}

#[test]
fn page_size_is_case_insensitive() {
    // CSS keywords are ASCII case-insensitive, as in
    // `page_pseudo_is_case_insensitive`. `auto` uses
    // `expect_ident_matching` (internally `eq_ignore_ascii_case`), while
    // `<page-size>` and orientation keywords use
    // `match_ignore_ascii_case!`; neither path distinguishes case.
    let auto = page_rules("@page { size: AUTO }");
    assert_eq!(auto[0].size_declarations[0].value, PageSize::Auto);

    let named = page_rules("@page { size: A4 LANDSCAPE }");
    assert_eq!(
        named[0].size_declarations[0].value,
        PageSize::Named {
            keyword: Some(PageSizeKeyword::A4),
            orientation: Some(PageOrientation::Landscape),
        }
    );

    // Spec writes this one as `JIS-B5` (mixed case) — lowercase must
    // still match.
    let jis = page_rules("@page { size: jis-b5 }");
    assert_eq!(
        jis[0].size_declarations[0].value,
        PageSize::Named {
            keyword: Some(PageSizeKeyword::JisB5),
            orientation: None,
        }
    );
}

#[test]
fn page_size_all_10_keyword_variants_accepted() {
    // All 10 `<page-size>` alternatives smoke-tested individually (arm
    // deletion regression detector) — same pattern as
    // `border_style_all_10_variants_accepted` in property.rs.
    fn named(source: &str, keyword: PageSizeKeyword) {
        let rules = page_rules(&format!("@page {{ size: {source} }}"));
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            rules[0].size_declarations[0].value,
            PageSize::Named {
                keyword: Some(keyword),
                orientation: None,
            },
            "size: {source}"
        );
    }
    named("A5", PageSizeKeyword::A5);
    named("A4", PageSizeKeyword::A4);
    named("A3", PageSizeKeyword::A3);
    named("B5", PageSizeKeyword::B5);
    named("B4", PageSizeKeyword::B4);
    named("JIS-B5", PageSizeKeyword::JisB5);
    named("JIS-B4", PageSizeKeyword::JisB4);
    named("letter", PageSizeKeyword::Letter);
    named("legal", PageSizeKeyword::Legal);
    named("ledger", PageSizeKeyword::Ledger);
}

#[test]
fn page_size_both_orientation_variants_accepted() {
    let portrait = page_rules("@page { size: portrait }");
    assert_eq!(
        portrait[0].size_declarations[0].value,
        PageSize::Named {
            keyword: None,
            orientation: Some(PageOrientation::Portrait),
        }
    );
    let landscape = page_rules("@page { size: landscape }");
    assert_eq!(
        landscape[0].size_declarations[0].value,
        PageSize::Named {
            keyword: None,
            orientation: Some(PageOrientation::Landscape),
        }
    );
}

#[test]
fn page_body_unknown_property_is_dropped_declaration_survives() {
    // regression guard: `PageDeclParser::parse_value`'s ordinary-property
    // arm dispatches to `crate::property::parse_value` and turns a
    // `None` return into an `Err` (`.ok_or_else`) for that one
    // declaration; cssparser's error recovery then skips just that
    // declaration, leaving the ones before and after it alone. This
    // pins the actually-unknown-property-name case specifically —
    // trailing garbage *after* a known property's value is a different
    // failure inside the same arm (`expect_exhausted` rejecting a
    // successful `parse_value` result), covered separately by
    // `page_body_ordinary_property_trailing_garbage_drops_declaration`
    // below.
    //
    // `cursor` is used as the unsupported-property canary, matching the
    // choice already made by `crate::rule::tests::drops_invalid_property_and_value`
    // and `crate::property::tests::unknown_property_returns_none` —
    // relocate to a different still-unimplemented property name if
    // `cursor` gains support.
    let rules = page_rules("@page { cursor: pointer; color: red }");
    assert_eq!(rules.len(), 1);
    assert_eq!(rules[0].declarations.len(), 1);
}

#[test]
fn page_body_ordinary_property_trailing_garbage_drops_declaration() {
    // `PageDeclParser::parse_value`'s ordinary-property arm (the
    // `crate::property::parse_value` dispatch, distinct from the `size`
    // arm) has its own `expect_exhausted` guard — this exercises *that*
    // copy specifically (the `size` arm's twin is already covered by
    // `page_size_three_lengths_rejected` and friends): `color: red` on
    // its own parses cleanly, but trailing garbage after the value must
    // still drop the whole declaration, exactly like the qualified-rule
    // path's `crate::rule::DeclParser` does.
    let rules = page_rules("@page { color: red garbage }");
    assert_eq!(rules.len(), 1);
    assert!(rules[0].declarations.is_empty());
}

#[test]
fn page_size_duplicate_declarations_kept_in_source_order() {
    // `PageRule::size_declarations` mirrors `PageRule::declarations` —
    // every declaration is kept in source order rather than reduced to
    // a single winner at parse time (see that field's doc for why).
    let rules = page_rules("@page { size: A4 !important; size: A5 }");
    assert_eq!(rules[0].size_declarations.len(), 2);
    assert_eq!(
        rules[0].size_declarations[0].value,
        PageSize::Named {
            keyword: Some(PageSizeKeyword::A4),
            orientation: None,
        }
    );
    assert!(rules[0].size_declarations[0].important);
    assert_eq!(
        rules[0].size_declarations[1].value,
        PageSize::Named {
            keyword: Some(PageSizeKeyword::A5),
            orientation: None,
        }
    );
    assert!(!rules[0].size_declarations[1].important);
}

// ── `marks` descriptor grammar ──
//
// Spec: CSS Paged Media Level 3, §7.2 "Crop and Registration Marks: the
// marks property" <https://www.w3.org/TR/css-page-3/#marks>. Grammar:
// `none | [ crop || cross ]`.

#[test]
fn page_marks_none() {
    let rules = page_rules("@page { marks: none }");
    assert_eq!(rules[0].marks_declarations.len(), 1);
    assert_eq!(rules[0].marks_declarations[0].value, PageMarks::None);
}

#[test]
fn page_marks_crop_alone() {
    // `||` combinator: `cross` is optional as long as `crop` is present.
    let rules = page_rules("@page { marks: crop }");
    assert_eq!(
        rules[0].marks_declarations[0].value,
        PageMarks::Marks {
            crop: true,
            cross: false,
        }
    );
}

#[test]
fn page_marks_cross_alone() {
    // `||` combinator: `crop` is optional as long as `cross` is present.
    let rules = page_rules("@page { marks: cross }");
    assert_eq!(
        rules[0].marks_declarations[0].value,
        PageMarks::Marks {
            crop: false,
            cross: true,
        }
    );
}

#[test]
fn page_marks_crop_and_cross_either_order() {
    // `||` combinator: both sub-components may appear, in either source
    // order.
    let forward = page_rules("@page { marks: crop cross }");
    let backward = page_rules("@page { marks: cross crop }");
    let expected = PageMarks::Marks {
        crop: true,
        cross: true,
    };
    assert_eq!(forward[0].marks_declarations[0].value, expected);
    assert_eq!(backward[0].marks_declarations[0].value, expected);
}

#[test]
fn page_marks_duplicate_crop_rejected() {
    // Each of `crop` / `cross` may appear at most once under `||` — a
    // second `crop` is trailing garbage, not a second valid alternative.
    let rules = page_rules("@page { marks: crop crop }");
    assert!(rules[0].marks_declarations.is_empty());
}

#[test]
fn page_marks_duplicate_cross_rejected() {
    // Mirrors `page_marks_duplicate_crop_rejected` for the other `||`
    // alternative — `parse_page_marks_value`'s `!cross` guard must reject
    // a second `cross` the same way its `!crop` guard rejects a second
    // `crop`.
    let rules = page_rules("@page { marks: cross cross }");
    assert!(rules[0].marks_declarations.is_empty());
}

#[test]
fn page_marks_unknown_keyword_rejected() {
    let rules = page_rules("@page { marks: foo }");
    assert!(rules[0].marks_declarations.is_empty());
}

#[test]
fn page_marks_none_with_trailing_garbage_rejected() {
    // `none` is a distinct top-level alternative, not combinable with
    // `crop`/`cross` — trailing garbage after it drops the whole
    // declaration (`expect_exhausted` in `PageDeclParser::parse_value`).
    let rules = page_rules("@page { marks: none crop }");
    assert!(rules[0].marks_declarations.is_empty());
}

#[test]
fn page_marks_important() {
    let rules = page_rules("@page { marks: crop !important }");
    assert_eq!(rules[0].marks_declarations.len(), 1);
    assert!(rules[0].marks_declarations[0].important);
}

#[test]
fn page_marks_is_case_insensitive() {
    let none = page_rules("@page { marks: NONE }");
    assert_eq!(none[0].marks_declarations[0].value, PageMarks::None);

    let mixed = page_rules("@page { marks: Crop Cross }");
    assert_eq!(
        mixed[0].marks_declarations[0].value,
        PageMarks::Marks {
            crop: true,
            cross: true,
        }
    );
}

// Runs the `value()` accessor body — see
// `page_size_declaration_value_accessor_matches_the_field`'s doc for why
// this needs its own dedicated test (direct field access elsewhere never
// calls the accessor).
#[test]
fn page_marks_declaration_value_accessor_matches_the_field() {
    let rules = page_rules("@page { marks: crop cross }");
    let decl = rules[0].marks_declarations[0];
    assert_eq!(
        decl.value(),
        PageMarks::Marks {
            crop: true,
            cross: true,
        }
    );
}

// ── `bleed` descriptor grammar ──
//
// Spec: CSS Paged Media Level 3, §7.3 "Bleed Area: the bleed property"
// <https://www.w3.org/TR/css-page-3/#bleed>. Grammar: `auto | <length>`.

#[test]
fn page_bleed_auto() {
    let rules = page_rules("@page { bleed: auto }");
    assert_eq!(rules[0].bleed_declarations.len(), 1);
    assert_eq!(rules[0].bleed_declarations[0].value, PageBleed::Auto);
}

#[test]
fn page_bleed_length() {
    let rules = page_rules("@page { bleed: 6pt }");
    assert_eq!(
        rules[0].bleed_declarations[0].value,
        PageBleed::Length(Length::Pt(6.0))
    );
}

#[test]
fn page_bleed_negative_length_accepted() {
    // Unlike `size`'s `<length>` alternative ("Negative lengths are
    // illegal"), `bleed`'s explicitly permits negative values: "Values
    // may be negative, but there may be implementation-specific
    // limits."
    let rules = page_rules("@page { bleed: -6pt }");
    assert_eq!(
        rules[0].bleed_declarations[0].value,
        PageBleed::Length(Length::Pt(-6.0))
    );
}

#[test]
fn page_bleed_percentage_rejected() {
    // Grammar is `<length>`, not `<length-percentage>` — `%` is outside
    // the `bleed` descriptor grammar entirely.
    let rules = page_rules("@page { bleed: 50% }");
    assert!(rules[0].bleed_declarations.is_empty());
}

#[test]
fn page_bleed_unknown_keyword_rejected() {
    let rules = page_rules("@page { bleed: foo }");
    assert!(rules[0].bleed_declarations.is_empty());
}

#[test]
fn page_bleed_trailing_garbage_drops_whole_declaration() {
    // Unlike `page_bleed_unknown_keyword_rejected` (nothing in the
    // grammar matches at all), this exercises the `bleed` arm's own
    // `expect_exhausted` guard in `PageDeclParser::parse_value`: `6pt`
    // parses cleanly as a valid `<length>`, but leftover tokens after it
    // must still drop the whole declaration rather than silently
    // truncating to the successfully-parsed prefix.
    let rules = page_rules("@page { bleed: 6pt garbage }");
    assert!(rules[0].bleed_declarations.is_empty());
}

#[test]
fn page_bleed_important() {
    let rules = page_rules("@page { bleed: 6pt !important }");
    assert_eq!(rules[0].bleed_declarations.len(), 1);
    assert!(rules[0].bleed_declarations[0].important);
}

#[test]
fn page_bleed_is_case_insensitive() {
    let rules = page_rules("@page { bleed: AUTO }");
    assert_eq!(rules[0].bleed_declarations[0].value, PageBleed::Auto);
}

// Runs the `value()` accessor body — see
// `page_size_declaration_value_accessor_matches_the_field`'s doc for why
// this needs its own dedicated test.
#[test]
fn page_bleed_declaration_value_accessor_matches_the_field() {
    let rules = page_rules("@page { bleed: 6pt }");
    let decl = rules[0].bleed_declarations[0];
    assert_eq!(decl.value(), PageBleed::Length(Length::Pt(6.0)));
}

#[test]
fn leading_charset_is_retained_despite_cssparser_special_case() {
    let mut tree = RuleTree::empty();
    tree.add_stylesheet(
        " /* comment */ @ChArSeT \"utf-8\"; p { color: red }",
        Origin::Author,
    );

    assert_eq!(tree.opaque_at_rules().len(), 1);
    assert_eq!(tree.opaque_at_rules()[0].name, "charset");
    assert_eq!(tree.opaque_at_rules()[0].prelude, " \"utf-8\"");
    assert_eq!(tree.rules()[0].kind, CssRuleKind::AtRule { index: 0 });
    assert_eq!(tree.rules()[1].kind, CssRuleKind::Style { index: 0 });
}

#[test]
fn escaped_leading_charset_name_is_retained() {
    let mut tree = RuleTree::empty();
    tree.add_stylesheet(r#"@ch\61 rset "utf-8"; p { color: red }"#, Origin::Author);

    assert_eq!(tree.opaque_at_rules().len(), 1);
    assert_eq!(tree.opaque_at_rules()[0].name, "charset");
    assert_eq!(tree.rules()[1].kind, CssRuleKind::Style { index: 0 });
}

#[test]
fn namespace_at_rules_feed_prefixed_and_default_selectors() {
    let mut tree = RuleTree::empty();
    tree.add_stylesheet(
        "@namespace svg url(http://www.w3.org/2000/svg); \
         @namespace url(http://www.w3.org/1999/xhtml); \
         svg|a { color: red } a { color: blue }",
        Origin::Author,
    );

    assert_eq!(tree.style_rules.len(), 2);
    assert_eq!(tree.opaque_at_rules().len(), 2);
    assert_eq!(tree.opaque_at_rules()[0].name, "namespace");
    assert_eq!(tree.opaque_at_rules()[1].name, "namespace");
}

#[test]
fn valid_at_rules_are_retained_and_media_rules_are_indexed() {
    let mut tree = RuleTree::empty();
    tree.add_stylesheet(
        "@media print { p { color: red } @nested feature; } \
         @supports (display: block) { p { color: green } } \
         p { color: blue }",
        Origin::Author,
    );

    assert_eq!(tree.page_rules.len(), 0);
    assert_eq!(tree.style_rules.len(), 2);
    assert_eq!(tree.media_rules.len(), 1);
    assert_eq!(tree.media_rules[0].rule.source_order, 0);
    assert_eq!(tree.style_rules[0].source_order, 1);
    assert_eq!(tree.style_rules[1].source_order, 2);
    assert_eq!(tree.opaque_at_rules().len(), 2);
    assert_eq!(tree.opaque_at_rules()[0].name, "media");
    assert_eq!(tree.opaque_at_rules()[1].name, "supports");
    assert_eq!(tree.opaque_at_rules()[0].source_order, 0);
    assert_eq!(tree.opaque_at_rules()[1].source_order, 2);

    let media = &tree.opaque_at_rules()[0];
    assert_eq!(
        media.body.as_block(),
        Some(" p { color: red } @nested feature; ")
    );
    assert_eq!(media.children().len(), 2);
    assert!(matches!(media.children()[0], RuleNode::Qualified(_)));
    let RuleNode::AtRule(nested) = &media.children()[1] else {
        panic!("nested at-rule was not retained");
    };
    assert_eq!(nested.name, "nested");
    assert_eq!(nested.body, AtRuleBody::Statement);
    assert_eq!(nested.source_order, 1);
}

#[test]
fn supports_exposes_supported_qualified_rules_but_keeps_opaque_record() {
    let mut tree = RuleTree::empty();
    tree.add_stylesheet(
        "@supports (display: block) { p { color: red } } \
         @supports (display: definitely-unsupported) { p { color: blue } } \
         @supports not (display: definitely-unsupported) { p { color: green } }",
        Origin::Author,
    );

    assert_eq!(tree.style_rules.len(), 2);
    assert_eq!(tree.opaque_at_rules().len(), 3);
    assert_eq!(tree.opaque_at_rules()[0].name, "supports");
    assert_eq!(tree.opaque_at_rules()[1].name, "supports");
    assert_eq!(tree.opaque_at_rules()[2].name, "supports");
}

#[test]
fn media_style_order_continues_across_stylesheets() {
    let mut tree = RuleTree::empty();
    tree.add_stylesheet("@media print { p { color: red } }", Origin::Author);
    tree.add_stylesheet("p { color: blue }", Origin::Author);

    assert_eq!(tree.media_rules.len(), 1);
    assert_eq!(tree.media_rules[0].rule.source_order, 0);
    assert_eq!(tree.style_rules.len(), 1);
    assert_eq!(tree.style_rules[0].source_order, 1);
}

#[test]
fn opaque_at_rule_preserves_statement_prelude_and_source_text() {
    let mut tree = RuleTree::empty();
    tree.add_stylesheet("@future /* keep */ feature;", Origin::User);
    let record = &tree.opaque_at_rules()[0];
    assert_eq!(record.name, "future");
    assert_eq!(record.prelude, " /* keep */ feature");
    assert_eq!(record.body, AtRuleBody::Statement);
    assert_eq!(record.origin, Origin::User);
    assert_eq!(record.to_css(), "@future /* keep */ feature;");
}

#[test]
fn malformed_opaque_at_rule_is_not_retained() {
    let mut tree = RuleTree::empty();
    tree.add_stylesheet(
        "@future { x: url(\"bad\n) } p { color: blue }",
        Origin::Author,
    );
    assert!(tree.opaque_at_rules().is_empty());
    assert_eq!(tree.style_rules.len(), 1);
}

#[test]
fn unterminated_rule_blocks_are_not_retained() {
    let mut tree = RuleTree::empty();
    tree.add_stylesheet("@future { p { color: red }", Origin::Author);
    assert!(tree.opaque_at_rules().is_empty());
    assert!(tree.rules().is_empty());

    let mut tree = RuleTree::empty();
    tree.add_stylesheet("@page { color: red", Origin::Author);
    assert!(tree.page_rules.is_empty());
    assert!(tree.rules().is_empty());

    let mut tree = RuleTree::empty();
    tree.add_stylesheet("p { color: red", Origin::Author);
    assert!(tree.style_rules.is_empty());
    assert!(tree.rules().is_empty());
}

#[test]
fn opaque_at_rule_accepts_braces_in_unquoted_url() {
    for prelude in ["url(foo{bar)", r"u\72l(foo{bar)"] {
        let mut tree = RuleTree::empty();
        tree.add_stylesheet(
            &format!("@future {prelude}; p {{ color: blue }}"),
            Origin::Author,
        );
        assert_eq!(tree.opaque_at_rules().len(), 1, "prelude: {prelude:?}");
        assert_eq!(tree.style_rules.len(), 1);
    }
}

#[test]
fn url_like_text_in_identifiers_does_not_hide_delimiters() {
    for prelude in [
        "@url(foo{bar)",
        "#url(foo{bar)",
        "éurl(foo{bar)",
        r"\.\url(foo{bar)",
        r"\2e \url(foo{bar)",
    ] {
        let mut tree = RuleTree::empty();
        tree.add_stylesheet(
            &format!("@future {prelude}; p {{ color: blue }}"),
            Origin::Author,
        );
        assert!(
            tree.opaque_at_rules().is_empty(),
            "malformed delimiter sequence was retained for prelude {prelude:?}"
        );
    }
}

#[test]
fn tokenizer_valid_eof_strings_and_comments_are_retained() {
    for source in [
        "@future \"unterminated",
        "@future /* unterminated",
        "@future url(foo\\",
    ] {
        let mut tree = RuleTree::empty();
        tree.add_stylesheet(source, Origin::Author);
        assert_eq!(tree.opaque_at_rules().len(), 1, "source: {source:?}");
    }
}

#[test]
fn nested_opaque_at_rule_bodies_respect_cumulative_byte_budget() {
    let mut css = "payload".repeat(32);
    for _ in 0..16 {
        css = format!("@future {{{css}}}");
    }

    let budget = css.len() * 2;
    let mut remaining = budget;
    let nodes = parse_nested_rule_nodes(&css, &mut remaining);

    assert!(!nodes.is_empty());
    assert!(remaining < budget);
    fn retained_body_bytes(nodes: &[RuleNode]) -> usize {
        nodes
            .iter()
            .map(|node| match node {
                RuleNode::AtRule(record) => {
                    record.body.as_block().map_or(0, str::len)
                        + retained_body_bytes(record.children())
                }
                RuleNode::Qualified(record) => record.body.len(),
            })
            .sum()
    }
    assert_eq!(retained_body_bytes(&nodes), budget - remaining);

    let mut depth = 0;
    let mut node = nodes.first();
    while let Some(RuleNode::AtRule(record)) = node {
        depth += 1;
        node = record.children().first();
    }
    assert!(depth < 16, "the byte budget must truncate the owned chain");
}

#[test]
fn rule_tree_caps_nested_opaque_body_retention() {
    let mut css = "x".repeat(512 * 1024);
    for _ in 0..20 {
        css = format!("@future {{{css}}}");
    }

    let mut tree = RuleTree::empty();
    tree.add_stylesheet(&css, Origin::Author);
    let record = &tree.opaque_at_rules()[0];

    fn retained_nested_body_bytes(nodes: &[RuleNode]) -> usize {
        nodes
            .iter()
            .map(|node| match node {
                RuleNode::AtRule(record) => {
                    record.body.as_block().map_or(0, str::len)
                        + retained_nested_body_bytes(record.children())
                }
                RuleNode::Qualified(record) => record.body.len(),
            })
            .sum()
    }

    let retained = retained_nested_body_bytes(record.children());
    assert!(retained > 0);
    assert!(retained <= MAX_CUMULATIVE_NESTED_OPAQUE_BODY_BYTES);
    assert_eq!(record.children().len(), 1);
}

#[test]
fn deeply_nested_opaque_at_rules_are_bounded_but_retained() {
    let mut css = String::new();
    for _ in 0..256 {
        css.push_str("@future {");
    }
    css.push_str("p { color: red }");
    for _ in 0..256 {
        css.push('}');
    }

    let mut tree = RuleTree::empty();
    tree.add_stylesheet(&css, Origin::Author);
    let record = &tree.opaque_at_rules()[0];
    assert_eq!(record.name, "future");
    assert!(record.body.as_block().is_some());

    let mut depth = 1;
    let mut node = record;
    while let Some(RuleNode::AtRule(nested)) = node.children().first() {
        depth += 1;
        node = nested;
    }
    // The top-level record is followed by raw-parser levels 0 through
    // `MAX_OPAQUE_RULE_NESTING_DEPTH`, so the retained chain has two
    // records beyond the configured recursion count.
    assert_eq!(depth, MAX_OPAQUE_RULE_NESTING_DEPTH + 2);
}

#[test]
fn valid_unknown_top_level_margin_box_at_rule_is_retained() {
    // A top-level margin-box at-rule has no semantics in this parser, but
    // it is still valid component-value syntax and must remain inspectable.
    let mut tree = RuleTree::empty();
    tree.add_stylesheet(
        "@top-left { content: 'x' } p { color: red }",
        Origin::Author,
    );
    assert_eq!(tree.page_rules.len(), 0);
    assert_eq!(tree.style_rules.len(), 1);
    assert_eq!(tree.opaque_at_rules().len(), 1);
    assert_eq!(tree.opaque_at_rules()[0].name, "top-left");
}

// ── Comment / whitespace transparency within compound ──
//
// CSS Syntax L3 §4.3.2 Consume comments specifies that a /*…*/
// sequence is consumed and the algorithm returns nothing — no token
// is emitted into the token stream (see
// <https://www.w3.org/TR/css-syntax-3/#consume-comment>). Within a
// `<page-selector>` compound, therefore, a comment between two
// components MUST behave as if absent —
// `:first/*x*/:left` == `:first:left`. Whitespace, by contrast, IS
// emitted as a <whitespace-token> and still breaks the compound per
// L3 §4.3.

#[test]
fn page_comment_between_pseudos_is_transparent() {
    // `@page :first/*sep*/:left` — comment only, no whitespace within
    // compound. Per CSS Syntax L3 tokenization, comments vanish, so this
    // is equivalent to `@page :first:left` → 1 rule with two pseudos.
    let rules = page_rules("@page :first/*sep*/:left { color: red }");
    assert_eq!(rules.len(), 1);
    assert_eq!(
        rules[0].selector,
        ps_single(None, vec![PagePseudo::First, PagePseudo::Left])
    );
}

#[test]
fn page_comment_between_colon_and_pseudo_is_transparent() {
    // `@page :/*x*/first` — comment between `:` and pseudo ident.
    // Comment vanishes at tokenization, so this equals `@page :first` →
    // 1 rule. Contrast with `@page : first` (whitespace-separated),
    // which is dropped by `page_whitespace_between_colon_and_pseudo_is_dropped`.
    let rules = page_rules("@page :/*x*/first { color: red }");
    assert_eq!(rules.len(), 1);
    assert_eq!(rules[0].selector, ps_single(None, vec![PagePseudo::First]));
}

#[test]
fn page_whitespace_plus_comment_between_pseudos_is_dropped() {
    // `@page :first /*sep*/:left` — comment is transparent, but the
    // leading whitespace is a real compound-boundary token per L3 §4.3.
    // Whole rule dropped (matches `page_multi_pseudo_with_whitespace_between_is_dropped`).
    let rules = page_rules("@page :first /*sep*/:left { color: red }");
    assert!(rules.is_empty());
}

// ── <custom-ident> case-sensitivity for named-page ident ──
//
// The named-page ident in a `<page-selector>` derives from the `page`
// property (CSS Paged Media L3 §8.1 `#using-named-pages`), which types
// its value as `<custom-ident>`. Per CSS Values L4 §4.2
// `#custom-idents`: "Such identifiers are fully case-sensitive
// (meaning they're compared using the 'identical to' operation), even
// in the ASCII range (e.g. example and EXAMPLE are two different,
// unrelated user-defined identifiers)." So `Cover` and `cover` MUST be
// preserved verbatim and treated as distinct named-pages.

#[test]
fn page_named_ident_is_case_sensitive() {
    let rules = page_rules("@page Cover { color: red } @page cover { color: blue }");
    assert_eq!(rules.len(), 2);
    assert_eq!(
        rules[0].selector,
        ps_single(Some(Atom::from("Cover")), vec![])
    );
    assert_eq!(
        rules[1].selector,
        ps_single(Some(Atom::from("cover")), vec![])
    );
}

// ── <page-selector># list-boundary invariants ──
//
// `<page-selector-list> = <page-selector>#` per CSS Paged Media L3 §4.3
// (anchor `#syntax-page-selector`). The `#` multiplier is "one or more,
// comma-separated" per CSS Values L4 `#component-multipliers`, so each
// list entry must be a *non-empty* `<page-selector>`. Trailing,
// leading, and empty-middle commas violate this and drop the whole
// `@page` rule. These 3 tests close 3 of 6 malformed-prelude cases;
// the remaining 3 (trailing colon on named page, adjacent idents, etc.)
// are covered by the tests below.

#[test]
fn page_trailing_comma_is_dropped() {
    let rules = page_rules("@page :first, { color: red }");
    assert!(rules.is_empty());
}

#[test]
fn page_leading_comma_is_dropped() {
    let rules = page_rules("@page ,:first { color: red }");
    assert!(rules.is_empty());
}

#[test]
fn page_empty_middle_entry_is_dropped() {
    let rules = page_rules("@page :first, , :left { color: red }");
    assert!(rules.is_empty());
}

// ── Malformed prelude — trailing/isolated colon and adjacent idents ──
//
// Companion to the list-boundary banner above, which pinned the 3
// list-boundary cases (trailing / leading / empty-middle commas) of
// `<page-selector>#`; the tests below check the 3 compound-internal cases
// against the CSS Paged Media L3 §4.3 (anchor `#syntax-page-selector`)
// compound grammar `<page-selector> = [ <ident-token>? <pseudo-page>* ]!`
// with `<pseudo-page> = ':' [ left | right | first | blank ]`. Together
// the two banners close a 6-case malformed-prelude set:
//   - `named:`      — trailing colon, missing required left|right|first|blank keyword
//   - `:`           — bare colon, same
//   - `named other` — two adjacent idents, compound allows only one
// Each case drops the whole `@page` rule (declarations not captured).

#[test]
fn page_named_trailing_colon_is_dropped() {
    // `@page named:` — the block begins after the named-page identifier
    // and `:`, without the required `<pseudo-page>` keyword
    // (left/right/first/blank). Drop it: the compound grammar
    // `[ <ident-token>? <pseudo-page>* ]!` requires a keyword in
    // `<pseudo-page> = ':' [ left | right | first | blank ]`.
    let rules = page_rules("@page named: { color: red }");
    assert!(rules.is_empty());
}

#[test]
fn page_bare_colon_is_dropped() {
    // `@page :` — a bare colon; drop it because
    // `<pseudo-page> = ':' [ left | right | first | blank ]` requires a keyword.
    let rules = page_rules("@page : { color: red }");
    assert!(rules.is_empty());
}

#[test]
fn page_two_adjacent_idents_are_dropped() {
    // `@page named other` — a compound permits only one initial
    // `<ident-token>`. The second identifier can neither continue the
    // compound nor start the next entry (there is no comma), so
    // `parse_comma_separated`'s entry-parses-entirely check finds leftover
    // input, returns `Err`, and drops the whole rule.
    let rules = page_rules("@page named other { color: red }");
    assert!(rules.is_empty());
}

#[test]
fn page_rules_captured_via_build_rule_tree_from_dom() {
    // build_rule_tree also populates page_rules through the DOM.
    let doc = dom_with_style("@page :first { color: red } p { color: blue }");
    let tree = build_rule_tree(&doc);
    assert_eq!(tree.page_rules.len(), 1);
    assert_eq!(
        tree.page_rules[0].selector,
        ps_single(None, vec![PagePseudo::First])
    );
    assert_eq!(tree.style_rules.len(), 1);
}

// ── counter_styles integration (origin-aware) ──

#[test]
fn empty_rule_tree_has_empty_counter_styles() {
    assert!(RuleTree::empty().counter_styles().is_empty());
}

#[test]
fn add_stylesheet_populates_counter_styles() {
    let mut tree = RuleTree::empty();
    tree.add_stylesheet(
        r#"@counter-style thumbs { system: cyclic; symbols: "*"; }"#,
        Origin::Author,
    );
    assert_eq!(tree.counter_styles().len(), 1);
    let rule = tree
        .counter_styles()
        .get("thumbs")
        .expect("thumbs registered");
    assert_eq!(rule.symbols.len(), 1);
}

#[test]
fn add_stylesheet_invalid_counter_style_rule_is_dropped() {
    // `system: cyclic` with zero symbols fails is_valid() and is dropped
    // (same shape as the corresponding counter_style.rs test).
    let mut tree = RuleTree::empty();
    tree.add_stylesheet("@counter-style foo { system: cyclic; }", Origin::Author);
    assert!(tree.counter_styles().is_empty());
}

#[test]
fn add_stylesheet_counter_style_alongside_style_and_page_rules() {
    // Verify that one add_stylesheet call populates all three kinds of
    // rules from the same source; the separate second pass must not
    // interfere with collecting style_rules or page_rules.
    let mut tree = RuleTree::empty();
    tree.add_stylesheet(
        r#"
        @counter-style thumbs { system: cyclic; symbols: "*"; }
        @page :first { color: red }
        p { color: blue }
        "#,
        Origin::Author,
    );
    assert_eq!(tree.counter_styles().len(), 1);
    assert_eq!(tree.page_rules.len(), 1);
    assert_eq!(tree.style_rules.len(), 1);
    assert_eq!(tree.opaque_at_rules().len(), 1);
    assert_eq!(tree.opaque_at_rules()[0].name, "counter-style");
    assert_eq!(
        tree.rules()
            .iter()
            .map(|rule| rule.kind.clone())
            .collect::<Vec<_>>(),
        vec![
            CssRuleKind::AtRule { index: 0 },
            CssRuleKind::Page { index: 0 },
            CssRuleKind::Style { index: 0 },
        ]
    );
}

#[test]
fn counter_styles_same_name_later_author_call_replaces_earlier_entirely() {
    // Check across multiple add_stylesheet(Origin::Author) calls the
    // CounterStyleRegistry type doc's guarantee that a later definition
    // atomically replaces an earlier one with the same name (the
    // counter-style counterpart of
    // page_source_order_monotonic_across_add_stylesheet_calls). Both are
    // Author origin: the same-origin source-order tie-break follows the
    // "standard cascade rules" of CSS Counter Styles L3 §3.
    let mut tree = RuleTree::empty();
    tree.add_stylesheet(
        r#"@counter-style thumbs { system: cyclic; symbols: "*"; }"#,
        Origin::Author,
    );
    tree.add_stylesheet(
        r#"@counter-style thumbs { system: cyclic; symbols: "+" "-"; }"#,
        Origin::Author,
    );
    assert_eq!(tree.counter_styles().len(), 1);
    let rule = tree
        .counter_styles()
        .get("thumbs")
        .expect("thumbs registered");
    assert_eq!(rule.symbols.len(), 2);
}

#[test]
fn add_stylesheet_useragent_origin_alone_populates_counter_styles() {
    // CSS Counter Styles L3 §3: defining an @counter-style makes it
    // available unconditionally — the "only one wins, according to
    // standard cascade rules" sentence only applies when there IS a
    // same-name conflict. A standalone Origin::UserAgent rule with no
    // competing Origin::Author rule has no conflict, so it must be
    // available. An Author-only gate in add_stylesheet would drop this
    // unconditionally regardless of conflict — this test pins that it
    // doesn't (previously named *_does_not_populate_counter_styles and
    // asserted the opposite). The existing
    // add_stylesheet_ua_and_author_populate_rule_tree separately checks
    // that style_rules are populated regardless of origin.
    let mut tree = RuleTree::empty();
    tree.add_stylesheet(
        r#"@counter-style thumbs { system: cyclic; symbols: "*"; }"#,
        Origin::UserAgent,
    );
    assert_eq!(tree.counter_styles().len(), 1);
    let rule = tree
        .counter_styles()
        .get("thumbs")
        .expect("thumbs registered");
    assert_eq!(rule.symbols.len(), 1);
}

#[test]
fn add_stylesheet_useragent_after_author_does_not_override_counter_styles() {
    // CSS Counter Styles L3 §3: "only one wins, according to standard
    // cascade rules" — origin takes priority, so UA always loses to
    // Author. Define Author first, then add_stylesheet with a same-name
    // UA @counter-style, and check that the Author definition survives.
    // This guarantee comes not from an Origin::Author gate in
    // add_stylesheet (which would drop UA unconditionally), but from
    // CounterStyleRegistry::insert_with_origin tracking each same-name
    // entry's origin and applying origin precedence (the resolution table
    // in its type doc). This remains a regression check against the spec
    // violation where flat call-order last-wins lets a later UA override
    // Author.
    let mut tree = RuleTree::empty();
    tree.add_stylesheet(
        r#"@counter-style thumbs { system: cyclic; symbols: "*"; }"#,
        Origin::Author,
    );
    tree.add_stylesheet(
        r#"@counter-style thumbs { system: cyclic; symbols: "+" "-"; }"#,
        Origin::UserAgent,
    );
    assert_eq!(tree.counter_styles().len(), 1);
    let rule = tree
        .counter_styles()
        .get("thumbs")
        .expect("thumbs registered");
    // The Author one-symbol definition wins, not the UA two-symbol one.
    assert_eq!(rule.symbols.len(), 1);
}

#[test]
fn add_stylesheet_author_after_useragent_overrides_counter_styles() {
    // Reverse the call order of the test above: define UA first, then
    // add_stylesheet with a same-name Author @counter-style. Origin
    // precedence (Author > UserAgent) must not depend on call order, so
    // Author wins here too (CSS Counter Styles L3 §3, "standard cascade
    // rules" gives origin first priority).
    let mut tree = RuleTree::empty();
    tree.add_stylesheet(
        r#"@counter-style thumbs { system: cyclic; symbols: "*"; }"#,
        Origin::UserAgent,
    );
    tree.add_stylesheet(
        r#"@counter-style thumbs { system: cyclic; symbols: "+" "-"; }"#,
        Origin::Author,
    );
    assert_eq!(tree.counter_styles().len(), 1);
    let rule = tree
        .counter_styles()
        .get("thumbs")
        .expect("thumbs registered");
    // The Author two-symbol definition wins.
    assert_eq!(rule.symbols.len(), 2);
}

#[test]
fn add_stylesheet_author_after_user_overrides_counter_styles() {
    // The Origin::User version of
    // add_stylesheet_author_after_useragent_overrides_counter_styles.
    // Consumer-provided `extra_stylesheets` really route to Origin::User
    // (raikiri-html's retag plus umbrella's stylesheet_kind_to_origin
    // extension), so this User-then-Author call-order pair is genuinely
    // reachable in production. Define User first and then add_stylesheet
    // with a same-name Author @counter-style. Origin precedence (Author
    // normal rank 3 > User normal rank 1, cascade::cascade_rank) must not
    // depend on call order, so Author wins here too (CSS Counter Styles
    // L3 §3, "standard cascade rules" gives origin first priority).
    let mut tree = RuleTree::empty();
    tree.add_stylesheet(
        r#"@counter-style thumbs { system: cyclic; symbols: "*"; }"#,
        Origin::User,
    );
    tree.add_stylesheet(
        r#"@counter-style thumbs { system: cyclic; symbols: "+" "-"; }"#,
        Origin::Author,
    );
    assert_eq!(tree.counter_styles().len(), 1);
    let rule = tree
        .counter_styles()
        .get("thumbs")
        .expect("thumbs registered");
    // The Author two-symbol definition wins.
    assert_eq!(rule.symbols.len(), 2);
}

#[test]
fn add_stylesheet_useragent_same_name_later_call_replaces_earlier() {
    // Origin::UserAgent counterpart of
    // counter_styles_same_name_later_author_call_replaces_earlier_entirely:
    // check that the same-origin (UserAgent) source-order tie-break (later
    // wins) works symmetrically with Author (resolution table in the
    // CounterStyleRegistry::insert_with_origin type doc).
    let mut tree = RuleTree::empty();
    tree.add_stylesheet(
        r#"@counter-style thumbs { system: cyclic; symbols: "*"; }"#,
        Origin::UserAgent,
    );
    tree.add_stylesheet(
        r#"@counter-style thumbs { system: cyclic; symbols: "+" "-"; }"#,
        Origin::UserAgent,
    );
    assert_eq!(tree.counter_styles().len(), 1);
    let rule = tree
        .counter_styles()
        .get("thumbs")
        .expect("thumbs registered");
    assert_eq!(rule.symbols.len(), 2);
}

#[test]
fn add_stylesheet_useragent_and_author_different_names_both_populate_counter_styles() {
    // The headline spec claim this whole set of tests is about: CSS
    // Counter Styles L3 §3 makes defining an @counter-style
    // available unconditionally — availability, not just same-name
    // conflict resolution. Every other origin-mixing test above reuses
    // the same rule name ("thumbs") specifically to exercise conflict
    // resolution; this one pins the non-conflicting case those can't:
    // two differently-named rules from different origins must both
    // survive together in the registry.
    let mut tree = RuleTree::empty();
    tree.add_stylesheet(
        r#"@counter-style ua-thumbs { system: cyclic; symbols: "*"; }"#,
        Origin::UserAgent,
    );
    tree.add_stylesheet(
        r#"@counter-style author-thumbs { system: cyclic; symbols: "+" "-"; }"#,
        Origin::Author,
    );
    assert_eq!(tree.counter_styles().len(), 2);
    assert!(tree.counter_styles().get("ua-thumbs").is_some());
    assert!(tree.counter_styles().get("author-thumbs").is_some());
}

#[test]
fn build_rule_tree_populates_counter_styles_from_dom_style_element() {
    let doc = dom_with_style(
        r#"@counter-style thumbs { system: cyclic; symbols: "*"; } p { color: red }"#,
    );
    let tree = build_rule_tree(&doc);
    assert_eq!(tree.counter_styles().len(), 1);
    assert!(tree.counter_styles().get("thumbs").is_some());
    // The same source still populates its style rule (the second pass
    // must not interfere with the existing walk).
    assert_eq!(tree.style_rules.len(), 1);
}

#[test]
fn build_rule_tree_counter_styles_across_multiple_style_elements_last_wins() {
    // Two sibling <style> elements with the same-name @counter-style:
    // the later <style> wins, following walk_and_collect's document-order
    // calls.
    let mut doc = TestDoc::new();
    let s1 = doc.push_element(0, "style", None);
    doc.push_text(
        s1,
        r#"@counter-style thumbs { system: cyclic; symbols: "*"; }"#,
    );
    let s2 = doc.push_element(0, "style", None);
    doc.push_text(
        s2,
        r#"@counter-style thumbs { system: cyclic; symbols: "+" "-"; }"#,
    );
    let tree = build_rule_tree(&doc);
    assert_eq!(tree.counter_styles().len(), 1);
    let rule = tree
        .counter_styles()
        .get("thumbs")
        .expect("thumbs registered");
    assert_eq!(rule.symbols.len(), 2);
}

const MEDIA_SHEET: &str = r#"
@font-face { font-family: 'Ahem'; src: url('/fonts/Ahem.ttf'); }
@counter-style thumbs { system: cyclic; symbols: "*"; }
@page { margin: 1in }
p { color: red }
@media (min-width: 1px) { div { color: blue } }
"#;

#[test]
fn stylesheet_media_guards_every_rule_kind() {
    let mut tree = RuleTree::empty();
    tree.add_stylesheet("h1 { color: green }", Origin::Author);
    tree.add_stylesheet_with_media(MEDIA_SHEET, Origin::Author, Some("print"));

    // Style rules join the media-guarded view and keep the shared order.
    assert_eq!(tree.style_rules().len(), 1);
    assert!(
        tree.rules()
            .iter()
            .all(|rule| !matches!(rule.kind, CssRuleKind::Style { index } if index > 0))
    );
    let orders: Vec<_> = tree
        .media_rules
        .iter()
        .map(|m| m.rule.source_order)
        .collect();
    assert_eq!(orders, [1, 2]);
    let print = MediaContext::print();
    let screen = MediaContext::screen();
    assert!(tree.media_rules.iter().all(|m| m.condition.matches(&print)));
    assert!(
        tree.media_rules
            .iter()
            .all(|m| !m.condition.matches(&screen))
    );

    // `@page` carries the stylesheet condition.
    assert_eq!(tree.page_rules.len(), 1);
    let page_condition = tree.page_rules[0].media_condition.as_ref().unwrap();
    assert!(page_condition.matches(&print));
    assert!(!page_condition.matches(&screen));

    // Registries hold the rules only for a matching context.
    assert!(tree.font_faces().get("Ahem").is_none());
    assert!(tree.font_faces_for(&print).get("Ahem").is_some());
    assert!(tree.font_faces_for(&screen).get("Ahem").is_none());
    assert!(tree.counter_styles().get("thumbs").is_none());
    assert!(tree.counter_styles_for(&print).get("thumbs").is_some());
    assert!(tree.counter_styles_for(&screen).get("thumbs").is_none());
}

#[test]
fn nested_media_intersects_with_stylesheet_media() {
    let mut tree = RuleTree::empty();
    tree.add_stylesheet_with_media(
        "@media (min-width: 500px) { p { color: red } }",
        Origin::Author,
        Some("print"),
    );
    assert_eq!(tree.media_rules.len(), 1);
    let condition = &tree.media_rules[0].condition;
    assert!(condition.matches(&MediaContext::with_viewport(
        crate::media::MediaType::Print,
        600,
        100
    )));
    assert!(!condition.matches(&MediaContext::with_viewport(
        crate::media::MediaType::Screen,
        600,
        100
    )));
    assert!(!condition.matches(&MediaContext::print()));
}

#[test]
fn layered_and_supports_rules_keep_stylesheet_media() {
    let mut tree = RuleTree::empty();
    tree.add_stylesheet_with_media(
        "@layer base { p { color: red } } @supports (color: red) { div { color: blue } }",
        Origin::Author,
        Some("print"),
    );
    assert!(tree.style_rules().is_empty());
    assert_eq!(tree.media_rules.len(), 2);
    assert!(
        tree.media_rules
            .iter()
            .all(|m| m.condition.matches(&MediaContext::print())
                && !m.condition.matches(&MediaContext::screen()))
    );
}

#[test]
fn empty_stylesheet_media_means_all() {
    for media in [None, Some(""), Some("  \t")] {
        let mut tree = RuleTree::empty();
        tree.add_stylesheet_with_media(MEDIA_SHEET, Origin::Author, media);
        assert_eq!(tree.style_rules().len(), 1, "{media:?}");
        assert!(tree.page_rules[0].media_condition.is_none(), "{media:?}");
        assert!(tree.font_faces().get("Ahem").is_some(), "{media:?}");
    }
}

#[test]
fn never_matching_stylesheet_media_adds_nothing() {
    for media in ["not all", "(hover: hover)", "print,"] {
        let mut tree = RuleTree::empty();
        tree.add_stylesheet_with_media(MEDIA_SHEET, Origin::Author, Some(media));
        assert!(tree.style_rules().is_empty(), "{media}");
        assert!(tree.media_rules.is_empty(), "{media}");
        assert!(tree.page_rules.is_empty(), "{media}");
        assert!(tree.rules().is_empty(), "{media}");
        assert!(
            tree.font_faces_for(&MediaContext::print()).is_empty(),
            "{media}"
        );
        assert!(
            tree.counter_styles_for(&MediaContext::print()).is_empty(),
            "{media}"
        );
    }
}

#[test]
fn registry_replay_keeps_insertion_precedence() {
    let mut tree = RuleTree::empty();
    tree.add_stylesheet(
        "@font-face { font-family: 'Ahem'; src: url('/first.ttf'); }",
        Origin::Author,
    );
    tree.add_stylesheet_with_media(
        "@font-face { font-family: 'Ahem'; src: url('/print.ttf'); }",
        Origin::Author,
        Some("print"),
    );
    let print = tree.font_faces_for(&MediaContext::print());
    let screen = tree.font_faces_for(&MediaContext::screen());
    assert_ne!(print.get("Ahem"), screen.get("Ahem"));
    assert_eq!(screen.get("Ahem"), tree.font_faces().get("Ahem"));
}

/// The previous two-pass implementation of `consume_raw_component_values`:
/// collect the top-level tokens, then re-tokenize the text to check nested
/// blocks.
fn two_pass_raw_component_values(source: &str) -> Option<String> {
    let mut input = ParserInput::new(source);
    let mut parser = Parser::new(&mut input);
    let start = parser.position();
    while let Ok(token) = parser.next_including_whitespace_and_comments() {
        if token.is_parse_error() {
            return None;
        }
    }
    let raw = parser.slice(start..parser.position()).to_owned();
    css_component_values_are_balanced(&raw).then_some(raw)
}

#[test]
fn single_pass_raw_component_values_match_the_two_pass_check() {
    let deep = MAX_OPAQUE_RULE_NESTING_DEPTH + 3;
    let mut sources: Vec<String> = [
        "",
        "  /* c */ a b c ",
        "x: url(\"bad\n)",
        "a { b: c }",
        "a { b: c",
        "f(a, [b, {c}])",
        "f(a, [b, {c})",
        "f(a, [b, {c}]",
        "p { content: \"oops\n; } q { }",
        "p { background: url(bad url) }",
        "url(foo{bar) baz",
        "a ) b",
        "a ] b",
        "a } b",
        "{ ( }",
        "( { ) }",
    ]
    .iter()
    .map(|s| (*s).to_owned())
    .collect();
    sources.push(format!("{}x{}", "(".repeat(deep), ")".repeat(deep)));
    sources.push(format!("{}x{}", "(".repeat(deep), ")".repeat(deep - 1)));
    sources.push(format!("{}\"bad\n{}", "(".repeat(deep), ")".repeat(deep)));
    sources.push(format!("{}x ]{}", "(".repeat(deep), ")".repeat(deep)));
    sources.push(format!("{}\"bad\n{}", "[".repeat(3), "]".repeat(3)));
    let mut accepted = 0;
    for source in &sources {
        let mut input = ParserInput::new(source);
        let mut parser = Parser::new(&mut input);
        let single = consume_raw_component_values(&mut parser, source).ok();
        assert_eq!(single, two_pass_raw_component_values(source), "{source:?}");
        accepted += usize::from(single.is_some());
    }
    // The corpus exercises both outcomes.
    assert!(accepted > 0 && accepted < sources.len(), "{accepted}");
}

#[test]
fn namespace_with_a_block_is_retained_as_an_opaque_record() {
    let mut tree = RuleTree::empty();
    tree.add_stylesheet(
        "@namespace svg url(http://www.w3.org/2000/svg) { a: b } p { color: red }",
        Origin::Author,
    );
    let records = tree.opaque_at_rules();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].name, "namespace");
    assert!(matches!(&records[0].body, AtRuleBody::Block(body) if body.trim() == "a: b"));
    assert_eq!(tree.style_rules().len(), 1);

    // A block with a tokenizer error token is not retained.
    let mut tree = RuleTree::empty();
    tree.add_stylesheet(
        "@namespace svg url(http://www.w3.org/2000/svg) { a: \"bad\n } p { color: red }",
        Origin::Author,
    );
    assert!(tree.opaque_at_rules().is_empty());
}

#[test]
fn error_tokens_inside_media_drop_only_the_bad_declaration() {
    for bad in ["content: \"oops\n", "background: url(bad url)"] {
        let rules = format!("p {{ color: red; {bad}; }} div {{ color: blue }}");
        let mut flat = RuleTree::empty();
        flat.add_stylesheet(&rules, Origin::Author);
        let flat_shape: Vec<_> = flat
            .style_rules()
            .iter()
            .map(|rule| rule.declarations.len())
            .collect();
        assert_eq!(flat_shape.len(), 2, "{bad:?}");

        for wrapped in [
            format!("@media print {{ {rules} }}"),
            format!("@media all {{ @media print {{ {rules} }} }}"),
        ] {
            let mut tree = RuleTree::empty();
            tree.add_stylesheet(&wrapped, Origin::Author);
            let media_shape: Vec<_> = tree
                .media_rules
                .iter()
                .map(|media| media.rule.declarations.len())
                .collect();
            assert_eq!(media_shape, flat_shape, "{wrapped:?}");
            // The block stays in the inspection view as well.
            assert_eq!(tree.opaque_at_rules().len(), 1, "{wrapped:?}");
        }
    }
}

#[test]
fn error_tokens_inside_media_keep_page_rules() {
    let mut tree = RuleTree::empty();
    tree.add_stylesheet(
        "@media print { @page { margin: 1in; content: \"oops\n; } p { color: red } }",
        Origin::Author,
    );
    assert_eq!(tree.page_rules.len(), 1);
    assert_eq!(tree.media_rules.len(), 1);
}

#[test]
fn media_content_edge_cases() {
    // An unsupported selector is dropped without consuming source order.
    let mut tree = RuleTree::empty();
    tree.add_stylesheet(
        "@media print { div:hover { color: red } p { color: blue } }",
        Origin::Author,
    );
    assert_eq!(tree.media_rules.len(), 1);
    assert_eq!(tree.media_rules[0].rule.source_order, 0);

    // A nested list that can never match executes nothing, but its siblings do.
    let mut tree = RuleTree::empty();
    tree.add_stylesheet(
        "@media print { @media not all { p { color: red } } div { color: blue } }",
        Origin::Author,
    );
    assert_eq!(tree.media_rules.len(), 1);

    // The statement form is retained but executes nothing.
    let mut tree = RuleTree::empty();
    tree.add_stylesheet("@media print; p { color: red }", Origin::Author);
    assert_eq!(tree.opaque_at_rules().len(), 1);
    assert!(tree.media_rules.is_empty());
    assert_eq!(tree.style_rules().len(), 1);

    // Unterminated blocks are dropped, as at the top level.
    for source in [
        "@media print { p { color: red }",
        "@media print { p { color: red",
    ] {
        let mut tree = RuleTree::empty();
        tree.add_stylesheet(source, Origin::Author);
        assert!(tree.media_rules.is_empty(), "{source:?}");
        assert!(tree.opaque_at_rules().is_empty(), "{source:?}");
    }

    // Content nested deeper than the bound is not executed.
    let depth = MAX_OPAQUE_RULE_NESTING_DEPTH + 2;
    let source = format!(
        "{}p {{ color: red }}{}",
        "@media print { ".repeat(depth),
        " }".repeat(depth)
    );
    let mut tree = RuleTree::empty();
    tree.add_stylesheet(&source, Origin::Author);
    assert!(tree.media_rules.is_empty());
}

#[test]
fn unterminated_strings_end_at_newlines_in_text_passes() {
    // CSS Syntax 3 §4.3.5: an unescaped newline ends a string (bad-string),
    // so only the declaration holding it is invalid, wherever the rules sit.
    let rules = "p { color: red; content: \"oops\n; } div { color: blue }";
    let cases = [
        ("top level", rules.to_owned(), 2),
        (
            "@supports",
            format!("@supports (color: red) {{ {rules} }}"),
            2,
        ),
        ("@layer", format!("@layer base {{ {rules} }}"), 2),
        ("nesting", format!("section {{ {rules} }}"), 2),
        (
            "@supports after it",
            format!("{rules} @supports (color: red) {{ em {{ color: green }} }}"),
            3,
        ),
        (
            "@layer after it",
            format!("{rules} @layer base {{ em {{ color: green }} }}"),
            3,
        ),
    ];
    for (name, css, count) in cases {
        let mut tree = RuleTree::empty();
        tree.add_stylesheet(&css, Origin::Author);
        let shape: Vec<_> = tree
            .style_rules()
            .iter()
            .map(|rule| rule.declarations.len())
            .collect();
        assert_eq!(shape, vec![1; count], "{name}");
    }
}

#[test]
fn escaped_newlines_continue_strings_in_text_passes() {
    // A backslash before a newline continues the string, so the brace after
    // it is string content, not the end of the block.
    for newline in ["\n", "\r\n", "\x0C"] {
        let css = format!(
            "@supports (color: red) {{ p {{ content: \"a\\{newline}}}\"; color: red }} }} div {{ color: blue }}"
        );
        let mut tree = RuleTree::empty();
        tree.add_stylesheet(&css, Origin::Author);
        assert_eq!(tree.style_rules().len(), 2, "{newline:?}");
    }
}

mod descriptor_rule_tests;
mod group_rule_tests;
mod supports_condition_tests;

mod nesting_tests;

#[test]
fn contextual_highlight_background_wins_over_an_earlier_literal() {
    let mut tree = RuleTree::empty();
    tree.add_stylesheet(
        "::highlight(mark){background-color:red;background-color:currentcolor}",
        Origin::Author,
    );
    assert_eq!(
        tree.custom_highlight_styles().get("mark"),
        Some(&crate::CssColor::BLACK)
    );
}

#[test]
fn contextual_highlight_mix_remains_a_valid_background() {
    let mut tree = RuleTree::empty();
    tree.add_stylesheet(
        "::highlight(mark){background-color:color-mix(in srgb,currentcolor,white)}",
        Origin::Author,
    );
    assert_eq!(
        tree.custom_highlight_styles().get("mark"),
        Some(&crate::CssColor {
            r: 128,
            g: 128,
            b: 128,
            a: 255
        })
    );
}

#[test]
fn contextual_highlight_sources_follow_literal_layer_and_important_precedence() {
    let blue = crate::CssColor {
        r: 0,
        g: 0,
        b: 255,
        a: 255,
    };
    let red = crate::CssColor {
        r: 255,
        g: 0,
        b: 0,
        a: 255,
    };
    let cases = [
        ("::highlight(mark){background-color:currentcolor}", blue),
        (
            "::highlight(mark){background-color:color-mix(in srgb,currentcolor,red)}",
            crate::CssColor {
                r: 128,
                g: 0,
                b: 128,
                a: 255,
            },
        ),
        (
            "::highlight(mark){background-color:red!important;background-color:currentcolor}",
            red,
        ),
        (
            "::highlight(mark){background-color:currentcolor!important;background-color:red}",
            blue,
        ),
        (
            "@layer a,b; @layer a{::highlight(mark){background-color:red}} @layer b{::highlight(mark){background-color:currentcolor}}",
            blue,
        ),
        (
            "@layer a,b; @layer a{::highlight(mark){background-color:red}} @layer b{::highlight(mark){background-color:currentcolor;background-color:revert-layer}}",
            red,
        ),
        (
            "::highlight(mark){background-color:red;color:currentcolor}",
            red,
        ),
    ];
    for (css, expected) in cases {
        let mut tree = RuleTree::empty();
        tree.add_stylesheet(css, Origin::Author);
        let result = crate::cascade(&TestDoc::new(), &tree).unwrap();
        assert_eq!(
            result.custom_highlight_background("mark", blue),
            Some(expected)
        );
        assert_eq!(result.custom_highlight_background("missing", blue), None);
    }
}
