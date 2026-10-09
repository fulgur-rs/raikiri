use super::*;
use crate::cascade::cascade;
use crate::cascade::test_support::*;
use crate::computed::ComputedValues;
use crate::property::DisplayValue;
use crate::ruletree::build_rule_tree;
use crate::test_dom::TestDoc;

#[test]
fn inline_style_beats_type_selector() {
    let cv = cascade_doc("p { color: red }", "p", Some("color: blue"));
    assert_eq!(cv.color, BLUE);
}

#[test]
fn animation_origin_beats_normal_author_rules_and_inline_style() {
    let mut doc = TestDoc::new();
    let style = doc.push_element(0, "style", None);
    doc.push_text(style, "p { color: green }");
    let p = doc.push_element(0, "p", Some("color: red"));
    doc.nodes[p].animation_style = Some("color: blue".to_owned());

    let tree = build_rule_tree(&doc);
    let result = cascade(&doc, &tree).expect("cascade succeeds");

    assert_eq!(result.computed[p].color, BLUE);
}

#[test]
fn author_important_beats_animation_origin() {
    let mut doc = TestDoc::new();
    let style = doc.push_element(0, "style", None);
    doc.push_text(style, "p { color: red !important }");
    let p = doc.push_element(0, "p", Some("color: green"));
    doc.nodes[p].animation_style = Some("color: blue".to_owned());

    let tree = build_rule_tree(&doc);
    let result = cascade(&doc, &tree).expect("cascade succeeds");

    assert_eq!(result.computed[p].color, RED);
}

#[test]
fn animation_origin_inherited_value_reaches_children_unless_overridden() {
    let mut doc = TestDoc::new();
    let parent = doc.push_element(0, "div", None);
    doc.nodes[parent].animation_style = Some("color: blue".to_owned());
    let inherited_child = doc.push_element(parent, "span", None);
    let overridden_child = doc.push_element(parent, "span", Some("color: red"));

    let tree = build_rule_tree(&doc);
    let result = cascade(&doc, &tree).expect("cascade succeeds");

    assert_eq!(result.computed[inherited_child].color, BLUE);
    assert_eq!(result.computed[overridden_child].color, RED);
}

#[test]
fn outer_svg_dimension_attributes_are_overridable_geometry_hints() {
    use crate::resolve::ComputedLengthPercentageOrAuto as Dimension;
    const NS: &str = "http://www.w3.org/2000/svg";
    let mut doc = TestDoc::new();
    let style = doc.push_element(0, "style", None);
    doc.push_text(style, ".override {width:90px} * {height:40px}");
    let svg = doc.push_element_with_namespace(
        0,
        "svg",
        NS,
        &[("width", "60"), ("height", "20"), ("class", "override")],
    );
    let nested = doc.push_element_with_namespace(svg, "svg", NS, &[("width", "10")]);
    let html = doc.push_element_with_attrs(0, "svg", None, &[("width", "10")]);
    let result = cascade(&doc, &build_rule_tree(&doc)).unwrap();
    assert_eq!(result.computed[svg].width, Dimension::Px(90.0));
    assert_eq!(result.computed[svg].height, Dimension::Px(40.0));
    assert_eq!(result.computed[nested].width, Dimension::Auto);
    assert_eq!(result.computed[html].width, Dimension::Auto);

    for (raw, expected) in [
        ("60", Dimension::Px(60.0)),
        ("50%", Dimension::Percent(50.0)),
        ("12pt", Dimension::Px(16.0)),
        ("0", Dimension::Px(0.0)),
        ("auto", Dimension::Auto),
        ("-2", Dimension::Auto),
        ("NaN", Dimension::Auto),
        ("2px garbage", Dimension::Auto),
        ("3!important", Dimension::Auto),
    ] {
        let mut doc = TestDoc::new();
        let svg = doc.push_element_with_namespace(0, "svg", NS, &[("width", raw)]);
        let result = cascade(&doc, &build_rule_tree(&doc)).unwrap();
        assert_eq!(result.computed[svg].width, expected, "{raw}");
    }

    let mut doc = TestDoc::new();
    let svg = doc.push_element_with_namespace(
        0,
        "svg",
        NS,
        &[("width", "var(--size)"), ("height", "calc(50% + 10px)")],
    );
    doc.nodes[svg].inline_style = Some("--size:25px".into());
    let css = doc.push_element_with_namespace(0, "svg", NS, &[]);
    doc.nodes[css].inline_style = Some("height:calc(50% + 10px)".into());
    let result = cascade(&doc, &build_rule_tree(&doc)).unwrap();
    assert_eq!(result.computed[svg].width, Dimension::Px(25.0));
    assert_eq!(result.computed[svg].height, result.computed[css].height);
    for (css, expected) in [
        (
            "@layer x {svg{width:30px;height:15px}}",
            (Dimension::Px(30.0), Dimension::Px(15.0)),
        ),
        (
            "@layer x {svg{width:30px;height:15px;all:revert-layer}}",
            (Dimension::Px(10.0), Dimension::Px(10.0)),
        ),
    ] {
        let mut doc = TestDoc::new();
        let style = doc.push_element(0, "style", None);
        doc.push_text(style, css);
        let svg =
            doc.push_element_with_namespace(0, "svg", NS, &[("width", "10"), ("height", "10")]);
        let result = cascade(&doc, &build_rule_tree(&doc)).unwrap();
        assert_eq!(
            (result.computed[svg].width, result.computed[svg].height),
            expected
        );
    }
}

#[test]
fn inline_svg_root_opacity_attribute_is_a_stylesheet_overridable_hint() {
    const SVG_NAMESPACE: &str = "http://www.w3.org/2000/svg";

    let mut attr_only = TestDoc::new();
    let svg =
        attr_only.push_element_with_namespace(0, "svg", SVG_NAMESPACE, &[("opacity", "0.25")]);
    let rules = build_rule_tree(&attr_only);
    let result = cascade(&attr_only, &rules).expect("cascade Ok");
    assert_eq!(result.computed[svg].opacity, 0.25);
    assert!(result.opacity_specified[svg]);

    let mut stylesheet_override = TestDoc::new();
    let style = stylesheet_override.push_element(0, "style", None);
    stylesheet_override.push_text(style, ".root { opacity: 0.75 } ");
    let svg = stylesheet_override.push_element_with_namespace(
        0,
        "svg",
        SVG_NAMESPACE,
        &[("opacity", "0.25"), ("class", "root")],
    );
    let rules = build_rule_tree(&stylesheet_override);
    let result = cascade(&stylesheet_override, &rules).expect("cascade Ok");
    assert_eq!(result.computed[svg].opacity, 0.75);
    assert!(result.opacity_specified[svg]);

    let mut deferred = TestDoc::new();
    let svg = deferred.push_element_with_namespace(
        0,
        "svg",
        SVG_NAMESPACE,
        &[("opacity", "var(--svg-opacity)")],
    );
    deferred.nodes[svg].inline_style = Some("--svg-opacity:0.6".to_owned());
    let rules = build_rule_tree(&deferred);
    let result = cascade(&deferred, &rules).expect("cascade Ok");
    assert_eq!(result.computed[svg].opacity, 0.6);
    assert!(result.opacity_specified[svg]);

    let mut initial = TestDoc::new();
    let svg = initial.push_element_with_namespace(0, "svg", SVG_NAMESPACE, &[]);
    let rules = build_rule_tree(&initial);
    let result = cascade(&initial, &rules).expect("cascade Ok");
    assert!(!result.opacity_specified[svg]);
}

#[test]
fn outer_svg_dimension_hints_accept_both_axes_and_reject_intrinsic_keywords() {
    use crate::resolve::ComputedLengthPercentageOrAuto as Dimension;
    const NS: &str = "http://www.w3.org/2000/svg";

    for (raw, expected) in [
        ("12pt", Dimension::Px(16.0)),
        ("25%", Dimension::Percent(25.0)),
        ("1e999px", Dimension::Auto),
        ("min-content", Dimension::Auto),
        ("max-content", Dimension::Auto),
    ] {
        let mut doc = TestDoc::new();
        let svg = doc.push_element_with_namespace(0, "svg", NS, &[("width", raw), ("height", raw)]);
        let result = cascade(&doc, &build_rule_tree(&doc)).unwrap();
        assert_eq!(result.computed[svg].width, expected, "width={raw}");
        assert_eq!(result.computed[svg].height, expected, "height={raw}");
    }

    let mut doc = TestDoc::new();
    let hinted = doc.push_element_with_namespace(
        0,
        "svg",
        NS,
        &[
            ("width", "calc(50% + 10px)"),
            ("height", "calc(25% + 20px)"),
        ],
    );
    let styled = doc.push_element_with_namespace(0, "svg", NS, &[]);
    doc.nodes[styled].inline_style =
        Some("width:calc(50% + 10px);height:calc(25% + 20px)".to_owned());
    let result = cascade(&doc, &build_rule_tree(&doc)).unwrap();
    assert_ne!(result.computed[hinted].width, Dimension::Auto);
    assert_ne!(result.computed[hinted].height, Dimension::Auto);
    assert_eq!(result.computed[hinted].width, result.computed[styled].width);
    assert_eq!(
        result.computed[hinted].height,
        result.computed[styled].height
    );
}

#[test]
fn comma_separated_selector_list_uses_max_specificity_across_matches() {
    // `match_complex_selector_list` tracks the *best* (highest)
    // specificity across every selector in a comma-separated list that
    // matches the element — not just the first hit. An element matched
    // by 2+ selectors in the same rule's list must exercise the
    // `Some(prev) => prev.max(spec)` fold, not just its `None => spec`
    // base case.
    let mut doc = TestDoc::new();
    let s = doc.push_element(0, "style", None);
    doc.push_text(s, ".a, div.b { font-weight: bold }");
    let p = doc.push_element(0, "div", None);
    doc.set_attr(p, "class", "a b");

    let tree = build_rule_tree(&doc);
    let r = cascade(&doc, &tree).expect("cascade Ok");
    assert_eq!(r.computed[p].font_weight, 700.0);
}

#[test]
fn id_selector_specificity_beats_class_selector() {
    // CSS Cascading L4 §6.1 sort criterion (b): higher specificity wins.
    // ID (0,1,0,0) > class (0,0,1,0) — both target the same element via
    // separate rules, later source order for the loser to make sure the
    // win is attributable to specificity, not source order.
    let mut doc = TestDoc::new();
    let s = doc.push_element(0, "style", None);
    doc.push_text(s, ".foo { color: blue } #bar { color: red }");
    let div = doc.push_element(0, "div", None);
    doc.set_attr(div, "class", "foo");
    doc.set_attr(div, "id", "bar");

    let tree = build_rule_tree(&doc);
    let r = cascade(&doc, &tree).expect("cascade Ok");
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        r.computed[div].color, RED,
        "id selector must win over class selector"
    );
}

#[test]
fn inline_style_beats_maximally_specific_selector_via_cascade() {
    let mut doc = TestDoc::new();
    let s = doc.push_element(0, "style", None);
    doc.push_text(s, "#a.b.c.d { color: red }");
    let p = doc.push_element(0, "p", Some("color: blue"));
    doc.set_attr(p, "id", "a");
    doc.set_attr(p, "class", "b c d");

    let tree = build_rule_tree(&doc);
    let r = cascade(&doc, &tree).expect("cascade Ok");
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        r.computed[p].color, BLUE,
        "inline style must win over #a.b.c.d despite its high selector specificity"
    );
}

#[test]
fn descendant_combinator_selector_specificity_includes_ancestor_compound() {
    // Specificity of a complex selector accounts for every compound in
    // the chain, not just the rightmost one matched against `elem` —
    // `.chapter h2` (0,1,1,0) must beat a plain `h2` (0,0,0,1) rule
    // targeting the same element, CSS Cascading L4 §6.1 sort criterion
    // (b). Both rules are given later source order for the loser so the
    // win is attributable to specificity, not source order.
    let mut doc = TestDoc::new();
    let s = doc.push_element(0, "style", None);
    doc.push_text(s, "h2 { color: blue } .chapter h2 { color: red }");
    let div = doc.push_element(0, "div", None);
    doc.set_attr(div, "class", "chapter");
    let h2 = doc.push_element(div, "h2", None);

    let tree = build_rule_tree(&doc);
    let r = cascade(&doc, &tree).expect("cascade Ok");
    assert_eq!(
        r.computed[h2].color, RED,
        ".chapter h2 must win over plain h2 on specificity"
    );
}

#[test]
fn descendant_combinator_after_backtracking_past_a_sibling_subtree() {
    // Regression for the `ancestor_path` depth-truncation technique in
    // `collect_cascaded`: after the DFS finishes a `.wrap` subtree and
    // returns to process a *sibling* `.target`, `.target`'s own
    // ancestor_path must not still contain anything pushed while
    // visiting the sibling's subtree. `.wrap p` must not leak onto
    // `.target`'s child.
    let mut doc = TestDoc::new();
    let s = doc.push_element(0, "style", None);
    doc.push_text(s, ".wrap p { color: red }");
    let root = doc.push_element(0, "div", None);
    let wrap = doc.push_element(root, "div", None);
    doc.set_attr(wrap, "class", "wrap");
    let wrapped_p = doc.push_element(wrap, "p", None);
    let target = doc.push_element(root, "div", None);
    doc.set_attr(target, "class", "target"); // sibling of `wrap`, NOT `.wrap`
    let p_under_target = doc.push_element(target, "p", None);

    let tree = build_rule_tree(&doc);
    let r = cascade(&doc, &tree).expect("cascade Ok");
    // Positive control: without this, a matcher
    // that never matches `.wrap p` anywhere would also make the
    // negative assertion below pass vacuously — assert the rule
    // actually fired where it should have before asserting it didn't
    // leak where it shouldn't.
    assert_eq!(
        r.computed[wrapped_p].color, RED,
        ".wrap p must match its own direct target under .wrap"
    );
    assert_eq!(
        r.computed[p_under_target].color,
        ComputedValues::initial().color,
        "p under .target must not match .wrap p just because a sibling .wrap subtree was visited earlier"
    );
}

#[test]
fn inline_specificity_exceeds_max_reachable_packed_specificity() {
    // Build a selector whose specificity exceeds the inline-style
    // constant. Repeated IDs, classes, and type selectors exercise all
    // specificity components through the parser's public API.
    const FIELD_REPEAT: usize = 4096;
    // A descendant chain accumulates type-selector specificity across
    // compounds, while the final compound accumulates IDs and classes.
    const TYPE_CHAIN_REPEAT: usize = 1200;
    let type_chain: String = "div ".repeat(TYPE_CHAIN_REPEAT);
    let ids: String = "#a".repeat(FIELD_REPEAT);
    let classes: String = ".b".repeat(FIELD_REPEAT);
    let selector_str = format!("{type_chain}div{ids}{classes}:hover:active");
    let list = crate::parse_selector_list(&selector_str).expect("maximal selector must parse");
    let measured_specificity: Specificity = list
        .slice()
        .iter()
        .map(|s| s.specificity())
        .max()
        .expect("selector list is non-empty");

    assert!(
        INLINE_SPECIFICITY > measured_specificity,
        "inline specificity must exceed the strongest supported selector"
    );
}

#[test]
fn type_selector_beats_universal() {
    let cv = cascade_doc("* { color: red } p { color: blue }", "p", None);
    assert_eq!(cv.color, BLUE);
}

#[test]
fn important_beats_normal_across_specificity() {
    // A universal !important declaration beats a normal type-selector declaration.
    let cv = cascade_doc("p { color: red } * { color: blue !important }", "p", None);
    assert_eq!(cv.color, BLUE);
}

#[test]
fn source_order_tiebreak_later_wins() {
    let cv = cascade_doc("p { color: red } p { color: blue }", "p", None);
    assert_eq!(cv.color, BLUE);
}

#[test]
fn later_duplicate_in_same_rule_wins() {
    // Duplicate properties in one rule: CSS Cascading L4 §6.1 "Order of
    // Appearance" <https://www.w3.org/TR/css-cascade-4/#cascade-sort>:
    // "The last declaration in document order wins."
    let cv = cascade_doc("p { color: red; color: blue }", "p", None);
    assert_eq!(cv.color, BLUE);
}

#[test]
fn later_duplicate_in_inline_wins() {
    // In an inline style with duplicate properties, the later one also wins.
    let cv = cascade_doc("", "p", Some("color: red; color: blue"));
    assert_eq!(cv.color, BLUE);
}

#[test]
fn winner_does_not_leak_into_next_sibling() {
    let mut doc = TestDoc::new();
    let s = doc.push_element(0, "style", None);
    doc.push_text(
        s,
        "p { background-color: red } span { font-weight: bolder }",
    );
    let wrapper = doc.push_element(0, "div", None);
    let p = doc.push_element(wrapper, "p", None);
    let span = doc.push_element(wrapper, "span", None);
    let tree = build_rule_tree(&doc);
    let r = cascade(&doc, &tree).unwrap();

    let initial = ComputedValues::initial();
    assert_eq!(r.computed[p].background_color, RED);
    assert_eq!(
        r.computed[span].background_color, initial.background_color,
        "span に p の background-color winner が漏れた"
    );
    // Parent <div> has the initial weight 400. Per the CSS Fonts 4 §2.2.1 table,
    // 350 <= 400 < 550 gives bolder = 700. Applying it twice would give 900.
    assert_eq!(
        r.computed[span].font_weight, 700.0,
        "font-weight: bolder が 2 回適用された (slot leak による二重 drain)"
    );
}

#[test]
fn collect_cascaded_groups_are_unchanged_by_flat_arena_refactor() {
    let mut doc = TestDoc::new();
    let s = doc.push_element(0, "style", None);
    doc.push_text(s, "p { color: red } p { background-color: blue }");

    let p1 = doc.push_element(0, "p", Some("display: block"));
    let p2 = doc.push_element(0, "p", None);
    let empty = doc.push_element(0, "hr", None);
    let p3 = doc.push_element(0, "p", Some("display: inline"));

    let tree = build_rule_tree(&doc);
    let mut arena = CascadedArena::new();
    collect_cascaded(&doc, doc.root_id(), &tree, &mut arena);

    let id = |i: usize| StyleNodeId::new(i as u64);

    // `hr` matches no rule and has no inline style — old code's
    // `if !per_node.is_empty()` guard meant no map entry at all; the
    // arena must not create a zero-length range for it either.
    assert!(
        arena.candidates(id(empty)).is_none(),
        "element with zero candidate declarations must get no arena entry"
    );

    // p2: 2 stylesheet decls, no inline — order = rule/source order.
    let p2c = arena.candidates(id(p2)).expect("p2 has 2 stylesheet decls");
    assert_eq!(p2c.len(), 2);
    assert_eq!(p2c[0].value, PropertyValue::Color(RED));
    assert_eq!(p2c[1].value, PropertyValue::BackgroundColor(BLUE));
    assert_eq!(p2c[0].source_order, 0, "first rule keeps its source_order");
    assert_eq!(p2c[1].source_order, 1, "second rule keeps its source_order");
    // Every candidate carries the key of its value.
    assert_eq!(p2c[0].key, PropertyKey::Color);
    assert_eq!(p2c[1].key, PropertyKey::BackgroundColor);

    // p1 / p3: same 2 stylesheet decls, PLUS inline style appended last
    // (collect_cascaded pushes stylesheet rules before inline style).
    let p1c = arena.candidates(id(p1)).expect("p1 has decls");
    assert_eq!(p1c.len(), 3, "2 stylesheet decls + 1 inline, inline last");
    assert_eq!(p1c[0].value, PropertyValue::Color(RED));
    assert_eq!(p1c[1].value, PropertyValue::BackgroundColor(BLUE));
    assert_eq!(p1c[2].value, PropertyValue::Display(DisplayValue::Block));
    assert_eq!(p1c[2].key, PropertyKey::Display);
    assert_eq!(p1c[2].specificity, INLINE_SPECIFICITY);
    assert_eq!(p1c[2].source_order, INLINE_SOURCE_ORDER);

    let p3c = arena.candidates(id(p3)).expect("p3 has decls");
    assert_eq!(p3c.len(), 3);
    assert_eq!(p3c[2].value, PropertyValue::Display(DisplayValue::Inline));

    // Stylesheet-only decls (p1/p2/p3 all matched the same 2 `p` rules)
    // carry identical specificity to each other — cross-node consistency
    // the old shared-selector-per-rule code guaranteed too.
    assert_eq!(p1c[0].specificity, p2c[0].specificity);
    assert_eq!(p2c[0].specificity, p3c[0].specificity);

    // Ranges must not overlap — a flat arena has to hold this invariant
    // that per-node `Vec`s never needed to: if two nodes' ranges ever
    // overlapped, `candidates(id)` would silently hand `pick_winners` a
    // slice containing another node's declarations too
    // (a known "global index space" hazard, arena-shaped).
    //
    // The `windows(2)` loop below only checks pairwise overlap among the
    // three explicitly-named nodes (p1/p2/p3) — it would miss a stray
    // arena slot that belongs to no range, or one double-counted across
    // two ranges. The load-bearing check for "no slot unaccounted for"
    // is the trailing `assert_eq!` after the loop: it compares the
    // arena's total length against the sum of every named range's
    // length, so any leaked/duplicated/orphaned slot shows up as a
    // length mismatch even if no two of the three named ranges overlap
    // each other directly.
    let mut ranges: Vec<_> = [p1, p2, p3]
        .iter()
        .map(|&n| arena.ranges.get(&id(n)).unwrap().clone())
        .collect();
    ranges.sort_by_key(|r| r.start);
    for w in ranges.windows(2) {
        assert!(
            w[0].end <= w[1].start,
            "per-node ranges must not overlap: {:?} vs {:?}",
            w[0],
            w[1]
        );
    }
    assert_eq!(
        arena.decls.len(),
        ranges.iter().map(|r| r.len()).sum::<usize>(),
        "every arena slot belongs to exactly one node's range"
    );
}

#[test]
fn ua_display_block_applied_when_no_author_rule() {
    // UA CSS alone makes <p> display as Block.
    let cv = cascade_with_ua("p { display: block }", "", "p", None);
    assert_eq!(cv.display, DisplayValue::Block);
}

#[test]
fn author_display_inline_overrides_ua_block() {
    // Normal Author > Normal UA (`cascade_rank` doc has the exact values)
    let cv = cascade_with_ua("p { display: block }", "p { display: inline }", "p", None);
    assert_eq!(cv.display, DisplayValue::Inline);
}

#[test]
fn important_ua_beats_important_author_display() {
    // Important UA > Important Author (!important reverses origin order; `cascade_rank`
    // doc has the exact values)
    let cv = cascade_with_ua(
        "p { display: block !important }",
        "p { display: inline !important }",
        "p",
        None,
    );
    assert_eq!(cv.display, DisplayValue::Block);
}

#[test]
fn cascade_rank_orders_ua_user_hint_author_normal_then_reverses_for_important() {
    // Check the complete ordering without depending on the internal rank
    // numbers; consumers compare ranks rather than interpreting their
    // numeric representation.
    //
    // The Normal-tier ordering (UA < User < hint < Author) combines two
    // spec-verbatim facts: CSS Cascading L4 §6.1's origin list gives
    // UA < User < Author directly, and CSS Cascading L5 §6.5's verbatim
    // text places the hint strictly between User and Author.
    //
    // The Important-tier ordering (Author < hint < User < UA) also
    // combines two facts, but only one is spec-verbatim: §6.1 directly
    // gives Author < User < UA for the important tier — exactly the
    // reverse of the normal-tier UA < User < Author order, read
    // straight off §6.1's list, not an analogy. The hint's position in
    // that reversal is *not* spec-verbatim: §6.5 never defines an
    // important presentational hint (host languages only ever emit
    // normal-tier hints), so this half pins `cascade_rank`'s own
    // minimal, non-arbitrary extension of that same reversal mechanism
    // to the hint (origin independence + CSS Cascading L4 §6.3's
    // importance reversal, <https://www.w3.org/TR/css-cascade-4/#importance>)
    // and its status as a total function, not an external requirement.
    // No production code path emits an `!important` presentational
    // hint (`push_img_dimension_hints` always pushes
    // `important = false`), so `(AuthorPresentationalHint, true)`
    // stays production-unreached. `(User, false)` / `(User, true)`
    // used to be production-unreached too (no code path routed any
    // declaration to `Origin::User`) until consumer `extra_stylesheets`
    // was wired to it — this test remains the only
    // place `(AuthorPresentationalHint, true)` is exercised, but the
    // two `User` arms now also have real end-to-end coverage via
    // `crates/raikiri/tests/build_cascaded.rs`'s
    // `extra_stylesheets_user_rule_overrides_ua_via_umbrella` (normal)
    // and `user_important_beats_normal_ua_via_umbrella` (important).
    let normal_ua = cascade_rank(Origin::UserAgent, false);
    let normal_user = cascade_rank(Origin::User, false);
    let normal_hint = cascade_rank(Origin::AuthorPresentationalHint, false);
    let normal_author = cascade_rank(Origin::Author, false);
    let animation = cascade_rank(Origin::Animation, false);
    let important_author = cascade_rank(Origin::Author, true);
    let important_hint = cascade_rank(Origin::AuthorPresentationalHint, true);
    let important_user = cascade_rank(Origin::User, true);
    let important_ua = cascade_rank(Origin::UserAgent, true);

    // Spec-verbatim (§6.1): Normal UA < Normal User < Normal Author.
    //
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert!(
        normal_ua < normal_user,
        "normal UA must lose to normal user"
    );
    // Spec-verbatim (§6.5): the hint sits strictly between Normal User
    // and Normal Author. `normal_user < normal_author` follows
    // transitively from this assert and the next one, so it isn't
    // pinned separately.
    //
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert!(
        normal_user < normal_hint,
        "normal user must lose to the hint"
    );
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert!(
        normal_hint < normal_author,
        "normal hint must lose to a real author declaration"
    );
    // Spec-verbatim (§6.1/§6.3): any important declaration beats any
    // normal declaration.
    //
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert!(
        normal_author < important_author,
        "any important declaration must beat any normal declaration"
    );
    assert!(
        normal_author < animation && animation < important_author,
        "animation declarations must rank above normal author and below important author"
    );
    // Spec-verbatim (§6.1): Important Author < Important User < Important UA.
    //
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert!(
        important_user < important_ua,
        "important user must lose to important UA"
    );
    // Derived (not spec-verbatim, see comment above): symmetry of
    // origin independence under importance reversal places the hint
    // strictly between Important Author and Important User, mirroring
    // its Normal-tier position between Normal User and Normal Author.
    //
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert!(
        important_author < important_hint,
        "derived symmetry: important hint ranks above important author"
    );
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert!(
        important_hint < important_user,
        "derived symmetry: important user ranks above important hint"
    );
}

#[test]
fn pseudo_element_selector_specificity_participates_in_cascade_ranking() {
    // The pseudo arena runs through the same `pick_winners`/`beats`
    // ranking as the real-element arena — this pins that
    // `specificity_of(selector)` (not some constant) is actually what
    // gets passed through for the `::before` path specifically.
    // `.foo::before` (0,1,0 + pseudo-element count) must beat plain
    // `p::before` (0,0,1 + pseudo-element count) *despite* losing on
    // source order — `.foo::before` is declared **first** here
    // deliberately, so that if the specificity value were accidentally
    // dropped (e.g. a constant passed instead of
    // `specificity_of(selector)`), the later, lower-specificity
    // `p::before` would win on the source-order tie-break instead and
    // this assertion would catch it.
    use crate::property::ContentComponent;
    let mut doc = TestDoc::new();
    let s = doc.push_element(0, "style", None);
    doc.push_text(
        s,
        r#".foo::before { content: "higher-specificity" } p::before { content: "later-source-order" }"#,
    );
    let p = doc.push_element(0, "p", None);
    doc.set_attr(p, "class", "foo");

    let tree = build_rule_tree(&doc);
    let r = cascade(&doc, &tree).expect("cascade Ok");
    let before = &r.pseudo[&(StyleNodeId(p as u64), PseudoElem::Before)];
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        *before.content,
        vec![ContentComponent::Literal("higher-specificity".into())],
        "higher-specificity .foo::before must win over the \
         later-declared, lower-specificity p::before"
    );
}

#[test]
#[should_panic(expected = "pick_winners は空の scratch buffer を要求する")]
fn pick_winners_panics_on_non_empty_scratch_buffer() {
    let mut winners: Vec<Option<RankedDecl>> = vec![Some(RankedDecl {
        rank: 0,
        layer_priority: (false, u32::MAX),
        specificity: 0,
        source_order: 0,
        idx: 0,
    })];
    pick_winners(&[], &mut winners);
}

fn source_hash(source: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    source.hash(&mut hasher);
    hasher.finish()
}

#[test]
fn declaration_block_cache_stores_a_source_on_its_second_sight() {
    let tree = RuleTree::empty();
    let mut cache = DeclarationBlockCache::default();
    let first = cache
        .declarations("color: red; margin: 1px", &tree)
        .into_owned();
    assert!(matches!(
        cache.entries.get(&source_hash("color: red; margin: 1px")),
        Some(DeclarationBlockEntry::SeenOnce)
    ));
    assert!(matches!(
        cache.declarations("color: red; margin: 1px", &tree),
        Cow::Borrowed(_)
    ));
    let third = cache.declarations("color: red; margin: 1px", &tree);
    assert!(matches!(third, Cow::Borrowed(_)));
    // `margin` is expanded to its four longhands either way.
    assert_eq!(first.len(), 5);
    assert_eq!(third.as_ref(), first.as_slice());
}

#[test]
fn declaration_block_cache_never_reuses_a_colliding_source() {
    let tree = RuleTree::empty();
    let mut cache = DeclarationBlockCache::default();
    cache.entries.insert(
        source_hash("color: red"),
        DeclarationBlockEntry::Parsed("color: blue".into(), Vec::new()),
    );
    let parsed = cache.declarations("color: red", &tree);
    assert!(matches!(parsed, Cow::Owned(_)));
    assert_eq!(parsed.len(), 1);
}

#[test]
fn repeated_inline_styles_cascade_like_unique_ones() {
    let mut doc = TestDoc::new();
    let ids = (0..4)
        .map(|i| {
            let style = if i == 2 {
                "opacity: 0.5"
            } else {
                "color: blue; opacity: 0.25"
            };
            doc.push_element(0, "p", Some(style))
        })
        .collect::<Vec<_>>();
    let tree = build_rule_tree(&doc);
    let result = cascade(&doc, &tree).expect("cascade Ok");
    for (i, id) in ids.into_iter().enumerate() {
        let cv = &result.computed[id];
        if i == 2 {
            assert_eq!(cv.opacity, 0.5);
        } else {
            assert_eq!(cv.color, BLUE);
            assert_eq!(cv.opacity, 0.25);
        }
        assert!(result.opacity_specified[id]);
        assert!(!result.background_color_specified[id]);
    }
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "stale declaration key")]
fn a_value_written_past_update_value_is_caught() {
    let mut decl = Declaration::new(PropertyValue::Opacity(1.0), false);
    decl.value = PropertyValue::Color(crate::property::CssColor::BLACK);
    let _ = CascadedDecl::new(decl, Origin::Author, 0, 0, LayerPosition::default());
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "stale declaration rollback")]
fn a_rollback_kind_left_behind_by_a_value_write_is_caught() {
    use crate::property::{BorderColor, CssWideKeyword};
    let mut decl = Declaration::new(
        PropertyValue::BorderTopColor(BorderColor::CurrentColor),
        false,
    );
    decl.value = PropertyValue::BorderTopColorCssWide(CssWideKeyword::Revert);
    let _ = CascadedDecl::new(decl, Origin::Author, 0, 0, LayerPosition::default());
}
