use super::{DomRuntime, host, ok, ok_after_tasks, with_state};
use crate::image_adoption_diagnostics::{self as diagnostics, Work};
use crate::runtime::interfaces::wrap;
use boa_engine::{JsString, property::Attribute};

fn expose(runtime: &mut DomRuntime, name: &str, index: usize) {
    let object = wrap(runtime.context_mut(), index).unwrap();
    runtime
        .context_mut()
        .register_global_property(JsString::from(name), object, Attribute::all())
        .unwrap();
}

fn fixture(n: usize) -> (DomRuntime, usize, usize) {
    let (mut host, body) = host();
    let d = &mut host.document;
    let target = d.create_detached_element("div").unwrap();
    d.append_child(body, target).unwrap();
    let text = d.append_text(target, "start");
    let template = d.create_detached_element("template").unwrap();
    d.append_child(body, template).unwrap();
    let contents = d.allocate_template_fragment_root(template);
    let mut first = 0;
    for i in 0..n {
        let active = d.create_detached_element("img").unwrap();
        d.set_element_attribute(active, "src", "").unwrap();
        d.append_child(body, active).unwrap();
        if i == 0 {
            first = active;
        }
        let inert = d.create_detached_element("img").unwrap();
        d.set_element_attribute(inert, "src", "").unwrap();
        d.append_child(contents, inert).unwrap();
    }
    d.mark_in_document_flags();
    let mut runtime = DomRuntime::new(host).unwrap();
    for (name, index) in [("target", target), ("text", text), ("contents", contents)] {
        expose(&mut runtime, name, index);
    }
    if n > 0 {
        expose(&mut runtime, "img", first);
    }
    let report = runtime.run_document();
    assert!(report.aborted.is_none());
    assert!(report.host_failures.is_empty());
    assert!(report.uncaught_errors.is_empty());
    let counts = with_state(runtime.context_mut(), |s| {
        (s.active_images.len(), s.inert_images.len())
    })
    .unwrap();
    assert_eq!(counts, (n, n));
    runtime
        .evaluate("target.setAttribute('data-warm','ready');")
        .unwrap();
    (runtime, contents, first)
}

fn measured(runtime: &mut DomRuntime, script: &str) -> Work {
    diagnostics::begin();
    let result = runtime.evaluate(script);
    let work = diagnostics::finish();
    assert!(result.unwrap().to_boolean());
    work
}

#[test]
fn unrelated_attributes_and_character_data_do_not_check_active_or_inert_images() {
    for n in [0, 32, 128] {
        let (mut runtime, _, _) = fixture(n);
        for script in [
            "(()=>{for(let i=0;i<500;i++)target.setAttribute('data-x',String(i));return target.getAttribute('data-x')==='499';})()",
            "(()=>{for(let i=0;i<500;i++)text.data=String(i);return text.data==='499';})()",
        ] {
            assert_eq!(measured(&mut runtime, script), Work::default());
        }
    }
}

#[test]
fn detached_arena_growth_is_not_a_topology_change() {
    let (mut runtime, _, _) = fixture(32);
    assert_eq!(
        measured(
            &mut runtime,
            "(()=>{for(let i=0;i<50;i++){document.createElement('span');document.createTextNode('x');}return true;})()"
        ),
        Work::default()
    );
}

#[test]
fn topology_observed_without_images_is_consumed_before_the_first_source_update() {
    let (mut runtime, _, _) = fixture(0);
    runtime.evaluate("target.appendChild(document.createElement('span'));globalThis.img=document.createElement('img');img.src='';").unwrap();
    assert_eq!(
        measured(&mut runtime, "target.setAttribute('data-x','done');true"),
        Work::default()
    );
    assert_eq!(diagnostics::finish(), Work::default());
}

#[test]
fn native_moves_survive_empty_syncs_and_cancel_pending_requests() {
    let (mut runtime, contents, index) = fixture(1);
    runtime
        .evaluate("globalThis.log=[];img.onload=img.onerror=e=>log.push(e.type);img.src='ok.png';")
        .unwrap();
    with_state(runtime.context_mut(), |s| {
        s.host.document_mut().append_child(contents, index)
    })
    .unwrap()
    .unwrap();
    ok(&mut runtime, "img.tagName==='IMG'&&target.tagName==='DIV'");
    let work = measured(
        &mut runtime,
        "target.setAttribute('data-x','native');img.complete",
    );
    assert_eq!(
        work,
        Work {
            settlements: 1,
            active_checks: 1,
            inert_checks: 2
        }
    );
    ok_after_tasks(&mut runtime, "log.length===0");
    ok(&mut runtime, "img.remove();img.src='ok.png';img.complete");
    ok_after_tasks(&mut runtime, "log.length===0");
    ok(&mut runtime, "target.appendChild(img);!img.complete");
    ok_after_tasks(&mut runtime, "log.join(',')==='load'");
    assert_eq!(
        measured(&mut runtime, "target.setAttribute('data-x','again');true"),
        Work::default()
    );
    ok_after_tasks(&mut runtime, "log.join(',')==='load'");
}

#[test]
fn argument_reentry_settles_the_move_without_repeating_it_for_the_attribute() {
    let (mut runtime, _, _) = fixture(1);
    runtime
        .evaluate("globalThis.log=[];img.onload=img.onerror=e=>log.push(e.type);img.src='ok.png';")
        .unwrap();
    let work = measured(
        &mut runtime,
        "target.setAttribute('data-x',{toString(){contents.appendChild(img);return 'moved';}});img.complete",
    );
    assert_eq!(
        work,
        Work {
            settlements: 1,
            active_checks: 1,
            inert_checks: 2
        }
    );
    ok_after_tasks(&mut runtime, "log.length===0");
}

#[test]
fn ancestor_and_fragment_adoption_restart_each_image_once() {
    let (mut runtime, _, _) = fixture(0);
    ok(
        &mut runtime,
        "globalThis.log=[];globalThis.ancestor=document.createElement('div');globalThis.fragment=document.createDocumentFragment();for(let i=1;i<=2;i++){const image=new Image();image.onload=image.onerror=e=>log.push(i+':'+e.type);image.src='ok.png';ancestor.appendChild(image);}contents.appendChild(ancestor);true",
    );
    ok_after_tasks(&mut runtime, "log.length===0");
    ok(
        &mut runtime,
        "fragment.appendChild(ancestor);ancestor.firstChild.src='ok.png';true",
    );
    ok_after_tasks(&mut runtime, "log.length===0");
    ok(&mut runtime, "document.body.appendChild(fragment);true");
    ok_after_tasks(
        &mut runtime,
        "log.slice().sort().join(',')==='1:load,2:load'",
    );
    ok(
        &mut runtime,
        "contents.appendChild(ancestor);document.body.appendChild(ancestor);true",
    );
    ok_after_tasks(
        &mut runtime,
        "log.slice().sort().join(',')==='1:load,1:load,2:load,2:load'",
    );
}

#[test]
fn element_text_content_is_still_a_structural_change() {
    let (mut runtime, _, _) = fixture(1);
    runtime.evaluate("target.appendChild(img);").unwrap();
    let work = measured(
        &mut runtime,
        "target.textContent='replacement';img.parentNode===null&&target.textContent==='replacement'",
    );
    assert_eq!(
        work,
        Work {
            settlements: 1,
            active_checks: 1,
            inert_checks: 1
        }
    );
}

struct ScheduledHost {
    inner: crate::runtime::test_host::StubHost,
    image: usize,
    destination: std::rc::Rc<std::cell::Cell<usize>>,
    remaining: std::rc::Rc<std::cell::Cell<Option<usize>>>,
    loads: std::rc::Rc<std::cell::Cell<usize>>,
}
impl crate::runtime::DocumentHost for ScheduledHost {
    fn document(&self) -> &raikiri_dom::Document {
        &self.inner.document
    }
    fn document_mut(&mut self) -> &mut raikiri_dom::Document {
        match self.remaining.get() {
            Some(0) => {
                self.remaining.set(None);
                self.inner
                    .document
                    .append_child(self.destination.get(), self.image)
                    .unwrap();
            }
            Some(n) => self.remaining.set(Some(n - 1)),
            None => {}
        }
        &mut self.inner.document
    }
    fn flush(&mut self) -> Result<(), crate::runtime::HostError> {
        self.inner.flush()
    }
    fn box_geometry(
        &mut self,
        n: usize,
    ) -> Result<Option<crate::runtime::BoxGeometry>, crate::runtime::HostError> {
        self.inner.box_geometry(n)
    }
    fn computed_value(
        &mut self,
        n: usize,
        p: &str,
    ) -> Result<Option<String>, crate::runtime::HostError> {
        self.inner.computed_value(n, p)
    }
    fn parse_fragment(
        &mut self,
        h: &str,
        t: &str,
        n: &str,
    ) -> Result<raikiri_dom::Document, crate::runtime::HostError> {
        self.inner.parse_fragment(h, t, n)
    }
    fn document_url(&self) -> Option<String> {
        self.inner.document_url.clone()
    }
    fn load_image(&mut self, url: &str) -> Result<(), crate::runtime::HostError> {
        self.loads.set(self.loads.get() + 1);
        self.inner.load_image(url)
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
    fn into_any(self: Box<Self>) -> Box<dyn std::any::Any> {
        self
    }
}

fn host_move_during_dirty_access(skip: usize) {
    use std::{cell::Cell, rc::Rc};
    let (mut inner, body) = host();
    let d = &mut inner.document;
    let template = d.create_detached_element("template").unwrap();
    d.append_child(body, template).unwrap();
    let contents = d.allocate_template_fragment_root(template);
    let image = d.create_detached_element("img").unwrap();
    d.append_child(body, image).unwrap();
    d.mark_in_document_flags();
    let destination = Rc::new(Cell::new(contents));
    let remaining = Rc::new(Cell::new(None));
    let loads = Rc::new(Cell::new(0));
    let mut runtime = DomRuntime::new(ScheduledHost {
        inner,
        image,
        destination: destination.clone(),
        remaining: remaining.clone(),
        loads: loads.clone(),
    })
    .unwrap();
    expose(&mut runtime, "img", image);
    let report = runtime.run_document();
    assert!(report.aborted.is_none());
    runtime
        .evaluate("globalThis.log=[];img.onload=img.onerror=e=>log.push(e.type);img.src='ok.png';")
        .unwrap();
    remaining.set(Some(skip));
    crate::runtime::node::mark_dirty(runtime.context_mut()).unwrap();
    assert_eq!(remaining.get(), None);
    runtime.run_until_idle().unwrap();
    assert_eq!(loads.get(), 0);
    ok(&mut runtime, "log.length===0");
    destination.set(body);
    remaining.set(Some(skip));
    crate::runtime::node::mark_dirty(runtime.context_mut()).unwrap();
    assert_eq!(remaining.get(), None);
    runtime.run_until_idle().unwrap();
    assert_eq!(loads.get(), 1);
    ok(&mut runtime, "log.join(',')==='load'");
}

#[test]
fn host_move_during_focus_access_is_settled_before_return() {
    host_move_during_dirty_access(1);
}
#[test]
fn host_move_during_post_sync_access_is_settled_before_return() {
    host_move_during_dirty_access(2);
}
