//! Public-runtime listener behavior with the library built without cfg(test).
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
fn public_runtime_preserves_listener_identity_and_once_cleanup() {
    let mut rt = DomRuntime::new(Host(Document::new())).unwrap();
    assert!(rt.evaluate("(()=>{let t=document.createElement('div'),seen=[];function f(){seen.push('f')}t.addEventListener('x',()=>{seen.push('a');t.removeEventListener('x',f);t.addEventListener('x',f)},{once:true});t.addEventListener('x',f);t.dispatchEvent(new Event('x'));t.dispatchEvent(new Event('x'));return seen.join(',')==='a,f'})()").unwrap().to_boolean());
}
