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

thread_local! {
    static UTF16_VISITS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

pub(super) fn record_utf16_visit() {
    UTF16_VISITS.set(UTF16_VISITS.get() + 1);
}

fn take_utf16_visits() -> usize {
    UTF16_VISITS.replace(0)
}

#[test]
fn range_boundary_prefix_validation_does_not_visit_the_suffix() {
    for size in [4_096, 65_536, 1_048_576] {
        for character in ["a", "😀"] {
            let (mut host, _, _, body) = StubHost::page();
            host.document.append_text(body, character.repeat(size));
            let mut rt = DomRuntime::new(host).unwrap();
            rt.evaluate("var text=document.body.firstChild;var range=document.createRange();")
                .unwrap();
            for offset in [0, 1, 2, 3] {
                take_utf16_visits();
                ok(
                    &mut rt,
                    &format!(
                        "for(let i=0;i<100;i++){{range.setStart(text,{offset});range.setEnd(text,{offset});}}\
                     range.startContainer===text && range.endContainer===text && \
                     range.startOffset==={offset} && range.endOffset==={offset}"
                    ),
                );
                assert_eq!(
                    take_utf16_visits(),
                    200 * offset,
                    "size={size}, character={character:?}, offset={offset}"
                );
            }
        }
    }
}

#[test]
fn range_boundaries_and_content_selection_preserve_all_character_data_offsets() {
    let mut rt = rt();
    let pi = with_state(rt.context_mut(), |state| {
        state
            .host
            .document_mut()
            .append_processing_instruction(None, "target", "")
    })
    .unwrap();
    let object = super::wrap(rt.context_mut(), pi).unwrap();
    rt.context_mut()
        .register_global_property(JsString::from("pi"), object, Attribute::all())
        .unwrap();
    ok(
        &mut rt,
        r#"(()=>{
        const range=document.createRange();
        for(const node of [document.createTextNode(''),document.createComment(''),pi]){
            for(const value of ['','abc','A😀B','日本語','😀😀']){
                node.data=value;
                for(let offset=0;offset<=value.length;offset++){
                    range.setStart(node,offset);range.setEnd(node,offset);
                    if(range.startContainer!==node || range.endContainer!==node ||
                       range.startOffset!==offset || range.endOffset!==offset)return false;
                }
                for(const method of ['setStart','setEnd']){
                    try{range[method](node,value.length+1);return false;}
                    catch(e){if(!(e instanceof DOMException)||e.name!=='IndexSizeError')return false;}
                    if(range.startOffset!==value.length || range.endOffset!==value.length)return false;
                }
                range.selectNodeContents(node);
                if(range.startOffset!==0 || range.endOffset!==value.length)return false;
            }
        }
        return true;
    })()"#,
    );
    with_state(rt.context_mut(), |state| {
        state
            .host
            .document_mut()
            .set_character_data(pi, "😀")
            .unwrap()
    })
    .unwrap();
    ok(
        &mut rt,
        "var nativeRange=document.createRange();nativeRange.setStart(pi,1);nativeRange.setEnd(pi,2);nativeRange.startOffset===1 && nativeRange.endOffset===2",
    );
    with_state(rt.context_mut(), |state| {
        state
            .host
            .document_mut()
            .set_character_data(pi, "")
            .unwrap()
    })
    .unwrap();
    ok(
        &mut rt,
        "try{nativeRange.setStart(pi,1);false}catch(e){e.name==='IndexSizeError'}",
    );
}

#[test]
fn invalid_range_offsets_scan_to_the_end_and_exact_selection_reads_the_whole_text() {
    let mut rt = rt();
    rt.evaluate("var text=document.createTextNode('A😀B');var range=document.createRange();")
        .unwrap();
    for method in ["setStart", "setEnd"] {
        take_utf16_visits();
        ok(
            &mut rt,
            &format!("try{{range.{method}(text,5);false}}catch(e){{e.name==='IndexSizeError'}}"),
        );
        assert_eq!(take_utf16_visits(), 4);
    }
    take_utf16_visits();
    ok(
        &mut rt,
        "range.selectNodeContents(text);range.startOffset===0 && range.endOffset===4",
    );
    assert_eq!(take_utf16_visits(), 4);
}

#[test]
fn range_boundary_conversions_keep_their_order_and_validate_the_current_text() {
    let mut rt = rt();
    ok(
        &mut rt,
        r#"(()=>{
        const text=document.createTextNode('abc'), range=document.createRange(), sentinel={};
        for(const method of ['setStart','setEnd']){
            let converted=0;
            const offset={valueOf(){converted++;return 0;}};
            try{Range.prototype[method].call({},text,offset);return false;}catch(e){if(!(e instanceof TypeError))return false;}
            try{range[method]({},offset);return false;}catch(e){if(!(e instanceof TypeError))return false;}
            if(converted!==0)return false;
            try{range[method](text);return false;}catch(e){if(!(e instanceof TypeError))return false;}
            try{range[method](text,{valueOf(){throw sentinel;}});return false;}catch(e){if(e!==sentinel)return false;}
            text.data='';
            range[method](text,{valueOf(){text.data='😀';return 2;}});
            if(range[method==='setStart'?'startOffset':'endOffset']!==2)return false;
            try{range[method](text,{valueOf(){text.data='';return 1;}});return false;}
            catch(e){if(e.name!=='IndexSizeError')return false;}
            range[method](text,4294967296);
            if(range[method==='setStart'?'startOffset':'endOffset']!==0)return false;
        }
        return true;
    })()"#,
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
fn range_boundaries_validate_child_counts_for_structural_nodes() {
    let mut rt = rt();
    ok(
        &mut rt,
        r#"(()=>{
        const element=document.createElement('div'), fragment=document.createDocumentFragment();
        element.append(document.createTextNode('long text'),document.createElement('br'));
        fragment.appendChild(element);
        const range=document.createRange();
        for(const [node,length] of [[element,2],[fragment,1],[document,1]]){
            for(const method of ['setStart','setEnd']){
                range[method](node,length);
                if(range[method==='setStart'?'startOffset':'endOffset']!==length)return false;
                try{range[method](node,length+1);return false;}catch(e){if(e.name!=='IndexSizeError')return false;}
            }
        }
        return true;
    })()"#,
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
fn tree_walker_current_node_setter_moves_the_walker() {
    let mut rt = rt();
    ok(
        &mut rt,
        "var root = document.createElement('div'); \
         var nested = document.createElement('span'); \
         var text = document.createTextNode('A'); \
         nested.appendChild(text); root.appendChild(nested); document.body.append(root); \
         var walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT); \
         walker.currentNode = text; \
         walker.currentNode === text && walker.nextNode() === null",
    );
}

#[test]
fn tree_walker_current_node_setter_accepts_nodes_outside_the_root() {
    let mut rt = rt();
    ok(
        &mut rt,
        "var root = document.createElement('div'); \
         var inside = document.createTextNode('A'); root.appendChild(inside); \
         var outside = document.createElement('p'); \
         var before = document.createTextNode('B'); outside.appendChild(before); \
         var after = document.createTextNode('C'); \
         document.body.append(outside, root, after); \
         var walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT); \
         walker.currentNode = outside; \
         walker.currentNode === outside \
         && walker.nextNode() === before && walker.nextNode() === inside \
         && walker.nextNode() === null && walker.currentNode === inside",
    );
}

#[test]
fn tree_walker_next_node_from_outside_the_root_walks_to_the_document_end() {
    let mut rt = rt();
    ok(
        &mut rt,
        "var root = document.createElement('div'); \
         var first = document.createTextNode('A'); \
         var second = document.createTextNode('B'); \
         document.body.append(root, first, second); \
         var walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT); \
         walker.currentNode = first; \
         walker.nextNode() === second && walker.nextNode() === null \
         && walker.currentNode === second",
    );
}

#[test]
fn tree_walker_next_node_from_an_ancestor_or_a_detached_node() {
    let mut rt = rt();
    ok(
        &mut rt,
        "var root = document.createElement('div'); \
         var inside = document.createTextNode('A'); root.appendChild(inside); \
         document.body.append(root); \
         var walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT); \
         walker.currentNode = document.body; \
         var fromAncestor = walker.nextNode() === inside && walker.nextNode() === null; \
         var detached = document.createTextNode('x'); \
         walker.currentNode = detached; \
         fromAncestor && walker.nextNode() === null && walker.currentNode === detached",
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
fn range_operations_reject_invalid_node_handles_even_at_offset_zero() {
    let mut rt = rt();
    let node = JsObject::from_proto_and_data(
        Some(protos(rt.context_mut()).node.clone()),
        NodeHandle {
            index: usize::MAX,
            record: crate::runtime::node_gc::NodeRecord::new(usize::MAX),
        },
    );
    rt.context_mut()
        .register_global_property(JsString::from("invalidNode"), node, Attribute::all())
        .unwrap();
    ok(
        &mut rt,
        "(()=>{const range=document.createRange(); \
         for(const method of ['selectNodeContents','setStart','setEnd']){ \
             try{range[method](globalThis.invalidNode,0);return false;} \
             catch(e){if(!(e instanceof TypeError))return false;} \
         }return true;})()",
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

#[test]
fn silent_enqueue_abort_does_not_mutate_tree_walker_data() {
    use crate::runtime::{Abort, Limits, RunOptions, RuntimeError};
    let (host, ..) = StubHost::page();
    let mut runtime = DomRuntime::with_options(
        host,
        RunOptions {
            limits: Limits {
                max_enqueued_jobs: 0,
                ..Limits::default()
            },
        },
    )
    .unwrap();
    let walker = runtime.evaluate("var walker=document.createTreeWalker(document.body); var other=document.createElement('p'); walker").unwrap().as_object().unwrap();
    let before = walker
        .downcast_ref::<super::TreeWalkerData>()
        .unwrap()
        .current_node
        .get();
    assert_eq!(
        runtime.evaluate("Promise.resolve().then(()=>{}); walker.currentNode=other;"),
        Err(RuntimeError::Aborted(Abort::EnqueuedJobs))
    );
    assert_eq!(
        walker
            .downcast_ref::<super::TreeWalkerData>()
            .unwrap()
            .current_node
            .get(),
        before
    );
}

#[test]
fn silent_enqueue_abort_does_not_mutate_highlight_data() {
    use crate::runtime::{Abort, Limits, RunOptions, RuntimeError};
    let (host, ..) = StubHost::page();
    let mut runtime = DomRuntime::with_options(
        host,
        RunOptions {
            limits: Limits {
                max_enqueued_jobs: 0,
                ..Limits::default()
            },
        },
    )
    .unwrap();
    let highlight = runtime
        .evaluate("var highlight=new Highlight(); var range=document.createRange(); highlight")
        .unwrap()
        .as_object()
        .unwrap();
    assert_eq!(
        runtime.evaluate("Promise.resolve().then(()=>{}); highlight.add(range);"),
        Err(RuntimeError::Aborted(Abort::EnqueuedJobs))
    );
    assert!(
        highlight
            .downcast_ref::<super::HighlightData>()
            .unwrap()
            .ranges
            .borrow()
            .is_empty()
    );
}
