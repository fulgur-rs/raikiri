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
             el.addEventListener(type, function (e) {\
               if (!(e instanceof FocusEvent) || !e.isTrusted || !e.composed || e.view !== window)\
                 throw new Error('not a trusted FocusEvent');\
               var other = e.relatedTarget === a ? 'a' : e.relatedTarget === b ? 'b' : '-';\
               log.push(name + ':' + e.type + ':' + e.bubbles + ':' + other); });\
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
         && log.join() === 'a:focus:false:-,a:focusin:true:-'",
    );
    ok(
        &mut rt,
        "log = []; b.focus(); a.focus(); a.focus(); document.activeElement === a \
         && log.join() === 'a:blur:false:b,a:focusout:true:b,b:focus:false:a,b:focusin:true:a,\
b:blur:false:a,b:focusout:true:a,a:focus:false:b,a:focusin:true:b'",
    );
    ok(
        &mut rt,
        "log = []; b.blur(); var kept = document.activeElement === a;\
         a.blur(); kept && document.activeElement === document.body \
         && log.join() === 'a:blur:false:-,a:focusout:true:-'",
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
         && (function () {\
              var fieldset = make('fieldset', {disabled: ''});\
              var legend = make('legend', {}, fieldset), late = make('legend', {}, fieldset);\
              var outer = make('fieldset', {disabled: ''}), outerLegend = make('legend', {}, outer);\
              return takes(make('input', {}, legend)) && !takes(make('input', {}, late))\
                && !takes(make('select', {}, fieldset))\
                && !takes(make('button', {}, make('legend', {}, outerLegend).appendChild(\
                     make('fieldset', {disabled: ''}))))\
                && takes(make('textarea', {}, outerLegend))\
                && takes(make('button', {}, make('fieldset')));\
            })()\
         && takes(make('span', {tabindex: '-1'})) && !takes(make('span', {tabindex: 'x'}))\
         && takes(make('span', {contenteditable: ''}))\
         && !takes(make('span', {contenteditable: 'false'}))\
         && takes(make('video', {controls: ''})) && !takes(make('video'))\
         && takes(first) && !takes(second) && !takes(make('summary'))\
         && !takes(make('button', {}, inert))\
         && !takes(document.createElement('button')) && !takes(make('div'))\
         && document.createElement('summary').tabIndex === -1\
         && (function () {\
              function svg(tag, attrs) {\
                var el = document.createElementNS('http://www.w3.org/2000/svg', tag);\
                for (var name in attrs) el.setAttribute(name, attrs[name]);\
                document.body.appendChild(el); return el; }\
              var math = document.createElementNS('http://www.w3.org/1998/Math/MathML', 'mi');\
              math.setAttribute('tabindex', '0'); document.body.appendChild(math);\
              HTMLElement.prototype.focus.call(math);\
              return document.activeElement !== math\
                && svg('svg') instanceof SVGElement && !takes(svg('svg'))\
                && takes(svg('svg', {tabindex: '0'})) && takes(svg('a', {href: '#'}))\
                && takes(svg('a', {'xlink:href': '#'})) && !takes(svg('a'))\
                && !takes(svg('rect', {contenteditable: ''})); })()",
    );
}

#[test]
fn tab_index_reflects_the_attribute_with_per_element_defaults() {
    let mut rt = rt();
    ok(
        &mut rt,
        "function make(tag, attrs, parent) { var el = document.createElement(tag);\
           for (var name in attrs) el.setAttribute(name, attrs[name]);\
           if (parent) parent.appendChild(el); return el; }\
         var div = make('div'), details = make('details');\
         var svgLink = document.createElementNS('http://www.w3.org/2000/svg', 'a');\
         var svgRect = document.createElementNS('http://www.w3.org/2000/svg', 'rect');\
         var before = div.tabIndex === -1 && make('a').tabIndex === 0\
           && make('input', {type: 'hidden'}).tabIndex === 0\
           && make('textarea').tabIndex === 0 && make('video', {controls: ''}).tabIndex === -1\
           && make('summary', {}, details).tabIndex === 0\
           && make('summary', {}, details).tabIndex === -1\
           && HTMLElement.prototype.__lookupGetter__('tabIndex').call(svgLink) === 0\
           && HTMLElement.prototype.__lookupGetter__('tabIndex').call(svgRect) === -1;\
         div.tabIndex = 3.7;\
         before && div.getAttribute('tabindex') === '3' && div.tabIndex === 3",
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
    // Inserting it again does not bring focus back.
    ok(
        &mut rt,
        "document.body.appendChild(b); document.activeElement === document.body",
    );
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

#[test]
fn focus_and_ui_event_constructors_read_their_dictionaries() {
    let mut rt = rt();
    ok(
        &mut rt,
        "var el = document.createElement('div');\
         var e = new FocusEvent('focus', {relatedTarget: el, view: window, detail: 2.9, bubbles: true});\
         var w = new FocusEvent('blur', {relatedTarget: window});\
         var plain = new UIEvent('x'), bare = new FocusEvent('y', {});\
         var nullView = new UIEvent('z', {view: null});\
         function throws(f) { try { f(); return false; } catch (err) { return err instanceof TypeError; } }\
         e instanceof UIEvent && e instanceof Event && e.relatedTarget === el\
         && e.view === window && e.detail === 2 && e.bubbles && !e.isTrusted\
         && w.relatedTarget === window && bare.relatedTarget === null\
         && plain.view === null && plain.detail === 0 && !('relatedTarget' in plain)\
         && nullView.view === null && new FocusEvent('n').relatedTarget === null\
         && throws(function () { new UIEvent('x', {view: {}}); })\
         && throws(function () { new FocusEvent('x', {relatedTarget: {}}); })\
         && throws(function () { FocusEvent('x'); })",
    );
}

#[test]
fn focus_handler_attributes_run_and_body_ones_forward_to_the_window() {
    let mut rt = rt();
    ok(
        &mut rt,
        "var log = [], b = document.createElement('button'); document.body.appendChild(b);\
         b.onfocus = function (e) { log.push('focus:' + e.type); };\
         b.onblur = function (e) { log.push('blur:' + e.type); };\
         b.focus(); b.blur();\
         document.body.onfocus = function () {};\
         log.join() === 'focus:focus,blur:blur' && typeof b.onfocus === 'function'\
         && typeof b.onblur === 'function' && window.onfocus === document.body.onfocus\
         && window.onblur === null",
    );
}

#[test]
fn a_frame_takes_focus_from_an_element_that_stopped_being_focusable() {
    let mut rt = rt();
    ok(
        &mut rt,
        "var log = [], b = document.createElement('button'); document.body.appendChild(b);\
         b.addEventListener('blur', function () { log.push('blur'); });\
         b.addEventListener('focusout', function () { log.push('focusout'); });\
         b.focus(); b.setAttribute('disabled', '');\
         var kept = document.activeElement === b;\
         requestAnimationFrame(function () {});\
         kept",
    );
    rt.run_until_idle().unwrap();
    ok(
        &mut rt,
        "document.activeElement === document.body && log.join() === 'blur,focusout'",
    );
}
