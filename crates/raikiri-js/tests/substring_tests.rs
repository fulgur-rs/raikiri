//! Public-runtime substring behavior with the library built without cfg(test).
use raikiri_dom::Document;
use raikiri_js::runtime::{BoxGeometry, DocumentHost, DomRuntime, HostError};
use std::any::Any;

struct Host(Document);
impl DocumentHost for Host {
    fn document(&self) -> &Document {
        &self.0
    }
    fn document_mut(&mut self) -> &mut Document {
        &mut self.0
    }
    fn flush(&mut self) -> Result<(), HostError> {
        panic!("unexpected layout")
    }
    fn box_geometry(&mut self, _: usize) -> Result<Option<BoxGeometry>, HostError> {
        panic!("unexpected geometry")
    }
    fn computed_value(&mut self, _: usize, _: &str) -> Result<Option<String>, HostError> {
        panic!("unexpected style")
    }
    fn parse_fragment(&mut self, _: &str, _: &str, _: &str) -> Result<Document, HostError> {
        panic!("unexpected parser")
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
    fn into_any(self: Box<Self>) -> Box<dyn Any> {
        self
    }
}

#[test]
fn public_runtime_reads_converted_utf16_ranges_without_layout() {
    let mut runtime = DomRuntime::new(Host(Document::new())).unwrap();
    runtime
        .evaluate("globalThis.text=document.createTextNode('x'.repeat(65536));")
        .unwrap();
    let result=runtime.evaluate(r#"(()=>{
        if(text.substringData(0,1)!=='x'||text.substringData(0,0)!=='')return false;
        const offset={valueOf(){text.data='before';return 2;}};
        const count={valueOf(){text.data='z😀!';return 1;}};
        const half=text.substringData(offset,count);
        if(half.length!==1||half.charCodeAt(0)!==0xde00)return false;
        try{text.substringData(5,0);return false;}catch(e){if(e.name!=='IndexSizeError')return false;}
        return text.substringData(1,-1)==='😀!'&&text.data==='z😀!';
    })()"#).unwrap();
    assert!(result.to_boolean());
}
