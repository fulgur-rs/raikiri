//! Public-runtime image adoption after native host topology changes.
use raikiri_dom::Document;
use raikiri_js::runtime::{BoxGeometry, DocumentHost, DomRuntime, HostError};
use std::{any::Any, cell::Cell, rc::Rc};

struct Host {
    document: Document,
    image: usize,
    move_to: Rc<Cell<Option<usize>>>,
    loads: Rc<Cell<usize>>,
}
impl DocumentHost for Host {
    fn document(&self) -> &Document {
        &self.document
    }
    fn document_mut(&mut self) -> &mut Document {
        if let Some(parent) = self.move_to.take() {
            self.document.append_child(parent, self.image).unwrap();
        }
        &mut self.document
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
    fn load_image(&mut self, url: &str) -> Result<(), HostError> {
        assert!(url.starts_with("https://example.test/"));
        self.loads.set(self.loads.get() + 1);
        Ok(())
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
fn element(document: &mut Document, parent: usize, tag: &str) -> usize {
    let node = document.create_detached_element(tag).unwrap();
    document.append_child(parent, node).unwrap();
    node
}
fn check(runtime: &mut DomRuntime, script: &str) {
    assert!(runtime.evaluate(script).unwrap().to_boolean());
}

#[test]
fn native_moves_cancel_then_restart_requests_and_src_changes_stay_live() {
    let mut document = Document::new();
    let root = document.root_index();
    let html = element(&mut document, root, "html");
    let body = element(&mut document, html, "body");
    let target = element(&mut document, body, "div");
    document
        .set_element_attribute(target, "id", "target")
        .unwrap();
    let image = element(&mut document, body, "img");
    document
        .set_element_attribute(image, "id", "image")
        .unwrap();
    let template = element(&mut document, body, "template");
    let contents = document.allocate_template_fragment_root(template);
    document.mark_in_document_flags();
    let move_to = Rc::new(Cell::new(None));
    let loads = Rc::new(Cell::new(0));
    let mut runtime = DomRuntime::new(Host {
        document,
        image,
        move_to: move_to.clone(),
        loads: loads.clone(),
    })
    .unwrap();
    let report = runtime.run_document();
    assert!(report.aborted.is_none());
    assert!(report.host_failures.is_empty());
    runtime.evaluate("globalThis.img=document.getElementById('image');globalThis.target=document.getElementById('target');globalThis.log=[];img.onload=img.onerror=e=>log.push(e.type);img.src='https://example.test/a.png';").unwrap();
    move_to.set(Some(contents));
    runtime
        .evaluate("img.tagName;target.tagName;target.setAttribute('data-x','1');")
        .unwrap();
    runtime.run_until_idle().unwrap();
    assert_eq!(loads.get(), 0);
    check(&mut runtime, "img.complete&&log.length===0");
    move_to.set(Some(body));
    runtime
        .evaluate("img.tagName;target.tagName;target.setAttribute('data-x','2');")
        .unwrap();
    runtime.run_until_idle().unwrap();
    assert_eq!(loads.get(), 1);
    check(&mut runtime, "img.complete&&log.join(',')==='load'");
    runtime
        .evaluate("img.src='https://example.test/b.png';")
        .unwrap();
    runtime.run_until_idle().unwrap();
    assert_eq!(loads.get(), 2);
    check(&mut runtime, "log.join(',')==='load,load'");
}
