use boa_engine::JsString;
use boa_engine::object::JsObject;
use boa_engine::property::Attribute;

use crate::runtime::DomRuntime;
use crate::runtime::interfaces::{NodeHandle, protos};
use crate::runtime::test_host::StubHost;
use crate::runtime::webidl::with_state;

fn rt() -> DomRuntime {
    let (host, ..) = StubHost::page();
    DomRuntime::new(host).unwrap()
}

fn ok(rt: &mut DomRuntime, src: &str) {
    assert!(
        rt.evaluate(src).unwrap().to_boolean(),
        "expected true: {src}"
    );
}

#[test]
fn create_range_and_window_selection_track_selected_node_contents() {
    let mut rt = rt();
    rt.evaluate(
        "var text = document.createTextNode('A😀'); document.body.appendChild(text); \
         var range = document.createRange(); range.selectNodeContents(text); \
         var selection = window.getSelection(); selection.addRange(range);",
    )
    .unwrap();
    ok(
        &mut rt,
        "range instanceof Range && selection instanceof Selection \
         && window.getSelection() === selection && selection.rangeCount === 1 \
         && selection.getRangeAt(0) === range && range.startContainer === text \
         && range.endContainer === text && range.startOffset === 0 \
         && range.endOffset === 3 && !range.collapsed && !selection.isCollapsed \
         && selection.anchorNode === text && selection.focusNode === text \
         && selection.anchorOffset === 0 && selection.focusOffset === 3",
    );
}

#[test]
fn range_select_node_contents_uses_child_offsets_for_elements() {
    let mut rt = rt();
    ok(
        &mut rt,
        "var target = document.createElement('div'); \
         target.appendChild(document.createTextNode('A')); \
         target.appendChild(document.createElement('br')); \
         target.appendChild(document.createTextNode('B')); \
         document.body.appendChild(target); \
         var range = document.createRange(); range.selectNodeContents(target); \
         range.startContainer === target && range.endContainer === target \
         && range.startOffset === 0 && range.endOffset === 3 && !range.collapsed",
    );
}

#[test]
fn tree_walker_visits_descendant_text_nodes_in_tree_order() {
    let mut rt = rt();
    ok(
        &mut rt,
        "var root = document.createElement('div'); \
         var first = document.createTextNode('A'); \
         var nested = document.createElement('span'); \
         var second = document.createTextNode('B'); \
         nested.appendChild(second); root.append(first, nested); document.body.append(root); \
         var walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT); \
         walker.root === root && walker.currentNode === root \
         && walker.nextNode() === first && walker.currentNode === first \
         && walker.nextNode() === second && walker.nextNode() === null \
         && walker.currentNode === second",
    );
}

#[test]
fn range_set_start_and_end_use_utf16_offsets_and_highlights_store_ranges() {
    let mut rt = rt();
    ok(
        &mut rt,
        "var text = document.createTextNode('A😀B'); document.body.appendChild(text); \
         var range = document.createRange(); range.setStart(text, 1); range.setEnd(text, 3); \
         var highlight = new Highlight(); highlight.add(range); \
         CSS.highlights.set('sample', highlight); \
         range.startContainer === text && range.endContainer === text \
         && range.startOffset === 1 && range.endOffset === 3 \
         && CSS.highlights.get('sample') === highlight",
    );
    let ranges = rt.custom_highlight_ranges().expect("highlight ranges");
    assert_eq!(ranges.len(), 1);
    assert_eq!(ranges[0].name, "sample");
    assert_eq!(ranges[0].start_container, ranges[0].end_container);
    assert_eq!(ranges[0].start_offset, 1);
    assert_eq!(ranges[0].end_offset, 3);
}

#[test]
fn selection_remove_all_ranges_clears_the_singleton_selection() {
    let mut rt = rt();
    ok(
        &mut rt,
        "var text = document.createTextNode('x'); document.body.appendChild(text); \
         var range = document.createRange(); range.selectNodeContents(text); \
         var selection = window.getSelection(); selection.addRange(range); \
         selection.removeAllRanges(); selection.rangeCount === 0 \
         && selection.isCollapsed && selection.anchorNode === null \
         && selection.focusNode === null && selection.anchorOffset === 0 \
         && selection.focusOffset === 0",
    );
}

#[test]
fn range_and_selection_methods_reject_incompatible_receivers() {
    let mut rt = rt();
    ok(
        &mut rt,
        "var badRangeThis = false; \
         try { Range.prototype.selectNodeContents.call({}, document.body); } \
         catch (e) { badRangeThis = e instanceof TypeError; } \
         var badSelectionThis = false; \
         try { Selection.prototype.getRangeAt.call({}, 0); } \
         catch (e) { badSelectionThis = e instanceof TypeError; } \
         badRangeThis && badSelectionThis",
    );
}

#[test]
fn range_select_node_contents_rejects_an_invalid_node_handle() {
    let mut rt = rt();
    let node = JsObject::from_proto_and_data(
        Some(protos(rt.context_mut()).node.clone()),
        NodeHandle { index: usize::MAX },
    );
    rt.context_mut()
        .register_global_property(JsString::from("invalidNode"), node, Attribute::all())
        .unwrap();
    ok(
        &mut rt,
        "var range = document.createRange(); \
         var caughtInvalidNode = false; \
         try { range.selectNodeContents(globalThis.invalidNode); } \
         catch (e) { caughtInvalidNode = e instanceof TypeError; } \
         caughtInvalidNode",
    );
}

#[test]
fn selection_rejects_corrupted_internal_range_state() {
    let mut rt = rt();
    rt.evaluate("var selection = window.getSelection();")
        .unwrap();
    let range = JsObject::from_proto_and_data(Some(protos(rt.context_mut()).range.clone()), ());
    with_state(rt.context_mut(), |state| state.selection_ranges.push(range)).unwrap();
    ok(
        &mut rt,
        "var invalidRange = false; \
         try { selection.isCollapsed; } \
         catch (e) { invalidRange = e instanceof TypeError; } \
         invalidRange",
    );
}

#[test]
fn selection_rejects_non_range_and_out_of_bounds_range_indexes() {
    let mut rt = rt();
    ok(
        &mut rt,
        "var selection = window.getSelection(); \
         var badRange = false; try { selection.addRange({}); } catch (e) { badRange = e instanceof TypeError; } \
         var badIndex = false; try { selection.getRangeAt(0); } catch (e) { badIndex = e.name === 'IndexSizeError'; } \
         badRange && badIndex",
    );
}
