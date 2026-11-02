//! Public runtime appendData allocation guards, including its first append.
#[path = "support/allocation_tests.rs"]
mod support;

#[global_allocator]
static ALLOCATOR: support::CountingAllocator = support::CountingAllocator;

#[test]
fn public_text_append_does_not_allocate_in_proportion_to_the_existing_prefix() {
    let mut runtime = support::runtime();
    for size in [4093, 65537, 1048576] {
        for addition in ["", "y"] {
            runtime
                .evaluate(&format!(
                    "globalThis.text=document.createTextNode('x'.repeat({size}));"
                ))
                .unwrap();
            let script = format!(
                "(()=>{{for(let i=0;i<100;i++)text.appendData('{addition}');return true;}})()"
            );
            let (result, allocation) = support::allocated_by(|| runtime.evaluate(&script));
            assert!(result.unwrap().to_boolean());
            assert!(
                allocation.bytes < 256 * 1024,
                "prefix {size}, addition {addition:?}: {allocation:?}"
            );
            let expected = format!("text.data==='x'.repeat({size})+'{addition}'.repeat(100)");
            assert!(runtime.evaluate(&expected).unwrap().to_boolean());
        }
    }
}

#[test]
fn public_append_preserves_conversion_order_surrogates_and_clone_isolation() {
    let mut runtime = support::runtime();
    let result = runtime
        .evaluate(
            r#"(()=>{
        const text=document.createTextNode('x'.repeat(4096));
        text.appendData('seed');const copy=text.cloneNode();
        if(text.data.length!==4100)return false;
        copy.appendData('\uD83D');text.appendData('\uDE00');
        if(!copy.data.endsWith('seed\uFFFD')||!text.data.endsWith('seed\uFFFD'))return false;
        copy.appendData('copy');if(text.data.endsWith('copy'))return false;
        text.appendData({toString(){text.data='changed';return '雪';}});
        if(text.data!=='changed雪')return false;
        text.data='a'.repeat(4096);text.appendData('😀');text.appendData('');
        return text.data.length===4098&&text.substringData(4096,2)==='😀';
    })()"#,
        )
        .unwrap();
    assert!(result.to_boolean());
}
