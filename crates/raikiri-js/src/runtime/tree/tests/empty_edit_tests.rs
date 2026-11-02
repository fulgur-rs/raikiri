use super::{expose, ok, rt, with_state};

#[test]
fn empty_edits_preserve_all_character_data_kinds_and_utf16_boundaries() {
    let mut rt = rt();
    let pi = with_state(rt.context_mut(), |s| {
        s.host
            .document_mut()
            .append_processing_instruction(None, "target", "")
    })
    .unwrap();
    expose(&mut rt, "pi", pi);
    ok(
        &mut rt,
        r#"(()=>{
        const edits=[(n,o)=>n.insertData(o,''),(n,o)=>n.deleteData(o,0),(n,o)=>n.replaceData(o,0,'')];
        for(const node of [document.createTextNode(''),document.createComment(''),pi]){
            for(const value of ['','abc','a😀b','日本語','😀😀']){
                node.data=value;
                for(let offset=0;offset<=value.length;offset++){
                    for(const edit of edits){if(edit(node,offset)!==undefined||node.data!==value)return false;}
                }
                for(const offset of [value.length+1,-1,4294967295]){
                    for(const edit of edits){
                        try{edit(node,offset);return false;}catch(e){if(e.name!=='IndexSizeError'||!(e instanceof DOMException))return false;}
                        if(node.data!==value)return false;
                    }
                }
                for(const edit of edits)for(const zero of [NaN,Infinity,4294967296,-0])edit(node,zero);
                if(node.data!==value)return false;
            }
            node.data='a😀b';node.insertData(1,'x');node.deleteData(2,1);node.replaceData(2,1,'z');
            if(node.data!=='axzb')return false;
        }
        return true;
    })()"#,
    );
}

#[test]
fn empty_edit_conversions_finish_before_validating_current_data() {
    let mut rt = rt();
    ok(
        &mut rt,
        r#"(()=>{
        const t=document.createTextNode('abc');let log=[];
        try{t.insertData({valueOf(){log.push('offset');t.data='';return 1;}},{toString(){log.push('data');return '';}});return false;}
        catch(e){if(e.name!=='IndexSizeError'||log.join(',')!=='offset,data')return false;}
        log=[];
        t.replaceData({valueOf(){log.push('offset');return 2;}},{valueOf(){log.push('count');return 0;}},{toString(){log.push('data');t.data='a😀';return '';}});
        if(log.join(',')!=='offset,count,data'||t.data!=='a😀')return false;
        log=[];
        try{t.deleteData({valueOf(){log.push('offset');return 2;}},{valueOf(){log.push('count');t.data='';return 0;}});return false;}
        catch(e){if(e.name!=='IndexSizeError'||log.join(',')!=='offset,count')return false;}
        const token={};
        try{t.replaceData(99,0,{toString(){throw token;}});return false;}catch(e){if(e!==token)return false;}
        for(const method of ['insertData','deleteData','replaceData']){
            let touched=false;const argument={valueOf(){touched=true;return 0;},toString(){touched=true;return '';}};
            try{CharacterData.prototype[method].call(document.body,argument,argument,argument);return false;}catch(e){if(!(e instanceof TypeError)||touched)return false;}
        }
        return t.data==='';
    })()"#,
    );
}

#[test]
fn empty_edits_keep_dirty_generation_and_cssom_source_invalidation() {
    let mut rt = rt();
    rt.evaluate("globalThis.text=document.createTextNode('a😀b');")
        .unwrap();
    for script in [
        "text.insertData(0,'')",
        "text.deleteData(2,0)",
        "text.replaceData(4,0,'')",
    ] {
        let before = with_state(rt.context_mut(), |s| {
            s.dirty = false;
            s.generation
        })
        .unwrap();
        rt.evaluate(script).unwrap();
        let after = with_state(rt.context_mut(), |s| (s.dirty, s.generation)).unwrap();
        assert_eq!(after, (true, before.wrapping_add(1)));
    }
    let before = with_state(rt.context_mut(), |s| {
        s.dirty = false;
        s.generation
    })
    .unwrap();
    ok(
        &mut rt,
        "(()=>{try{text.replaceData(99,0,{toString(){throw 42;}});return false;}catch(e){return e===42;}})()",
    );
    assert_eq!(
        with_state(rt.context_mut(), |s| (s.dirty, s.generation)).unwrap(),
        (false, before)
    );
    ok(
        &mut rt,
        r#"(()=>{
        for(const edit of [n=>n.insertData(0,''),n=>n.deleteData(0,0),n=>n.replaceData(0,0,'')]){
            const el=document.createElement('style');el.textContent='p { color: red }';document.head.appendChild(el);
            const sheet=el.sheet,old=sheet.cssRules[0];sheet.deleteRule(0);
            if(sheet.cssRules.length!==0)return false;
            edit(el.firstChild);const fresh=sheet.cssRules[0];
            if(!fresh||fresh===old||fresh.style.color!=='red')return false;
            el.remove();
        }
        return true;
    })()"#,
    );
}
