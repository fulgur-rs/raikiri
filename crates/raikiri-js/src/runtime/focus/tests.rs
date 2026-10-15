use super::parse_integer;
use crate::runtime::DomRuntime;
use crate::runtime::test_host::StubHost;
use crate::runtime::webidl::with_state;

fn rt() -> DomRuntime {
    let (host, _, _, _) = StubHost::page();
    DomRuntime::new(host).unwrap()
}

fn ok(rt: &mut DomRuntime, src: &str) {
    assert!(
        rt.evaluate(src).unwrap().to_boolean(),
        "expected true: {src}"
    );
}

#[test]
fn integers_parse_by_the_html_rules() {
    for (value, expected) in [
        ("0", Some(0)),
        ("  7px", Some(7)),
        ("-3", Some(-3)),
        ("+2", Some(2)),
        ("\n12", Some(12)),
        ("", None),
        ("-", None),
        ("x1", None),
        ("99999999999", None),
    ] {
        assert_eq!(parse_integer(value), expected, "{value:?}");
    }
}

#[test]
fn focus_moves_between_focusable_elements_and_fires_events() {
    let mut rt = rt();
    ok(
        &mut rt,
        "var log = [];\
         function watch(name, el) {\
           ['focus', 'blur', 'focusin', 'focusout'].forEach(function (type) {\
             el.addEventListener(type, function (e) { log.push(name + ':' + e.type + ':' + e.bubbles); });\
           });\
         }\
         var a = document.createElement('div'); a.tabIndex = 0;\
         var b = document.createElement('button');\
         document.body.appendChild(a); document.body.appendChild(b);\
         watch('a', a); watch('b', b);\
         document.activeElement === document.body && document.hasFocus()",
    );
    ok(
        &mut rt,
        "a.focus(); document.activeElement === a \
         && log.join() === 'a:focus:false,a:focusin:true'",
    );
    ok(
        &mut rt,
        "log = []; b.focus(); a.focus(); a.focus(); document.activeElement === a \
         && log.join() === 'a:blur:false,a:focusout:true,b:focus:false,b:focusin:true,\
b:blur:false,b:focusout:true,a:focus:false,a:focusin:true'",
    );
    ok(
        &mut rt,
        "log = []; b.blur(); var kept = document.activeElement === a;\
         a.blur(); kept && document.activeElement === document.body \
         && log.join() === 'a:blur:false,a:focusout:true'",
    );
}

#[test]
fn only_connected_focusable_areas_take_focus() {
    let mut rt = rt();
    ok(
        &mut rt,
        "function takes(el) { el.focus(); var got = document.activeElement === el;\
           if (got) el.blur(); return got; }\
         function make(tag, attrs, parent) { var el = document.createElement(tag);\
           for (var name in attrs) el.setAttribute(name, attrs[name]);\
           (parent || document.body).appendChild(el); return el; }\
         var details = make('details'), first = make('summary', {}, details);\
         var second = make('summary', {}, details), inert = make('div', {inert: ''});\
         takes(make('a', {href: 'x'})) && !takes(make('a'))\
         && takes(make('input')) && !takes(make('input', {type: 'HIDDEN'}))\
         && !takes(make('button', {disabled: ''}))\
         && takes(make('span', {tabindex: '-1'})) && !takes(make('span', {tabindex: 'x'}))\
         && takes(make('span', {contenteditable: ''}))\
         && !takes(make('span', {contenteditable: 'false'}))\
         && takes(make('video', {controls: ''})) && !takes(make('video'))\
         && takes(first) && !takes(second) && !takes(make('summary'))\
         && !takes(make('button', {}, inert))\
         && !takes(document.createElement('button')) && !takes(make('div'))",
    );
}

#[test]
fn tab_index_reflects_the_attribute_with_per_element_defaults() {
    let mut rt = rt();
    ok(
        &mut rt,
        "var div = document.createElement('div'), link = document.createElement('a');\
         var before = div.tabIndex === -1 && link.tabIndex === -1 \
           && document.createElement('textarea').tabIndex === 0;\
         link.setAttribute('href', '#'); div.tabIndex = 3.7;\
         before && link.tabIndex === 0 && div.getAttribute('tabindex') === '3' \
           && div.tabIndex === 3",
    );
}

#[test]
fn a_removed_focused_element_loses_focus() {
    let mut rt = rt();
    ok(
        &mut rt,
        "var b = document.createElement('button'); document.body.appendChild(b);\
         b.focus(); var had = document.activeElement === b;\
         b.remove(); had && document.activeElement === document.body",
    );
    let focused = with_state(rt.context_mut(), |s| s.host.document().focused_element()).unwrap();
    assert_eq!(focused, None);
}

#[test]
fn without_a_body_the_document_element_is_active() {
    let mut rt = rt();
    ok(
        &mut rt,
        "document.body.remove(); document.activeElement === document.documentElement",
    );
    ok(
        &mut rt,
        "document.documentElement.remove(); document.activeElement === null",
    );
}
