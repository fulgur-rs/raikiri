mod layer_tests;
mod nesting_tests;

use super::*;
use crate::cascade::test_support::*;
use crate::ruletree::{Origin, build_rule_tree};
use crate::test_dom::TestDoc;

#[test]
fn empty_dom_root_has_initial() {
    let doc = TestDoc::new();
    let tree = build_rule_tree(&doc);
    let r = cascade(&doc, &tree).unwrap();
    assert_eq!(r.computed[0], ComputedValues::initial());
}

fn context_cascade_doc(css: &str, context: MediaContext) -> (TestDoc, usize, CascadeResult) {
    let mut doc = TestDoc::new();
    let s = doc.push_element(0, "style", None);
    doc.push_text(s, css);
    let element = doc.push_element(0, "p", None);
    let tree = build_rule_tree(&doc);
    let result = cascade_with_media_context(&doc, &tree, &context).expect("cascade Ok");
    (doc, element, result)
}

#[test]
fn media_context_selects_print_and_screen_rules() {
    let css = "@media print { p { color: red } } @media screen { p { color: blue } }";
    let (_, element, print_result) = context_cascade_doc(css, MediaContext::print());
    assert_eq!(print_result.computed[element].color, RED);
    let (_, element, screen_result) = context_cascade_doc(css, MediaContext::screen());
    assert_eq!(screen_result.computed[element].color, BLUE);
}

#[test]
fn media_modifiers_select_rules_and_conjoin_nested_conditions() {
    for css in [
        "p { color: blue } @media not screen { p { color: red } }",
        "p { color: blue } @media only print { p { color: red } }",
        "p { color: blue } @media not all, only print { p { color: red } }",
        "p { color: blue } @media not screen { @media only print { p { color: red } } }",
    ] {
        let (_, element, result) = context_cascade_doc(css, MediaContext::print());
        assert_eq!(result.computed[element].color, RED, "{css}");
        let (_, element, result) = context_cascade_doc(css, MediaContext::screen());
        assert_eq!(result.computed[element].color, BLUE, "{css}");
    }

    let css = "p { color: red } @media not print { p { color: blue } }";
    let (_, element, result) = context_cascade_doc(css, MediaContext::screen());
    assert_eq!(result.computed[element].color, BLUE);

    let css = "p { color: blue } @media not screen { @media only screen { p { color: red } } }";
    for context in [MediaContext::print(), MediaContext::screen()] {
        let (_, element, result) = context_cascade_doc(css, context);
        assert_eq!(result.computed[element].color, BLUE);
    }
}

#[test]
fn media_all_matches_both_contexts_and_default_is_print() {
    let css = "@media all { p { color: red } }";
    let (_, element, default_result) = context_cascade_doc(css, MediaContext::default());
    assert_eq!(default_result.computed[element].color, RED);
    let (_, element, screen_result) = context_cascade_doc(css, MediaContext::screen());
    assert_eq!(screen_result.computed[element].color, RED);
}

#[test]
fn media_rules_keep_source_order_against_direct_rules() {
    let css = "p { color: red } @media print { p { color: blue } } p { color: red }";
    let (_, element, result) = context_cascade_doc(css, MediaContext::print());
    assert_eq!(result.computed[element].color, RED);

    let css = "p { color: red } @media print { p { color: blue } }";
    let (_, element, result) = context_cascade_doc(css, MediaContext::print());
    assert_eq!(result.computed[element].color, BLUE);
}

#[test]
fn nested_media_conditions_are_conjoined_and_unknown_wrappers_do_not_leak() {
    let css =
        "@media print { @media all { p { color: red } } @media screen { p { color: blue } } }";
    let (_, element, print_result) = context_cascade_doc(css, MediaContext::print());
    assert_eq!(print_result.computed[element].color, RED);
    let (_, element, screen_result) = context_cascade_doc(css, MediaContext::screen());
    assert_ne!(screen_result.computed[element].color, RED);

    let css = "@future feature { @media print { p { color: red } } }";
    let (_, element, result) = context_cascade_doc(css, MediaContext::print());
    assert_ne!(result.computed[element].color, RED);
}

#[test]
fn conditional_groups_reach_cascade_only_when_all_conditions_match() {
    for css in [
        "p {color:blue} @supports (display:block) {@media print {p {color:red}}}",
        "p {color:blue} @media print {@supports (color:red) {p {color:red}}}",
        "p {color:blue} @supports (color:red) {@supports (display:block) {\
         @media print {p {color:red}}}}",
    ] {
        let (_, element, result) = context_cascade_doc(css, MediaContext::print());
        assert_eq!(result.computed[element].color, RED, "{css}");
        let (_, element, result) = context_cascade_doc(css, MediaContext::screen());
        assert_eq!(result.computed[element].color, BLUE, "{css}");
    }
    for css in [
        "p {color:blue} @media print {@supports (display:invalid) {p {color:red}}}",
        "p {color:blue} @supports (color:red) {@supports (display:invalid) {\
         @media print {p {color:red}}}}",
    ] {
        let (_, element, result) = context_cascade_doc(css, MediaContext::print());
        assert_eq!(result.computed[element].color, BLUE, "{css}");
    }
}

#[test]
fn layers_inside_supports_do_not_override_unlayered_styles() {
    let css = "p {color:blue} @supports (color:red) {div {color:blue} \
        @layer a {p {color:red}}}";
    let (_, element, result) = context_cascade_doc(css, MediaContext::print());
    assert_eq!(result.computed[element].color, BLUE);
}

#[test]
fn media_comma_list_and_invalid_features_are_safe() {
    let css = "@media print, projection { p { color: red } }";
    let (_, element, print_result) = context_cascade_doc(css, MediaContext::print());
    assert_eq!(print_result.computed[element].color, RED);
    let (_, element, screen_result) = context_cascade_doc(css, MediaContext::screen());
    assert_ne!(screen_result.computed[element].color, RED);

    let css = "@media print and (color) { p { color: red } }";
    let (_, element, result) = context_cascade_doc(css, MediaContext::print());
    assert_ne!(result.computed[element].color, RED);

    let css = "@media print, { p { color: red } }";
    let (_, element, result) = context_cascade_doc(css, MediaContext::print());
    assert_ne!(result.computed[element].color, RED);
}

#[test]
fn media_condition_is_applied_to_pseudo_elements() {
    let css = "@media screen { p::before { content: \"x\"; color: red } }";
    let (doc, element, print_result) = context_cascade_doc(css, MediaContext::print());
    let element_id = StyleNodeId::new(element as u64);
    assert!(
        !print_result
            .pseudo
            .contains_key(&(element_id, PseudoElem::Before))
    );
    let (_, element, screen_result) = context_cascade_doc(css, MediaContext::screen());
    let element_id = StyleNodeId::new(element as u64);
    assert!(
        screen_result
            .pseudo
            .contains_key(&(element_id, PseudoElem::Before))
    );
    assert_eq!(doc.node_count(), 4);
}

#[test]
fn first_line_pseudo_element_absent_without_matching_rule() {
    let css = "p { color: blue }";
    let (_, element, result) = context_cascade_doc(css, MediaContext::default());
    let element_id = StyleNodeId::new(element as u64);
    assert!(
        !result
            .pseudo
            .contains_key(&(element_id, PseudoElem::FirstLine))
    );
}

#[test]
fn first_line_pseudo_element_inherits_and_overrides() {
    let css = "p { color: blue; font-weight: bold } p::first-line { color: red }";
    let (_, element, result) = context_cascade_doc(css, MediaContext::default());
    let element_id = StyleNodeId::new(element as u64);
    let pseudo = result
        .pseudo
        .get(&(element_id, PseudoElem::FirstLine))
        .expect("a matching ::first-line rule must produce a pseudo entry");
    // Own declaration wins over the originating element's value.
    assert_eq!(pseudo.color, RED);
    // Not set by the `::first-line` rule, so it is inherited unchanged
    // from the originating element, same as a real child would inherit
    // it (CSS Pseudo-Elements Module Level 4 §4 `#treelike`, which this
    // crate applies uniformly to every entry in `CascadeResult::pseudo`
    // regardless of whether that specific pseudo-element is itself
    // tree-abiding — see `PseudoElem` doc).
    assert_eq!(pseudo.font_weight, result.computed[element].font_weight);
}

#[test]
fn cascade_result_is_send() {
    fn assert_send<T: Send>() {}
    assert_send::<CascadeResult>();
}

#[test]
fn cascade_deterministic_across_10_runs() {
    let mut doc = TestDoc::new();
    let s = doc.push_element(0, "style", None);
    doc.push_text(s, "p { color: red } * { color: blue !important }");
    let p = doc.push_element(0, "p", Some("font-size: 20px"));
    doc.push_element(p, "span", None);
    let tree = build_rule_tree(&doc);

    let baseline = cascade(&doc, &tree).unwrap();
    for _ in 0..9 {
        let run = cascade(&doc, &tree).unwrap();
        assert_eq!(run.computed.len(), baseline.computed.len());
        for i in 0..run.computed.len() {
            assert_eq!(run.computed[i], baseline.computed[i], "differ at node {i}");
        }
    }
}

fn deep_chain_doc(depth: usize) -> (TestDoc, usize) {
    let mut doc = TestDoc::new();
    let mut parent = 0usize;
    for _ in 0..depth {
        parent = doc.push_element(parent, "div", None);
    }
    let style = doc.push_element(parent, "style", None);
    doc.push_text(style, "div { color: red }");
    (doc, parent)
}

#[test]
fn deep_nesting_5000_cascade_no_overflow() {
    let (doc, deepest) = deep_chain_doc(5000);
    let tree = build_rule_tree(&doc);
    let result = cascade(&doc, &tree).expect("cascade Ok");
    assert_eq!(result.computed.len(), doc.nodes.len());
    assert_eq!(result.computed[deepest].color, RED);
}

#[test]
fn deep_nesting_small_stack_no_overflow() {
    let handle = std::thread::Builder::new()
        .stack_size(128 * 1024)
        .spawn(|| {
            let (doc, deepest) = deep_chain_doc(500);
            let tree = build_rule_tree(&doc);
            let result = cascade(&doc, &tree).expect("cascade Ok");
            result.computed[deepest].color
        })
        .expect("spawn thread");
    let color = handle
        .join()
        .expect("thread must not stack-overflow on deep DOM");
    assert_eq!(color, RED);
}

#[test]
fn cascade_with_ua_deterministic_across_10_runs() {
    // determinism regression (acceptance criteria for this stage of work)
    let mut doc = TestDoc::new();
    let s = doc.push_element(0, "style", None);
    doc.push_text(s, "p { color: red }");
    let p = doc.push_element(0, "p", Some("font-size: 20px"));
    doc.push_element(p, "span", None);

    let mut tree = build_rule_tree(&doc);
    tree.add_stylesheet(
        "p { display: block } span { display: inline }",
        Origin::UserAgent,
    );

    let baseline = cascade(&doc, &tree).unwrap();
    for _ in 0..9 {
        let run = cascade(&doc, &tree).unwrap();
        assert_eq!(run.computed.len(), baseline.computed.len());
        for i in 0..run.computed.len() {
            assert_eq!(run.computed[i], baseline.computed[i], "differ at node {i}");
        }
    }
}

use crate::property::{TextWrapMode, WhiteSpace, WhiteSpaceCollapse};

fn effective(css: &str, inline: Option<&str>) -> (WhiteSpaceCollapse, TextWrapMode) {
    let cv = cascade_doc(css, "div", inline);
    (
        cv.effective_white_space_collapse,
        cv.effective_text_wrap_mode,
    )
}

#[test]
fn a_later_white_space_longhand_overrides_the_legacy_keyword() {
    assert_eq!(
        effective("", Some("white-space:pre;white-space-collapse:collapse")),
        (WhiteSpaceCollapse::Collapse, TextWrapMode::Nowrap)
    );
    assert_eq!(
        effective("", Some("white-space:nowrap;white-space-collapse:preserve")),
        (WhiteSpaceCollapse::Preserve, TextWrapMode::Nowrap)
    );
}

#[test]
fn a_later_legacy_keyword_overrides_an_earlier_longhand() {
    // Drained in `PropertyKey` order, the legacy keyword would be applied
    // first; the written order decides instead.
    assert_eq!(
        effective("", Some("white-space-collapse:preserve;white-space:nowrap")),
        (WhiteSpaceCollapse::Collapse, TextWrapMode::Nowrap)
    );
}

#[test]
fn a_wrap_longhand_changes_only_the_wrap_half_in_either_order() {
    assert_eq!(
        effective("", Some("white-space:pre-wrap;text-wrap-mode:nowrap")),
        (WhiteSpaceCollapse::Preserve, TextWrapMode::Nowrap)
    );
    // The legacy keyword written later wins the wrap half too.
    assert_eq!(
        effective("", Some("text-wrap-mode:nowrap;white-space:pre-wrap")),
        (WhiteSpaceCollapse::Preserve, TextWrapMode::Wrap)
    );
}

#[test]
fn a_legacy_keyword_alone_sets_both_halves() {
    assert_eq!(
        effective("", Some("white-space:pre")),
        (WhiteSpaceCollapse::Preserve, TextWrapMode::Nowrap)
    );
}

#[test]
fn a_more_specific_longhand_beats_a_later_legacy_keyword() {
    // The longhand is decided by cascade rank, not by where it was written:
    // `div` is more specific than `*`, so its longhand wins in either order.
    let expected = (WhiteSpaceCollapse::Collapse, TextWrapMode::Nowrap);
    assert_eq!(
        effective(
            "* { white-space: pre } div { white-space-collapse: collapse }",
            None
        ),
        expected
    );
    assert_eq!(
        effective(
            "div { white-space-collapse: collapse } * { white-space: pre }",
            None
        ),
        expected
    );
}

#[test]
fn a_more_specific_legacy_keyword_beats_a_longhand_written_later() {
    assert_eq!(
        effective(
            "div { white-space: pre } * { white-space-collapse: collapse }",
            None
        ),
        (WhiteSpaceCollapse::Preserve, TextWrapMode::Nowrap)
    );
}

#[test]
fn a_legacy_keyword_from_a_custom_property_sets_the_effective_values() {
    // The winner arrives as a deferred `var()` value; the effective values
    // follow what it resolves to, like the legacy field does.
    let cv = cascade_doc(
        "",
        "div",
        Some("--ws:pre;white-space-collapse:preserve-breaks;white-space:var(--ws)"),
    );
    assert_eq!(cv.white_space, WhiteSpace::Pre);
    assert_eq!(
        (
            cv.effective_white_space_collapse,
            cv.effective_text_wrap_mode
        ),
        (WhiteSpaceCollapse::Preserve, TextWrapMode::Nowrap)
    );
}

#[test]
fn a_text_wrap_shorthand_sets_the_wrap_half_like_its_longhand() {
    assert_eq!(
        effective("", Some("white-space:pre;text-wrap:wrap")),
        (WhiteSpaceCollapse::Preserve, TextWrapMode::Wrap)
    );
    assert_eq!(
        effective(
            "",
            Some("--w:nowrap;white-space:pre-wrap;text-wrap:var(--w)")
        ),
        (WhiteSpaceCollapse::Preserve, TextWrapMode::Nowrap)
    );
}

#[test]
fn the_effective_values_are_inherited() {
    let mut doc = TestDoc::new();
    let parent = doc.push_element(0, "div", Some("white-space:pre-wrap"));
    let child = doc.push_element(parent, "span", None);
    let tree = build_rule_tree(&doc);
    let result = cascade(&doc, &tree).expect("cascade Ok");
    assert_eq!(
        result.computed[child].effective_white_space_collapse,
        WhiteSpaceCollapse::Preserve
    );
    assert_eq!(
        result.computed[child].effective_text_wrap_mode,
        TextWrapMode::Wrap
    );
}

#[test]
fn an_undeclared_half_keeps_the_inherited_effective_value() {
    // The parent's longhand wins over its legacy keyword; the child declares
    // only the wrap longhand, so its collapse half is the parent's effective
    // one, not the one its inherited legacy keyword stands for.
    let mut doc = TestDoc::new();
    let parent = doc.push_element(
        0,
        "div",
        Some("white-space:pre;white-space-collapse:collapse"),
    );
    let child = doc.push_element(parent, "span", Some("text-wrap-mode:wrap"));
    let tree = build_rule_tree(&doc);
    let result = cascade(&doc, &tree).expect("cascade Ok");
    assert_eq!(result.computed[child].white_space, WhiteSpace::Pre);
    assert_eq!(
        (
            result.computed[child].effective_white_space_collapse,
            result.computed[child].effective_text_wrap_mode
        ),
        (WhiteSpaceCollapse::Collapse, TextWrapMode::Wrap)
    );
}

#[test]
fn the_legacy_white_space_value_is_unchanged_by_a_longhand() {
    // The computed `white-space` is what the CSSOM serializes; the effective
    // fields are additive and must not rewrite it.
    let cv = cascade_doc(
        "",
        "div",
        Some("white-space:pre;white-space-collapse:collapse"),
    );
    assert_eq!(cv.white_space, WhiteSpace::Pre);
    assert_eq!(cv.white_space_collapse, WhiteSpaceCollapse::Collapse);
}

#[test]
fn every_legacy_keyword_maps_to_a_collapse_and_wrap_pair() {
    use TextWrapMode as W;
    use WhiteSpace::*;
    use WhiteSpaceCollapse as C;
    assert_eq!(Normal.collapse_and_wrap(), Some((C::Collapse, W::Wrap)));
    assert_eq!(Pre.collapse_and_wrap(), Some((C::Preserve, W::Nowrap)));
    assert_eq!(Nowrap.collapse_and_wrap(), Some((C::Collapse, W::Nowrap)));
    assert_eq!(PreWrap.collapse_and_wrap(), Some((C::Preserve, W::Wrap)));
    assert_eq!(
        PreLine.collapse_and_wrap(),
        Some((C::PreserveBreaks, W::Wrap))
    );
    assert_eq!(
        BreakSpaces.collapse_and_wrap(),
        Some((C::BreakSpaces, W::Wrap))
    );
}

fn page_queries() -> Vec<PageContextQuery> {
    let mut queries = Vec::new();
    for (name, is_first, is_left) in [
        (None, true, false),
        (None, false, true),
        (None, false, false),
        (Some("chapter"), false, true),
        (Some("chapter"), true, false),
        (Some("unknown"), false, false),
    ] {
        queries.push(PageContextQuery {
            page_name: name.map(crate::Atom::from),
            is_first,
            is_left,
            is_right: !is_left,
            ..PageContextQuery::default()
        });
    }
    queries.push(PageContextQuery::default());
    queries
}

#[test]
fn page_only_cascade_from_root_element_matches_the_full_page_cascade() {
    let css = "html { font-size: 20px; color: red }\
        @page { margin: 2em; size: 400px 300px }\
        @page :first { margin-top: 3em }\
        @page :left { margin-left: 10px; @top-left { content: \"L\" } }\
        @page :right { margin-right: 1.5em; @top-right { content: \"R\" } }\
        @page chapter { padding: 1em; @bottom-center { content: \"C\" } }\
        @page chapter:first { size: 200px 100px }";
    let mut doc = TestDoc::new();
    let html = doc.push_element(0, "html", None);
    let style = doc.push_element(html, "style", None);
    doc.push_text(style, css);
    doc.push_element(html, "body", None);
    let tree = build_rule_tree(&doc);
    let media = MediaContext::default();
    let base = cascade_with_media_context(&doc, &tree, &media).expect("cascade Ok");
    assert_eq!(
        base.root_element_computed() as *const ComputedValues,
        &base.computed[html] as *const ComputedValues
    );

    for query in page_queries() {
        let full =
            cascade_with_media_context_for_page(&doc, &tree, &media, &query).expect("cascade Ok");
        let page_only = cascade_page_with_media_context(
            &tree,
            &query,
            PageInheritance::FromRoot(base.root_element_computed()),
            &media,
        );
        assert_eq!(page_only, full.page, "page query {query:?}");
    }

    // The root-inherited font-size absolutizes `2em` to 40px rather than the
    // initial 16px basis.
    let first = cascade_page_with_media_context(
        &tree,
        &PageContextQuery::default(),
        PageInheritance::FromRoot(base.root_element_computed()),
        &media,
    );
    assert_eq!(
        first.declarations().get(&PropertyKey::MarginBottom),
        Some(&crate::property::PropertyValue::MarginBottom(
            crate::property::LengthOrAuto::Length(crate::property::Length::Px(40.0))
        ))
    );
}

#[test]
fn root_element_computed_falls_back_to_the_document_without_an_element() {
    let mut doc = TestDoc::new();
    let text = doc.push_text(0, "text only");
    let tree = build_rule_tree(&doc);
    let result = cascade(&doc, &tree).expect("cascade Ok");
    assert_ne!(text, 0);
    assert_eq!(
        result.root_element_computed() as *const ComputedValues,
        &result.computed[0] as *const ComputedValues
    );
}

#[test]
fn replace_page_swaps_the_page_context_and_starts_a_new_generation() {
    let css = "@page { margin: 10px } @page :first { margin-top: 30px }\
        p::before { content: \"x\" } p { page: chapter }";
    let mut doc = TestDoc::new();
    let html = doc.push_element(0, "html", None);
    let style = doc.push_element(html, "style", None);
    doc.push_text(style, css);
    let body = doc.push_element(html, "body", None);
    doc.push_element(body, "p", None);
    let tree = build_rule_tree(&doc);
    let media = MediaContext::default();
    let mut result = cascade_with_media_context(&doc, &tree, &media).expect("cascade Ok");
    let other = cascade_with_media_context(&doc, &tree, &media).expect("cascade Ok");
    let generation = result.generation();
    let computed = result.computed.clone();
    let computed_ptr = result.computed.as_ptr();
    let pseudo = result.pseudo.clone();
    let page_values = result.page_values.clone();
    let old_page = result.page.clone();

    let first_query = PageContextQuery {
        is_first: true,
        ..PageContextQuery::default()
    };
    let first_page = cascade_page_with_media_context(
        &tree,
        &first_query,
        PageInheritance::FromRoot(result.root_element_computed()),
        &media,
    );
    assert_ne!(first_page, old_page);
    result.replace_page(first_page.clone());

    assert_eq!(result.page, first_page);
    assert_eq!(result.computed.as_ptr(), computed_ptr);
    assert_eq!(result.computed, computed);
    assert_eq!(result.pseudo, pseudo);
    assert!(!result.pseudo.is_empty());
    assert_eq!(result.page_values, page_values);
    assert_ne!(result.generation(), generation);
    assert_ne!(result.generation(), other.generation());

    // Replacing with an equal page context still starts a new generation.
    let replaced = result.generation();
    result.replace_page(first_page);
    assert_ne!(result.generation(), replaced);
}

#[test]
fn hanging_punctuation_combinations_survive_cascade_inheritance_and_overrides() {
    let mut doc = TestDoc::new();
    let parent = doc.push_element(0, "div", Some("hanging-punctuation:last force-end first"));
    let inherited = doc.push_element(parent, "span", None);
    let explicit = doc.push_element(parent, "span", Some("hanging-punctuation:inherit"));
    let cleared = doc.push_element(parent, "span", Some("hanging-punctuation:none"));
    let child = doc.push_element(cleared, "span", None);
    let changed = doc.push_element(
        parent,
        "span",
        Some("--hang:last allow-end;hanging-punctuation:var(--hang)"),
    );
    let invalid = doc.push_element(
        parent,
        "span",
        Some("hanging-punctuation:first;hanging-punctuation:force-end allow-end"),
    );
    let result = cascade(&doc, &build_rule_tree(&doc)).expect("cascade");
    for (node, expected) in [
        (parent, "first force-end last"),
        (inherited, "first force-end last"),
        (explicit, "first force-end last"),
        (cleared, "none"),
        (child, "none"),
        (changed, "allow-end last"),
        (invalid, "first"),
    ] {
        assert_eq!(
            result.computed[node].hanging_punctuation.as_css_str(),
            expected
        );
    }
}

#[test]
fn hanging_punctuation_important_wins_cascade_and_is_inherited() {
    for css in [
        "none",
        "first",
        "last",
        "force-end",
        "allow-end",
        "first last",
        "first force-end",
        "first allow-end",
        "force-end last",
        "allow-end last",
        "first force-end last",
        "first allow-end last",
    ] {
        let mut doc = TestDoc::new();
        let sheet = doc.push_element(0, "style", None);
        doc.push_text(
            sheet,
            &format!("div {{ hanging-punctuation:{css} !important }}"),
        );
        let parent = doc.push_element(0, "div", Some("hanging-punctuation:first last"));
        let inherited = doc.push_element(parent, "span", None);
        let explicit = doc.push_element(parent, "span", Some("hanging-punctuation:inherit"));
        let result = cascade(&doc, &build_rule_tree(&doc)).expect("cascade");
        for node in [parent, inherited, explicit] {
            assert_eq!(
                result.computed[node].hanging_punctuation.as_css_str(),
                css,
                "{css}, node {node}"
            );
        }
    }
}
