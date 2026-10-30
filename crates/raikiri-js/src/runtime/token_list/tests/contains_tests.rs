use super::{ok, rt};
use crate::token_list_diagnostics as diagnostics;

#[test]
fn first_membership_does_not_build_or_clone_the_ordered_set() {
    let mut rt = rt();
    rt.evaluate("var e=document.createElement('div'); e.className='first '+Array.from({length:128},(_,i)=>'k'+i).join(' '); var list=e.classList;").unwrap();
    diagnostics::begin();
    let found = rt.evaluate("list.contains('first')").unwrap().to_boolean();
    let work = diagnostics::finish();
    assert!(found);
    assert_eq!(work.sets, 0);
    assert_eq!(work.tokens, 0);
    assert_eq!(work.dedup_comparisons, 0);
    assert_eq!(work.owned_tokens, 0);
    assert_eq!(work.attribute_bytes_cloned, 0);
    assert_eq!(work.contains_comparisons, 1);
}

#[test]
fn membership_matches_the_existing_ordered_set_oracle() {
    let mut rt = rt();
    ok(
        &mut rt,
        r#"(()=>{
        const e=document.createElement('div'), list=e.classList;
        const values=['','a','a a b a',' A a ','a\tb\nc\rd\fe','a\vb','a\u00a0b','日本 語 日本','😀 x 😀'];
        const needles=['','a','A','b','c','d','e','a b','\t','a\u00a0b','a\vb','日本','語','😀','missing'];
        if(list.contains('a'))return false;
        for(const value of values){
            e.setAttribute('class',value);
            const expected=Array.from({length:list.length},(_,i)=>list.item(i));
            for(const needle of needles)if(list.contains(needle)!==expected.includes(needle))return false;
        }
        e.removeAttribute('class');
        return !list.contains('a')&&!list.contains('');
    })()"#,
    );
}

#[test]
fn membership_is_live_across_all_class_writers() {
    let mut rt = rt();
    ok(
        &mut rt,
        r#"(()=>{
        const e=document.createElement('div'), list=e.classList;
        e.className='a a'; if(!list.contains('a'))return false;
        e.setAttribute('class','b'); if(list.contains('a')||!list.contains('b'))return false;
        list.value='c c d'; if(list.contains('b')||!list.contains('c'))return false;
        list.add('e'); list.remove('c'); list.toggle('d'); list.replace('e','f');
        return !list.contains('c')&&!list.contains('d')&&list.contains('f')&&list.length===1&&list.item(0)==='f';
    })()"#,
    );
}

#[test]
fn conversion_precedes_read_and_can_reenter_membership() {
    let mut rt = rt();
    ok(
        &mut rt,
        r#"(()=>{
        const e=document.createElement('div'); e.className='old'; const list=e.classList;
        let calls=0;
        const found=list.contains({toString(){calls++; if(!list.contains('old'))throw Error('reentry');e.className='new';return 'new';}});
        const sentinel={}; let caught=false;
        try{list.contains({toString(){throw sentinel;}});}catch(e){caught=e===sentinel;}
        return found&&calls===1&&caught&&e.className==='new'&&!list.contains('')&&!list.contains('a b');
    })()"#,
    );
}

#[test]
fn brand_check_precedes_argument_conversion() {
    let mut rt = rt();
    ok(
        &mut rt,
        r#"(()=>{
        const e=document.createElement('div'); let calls=0;
        const arg={toString(){calls++;return 'x';}};
        for(const receiver of [null,undefined,{},e]){
            try{DOMTokenList.prototype.contains.call(receiver,arg);return false;}catch(e){if(!(e instanceof TypeError))return false;}
        }
        try{e.classList.contains(Symbol());return false;}catch(e){if(!(e instanceof TypeError))return false;}
        return calls===0;
    })()"#,
    );
}

#[test]
fn diagnostics_still_measure_the_unchanged_ordered_set_readers() {
    let mut rt = rt();
    rt.evaluate("var e=document.createElement('div'); e.className='first first last'; var list=e.classList;").unwrap();
    diagnostics::begin();
    let length = rt.evaluate("list.length").unwrap().as_number();
    let work = diagnostics::finish();
    assert_eq!(length, Some(2.));
    assert_eq!(work.sets, 1);
    assert_eq!(work.tokens, 3);
    assert_eq!(work.dedup_comparisons, 2);
    assert_eq!(work.owned_tokens, 2);
    assert_eq!(work.attribute_bytes_cloned, 16);
    assert_eq!(work.contains_comparisons, 0);
    assert_eq!(diagnostics::finish(), diagnostics::Work::default());
}
