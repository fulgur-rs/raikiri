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

    let css = "@supports (display: block) { @media print { p { color: red } } }";
    let (_, element, result) = context_cascade_doc(css, MediaContext::print());
    assert_ne!(result.computed[element].color, RED);
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
