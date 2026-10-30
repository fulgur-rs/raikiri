use super::{expose, ok, rt, with_state};
use crate::substring_diagnostics::{self as diagnostics, Work};

#[test]
fn prefix_and_empty_reads_do_not_visit_or_materialize_the_suffix() {
    let mut rt = rt();
    rt.evaluate("var p=document.createTextNode('x'.repeat(65536));")
        .unwrap();
    for (script, expected) in [
        ("p.substringData(0,1)==='x'", 1),
        ("p.substringData(0,0)===''", 0),
    ] {
        diagnostics::begin();
        let result = rt.evaluate(script);
        let work = diagnostics::finish();
        assert!(result.unwrap().to_boolean());
        assert_eq!(
            work,
            Work {
                calls: 1,
                units_visited: expected,
                temporary_units: 0,
                result_units: expected
            }
        );
    }
    assert_eq!(diagnostics::finish(), Work::default());
}

#[test]
fn empty_results_still_validate_the_offset_by_visiting_its_prefix() {
    let mut rt = rt();
    rt.evaluate("var p=document.createTextNode('abc');")
        .unwrap();
    for script in [
        "p.substringData(3,0)===''",
        "(()=>{try{p.substringData(4,0);return false;}catch(e){return e.name==='IndexSizeError';}})()",
    ] {
        diagnostics::begin();
        let result = rt.evaluate(script);
        let work = diagnostics::finish();
        assert!(result.unwrap().to_boolean());
        assert_eq!(
            work,
            Work {
                calls: 1,
                units_visited: 3,
                temporary_units: 0,
                result_units: 0
            }
        );
    }
}

#[test]
fn substring_matches_utf16_slice_for_all_character_data_kinds() {
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
        for(const node of [document.createTextNode(''),document.createComment(''),pi]){
            for(const value of ['','abc','a😀b','日本語','e\u0301😀\u200dz','😀😀']){
                node.data=value;
                for(let offset=0;offset<=value.length+1;offset++){
                    for(const count of [0,1,2,3,99,4294967295]){
                        if(offset>value.length){
                            try{node.substringData(offset,count);return false;}catch(e){if(!(e instanceof DOMException)||e.name!=='IndexSizeError')return false;}
                        }else if(node.substringData(offset,count)!==value.slice(offset,offset+count))return false;
                    }
                }
                if(node.data!==value)return false;
            }
        }
        return true;
    })()"#,
    );
}

#[test]
fn both_argument_conversions_finish_before_reading_current_data() {
    let mut rt = rt();
    ok(
        &mut rt,
        r#"(()=>{
        const t=document.createTextNode('old'), log=[];
        const offset={valueOf(){log.push('offset');t.data='intermediate';return 1;}};
        const count={valueOf(){log.push('count');t.data='😀z';return 1;}};
        const result=t.substringData(offset,count);
        return result.length===1&&result.charCodeAt(0)===0xde00&&log.join(',')==='offset,count'&&t.data==='😀z';
    })()"#,
    );
}

#[test]
fn argument_failures_preserve_order_and_receiver_validation() {
    let mut rt = rt();
    ok(
        &mut rt,
        r#"(()=>{
        const t=document.createTextNode('abc'), sentinel={}, log=[];
        const offset={valueOf(){log.push('offset');return 0;}};
        const count={valueOf(){log.push('count');return 1;}};
        const throwing={valueOf(){throw sentinel;}};
        for(const receiver of [null,{},document.body]){
            try{CharacterData.prototype.substringData.call(receiver,offset,count);return false;}catch(e){if(!(e instanceof TypeError))return false;}
        }
        if(log.length)return false;
        try{t.substringData(throwing,count);return false;}catch(e){if(e!==sentinel)return false;}
        if(log.length)return false;
        try{t.substringData(offset,throwing);return false;}catch(e){if(e!==sentinel)return false;}
        if(log.join(',')!=='offset')return false;
        try{t.substringData(offset);return false;}catch(e){if(!(e instanceof TypeError))return false;}
        return log.join(',')==='offset,offset'&&t.substringData(4294967296,1)==='a'&&t.substringData(1,-1)==='bc'&&t.substringData(0,undefined)==='';
    })()"#,
    );
}

#[test]
fn substring_reads_are_live_across_character_data_edits() {
    let mut rt = rt();
    ok(
        &mut rt,
        r#"(()=>{
        const t=document.createTextNode('a😀b');
        if(t.substringData(1,1).charCodeAt(0)!==0xd83d)return false;
        t.data='xy';if(t.substringData(0,1)!=='x')return false;
        t.nodeValue='z';if(t.substringData(0,2)!=='z')return false;
        t.textContent='ab';t.appendData('c');t.insertData(1,'-');t.deleteData(2,1);t.replaceData(0,1,'!');
        return t.substringData(0,4294967295)==='!-c'&&t.substringData(t.length,0)==='';
    })()"#,
    );
}
