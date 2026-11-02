//! URL getters through the public runtime, including allocation regressions.
#[path = "support/allocation_tests.rs"]
mod support;

#[global_allocator]
static ALLOCATOR: support::CountingAllocator = support::CountingAllocator;

#[test]
fn short_url_components_do_not_copy_an_unrelated_long_query() {
    let mut runtime = support::runtime();
    for size in [4096, 65536, 1048576] {
        runtime.evaluate(&format!("globalThis.subject = new URL('https://example.test/path?x='+'x'.repeat({size}));globalThis.params=subject.searchParams;")).unwrap();
        for (property, expected) in [
            ("protocol", "'https:'"),
            ("hostname", "'example.test'"),
            ("username", "''"),
            ("password", "''"),
            ("host", "'example.test'"),
            ("port", "''"),
            ("origin", "'https://example.test'"),
            ("pathname", "'/path'"),
            ("hash", "''"),
            ("searchParams", "params"),
        ] {
            let script = format!(
                "(()=>{{for(let i=0;i<100;i++)if(subject.{property}!=={expected})return false;return true;}})()"
            );
            assert!(runtime.evaluate(&script).unwrap().to_boolean());
            let (result, allocation) = support::allocated_by(|| runtime.evaluate(&script));
            assert!(result.unwrap().to_boolean());
            assert!(
                allocation.bytes < 512 * 1024,
                "{property} copied unrelated query of {size} bytes: {allocation:?}"
            );
        }
    }
}

#[test]
fn every_url_getter_and_serializer_preserves_the_receiver_brand_check() {
    let mut runtime = support::runtime();
    let result = runtime.evaluate(r#"(()=>{
        const u=new URL('https://example.test/');
        let traps=0;
        const proxy=new Proxy(u,{get(){traps++;throw new Error('trap');}});
        const names=['href','origin','protocol','username','password','host','hostname','port','pathname','search','searchParams','hash'];
        const functions=names.map(n=>Object.getOwnPropertyDescriptor(URL.prototype,n).get);
        functions.push(URL.prototype.toString,URL.prototype.toJSON);
        for(const fn of functions)for(const receiver of [null,undefined,1,'x',{},URL.prototype,proxy]){
            try{fn.call(receiver);return false;}catch(e){
                if(!(e instanceof TypeError)||e.message!=="'this' is not a URL")return false;
            }
        }
        return traps===0;
    })()"#).unwrap();
    assert!(result.to_boolean());
}

#[test]
fn url_getters_preserve_serialization_identity_and_complete_long_outputs() {
    let mut runtime = support::runtime();
    let result = runtime.evaluate(r#"(()=>{
        const ipv6=new URL('https://user:pass@[::1]:8443/a?b=c#d');
        if(ipv6.hostname!=='[::1]'||ipv6.host!=='[::1]:8443'||ipv6.port!=='8443'||ipv6.username!=='user'||ipv6.password!=='pass'||ipv6.origin!=='https://[::1]:8443')return false;
        const normal=new URL('https://bücher.example:443/雪?#');
        if(normal.hostname!=='xn--bcher-kva.example'||normal.port!==''||normal.pathname!=='/%E9%9B%AA'||normal.search!==''||normal.hash!=='')return false;
        const opaque=new URL('data:text/plain,hello');
        if(opaque.origin!=='null'||opaque.host!==''||opaque.pathname!=='text/plain,hello')return false;
        if(new URL('blob:https://example.test/id').origin!=='https://example.test')return false;
        const path='p'.repeat(65536),query='q'.repeat(65536),fragment='f'.repeat(65536);
        const expected='https://example.test/'+path+'?'+query+'#'+fragment;
        const u=new URL(expected),params=u.searchParams;
        if(u.href!==expected||u.toString()!==expected||u.toJSON()!==expected||u.pathname!=='/'+path||u.search!=='?'+query||u.hash!=='#'+fragment)return false;
        try{u.hostname='other.test';}catch(e){}
        return u.hostname==='example.test'&&u.searchParams===params;
    })()"#).unwrap();
    assert!(result.to_boolean());
}
