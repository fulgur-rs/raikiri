//! Public runtime allocation guards for empty CharacterData range edits.
#[path = "support/allocation_tests.rs"]
mod support;

#[global_allocator]
static ALLOCATOR: support::CountingAllocator = support::CountingAllocator;

#[test]
fn empty_range_edits_do_not_allocate_in_proportion_to_the_existing_text() {
    let mut runtime = support::runtime();
    for size in [4096, 65536, 1048576] {
        for offset in [0, 1, 3] {
            for operation in [
                format!("text.insertData({offset},'')"),
                format!("text.deleteData({offset},0)"),
                format!("text.replaceData({offset},0,'')"),
            ] {
                runtime
                    .evaluate(&format!(
                        "globalThis.text=document.createTextNode('x'.repeat({size}));"
                    ))
                    .unwrap();
                let script =
                    format!("(()=>{{for(let i=0;i<100;i++){{{operation};}}return true;}})()");
                let (result, allocation) = support::allocated_by(|| runtime.evaluate(&script));
                assert!(result.unwrap().to_boolean());
                eprintln!("empty edit: size={size}, {operation}, {allocation:?}");
                assert!(
                    allocation.bytes < 256 * 1024,
                    "size={size}, {operation}: {allocation:?}"
                );
                assert!(
                    runtime
                        .evaluate(&format!("text.data==='x'.repeat({size})"))
                        .unwrap()
                        .to_boolean()
                );
            }
        }
    }
}

#[test]
fn zero_offset_empty_edits_leave_buffered_prefixes_and_clones_unmaterialized() {
    let mut runtime = support::runtime();
    for operation in [
        "text.insertData(0,'')",
        "text.deleteData(0,0)",
        "text.replaceData(0,0,'')",
    ] {
        runtime.evaluate("globalThis.text=document.createTextNode('x'.repeat(1048576));text.appendData('suffix');globalThis.copy=text.cloneNode();").unwrap();
        let script = format!("(()=>{{for(let i=0;i<100;i++){{{operation};}}return true;}})()");
        let (result, allocation) = support::allocated_by(|| runtime.evaluate(&script));
        assert!(result.unwrap().to_boolean());
        eprintln!("buffered empty edit: {operation}, {allocation:?}");
        assert!(allocation.bytes < 256 * 1024, "{operation}: {allocation:?}");
        assert!(runtime.evaluate("copy.appendData('copy');text.data==='x'.repeat(1048576)+'suffix'&&copy.data===text.data+'copy'").unwrap().to_boolean());
    }
}
