//! Canvas drawImage allocation and conversion regressions through the public runtime.
#[path = "support/allocation_tests.rs"]
mod support;

#[global_allocator]
static ALLOCATOR: support::CountingAllocator = support::CountingAllocator;

#[test]
fn no_op_and_small_crop_draws_do_not_copy_the_whole_source_bitmap() {
    for size in [64, 256, 1024, 2048] {
        let mut document = raikiri_dom::Document::new();
        let html = document.append_element(Some(0), "html", None::<&str>);
        let body = document.append_element(Some(html), "body", None::<&str>);
        let source = document.append_element(Some(body), "canvas", None::<&str>);
        document
            .set_element_attribute(source, "id", "source")
            .unwrap();
        document
            .set_element_attribute(source, "width", size.to_string())
            .unwrap();
        document
            .set_element_attribute(source, "height", size.to_string())
            .unwrap();
        let dest = document.append_element(Some(body), "canvas", None::<&str>);
        document.set_element_attribute(dest, "id", "dest").unwrap();
        document.set_element_attribute(dest, "width", "1").unwrap();
        document.set_element_attribute(dest, "height", "1").unwrap();
        assert!(document.canvas_fill_rect(source, 0, 0, 1, 1, [255, 0, 0, 255]));
        assert!(document.canvas_clear_rect(dest, 0, 0, 1, 1));
        let source_pointer = document.canvas_bitmap_ref(source).unwrap().rgba.as_ptr();
        document.mark_in_document_flags();
        let mut runtime = raikiri_js::runtime::DomRuntime::new(support::Host(document)).unwrap();
        runtime.evaluate("var source=document.getElementById('source'),dest=document.getElementById('dest');var src=source.getContext('2d'),dst=dest.getContext('2d');").unwrap();
        for (name, call) in [
            (
                "zero extent",
                "dst.drawImage(source,0,0,0,1,0,0,1,1)".to_owned(),
            ),
            (
                "destination clip",
                "dst.drawImage(source,0,0,1,1,2,2,1,1)".to_owned(),
            ),
            (
                "foreign crop",
                "dst.drawImage(source,0,0,1,1,0,0,1,1)".to_owned(),
            ),
            (
                "self crop",
                "src.drawImage(source,0,0,1,1,1,0,1,1)".to_owned(),
            ),
            (
                "non-origin self crop",
                "src.drawImage(source,1,0,1,1,2,0,1,1)".to_owned(),
            ),
            (
                "self source clip",
                format!("src.drawImage(source,{},0,1,1,0,0,1,1)", size + 1),
            ),
            (
                "clipped full self source",
                format!("src.drawImage(source,{}, {})", 1 - size, 1 - size),
            ),
        ] {
            let script =
                format!("for(let i=0;i<100;i++){{{call};}}source.width==={size} && dest.width===1");
            assert!(runtime.evaluate(&script).unwrap().to_boolean());
            let (result, allocation) = support::allocated_by(|| runtime.evaluate(&script));
            assert!(result.unwrap().to_boolean());
            eprintln!("canvas allocation: size={size}, {name}, {allocation:?}");
            assert!(
                allocation.bytes < 512 * 1024,
                "size={size}, {name}: {allocation:?}"
            );
        }
        let host = runtime.into_host();
        let source_bitmap = host.document().canvas_bitmap_ref(source).unwrap();
        assert_eq!(source_bitmap.rgba.as_ptr(), source_pointer);
        assert_eq!(&source_bitmap.rgba[..8], &[255, 0, 0, 255, 255, 0, 0, 255]);
        assert_eq!(
            host.document().canvas_bitmap_ref(dest).unwrap().rgba,
            [255, 0, 0, 255]
        );
    }
}

#[test]
fn draw_image_converts_all_numbers_before_checking_canvas_usability() {
    let mut runtime = support::runtime();
    assert!(runtime.evaluate(r#"(()=>{
        const source=document.createElement('canvas'),dest=document.createElement('canvas');
        const ctx=dest.getContext('2d');source.width=0;
        const log=[];
        const numbers=Array.from({length:8},(_,i)=>({valueOf(){log.push(i);return 0;}}));
        try{ctx.drawImage(source,...numbers);return false;}catch(e){if(e.name!=='InvalidStateError')return false;}
        if(log.join(',')!=='0,1,2,3,4,5,6,7')return false;
        if(ctx.drawImage(source,NaN,0)!==undefined)return false;
        source.width=1;const sentinel={};
        try{ctx.drawImage(source,{valueOf(){throw sentinel;}},0);return false;}catch(e){if(e!==sentinel)return false;}
        try{ctx.drawImage(source,{valueOf(){source.width=0;return 0;}},0,0,0);return false;}
        catch(e){return e.name==='InvalidStateError';}
    })()"#).unwrap().to_boolean());
}
