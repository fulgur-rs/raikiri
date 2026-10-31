//! Public runtime allocation guards for attribute existence queries.
#[path = "support/allocation_tests.rs"]
mod support;

#[global_allocator]
static ALLOCATOR: support::CountingAllocator = support::CountingAllocator;

#[test]
fn attribute_existence_does_not_copy_the_attribute_value() {
    let mut runtime = support::runtime();
    for size in [4096, 65536, 1048576] {
        for kind in ["long", "empty", "missing"] {
            runtime.evaluate(&format!(
                "globalThis.subject=document.createElement('div');subject.setAttribute('data-long','x'.repeat({size}));subject.setAttribute('data-empty','');"
            )).unwrap();
            let expected = if kind == "missing" { 0 } else { 100 };
            let script = format!(
                "(()=>{{let hits=0;for(let i=0;i<100;i++)if(subject.hasAttribute('data-{kind}'))hits++;return hits==={expected};}})()"
            );
            let (result, allocation) = support::allocated_by(|| runtime.evaluate(&script));
            assert!(result.unwrap().to_boolean());
            eprintln!("hasAttribute: size={size}, kind={kind}, {allocation:?}");
            assert!(
                allocation.bytes < 256 * 1024,
                "size={size}, kind={kind}: {allocation:?}"
            );
            assert!(runtime.evaluate(&format!("subject.getAttribute('data-long')==='x'.repeat({size})&&subject.getAttribute('data-empty')===''&&subject.getAttribute('data-missing')===null")).unwrap().to_boolean());
        }
    }
}
