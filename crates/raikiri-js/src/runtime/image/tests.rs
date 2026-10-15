use boa_engine::js_string;
use boa_engine::property::Attribute;

use crate::runtime::DomRuntime;
use crate::runtime::interfaces::wrap;
use crate::runtime::test_host::StubHost;
use crate::runtime::webidl::with_state;

const PAGE: &str = "https://example.test/dir/page.html";
const LOADS: &str = "https://example.test/dir/ok.png";

fn host() -> (StubHost, usize) {
    let (mut host, _, _, body) = StubHost::page();
    host.document_url = Some(PAGE.to_owned());
    host.images.insert(LOADS.to_owned());
    (host, body)
}

/// A runtime over an empty page whose host loads only [`LOADS`].
fn rt() -> DomRuntime {
    DomRuntime::new(host().0).unwrap()
}

fn ok(rt: &mut DomRuntime, src: &str) {
    assert!(
        rt.evaluate(src).unwrap().to_boolean(),
        "expected true: {src}"
    );
}

/// Run every queued task, then check `src`.
fn ok_after_tasks(rt: &mut DomRuntime, src: &str) {
    rt.run_until_idle().unwrap();
    ok(rt, src);
}

#[test]
fn img_elements_use_the_html_image_element_interface() {
    let mut rt = rt();
    ok(
        &mut rt,
        "var img = document.createElement('img');\
         img instanceof HTMLImageElement && img instanceof HTMLElement \
         && !(document.createElement('div') instanceof HTMLImageElement)",
    );
    ok(
        &mut rt,
        "!(document.createElementNS('http://www.w3.org/2000/svg', 'img') \
           instanceof HTMLImageElement)",
    );
    ok(
        &mut rt,
        "var threw = false;\
         try { Object.getOwnPropertyDescriptor(HTMLImageElement.prototype, 'src')\
               .get.call(document.createElement('div')); }\
         catch (e) { threw = e instanceof TypeError; }\
         threw",
    );
}

#[test]
fn src_reflects_the_resolved_attribute() {
    let mut rt = rt();
    ok(
        &mut rt,
        "var img = document.createElement('img'); img.src === ''",
    );
    ok(
        &mut rt,
        "img.src = 'ok.png';\
         img.getAttribute('src') === 'ok.png' \
         && img.src === 'https://example.test/dir/ok.png'",
    );
    ok(
        &mut rt,
        "img.src = '../a b.png?q#f'; var r1 = img.src;\
         img.src = 'data:image/png;base64,AA=='; var r2 = img.src;\
         img.src = 'http://['; \
         r1 === 'https://example.test/a%20b.png?q#f' \
         && r2 === 'data:image/png;base64,AA==' && img.src === 'http://['",
    );
}

#[test]
fn a_source_that_does_not_parse_fires_error() {
    let mut rt = rt();
    ok(
        &mut rt,
        "var log = []; var img = document.createElement('img');\
         img.onload = img.onerror = function (e) { log.push(e.type); };\
         img.src = 'http://['; true",
    );
    ok_after_tasks(&mut rt, "log.join() === 'error'");
}

#[test]
fn a_loading_image_fires_load_and_is_complete_afterwards() {
    let mut rt = rt();
    ok(
        &mut rt,
        "var log = []; var img = document.createElement('img');\
         img.onload = function (e) { log.push('load:' + e.isTrusted + ':' + e.bubbles); };\
         img.onerror = function () { log.push('error'); };\
         img.complete === true",
    );
    ok(
        &mut rt,
        "img.src = 'ok.png'; img.complete === false && log.length === 0",
    );
    ok_after_tasks(&mut rt, "img.complete && log.join() === 'load:true:false'");
}

#[test]
fn a_failing_or_empty_source_fires_error() {
    let mut rt = rt();
    ok(
        &mut rt,
        "var log = [];\
         function watch(img, name) {\
             img.onload = function () { log.push(name + ':load'); };\
             img.onerror = function () { log.push(name + ':error'); };\
             return img;\
         }\
         watch(document.createElement('img'), 'missing').src = 'missing.png';\
         watch(document.createElement('img'), 'empty').src = '';\
         true",
    );
    ok_after_tasks(&mut rt, "log.join() === 'missing:error,empty:error'");
}

#[test]
fn only_the_latest_source_fires() {
    let mut rt = rt();
    ok(
        &mut rt,
        "var log = []; var img = document.createElement('img');\
         img.onload = function () { log.push('load:' + img.getAttribute('src')); };\
         img.onerror = function () { log.push('error:' + img.getAttribute('src')); };\
         img.src = 'missing.png'; img.setAttribute('src', 'ok.png'); true",
    );
    ok_after_tasks(&mut rt, "log.join() === 'load:ok.png'");
}

#[test]
fn removing_src_cancels_the_pending_request() {
    let mut rt = rt();
    ok(
        &mut rt,
        "var log = []; var img = document.createElement('img');\
         img.onload = img.onerror = function (e) { log.push(e.type); };\
         img.src = 'ok.png'; img.removeAttribute('src');\
         img.complete",
    );
    ok_after_tasks(&mut rt, "log.length === 0");
}

#[test]
fn other_attributes_and_elements_start_no_request() {
    let mut rt = rt();
    ok(
        &mut rt,
        "var log = [];\
         var img = document.createElement('img');\
         img.onload = img.onerror = function (e) { log.push('img:' + e.type); };\
         img.setAttribute('alt', 'x');\
         var div = document.createElement('div');\
         div.onload = div.onerror = function (e) { log.push('div:' + e.type); };\
         div.setAttribute('src', 'ok.png');\
         true",
    );
    ok_after_tasks(&mut rt, "log.length === 0");
}

#[test]
fn image_constructor_makes_a_sized_img() {
    let mut rt = rt();
    ok(
        &mut rt,
        "var a = new Image(); var b = new Image(3, 4.7); var c = new Image(undefined, 5);\
         a instanceof HTMLImageElement && a.localName === 'img' && a.parentNode === null \
         && !a.hasAttribute('width') && !a.hasAttribute('height') \
         && b.getAttribute('width') === '3' && b.getAttribute('height') === '4' \
         && !c.hasAttribute('width') && c.getAttribute('height') === '5' \
         && Image.prototype === HTMLImageElement.prototype && Image.length === 0",
    );
    ok(
        &mut rt,
        "var threw = false; try { Image(); } catch (e) { threw = e instanceof TypeError; }\
         threw",
    );
    ok(
        &mut rt,
        "var log = []; var d = new Image();\
         d.onload = function () { log.push('load'); }; d.src = 'ok.png'; true",
    );
    ok_after_tasks(&mut rt, "log.join() === 'load'");
}

/// The requests a fragment parse starts: every `<img>` below the root, but
/// not the root itself.
#[test]
fn images_below_a_root_start_their_requests() {
    let mut rt = rt();
    ok(
        &mut rt,
        "var log = []; var outer = document.createElement('img');\
         var p = outer.appendChild(document.createElement('p'));\
         var inner = p.appendChild(document.createElement('img'));\
         outer.setAttribute('alt', '');\
         true",
    );
    let outer = with_state(rt.context_mut(), |s| {
        let document = s.host.document_mut();
        let images: Vec<usize> = (0..document.node_count())
            .filter(|&index| super::is_html_image(document, index))
            .collect();
        for &image in &images {
            document
                .set_element_attribute(image, "src", "ok.png")
                .unwrap();
        }
        images[0]
    })
    .unwrap();
    ok(
        &mut rt,
        "outer.onload = function () { log.push('outer'); };\
         inner.onload = function () { log.push('inner'); };\
         true",
    );
    super::update_images_below(rt.context_mut(), outer).unwrap();
    ok_after_tasks(&mut rt, "log.join() === 'inner'");
}

#[test]
fn parsed_images_fire_before_the_window_load_event() {
    let (mut host, body) = host();
    let document = &mut host.document;
    for src in ["ok.png", "missing.png"] {
        let img = document.create_detached_element("img").unwrap();
        document.set_element_attribute(img, "src", src).unwrap();
        document.append_child(body, img).unwrap();
    }
    let script = document.create_detached_element("script").unwrap();
    document.append_text(
        script,
        "var log = [];\
         document.querySelectorAll('img').forEach(function (img) {\
             img.onload = img.onerror = function (e) {\
                 log.push(img.getAttribute('src') + ':' + e.type);\
             };\
         });\
         window.addEventListener('load', function () { log.push('window'); });",
    );
    document.append_child(body, script).unwrap();
    document.mark_in_document_flags();
    let mut rt = DomRuntime::new(host).unwrap();
    let report = rt.run_document();
    assert!(report.aborted.is_none(), "{report:?}");
    ok(
        &mut rt,
        "log.join() === 'ok.png:load,missing.png:error,window'",
    );
}

#[test]
fn cloned_images_start_their_own_requests() {
    let mut rt = rt();
    ok(
        &mut rt,
        "var log = []; var img = document.createElement('img'); img.src = 'ok.png';\
         var div = document.createElement('div'); div.appendChild(img.cloneNode());\
         var copy = img.cloneNode(); var deep = document.importNode(div, true);\
         var shallow = div.cloneNode(false);\
         copy.onload = function () { log.push('copy'); };\
         deep.firstChild.onload = function () { log.push('deep'); };\
         true",
    );
    ok_after_tasks(&mut rt, "log.join() === 'copy,deep'");
}

#[test]
fn template_content_images_never_load() {
    let (mut host, body) = host();
    let document = &mut host.document;
    let template = document.create_detached_element("template").unwrap();
    document.append_child(body, template).unwrap();
    let contents = document.allocate_template_fragment_root(template);
    let img = document.create_detached_element("img").unwrap();
    document.append_child(contents, img).unwrap();
    document.mark_in_document_flags();
    let mut rt = DomRuntime::new(host).unwrap();
    let object = wrap(rt.context_mut(), img).unwrap();
    rt.context_mut()
        .register_global_property(js_string!("img"), object, Attribute::all())
        .unwrap();
    ok(
        &mut rt,
        "var log = [];\
         img.onload = img.onerror = function (e) { log.push(e.type); };\
         img.src = 'ok.png'; img.complete",
    );
    ok_after_tasks(&mut rt, "log.length === 0");
}

#[test]
fn an_empty_source_is_complete_while_its_error_is_queued() {
    let mut rt = rt();
    ok(
        &mut rt,
        "var log = []; var img = document.createElement('img');\
         img.onerror = function () { log.push('error'); };\
         img.src = ''; img.complete && log.length === 0",
    );
    ok_after_tasks(&mut rt, "log.join() === 'error'");
}

#[test]
fn a_base_element_sets_the_url_images_resolve_against() {
    let (mut host, _) = host();
    host.document.set_base_url(Some(
        url::Url::parse("https://example.test/dir/sub/").unwrap(),
    ));
    host.images
        .insert("https://example.test/dir/sub/ok.png".to_owned());
    let mut rt = DomRuntime::new(host).unwrap();
    ok(
        &mut rt,
        "var log = []; var img = document.createElement('img');\
         img.onload = function () { log.push('load'); }; img.src = 'ok.png';\
         img.src === 'https://example.test/dir/sub/ok.png'",
    );
    ok_after_tasks(&mut rt, "log.join() === 'load'");
}

/// A host that keeps [`DocumentHost::load_image`]'s default loads nothing,
/// so every request fires `error`.
#[test]
fn the_default_host_loads_no_images() {
    use std::any::Any;

    use crate::runtime::host::{BoxGeometry, DocumentHost, HostError};

    struct NoImages(StubHost);
    impl DocumentHost for NoImages {
        fn document(&self) -> &raikiri_dom::Document {
            self.0.document()
        }
        fn document_mut(&mut self) -> &mut raikiri_dom::Document {
            self.0.document_mut()
        }
        fn flush(&mut self) -> Result<(), HostError> {
            self.0.flush()
        }
        fn box_geometry(&mut self, node: usize) -> Result<Option<BoxGeometry>, HostError> {
            self.0.box_geometry(node)
        }
        fn computed_value(
            &mut self,
            node: usize,
            property: &str,
        ) -> Result<Option<String>, HostError> {
            self.0.computed_value(node, property)
        }
        fn parse_fragment(
            &mut self,
            tag: &str,
            ns: &str,
            markup: &str,
        ) -> Result<raikiri_dom::Document, HostError> {
            self.0.parse_fragment(tag, ns, markup)
        }
        fn document_url(&self) -> Option<String> {
            self.0.document_url()
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

    let mut rt = DomRuntime::new(NoImages(host().0)).unwrap();
    ok(
        &mut rt,
        "var log = []; var img = document.createElement('img');\
         img.onload = img.onerror = function (e) { log.push(e.type); };\
         img.src = 'ok.png'; document.body.innerHTML = 'x'; true",
    );
    ok_after_tasks(&mut rt, "log.join() === 'error'");
    assert!(rt.try_into_host::<NoImages>().is_ok());
}
