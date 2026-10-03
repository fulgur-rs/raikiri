use crate::runtime::DomRuntime;
use crate::runtime::test_host::StubHost;

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
fn selection_remove_all_ranges_clears_the_singleton_selection() {
    let mut rt = rt();
    ok(
        &mut rt,
        "var text = document.createTextNode('x'); document.body.appendChild(text); \
         var range = document.createRange(); range.selectNodeContents(text); \
         var selection = window.getSelection(); selection.addRange(range); \
         selection.removeAllRanges(); selection.rangeCount === 0 \
         && selection.isCollapsed && selection.anchorNode === null \
         && selection.focusNode === null",
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
